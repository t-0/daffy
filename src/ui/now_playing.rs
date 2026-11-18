use super::eq::{draw_eq_summary, draw_loudness};
use super::video::draw_video;
use super::{ACCENT, DIM, fmt_dur};
use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Wrap};
fn state_title(app: &App) -> &'static str {
    match (app.playing_video().is_some(), app.is_paused()) {
        (false, _) => " ⏹ Nothing Playing ",
        (true, true) => " ⏸ Paused ",
        (true, false) => " ▶ Playing ",
    }
}
fn progress(app: &App) -> Gauge<'static> {
    let total = app.playing_video().map(|t| t.duration_secs).unwrap_or(0.0);
    let pos = app.position();
    let ratio = if total > 0.0 {
        (pos / total).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Gauge::default()
        .gauge_style(Style::default().fg(ACCENT).bg(Color::Rgb(40, 40, 40)))
        .ratio(ratio)
        .label(format!("{} / {}", fmt_dur(pos), fmt_dur(total)))
}
pub(super) fn draw_now_playing(f: &mut Frame, area: Rect, app: &mut App, expanded: bool) {
    let mut title = vec![Span::styled(
        state_title(app),
        Style::default().fg(ACCENT).bold(),
    )];
    if expanded && let Some(t) = app.playing_video() {
        title.push(Span::styled(
            format!("{}  ", t.title),
            Style::default().fg(Color::White).bold(),
        ));
        title.push(Span::styled(
            format!("{}  ", t.folder),
            Style::default().fg(DIM),
        ));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM))
        .title(Line::from(title));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if expanded {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .margin(1)
            .split(inner);
        draw_video(f, rows[0], app);
        if app.playing_video().is_some() {
            f.render_widget(progress(app), rows[1]);
        } else {
            f.render_widget(
                Paragraph::new("Press x to go back to the list, or space to play the selection.")
                    .style(Style::default().fg(DIM)),
                rows[1],
            );
        }
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .margin(1)
        .split(inner);
    let Some(t) = app.playing_video() else {
        f.render_widget(
            Paragraph::new("Select with ↑/↓, press enter to play.").style(Style::default().fg(DIM)),
            rows[0],
        );
        if let Some(err) = &app.audio_error {
            let msg = Rect {
                x: rows[2].x,
                y: rows[2].y,
                width: rows[2].width,
                height: 4,
            };
            f.render_widget(
                Paragraph::new(format!("Audio output unavailable: {err}"))
                    .style(Style::default().fg(Color::Red))
                    .wrap(Wrap { trim: true }),
                msg,
            );
        }
        draw_speed(f, rows[10], rows[11], app);
        draw_loudness(f, rows[13], app);
        draw_eq_summary(f, rows[14], app);
        draw_video(f, rows[15], app);
        return;
    };
    let field = |label: &str, value: String| {
        Line::from(vec![
            Span::styled(format!("{label:<8}"), Style::default().fg(DIM)),
            Span::raw(value),
        ])
    };
    let picture = if t.has_video() {
        let fps = if t.fps > 0.0 {
            let n = format!("{:.3}", t.fps);
            format!("  {} fps", n.trim_end_matches('0').trim_end_matches('.'))
        } else {
            String::new()
        };
        format!("{}×{}  {}{fps}", t.width, t.height, t.vcodec)
    } else {
        "none".to_string()
    };
    let sound = match &t.acodec {
        Some(c) => {
            let ch = match t.channels {
                0 => String::new(),
                1 => "  mono".to_string(),
                2 => "  stereo".to_string(),
                n => format!("  {n} ch"),
            };
            let sr = if t.sample_rate > 0 {
                format!("  {} kHz", t.sample_rate as f64 / 1000.0)
            } else {
                String::new()
            };
            format!("{c}{ch}{sr}")
        }
        None => "none".to_string(),
    };
    let path = app.config.library_path(&t.path);
    f.render_widget(
        Paragraph::new(field("Title", t.title.clone())).style(Style::default().bold()),
        rows[0],
    );
    f.render_widget(Paragraph::new(field("Folder", t.folder.clone())), rows[1]);
    f.render_widget(
        Paragraph::new(field(
            "Year",
            t.year.map(|y| y.to_string()).unwrap_or_else(|| "—".into()),
        )),
        rows[2],
    );
    f.render_widget(
        Paragraph::new(field("Length", fmt_dur(t.duration_secs))),
        rows[3],
    );
    f.render_widget(Paragraph::new(field("Video", picture)), rows[4]);
    f.render_widget(Paragraph::new(field("Audio", sound)), rows[5]);
    f.render_widget(
        Paragraph::new(field("Type", t.filetype.to_uppercase())),
        rows[6],
    );
    f.render_widget(progress(app), rows[8]);
    draw_speed(f, rows[10], rows[11], app);
    draw_loudness(f, rows[13], app);
    draw_eq_summary(f, rows[14], app);
    draw_video(f, rows[15], app);
    f.render_widget(
        Paragraph::new(path).style(Style::default().fg(DIM)),
        rows[rows.len() - 1],
    );
}
pub(super) fn draw_speed(f: &mut Frame, label_area: Rect, slider_area: Rect, app: &App) {
    let pct = app.speed_pct;
    let label = Line::from(vec![
        Span::styled("Speed   ", Style::default().fg(DIM)),
        Span::styled(
            format!("{}%", 100 + pct),
            Style::default().fg(ACCENT).bold(),
        ),
        Span::styled(
            format!("  ({:.2}x)   +/- 1%, 0 resets to 100%", app.speed()),
            Style::default().fg(DIM),
        ),
    ]);
    f.render_widget(Paragraph::new(label), label_area);
    let w = slider_area.width as usize;
    if w < 12 {
        return;
    }
    let track_w = w - 10;
    let knob = ((pct + 50) as f64 / 150.0 * (track_w - 1) as f64).round() as usize;
    let center = (50.0 / 150.0 * (track_w - 1) as f64).round() as usize;
    let mut spans = vec![Span::styled(" 50% ", Style::default().fg(DIM))];
    for i in 0..track_w {
        if i == knob {
            spans.push(Span::styled("●", Style::default().fg(ACCENT).bold()));
        } else if i == center {
            spans.push(Span::styled("┼", Style::default().fg(Color::Gray)));
        } else {
            spans.push(Span::styled("─", Style::default().fg(DIM)));
        }
    }
    spans.push(Span::styled(" 200%", Style::default().fg(DIM)));
    f.render_widget(Paragraph::new(Line::from(spans)), slider_area);
}
