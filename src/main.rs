mod app;
mod config;
mod fx;
mod index;
mod output;
mod stderr;
mod ui;
mod video;
use anyhow::Result;
use app::{App, Placement};
use output::{Output, OutputWriter};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{Clear, ClearType, EnterAlternateScreen, enable_raw_mode};
use std::process::Command;
use std::time::{Duration, Instant};
const SEEK_SECS: f64 = 10.0;
const LONG_SEEK_SECS: f64 = 60.0;
/// How often the whole screen is rewritten cell by cell, so anything a
/// library or child process managed to print over the UI gets painted out.
const FULL_REPAINT: Duration = Duration::from_secs(5);
type Term = Terminal<CrosstermBackend<OutputWriter>>;
fn usage() -> ! {
    eprintln!(
        "usage: daffy [--version | --detect | --probe <video file>]\n\
         \n\
         --detect  print how video would be drawn in this terminal\n\
         --probe   show what ffprobe and the decoders make of a file"
    );
    std::process::exit(2);
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("daffy version {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--help" | "-h") => usage(),
        _ => {}
    }
    let config = config::load()?;
    match args.first().map(String::as_str) {
        Some("--probe") => {
            let Some(path) = args.get(1) else { usage() };
            match probe_report(&config, std::path::Path::new(path)) {
                Ok(report) => print!("{report}"),
                Err(e) => {
                    eprintln!("probe failed: {e:#}");
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        Some("--detect") => {
            // Not a terminal (piped output): skip the queries.
            let caps = if enable_raw_mode().is_ok() {
                let caps = video::term::query_terminal();
                ratatui::crossterm::terminal::disable_raw_mode()?;
                caps
            } else {
                video::term::Caps::default()
            };
            let env = video::term::Env::current();
            let (renderer, warn) = video::term::detect(config.renderer.as_deref(), &env, &caps);
            println!("terminal replies: {caps:?}");
            println!(
                "environment: TERM={} TERM_PROGRAM={} COLORTERM={} tmux={}",
                env.term, env.term_program, env.colorterm, env.tmux
            );
            println!(
                "video.renderer: {}",
                config.renderer.as_deref().unwrap_or("auto")
            );
            println!("chosen: {} ({renderer:?})", renderer.label());
            if let Some(w) = warn {
                println!("note: {w}");
            }
            return Ok(());
        }
        Some(other) if other.starts_with('-') => usage(),
        _ => {}
    }
    // Anything written to stderr while the TUI owns the screen (native audio
    // libraries, ffmpeg) would land on top of the UI, so send it to a log
    // file until the terminal is handed back.
    let _quiet = stderr::Redirect::to_file(&config::log_path());
    let (output, writer, thread) = Output::start();
    let mut terminal = init_terminal(writer)?;
    // Raw mode is on now, so the terminal's answers arrive as bytes rather
    // than echoed text.
    let caps = video::term::query_terminal();
    let (renderer, warn) = video::term::detect(
        config.renderer.as_deref(),
        &video::term::Env::current(),
        &caps,
    );
    let mut app = App::new(config, renderer);
    if let Some(w) = warn {
        app.set_status(w);
    }
    let result = run(&mut terminal, &output, app);
    if let Some(seq) = renderer.clear_sequence() {
        output.raw(seq.to_vec());
    }
    output.sync();
    ratatui::restore();
    drop(terminal);
    drop(output);
    let _ = thread.join();
    result
}
/// What `ratatui::init` does, but drawing through the writer thread.
fn init_terminal(writer: OutputWriter) -> Result<Term> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        hook(info);
    }));
    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(writer))?)
}
/// `--probe`: what the index would record, then a short decode of each
/// stream through the same ffmpeg commands playback uses.
fn probe_report(config: &config::Config, path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let mut out = String::new();
    let meta = std::fs::metadata(path)?;
    out.push_str(&format!(
        "file: {} ({} bytes)\n",
        path.display(),
        meta.len()
    ));
    out.push_str(&format!(
        "ffprobe: {}\nffmpeg: {}\n",
        config.ffprobe.display(),
        config.ffmpeg.display()
    ));
    let p = video::probe::probe(&config.ffprobe, path)?;
    out.push_str(&format!("{p:#?}\n"));
    let secs = 2.0;
    let mut v = Command::new(&config.ffmpeg)
        .args(["-nostdin", "-v", "error", "-t", &format!("{secs}"), "-i"])
        .arg(path)
        .args(["-an", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    let mut bytes = 0usize;
    let mut buf = vec![0u8; 1 << 16];
    if let Some(mut so) = v.stdout.take() {
        while let Ok(n) = so.read(&mut buf) {
            if n == 0 {
                break;
            }
            bytes += n;
        }
    }
    let st = v.wait()?;
    if p.width > 0 {
        let frame = (p.width * p.height * 3) as usize;
        out.push_str(&format!(
            "video decode: {} frames in the first {secs} s ({st})\n",
            bytes / frame.max(1)
        ));
    } else {
        out.push_str("video decode: no video stream\n");
    }
    if p.acodec.is_some() {
        let g = video::measure_gain(&config.ffmpeg, path)?;
        out.push_str(&format!(
            "normalise: {g:+.1} dB would bring it to {} dBFS\n",
            fx::NORMALIZE_TARGET_DB
        ));
    } else {
        out.push_str("audio decode: no audio stream\n");
    }
    Ok(out)
}
/// What the last frame put on screen through a graphics protocol.
#[derive(Default)]
struct ImageState {
    last: Option<Placement>,
    /// The frame the terminal was last given, if it went out.
    sent_frame: Option<u64>,
}
impl ImageState {
    /// Whether the last picture is still on screen somewhere `now` does
    /// not cover exactly: it moved, changed size or went away.
    fn stale(&self, now: Option<Placement>) -> bool {
        match (self.last, now) {
            (Some(l), Some(p)) => (l.x, l.y, l.cols, l.rows) != (p.x, p.y, p.cols, p.rows),
            (Some(_), None) => true,
            (None, _) => false,
        }
    }
}
/// Keys are read in bursts; a terminal reply (APC, OSC or DCS: `ESC _`,
/// `ESC ]`, `ESC P` ... `ESC \\`) would otherwise arrive as Alt+_ followed by
/// its text as ordinary keystrokes.
#[derive(Default)]
struct ReplyFilter {
    swallowing: Option<Instant>,
}
impl ReplyFilter {
    /// True when the event is part of a terminal reply and must be ignored.
    fn swallow(&mut self, key: &ratatui::crossterm::event::KeyEvent) -> bool {
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if let Some(since) = self.swallowing {
            let ends = matches!(key.code, KeyCode::Char('\\') if alt)
                || matches!(key.code, KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL));
            if ends || since.elapsed() > Duration::from_millis(250) {
                self.swallowing = None;
                return ends;
            }
            return true;
        }
        if alt && matches!(key.code, KeyCode::Char('_' | ']' | 'P' | 'X' | '^')) {
            self.swallowing = Some(Instant::now());
            return true;
        }
        false
    }
}
fn run(terminal: &mut Term, output: &Output, mut app: App) -> Result<()> {
    let mut last_tick = Instant::now();
    let mut last_repaint = Instant::now();
    let mut image = ImageState::default();
    let mut replies = ReplyFilter::default();
    // `DAFFY_KEYLOG=1` writes every key event to the log, to see what a
    // terminal is sending when something seems to press keys by itself.
    let keylog = std::env::var_os("DAFFY_KEYLOG").is_some();
    loop {
        app.update_video();
        if let Some(old) = app.renderer_was.take() {
            if let Some(seq) = old.clear_sequence() {
                output.raw(seq.to_vec());
            }
            image.sent_frame = None;
        }
        let repaint =
            last_repaint.elapsed() >= FULL_REPAINT || std::mem::take(&mut app.force_repaint);
        let drawn = terminal.draw(|f| ui::draw(f, &mut app))?;
        let stale = image.stale(app.image_out);
        let snapshot = (repaint || stale).then(|| drawn.buffer.clone());
        if let Some(buf) = snapshot {
            let backend = terminal.backend_mut();
            if stale {
                // Some terminals (Konsole, WezTerm, kitty placements) keep a
                // picture on a layer of its own that spaces do not erase, so
                // one that moved or went away is deleted and the display
                // erased before the cells go back, all in one write.
                if let Some(seq) = app.renderer.clear_sequence() {
                    std::io::Write::write_all(backend, seq)?;
                }
                execute!(backend, Clear(ClearType::All))?;
            }
            // ratatui only emits cells that changed since the last frame, so
            // stray text in otherwise static areas would survive forever.
            // Rewriting every cell (no clear, so no flicker) paints it out.
            backend.draw(buf.content.iter().enumerate().map(|(i, c)| {
                let (x, y) = buf.pos_of(i);
                (x, y, c)
            }))?;
            Backend::flush(backend)?;
            last_repaint = Instant::now();
        }
        flush_image(output, &mut app, &mut image, repaint || stale);
        let tick = if app.playing_active() {
            Duration::from_millis(33)
        } else {
            Duration::from_millis(200)
        };
        let mut wait = tick.saturating_sub(last_tick.elapsed());
        while event::poll(wait)? {
            match event::read()? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    let swallowed = replies.swallow(&key);
                    if keylog {
                        eprintln!(
                            "key {:?} {:?}{}",
                            key.code,
                            key.modifiers,
                            if swallowed {
                                " (terminal reply, ignored)"
                            } else {
                                ""
                            }
                        );
                    }
                    if !swallowed {
                        handle_key(&mut app, key.code, key.modifiers);
                    }
                }
                Event::Resize(_, _) => app.force_repaint = true,
                _ => {}
            }
            wait = Duration::ZERO;
            if app.should_quit {
                break;
            }
        }
        if let Some((name, cmd)) = app.pending_editor.take() {
            run_editor(&mut app, output, &name, cmd)?;
            // The alternate screen is blank again but ratatui's last frame is
            // not, so make the next frame rewrite every cell.
            last_repaint = Instant::now() - FULL_REPAINT;
            image.last = None;
        }
        if last_tick.elapsed() >= tick {
            app.tick()?;
            last_tick = Instant::now();
        }
        app.flush_play();
        if app.should_quit {
            return Ok(());
        }
    }
}
/// Hands the picture for this frame to the writer thread, after the cells,
/// when it is new or the screen was rewritten (which the main loop does
/// when the picture moves or goes away). A frame the terminal has no room
/// for yet is simply skipped.
fn flush_image(output: &Output, app: &mut App, image: &mut ImageState, repainted: bool) {
    let now = app.image_out;
    match now {
        Some(p) => {
            let fresh = image.sent_frame != Some(p.frame_id);
            if (fresh || repainted)
                && let Some(frame) = app.playback.as_ref().and_then(|pb| pb.frame())
            {
                let sent = output.image(app.renderer, frame.clone(), p.x, p.y, p.cols, p.rows);
                image.sent_frame = sent.then_some(p.frame_id);
            }
        }
        None => image.sent_frame = None,
    }
    image.last = now;
}
/// Runs a terminal video editor in the foreground: playback is paused, the
/// screen, raw mode and stderr are handed back to the terminal for the
/// child's lifetime, then everything is put back and playback resumes.
fn run_editor(app: &mut App, output: &Output, name: &str, mut cmd: Command) -> Result<()> {
    let was_playing = app.playing_active();
    if was_playing && let Some(p) = &mut app.playback {
        p.pause();
    }
    output.sync();
    ratatui::restore();
    let result = {
        let _stderr = stderr::suspend();
        cmd.status()
    };
    enable_raw_mode()?;
    // Not `Terminal::clear`: that asks the terminal where the cursor is and
    // waits for the answer, which an editor's leftover input can garble.
    execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        Clear(ClearType::All)
    )?;
    if was_playing && let Some(p) = &mut app.playback {
        p.resume();
    }
    app.editor_finished(name, result);
    Ok(())
}
fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    if app.search_input.is_some() {
        handle_search_key(app, code, mods);
        return;
    }
    if app.eq_open {
        handle_eq_key(app, code);
        return;
    }
    if app.sort_open {
        match code {
            KeyCode::Esc | KeyCode::Char('o') | KeyCode::Char('q') => app.sort_open = false,
            KeyCode::Enter | KeyCode::Char(' ') => app.apply_sort(),
            KeyCode::Up | KeyCode::Char('k') => app.sort_move(-1),
            KeyCode::Down | KeyCode::Char('j') => app.sort_move(1),
            KeyCode::Char(c) => {
                if let Some(row) = app::SortKey::ALL.iter().position(|k| k.shortcut() == c) {
                    app.sort_row = row;
                    app.apply_sort();
                }
            }
            _ => {}
        }
        return;
    }
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.should_quit = true,
        KeyCode::Char('v') => app.section = app::Section::Videos,
        KeyCode::Char('d') => app.section = app::Section::Folders,
        KeyCode::Char('c') => app.section = app::Section::Collections,
        KeyCode::Char(' ') => app.toggle_play(),
        KeyCode::Char('n') => app.step_video(1),
        KeyCode::Char('p') => app.step_video(-1),
        KeyCode::Up | KeyCode::Char('k') => app.move_cursor(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_cursor(1),
        KeyCode::Home => app.move_cursor(i64::MIN / 2),
        KeyCode::End => app.move_cursor(i64::MAX / 2),
        KeyCode::Left | KeyCode::Char(',') => app.seek(-SEEK_SECS),
        KeyCode::Right | KeyCode::Char('.') => app.seek(SEEK_SECS),
        KeyCode::Char('[') => app.seek(-LONG_SEEK_SECS),
        KeyCode::Char(']') => app.seek(LONG_SEEK_SECS),
        KeyCode::Char('=') | KeyCode::Char('+') => app.adjust_speed(1),
        KeyCode::Char('-') | KeyCode::Char('_') => app.adjust_speed(-1),
        KeyCode::PageUp => app.move_cursor(-(app.page_size as i64)),
        KeyCode::PageDown => app.move_cursor(app.page_size as i64),
        KeyCode::Char('0') => app.reset_speed(),
        KeyCode::Enter => app.activate_cursor(),
        KeyCode::Char('/') => app.open_search(),
        KeyCode::Char('x') => app.toggle_expand(),
        KeyCode::Char('r') => app.cycle_renderer(),
        KeyCode::Char('e') => app.eq_open = true,
        KeyCode::Char('o') => app.open_sort(),
        KeyCode::Char('l') => app.toggle_loudness(),
        KeyCode::Char('z') => app.toggle_shuffle(),
        KeyCode::Char('f') => app.reveal_in_finder(),
        KeyCode::Char('E') => app.edit_in_editor(),
        KeyCode::Char('i') => app.start_index(false),
        KeyCode::Char('I') => app.start_index(true),
        _ => {}
    }
}
fn handle_search_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let Some(buf) = app.search_input.as_mut() else {
        return;
    };
    match code {
        KeyCode::Esc => app.cancel_search(),
        KeyCode::Enter => app.submit_search(),
        KeyCode::Backspace => {
            buf.pop();
        }
        KeyCode::Char('u') if mods.contains(KeyModifiers::CONTROL) => buf.clear(),
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.cancel_search(),
        KeyCode::Char(c) => buf.push(c),
        _ => {}
    }
}
fn handle_eq_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('e') | KeyCode::Char('q') => app.close_eq(),
        KeyCode::Up | KeyCode::Char('k') => app.eq_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.eq_move(1),
        KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') | KeyCode::Char('_') => {
            app.eq_adjust(-1)
        }
        KeyCode::Right
        | KeyCode::Char('l')
        | KeyCode::Char('=')
        | KeyCode::Char('+')
        | KeyCode::Char(' ') => app.eq_adjust(1),
        KeyCode::PageUp => app.eq_adjust(3),
        KeyCode::PageDown => app.eq_adjust(-3),
        KeyCode::Char('0') => app.eq_reset_row(),
        _ => {}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEvent;
    fn key(c: char, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), mods)
    }
    #[test]
    fn terminal_replies_are_swallowed_but_ordinary_keys_pass() {
        let mut f = ReplyFilter::default();
        assert!(!f.swallow(&key('i', KeyModifiers::NONE)));
        // ESC _ G i = 3 1 ; O K ESC \ as crossterm delivers it.
        assert!(f.swallow(&key('_', KeyModifiers::ALT)));
        for c in "Gi=31;OK".chars() {
            assert!(f.swallow(&key(c, KeyModifiers::NONE)), "{c}");
        }
        assert!(f.swallow(&key('\\', KeyModifiers::ALT)));
        assert!(!f.swallow(&key('i', KeyModifiers::NONE)), "back to normal");
        // An OSC ending in BEL.
        assert!(f.swallow(&key(']', KeyModifiers::ALT)));
        assert!(f.swallow(&key('1', KeyModifiers::NONE)));
        assert!(f.swallow(&key('g', KeyModifiers::CONTROL)));
        assert!(!f.swallow(&key('q', KeyModifiers::NONE)));
        // Alt+other letters are not replies.
        assert!(!f.swallow(&key('x', KeyModifiers::ALT)));
    }
}
