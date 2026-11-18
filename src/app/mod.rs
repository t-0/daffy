mod collection;
mod eq_row;
mod folder;
mod normalize_state;
mod placement;
mod section;
mod settings;
mod sort_key;
#[cfg(test)]
mod tests;
mod view_state;
use crate::config::{Config, EditorMode, display_path};
use crate::fx::{
    EQ_RANGE_DB, EqMode, FxParams, PREAMP_MAX_DB, PREAMP_MIN_DB, StereoMode, WIDTH_MAX,
};
use crate::index::{self, IndexMsg, Video};
use crate::video::{Media, Playback, Protocol, Renderer, Target};
use anyhow::Result;
pub use collection::Collection;
pub use eq_row::EqRow;
pub use folder::Folder;
pub use normalize_state::NormalizeState;
pub use placement::Placement;
pub use section::Section;
use settings::write_settings;
pub use settings::{Settings, load_settings};
pub use sort_key::SortKey;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use view_state::ViewState;
fn reveal_command(path: &std::path::Path) -> Command {
    let mut c = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg("-R").arg(path);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("explorer");
        c.arg(format!("/select,{}", path.display()));
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(path));
        c
    };
    // The child would otherwise inherit our terminal and could print over the UI.
    c.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c
}
/// Builds the editor invocation: `words` is the configured command split
/// into program and arguments. Every `{file}` in the arguments is replaced
/// by the video path; without one the path goes last. On macOS a `.app`
/// bundle is started through `open -a`, which is how bundles are launched;
/// `open` returns as soon as the app has the file, so that only makes sense
/// in GUI mode.
const FILE_PLACEHOLDER: &str = "{file}";
fn editor_command(words: &[String], path: &std::path::Path) -> Command {
    let program = words[0].trim_end_matches('/');
    let file = path.to_string_lossy();
    let mut args: Vec<String> = words[1..]
        .iter()
        .map(|w| w.replace(FILE_PLACEHOLDER, &file))
        .collect();
    if !words[1..].iter().any(|w| w.contains(FILE_PLACEHOLDER)) {
        args.push(file.into_owned());
    }
    let mut c = if cfg!(target_os = "macos") && program.ends_with(".app") {
        let mut c = Command::new("open");
        c.arg("-a").arg(program).args(&args);
        c
    } else {
        let mut c = Command::new(program);
        c.args(&args);
        c
    };
    if let Some(dir) = path.parent() {
        c.current_dir(dir);
    }
    c
}
pub struct App {
    pub config: Config,
    pub videos: Vec<Video>,
    pub folders: Vec<Folder>,
    pub collections: Vec<Collection>,
    pub section: Section,
    pub view_videos: Vec<usize>,
    pub view_folders: Vec<usize>,
    pub view_collections: Vec<usize>,
    pub cursor_video: usize,
    pub cursor_folder: usize,
    pub cursor_collection: usize,
    pub sort: SortKey,
    pub sort_open: bool,
    pub sort_row: usize,
    sort_order: Vec<usize>,
    video_pos: Vec<usize>,
    pub search: String,
    pub search_input: Option<String>,
    pub playing: Option<usize>,
    pending_play: Option<usize>,
    /// Load the pending video paused rather than playing (used when the
    /// last session's video is restored at startup).
    start_paused: bool,
    /// The video that was playing when the app last ran, selected and loaded
    /// paused once the index has been read; `None` after that.
    resume_video: Option<PathBuf>,
    pub offsets: [usize; 3],
    /// Per section: the cursor was moved by a video change rather than by the
    /// user, so if it landed off screen the list should scroll to centre it.
    pub recenter: [bool; 3],
    pub speed_pct: i32,
    pub shuffle: bool,
    shuffle_rng: u64,
    shuffle_history: Vec<usize>,
    pub playback: Option<Playback>,
    pub audio_error: Option<String>,
    /// A background level measurement for auto-normalisation: which video,
    /// and the channel its result arrives on.
    measuring: Option<(usize, Receiver<Result<f32>>)>,
    pub fx: FxParams,
    pub eq_open: bool,
    pub eq_row: usize,
    /// The picture fills the window instead of sharing it with the list.
    pub expanded: bool,
    pub renderer: Renderer,
    /// What detection (or `video.renderer`) chose; `r` cycles away from it.
    detected: Renderer,
    /// The setting name of the renderer picked with `r`, if any.
    renderer_choice: Option<String>,
    /// A renderer just switched away from, whose picture may need clearing.
    pub renderer_was: Option<Renderer>,
    /// Set by the UI each frame when a picture is to be sent with a
    /// graphics protocol; cleared when it is not.
    pub image_out: Option<Placement>,
    /// The UI needs the whole screen rewritten next frame (stale picture
    /// cells that the cell diff would not touch).
    pub force_repaint: bool,
    pub status: String,
    pub status_at: Instant,
    pub warning: Option<String>,
    pub indexing: Option<(usize, usize)>,
    pub page_size: usize,
    index_rx: Option<Receiver<IndexMsg>>,
    /// A terminal video editor waiting to be run by the main loop, which has
    /// to hand the screen over first (see `main::run_editor`).
    pub pending_editor: Option<(String, Command)>,
    pub should_quit: bool,
}
impl App {
    pub fn new(config: Config, renderer: Renderer) -> Self {
        let settings = load_settings(&config);
        let (playback, audio_error) = match Playback::new(settings.fx, &config.ffmpeg) {
            Ok(a) => (Some(a), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let mut app = Self::build(config, settings.fx, playback, audio_error, renderer);
        app.shuffle = settings.shuffle;
        app.expanded = settings.expanded;
        if let Some(name) = settings.renderer.as_deref() {
            app.choose_renderer(name);
        }
        app.set_speed_pct(settings.speed_pct);
        app.resume_video = settings.last_video;
        app.start_index(false);
        app
    }
    fn build(
        config: Config,
        fx: FxParams,
        playback: Option<Playback>,
        audio_error: Option<String>,
        renderer: Renderer,
    ) -> Self {
        Self {
            config,
            videos: Vec::new(),
            folders: Vec::new(),
            collections: Vec::new(),
            section: Section::Videos,
            view_videos: Vec::new(),
            view_folders: Vec::new(),
            view_collections: Vec::new(),
            sort: SortKey::Folder,
            sort_open: false,
            sort_row: 0,
            sort_order: Vec::new(),
            video_pos: Vec::new(),
            search: String::new(),
            search_input: None,
            cursor_video: 0,
            cursor_folder: 0,
            cursor_collection: 0,
            playing: None,
            pending_play: None,
            start_paused: false,
            resume_video: None,
            offsets: [0; 3],
            recenter: [false; 3],
            speed_pct: 0,
            shuffle: false,
            shuffle_rng: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E37_79B9_7F4A_7C15)
                | 1,
            shuffle_history: Vec::new(),
            playback,
            audio_error,
            measuring: None,
            fx,
            eq_open: false,
            eq_row: 0,
            expanded: false,
            renderer,
            detected: renderer,
            renderer_choice: None,
            renderer_was: None,
            image_out: None,
            force_repaint: false,
            status: String::new(),
            warning: None,
            status_at: Instant::now(),
            indexing: None,
            page_size: 10,
            index_rx: None,
            pending_editor: None,
            should_quit: false,
        }
    }
    pub fn start_index(&mut self, force: bool) {
        let (tx, rx) = mpsc::channel();
        let root = self.config.video_home.clone();
        let file = self.config.index_file.clone();
        let ffprobe = self.config.ffprobe.clone();
        thread::spawn(move || index::build(root, file, ffprobe, force, tx));
        self.index_rx = Some(rx);
        self.indexing = Some((0, 0));
    }
    pub fn poll_index(&mut self) {
        let Some(rx) = &self.index_rx else { return };
        let mut finished = None;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                IndexMsg::Progress { done, total } => self.indexing = Some((done, total)),
                IndexMsg::Done(r) => finished = Some(r),
            }
        }
        if let Some(result) = finished {
            self.index_rx = None;
            self.indexing = None;
            match result {
                Ok(scan) => {
                    let n = scan.videos.len();
                    self.set_videos(scan.videos);
                    self.set_status(format!(
                        "indexed {n} videos, {} folders, {} collections",
                        self.folders.len(),
                        self.collections.len()
                    ));
                    let mut warnings = Vec::new();
                    if let Some(err) = scan.probe_failed {
                        warnings.push(format!(
                            "ffprobe is not available ({err}); nothing will play until ffmpeg is installed (brew install ffmpeg / apt install ffmpeg) or ffmpeg.dir is set in ~/.daffy; then press i."
                        ));
                    }
                    if !scan.unreadable.is_empty() {
                        let names: Vec<String> = scan
                            .unreadable
                            .iter()
                            .take(4)
                            .map(|p| display_path(p))
                            .collect();
                        let more = if scan.unreadable.len() > 4 {
                            format!(" and {} more", scan.unreadable.len() - 4)
                        } else {
                            String::new()
                        };
                        let hint = if cfg!(target_os = "macos") {
                            "Grant your terminal app access under System Settings → Privacy & Security → Files and Folders, then press i."
                        } else {
                            "Check the folder permissions, then press i to re-index."
                        };
                        warnings.push(format!(
                            "{} folder{} could not be read: {}{}. {hint}",
                            scan.unreadable.len(),
                            if scan.unreadable.len() == 1 { "" } else { "s" },
                            names.join(", "),
                            more
                        ));
                    }
                    self.warning = (!warnings.is_empty()).then(|| warnings.join(" "));
                }
                Err(e) => self.set_status(format!("index failed: {e:#}")),
            }
        }
    }
    fn set_videos(&mut self, videos: Vec<Video>) {
        let playing_path = self
            .playing
            .and_then(|i| self.videos.get(i))
            .map(|t| t.path.clone());
        // A re-index (after the editor, or `i`) should not move the user's
        // selection or scroll: remember what each cursor is on.
        let keep = (!self.videos.is_empty()).then(|| ViewState {
            video: self
                .view_videos
                .get(self.cursor_video)
                .map(|&i| self.videos[i].path.clone()),
            folder: self
                .view_folders
                .get(self.cursor_folder)
                .map(|&a| self.folders[a].name.clone()),
            collection: self
                .view_collections
                .get(self.cursor_collection)
                .map(|&r| self.collections[r].name.clone()),
            offsets: self.offsets,
        });
        // Keep levels measured since the index file was last read, as long
        // as the file itself has not changed.
        let measured: std::collections::HashMap<&std::path::Path, (u64, u64, f32)> = self
            .videos
            .iter()
            .filter_map(|t| t.gain_db.map(|g| (t.path.as_path(), (t.mtime, t.size, g))))
            .collect();
        let mut videos = videos;
        for t in videos.iter_mut().filter(|t| t.gain_db.is_none()) {
            if let Some(&(mtime, size, g)) = measured.get(t.path.as_path())
                && t.mtime == mtime
                && t.size == size
            {
                t.gain_db = Some(g);
            }
        }
        self.videos = videos;
        self.folders.clear();
        self.collections.clear();
        let mut i = 0;
        while i < self.videos.len() {
            let t = &self.videos[i];
            let collection = t.collection.clone();
            let name = t.folder.clone();
            let mut j = i + 1;
            while j < self.videos.len()
                && self.videos[j].collection == collection
                && self.videos[j].folder == name
            {
                j += 1;
            }
            let year = self.videos[i..j].iter().filter_map(|t| t.year).min();
            self.folders.push(Folder {
                name,
                collection,
                year,
                start: i,
                end: j,
            });
            i = j;
        }
        let mut a = 0;
        while a < self.folders.len() {
            let name = self.folders[a].collection.clone();
            let mut b = a + 1;
            while b < self.folders.len() && self.folders[b].collection == name {
                b += 1;
            }
            self.collections.push(Collection {
                name,
                folder_start: a,
                folder_end: b,
                video_start: self.folders[a].start,
                video_end: self.folders[b - 1].end,
            });
            a = b;
        }
        self.playing = playing_path.and_then(|p| self.videos.iter().position(|t| t.path == p));
        self.rebuild_sort_order();
        self.apply_search();
        if let Some(keep) = keep {
            self.restore_view(keep);
        }
        // First index after startup: pick up where the last session left
        // off, with the video cued but not playing.
        if let Some(p) = self.resume_video.take()
            && self.playing.is_none()
            && let Some(i) = self.videos.iter().position(|t| t.path == p)
        {
            self.start_paused = true;
            self.play_index(i);
        }
    }
    fn restore_view(&mut self, keep: ViewState) {
        if let Some(p) = keep.video
            && let Some(i) = self.videos.iter().position(|t| t.path == p)
            && let Some(pos) = self.video_view_pos(i)
        {
            self.cursor_video = pos;
        }
        if let Some(name) = keep.folder
            && let Some(a) = self.folders.iter().position(|f| f.name == name)
            && self.view_folders.binary_search(&a).is_ok()
        {
            self.cursor_folder = Self::view_pos(&self.view_folders, a);
        }
        if let Some(name) = keep.collection
            && let Some(r) = self.collections.iter().position(|c| c.name == name)
            && self.view_collections.binary_search(&r).is_ok()
        {
            self.cursor_collection = Self::view_pos(&self.view_collections, r);
        }
        // The lists are drawn with these offsets clamped so the cursor stays
        // visible, so a stale offset is harmless.
        self.offsets = keep.offsets;
        self.recenter = [false; 3];
    }
    fn rebuild_sort_order(&mut self) {
        let mut order: Vec<usize> = (0..self.videos.len()).collect();
        let videos = &self.videos;
        match self.sort {
            SortKey::Folder => {}
            SortKey::Title => {
                order.sort_by_cached_key(|&i| {
                    (
                        videos[i].title.to_lowercase(),
                        videos[i].folder.to_lowercase(),
                        i,
                    )
                });
            }
            SortKey::Newest => {
                order.sort_by_cached_key(|&i| (std::cmp::Reverse(videos[i].mtime), i));
            }
            SortKey::Length => {
                order.sort_by_cached_key(|&i| {
                    (
                        std::cmp::Reverse((videos[i].duration_secs * 1000.0) as u64),
                        i,
                    )
                });
            }
        }
        self.sort_order = order;
    }
    pub fn open_sort(&mut self) {
        self.sort_row = SortKey::ALL
            .iter()
            .position(|&k| k == self.sort)
            .unwrap_or(0);
        self.sort_open = true;
    }
    pub fn sort_move(&mut self, delta: i64) {
        let n = SortKey::ALL.len() as i64;
        self.sort_row = (self.sort_row as i64 + delta).rem_euclid(n) as usize;
    }
    pub fn apply_sort(&mut self) {
        self.sort_open = false;
        let key = SortKey::ALL[self.sort_row];
        if key != self.sort {
            self.sort = key;
            self.rebuild_sort_order();
            self.apply_search();
            self.offsets[0] = 0;
        }
        self.set_status(format!("sorted by {}", key.label().to_lowercase()));
    }
    pub fn apply_search(&mut self) {
        let q = self.search.trim().to_lowercase();
        let matched: Vec<bool> = if q.is_empty() {
            vec![true; self.videos.len()]
        } else {
            self.videos
                .iter()
                .map(|t| {
                    t.title.to_lowercase().contains(&q)
                        || t.folder.to_lowercase().contains(&q)
                        || t.collection.to_lowercase().contains(&q)
                        || t.file_name().to_lowercase().contains(&q)
                })
                .collect()
        };
        self.view_videos = self
            .sort_order
            .iter()
            .copied()
            .filter(|&i| matched[i])
            .collect();
        self.video_pos = vec![usize::MAX; self.videos.len()];
        for (pos, &i) in self.view_videos.iter().enumerate() {
            self.video_pos[i] = pos;
        }
        self.view_folders = (0..self.folders.len())
            .filter(|&a| {
                matched[self.folders[a].start..self.folders[a].end]
                    .iter()
                    .any(|&m| m)
            })
            .collect();
        let folder_matched: Vec<bool> = {
            let mut v = vec![false; self.folders.len()];
            for &a in &self.view_folders {
                v[a] = true;
            }
            v
        };
        self.view_collections = (0..self.collections.len())
            .filter(|&r| {
                folder_matched[self.collections[r].folder_start..self.collections[r].folder_end]
                    .iter()
                    .any(|&m| m)
            })
            .collect();
        self.cursor_video = 0;
        self.cursor_folder = 0;
        self.cursor_collection = 0;
        self.offsets = [0; 3];
        if let Some(i) = self.playing
            && self.video_view_pos(i).is_some()
        {
            self.sync_cursors_to(i);
        }
    }
    pub fn open_search(&mut self) {
        self.search_input = Some(String::new());
    }
    pub fn cancel_search(&mut self) {
        self.search_input = None;
    }
    pub fn submit_search(&mut self) {
        let Some(q) = self.search_input.take() else {
            return;
        };
        self.search = q.trim().to_string();
        self.apply_search();
        self.section = Section::Videos;
        if self.search.is_empty() {
            self.set_status("showing everything");
        } else {
            let n = self.view_videos.len();
            self.set_status(format!(
                "{n} video{} match \"{}\"",
                if n == 1 { "" } else { "s" },
                self.search
            ));
        }
    }
    fn first_in_view(&self, start: usize, end: usize) -> Option<usize> {
        (start..end.min(self.video_pos.len())).find(|&i| self.video_pos[i] != usize::MAX)
    }
    fn video_view_pos(&self, idx: usize) -> Option<usize> {
        self.video_pos.get(idx).copied().filter(|&p| p != usize::MAX)
    }
    fn view_pos(view: &[usize], idx: usize) -> usize {
        match view.binary_search(&idx) {
            Ok(p) => p,
            Err(p) => p.min(view.len().saturating_sub(1)),
        }
    }
    fn cursor_video(&self) -> Option<usize> {
        match self.section {
            Section::Videos => self.view_videos.get(self.cursor_video).copied(),
            Section::Folders => self
                .view_folders
                .get(self.cursor_folder)
                .and_then(|&a| self.first_in_view(self.folders[a].start, self.folders[a].end)),
            Section::Collections => self.view_collections.get(self.cursor_collection).and_then(|&r| {
                self.first_in_view(
                    self.collections[r].video_start,
                    self.collections[r].video_end,
                )
            }),
        }
    }
    pub fn set_status(&mut self, s: impl Into<String>) {
        self.status = s.into();
        self.status_at = Instant::now();
    }
    pub fn folder_of(&self, video_idx: usize) -> Option<usize> {
        self.folders
            .iter()
            .position(|a| video_idx >= a.start && video_idx < a.end)
    }
    pub fn collection_of_folder(&self, folder_idx: usize) -> Option<usize> {
        self.collections
            .iter()
            .position(|a| folder_idx >= a.folder_start && folder_idx < a.folder_end)
    }
    fn sync_cursors_to(&mut self, video_idx: usize) {
        self.recenter = [true; 3];
        if let Some(p) = self.video_view_pos(video_idx) {
            self.cursor_video = p;
        } else {
            self.cursor_video = self
                .cursor_video
                .min(self.view_videos.len().saturating_sub(1));
        }
        if let Some(al) = self.folder_of(video_idx) {
            self.cursor_folder = Self::view_pos(&self.view_folders, al);
            if let Some(ar) = self.collection_of_folder(al) {
                self.cursor_collection = Self::view_pos(&self.view_collections, ar);
            }
        }
    }
    pub fn speed(&self) -> f32 {
        1.0 + self.speed_pct as f32 / 100.0
    }
    pub fn playing_video(&self) -> Option<&Video> {
        self.playing.and_then(|i| self.videos.get(i))
    }
    fn anchor_pos(&self) -> Option<usize> {
        if self.view_videos.is_empty() {
            return None;
        }
        if let Some(pos) = self.playing.and_then(|p| self.video_view_pos(p)) {
            return Some(pos);
        }
        let idx = self.cursor_video().unwrap_or(self.view_videos[0]);
        Some(self.video_view_pos(idx).unwrap_or(0))
    }
    pub fn play_index(&mut self, i: usize) {
        if i >= self.videos.len() {
            return;
        }
        self.playing = Some(i);
        self.pending_play = Some(i);
        self.sync_cursors_to(i);
        self.save_settings();
    }
    pub fn flush_play(&mut self) {
        let Some(i) = self.pending_play.take() else {
            return;
        };
        let paused = std::mem::take(&mut self.start_paused);
        let Some(video) = self.videos.get(i) else {
            return;
        };
        if !video.has_video() && !video.has_audio() {
            self.playing = None;
            self.set_status(format!(
                "cannot play {}: no audio or video stream found (is ffmpeg installed? press i to re-index)",
                video.file_name()
            ));
            return;
        }
        let media = Media::from(video);
        let speed = self.speed();
        // The FX chain reads its gain when the source is built, so the
        // video's normalisation gain has to be in place before `load`.
        self.sync_track_gain();
        match self.playback.as_mut() {
            Some(playback) => match playback.load(media, 0.0) {
                Ok(()) => {
                    playback.set_speed(speed);
                    if paused {
                        playback.pause();
                    }
                    self.status.clear();
                }
                Err(e) => {
                    self.playing = None;
                    let mut msg = format!("cannot play: {e:#}");
                    if e.chain().any(|c| {
                        c.downcast_ref::<std::io::Error>()
                            .map(|io| io.kind() == std::io::ErrorKind::NotFound)
                            .unwrap_or(false)
                    }) {
                        msg.push_str(" — ffmpeg was not found; install it or set ffmpeg.dir in ~/.daffy");
                    } else if cfg!(target_os = "macos")
                        && e.chain().any(|c| {
                            c.downcast_ref::<std::io::Error>()
                                .map(|io| io.kind() == std::io::ErrorKind::PermissionDenied)
                                .unwrap_or(false)
                        })
                    {
                        msg.push_str(" — macOS is blocking this terminal: System Settings → Privacy & Security → Files and Folders");
                    }
                    self.set_status(msg);
                }
            },
            None => self.set_status(format!(
                "no audio output: {}",
                self.audio_error.as_deref().unwrap_or("unknown error")
            )),
        }
    }
    fn play_cursor(&mut self) {
        if let Some(i) = self.cursor_video() {
            self.play_index(i);
        }
    }
    pub fn toggle_play(&mut self) {
        if self.playing.is_some() {
            if let Some(p) = &mut self.playback {
                p.toggle_pause();
            }
        } else {
            self.play_cursor();
        }
    }
    pub fn step_video(&mut self, delta: i64) {
        if self.shuffle && self.view_videos.len() > 1 {
            self.step_shuffle(delta);
            return;
        }
        let Some(pos) = self.anchor_pos() else { return };
        let n = self.view_videos.len() as i64;
        let next = (pos as i64 + delta).rem_euclid(n) as usize;
        self.play_index(self.view_videos[next]);
    }
    fn next_random(&mut self) -> u64 {
        let mut x = self.shuffle_rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.shuffle_rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn step_shuffle(&mut self, delta: i64) {
        if delta < 0 {
            while let Some(prev) = self.shuffle_history.pop() {
                if Some(prev) != self.playing && self.videos.get(prev).is_some() {
                    self.play_index(prev);
                    return;
                }
            }
            return;
        }
        let n = self.view_videos.len();
        let r = (self.next_random() % n as u64) as usize;
        let mut pick = self.view_videos[r];
        if Some(pick) == self.playing {
            let pos = self.video_view_pos(pick).unwrap_or(0);
            pick = self.view_videos[(pos + 1) % n];
        }
        if let Some(cur) = self.playing {
            self.shuffle_history.push(cur);
            if self.shuffle_history.len() > 500 {
                self.shuffle_history.remove(0);
            }
        }
        self.play_index(pick);
    }
    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        self.shuffle_history.clear();
        self.set_status(if self.shuffle {
            "Shuffle on"
        } else {
            "Shuffle off"
        });
        self.save_settings();
    }
    /// Every renderer worth offering here, by setting name: the detected
    /// one first, then the graphics protocols the terminal might speak,
    /// then the text ones that always work.
    pub fn renderer_options(&self) -> Vec<(String, Renderer)> {
        let d = self.detected;
        let mut v = vec![("auto".to_string(), d)];
        for p in Protocol::ALL {
            if p == Protocol::Sixel && !d.cell_known {
                continue;
            }
            let r = d.with(p, true);
            v.push((r.setting_name(), r));
            if p.is_text() {
                let r = d.with(p, false);
                v.push((r.setting_name(), r));
            }
        }
        v
    }
    /// Switches to the renderer named `name` (`auto` for the detected one).
    pub fn choose_renderer(&mut self, name: &str) {
        let Some((_, r)) = self.renderer_options().into_iter().find(|(n, _)| n == name) else {
            return;
        };
        if r.protocol != self.renderer.protocol {
            self.renderer_was = Some(self.renderer);
        }
        self.renderer = r;
        self.renderer_choice = (name != "auto").then(|| name.to_string());
        self.force_repaint = true;
    }
    /// `r`: the next renderer in the list.
    pub fn cycle_renderer(&mut self) {
        let options = self.renderer_options();
        let current = self.renderer_choice.as_deref().unwrap_or("auto");
        let i = options.iter().position(|(n, _)| n == current).unwrap_or(0);
        let (name, _) = &options[(i + 1) % options.len()];
        let name = name.clone();
        self.choose_renderer(&name);
        let label = self.renderer.label();
        self.set_status(if name == "auto" {
            format!("video: {label} (detected)")
        } else {
            format!("video: {label}")
        });
        self.save_settings();
    }
    pub fn toggle_expand(&mut self) {
        self.expanded = !self.expanded;
        self.force_repaint = true;
        self.save_settings();
    }
    pub fn move_cursor(&mut self, delta: i64) {
        let (cur, len) = match self.section {
            Section::Videos => (&mut self.cursor_video, self.view_videos.len()),
            Section::Folders => (&mut self.cursor_folder, self.view_folders.len()),
            Section::Collections => (&mut self.cursor_collection, self.view_collections.len()),
        };
        if len == 0 {
            return;
        }
        *cur = (*cur as i64).saturating_add(delta).clamp(0, len as i64 - 1) as usize;
        self.recenter[self.section as usize] = false;
    }
    pub fn activate_cursor(&mut self) {
        self.play_cursor();
    }
    pub fn seek(&mut self, delta_secs: f64) {
        let Some(t) = self.playing_video() else {
            return;
        };
        let total = t.duration_secs;
        if let Some(p) = &mut self.playback
            && let Err(e) = p.seek_by(delta_secs, total)
        {
            self.set_status(format!("seek failed: {e:#}"));
        }
    }
    /// Current position in the playing video, in seconds.
    pub fn position(&self) -> f64 {
        self.playback.as_ref().map(|p| p.position()).unwrap_or(0.0)
    }
    pub fn is_paused(&self) -> bool {
        self.playback.as_ref().map(|p| p.is_paused()).unwrap_or(true)
    }
    pub fn adjust_speed(&mut self, delta: i32) {
        self.set_speed_pct(self.speed_pct + delta);
        self.save_settings();
    }
    pub fn reset_speed(&mut self) {
        self.set_speed_pct(0);
        self.save_settings();
    }
    fn set_speed_pct(&mut self, pct: i32) {
        self.speed_pct = pct.clamp(-50, 100);
        let s = self.speed();
        if let Some(p) = &mut self.playback {
            p.set_speed(s);
        }
    }
    pub fn reveal_in_finder(&mut self) {
        let idx = self.playing.or_else(|| self.cursor_video());
        let Some(t) = idx.and_then(|i| self.videos.get(i)) else {
            return;
        };
        let path = t.path.clone();
        match reveal_command(&path).spawn() {
            Ok(_) => self.set_status(format!("revealed {}", path.display())),
            Err(e) => self.set_status(format!("could not open file manager: {e}")),
        }
    }
    /// `E`: opens the selected video in the configured editor. A GUI
    /// editor is started and left alone; a CLI one is queued for the main
    /// loop, which gives it the terminal and waits.
    pub fn edit_in_editor(&mut self) {
        let Some(words) = self.config.video_editor.clone() else {
            self.set_status("no video editor: set video.editor in ~/.daffy");
            return;
        };
        let idx = self.cursor_video().or(self.playing);
        let Some(t) = idx.and_then(|i| self.videos.get(i)) else {
            return;
        };
        let path = t.path.clone();
        let name = std::path::Path::new(words[0].trim_end_matches('/'))
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| words[0].clone());
        let mut cmd = editor_command(&words, &path);
        match self.config.video_editor_mode {
            EditorMode::Gui => {
                cmd.stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                match cmd.spawn() {
                    Ok(_) => self.set_status(format!(
                        "opened {} in {name}",
                        self.config.library_path(&path)
                    )),
                    Err(e) => self.set_status(format!("could not start {name}: {e}")),
                }
            }
            EditorMode::Cli => {
                cmd.stdin(Stdio::inherit())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
                self.pending_editor = Some((name, cmd));
            }
        }
    }
    /// Called by the main loop once a CLI editor has exited and the UI is
    /// back on screen.
    pub fn editor_finished(
        &mut self,
        program: &str,
        result: std::io::Result<std::process::ExitStatus>,
    ) {
        match result {
            Ok(st) if st.success() => {
                self.set_status(format!("{program} finished"));
                // Tags or length may have changed: pick that up without
                // rereading everything.
                if self.index_rx.is_none() {
                    self.start_index(false);
                }
            }
            Ok(st) => self.set_status(format!("{program} exited with {st}")),
            Err(e) => self.set_status(format!("could not start {program}: {e}")),
        }
    }
    fn push_fx(&mut self) {
        if let Some(p) = &self.playback {
            p.set_fx(self.fx);
        }
    }
    fn settings(&self) -> Settings {
        // Before the index has loaded nothing is playing yet; keep the video
        // waiting to be restored rather than forgetting it.
        let last_video = self
            .playing
            .and_then(|i| self.videos.get(i))
            .map(|t| t.path.clone())
            .or_else(|| self.resume_video.clone());
        Settings {
            fx: self.fx,
            shuffle: self.shuffle,
            speed_pct: self.speed_pct,
            expanded: self.expanded,
            renderer: self.renderer_choice.clone(),
            last_video,
        }
    }
    fn save_settings(&mut self) {
        if let Err(e) = write_settings(&self.config, &self.settings()) {
            self.set_status(format!("could not save settings: {e:#}"));
        }
    }
    pub fn eq_rows(&self) -> Vec<EqRow> {
        let mut rows = vec![EqRow::Preamp, EqRow::Normalize, EqRow::Mode];
        rows.extend((0..self.fx.eq.mode.bands()).map(EqRow::Band));
        rows.push(EqRow::Loudness);
        rows.push(EqRow::Stereo);
        if self.fx.stereo == StereoMode::Width {
            rows.push(EqRow::Width);
        }
        rows
    }
    pub fn eq_move(&mut self, delta: i64) {
        let n = self.eq_rows().len() as i64;
        self.eq_row = (self.eq_row as i64 + delta).rem_euclid(n) as usize;
    }
    pub fn eq_adjust(&mut self, delta: i32) {
        let rows = self.eq_rows();
        let Some(&row) = rows.get(self.eq_row) else {
            return;
        };
        let step = if delta < 0 { -1 } else { 1 };
        match row {
            EqRow::Preamp => {
                self.fx.preamp = (self.fx.preamp as i32 + delta)
                    .clamp(PREAMP_MIN_DB as i32, PREAMP_MAX_DB as i32)
                    as i8
            }
            EqRow::Normalize => self.set_normalize(!self.fx.auto_normalize),
            EqRow::Mode => {
                let i = EqMode::ALL
                    .iter()
                    .position(|&m| m == self.fx.eq.mode)
                    .unwrap_or(0) as i64;
                self.fx.eq.mode =
                    EqMode::ALL[(i + step).rem_euclid(EqMode::ALL.len() as i64) as usize];
            }
            EqRow::Band(b) => {
                if let Some(g) = self.fx.eq.gain_mut(b) {
                    *g = (*g as i32 + delta).clamp(-(EQ_RANGE_DB as i32), EQ_RANGE_DB as i32) as i8;
                }
            }
            EqRow::Loudness => self.fx.loudness = !self.fx.loudness,
            EqRow::Stereo => {
                let i = StereoMode::ALL
                    .iter()
                    .position(|&m| m == self.fx.stereo)
                    .unwrap_or(0) as i64;
                self.fx.stereo =
                    StereoMode::ALL[(i + step).rem_euclid(StereoMode::ALL.len() as i64) as usize];
            }
            EqRow::Width => {
                self.fx.width = (self.fx.width as i32 + delta * 5).clamp(0, WIDTH_MAX as i32) as u8
            }
        }
        self.eq_row = self.eq_row.min(self.eq_rows().len() - 1);
        self.push_fx();
    }
    pub fn eq_reset_row(&mut self) {
        let rows = self.eq_rows();
        let Some(&row) = rows.get(self.eq_row) else {
            return;
        };
        match row {
            EqRow::Preamp => self.fx.preamp = 0,
            EqRow::Normalize => self.set_normalize(false),
            EqRow::Mode => self.fx.eq.mode = EqMode::Off,
            EqRow::Band(b) => {
                if let Some(g) = self.fx.eq.gain_mut(b) {
                    *g = 0;
                }
            }
            EqRow::Loudness => self.fx.loudness = false,
            EqRow::Stereo => self.fx.stereo = StereoMode::Normal,
            EqRow::Width => self.fx.width = 100,
        }
        self.eq_row = self.eq_row.min(self.eq_rows().len() - 1);
        self.push_fx();
    }
    fn set_normalize(&mut self, on: bool) {
        self.fx.auto_normalize = on;
        self.sync_track_gain();
    }
    /// What the Preamp row should say about normalisation: the gain in use,
    /// or that it is still being measured.
    pub fn normalize_state(&self) -> NormalizeState {
        if !self.fx.auto_normalize {
            return NormalizeState::Off;
        }
        match self.playing.and_then(|i| self.videos.get(i)) {
            Some(t) => match t.gain_db {
                Some(g) => NormalizeState::Gain(g),
                None => NormalizeState::Measuring,
            },
            None => NormalizeState::Idle,
        }
    }
    pub fn close_eq(&mut self) {
        self.eq_open = false;
        self.save_settings();
    }
    /// Points the FX chain at the playing video's normalisation gain (0 dB
    /// while unknown) and starts measuring it if needed.
    fn sync_track_gain(&mut self) {
        let playing = self
            .playing
            .and_then(|i| self.videos.get(i).map(|t| (i, t.gain_db, t.has_audio())));
        self.fx.track_gain = match playing {
            Some((_, Some(g), _)) if self.fx.auto_normalize => g,
            _ => 0.0,
        };
        self.push_fx();
        if let Some((i, None, true)) = playing
            && self.fx.auto_normalize
            && self.measuring.as_ref().map(|(m, _)| *m) != Some(i)
        {
            let path = self.videos[i].path.clone();
            let ffmpeg = self.config.ffmpeg.clone();
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let _ = tx.send(crate::video::measure_gain(&ffmpeg, &path));
            });
            self.measuring = Some((i, rx));
        }
    }
    pub fn poll_measurement(&mut self) {
        let Some((i, rx)) = &self.measuring else {
            return;
        };
        let Ok(result) = rx.try_recv() else {
            return;
        };
        let i = *i;
        self.measuring = None;
        match result {
            Ok(g) => {
                if let Some(t) = self.videos.get_mut(i) {
                    t.gain_db = Some(g);
                }
                if self.playing == Some(i) {
                    self.sync_track_gain();
                }
                // Remember it for next time, off the UI thread.
                let file = self.config.index_file.clone();
                let videos = self.videos.clone();
                thread::spawn(move || {
                    let _ = index::save(&file, &videos);
                });
            }
            Err(e) => self.set_status(format!("could not measure level: {e:#}")),
        }
    }
    pub fn toggle_loudness(&mut self) {
        self.fx.loudness = !self.fx.loudness;
        self.push_fx();
        self.set_status(if self.fx.loudness {
            "Loudness on"
        } else {
            "Loudness off"
        });
        self.save_settings();
    }
    /// Something is playing right now (not paused, not at the end).
    pub fn playing_active(&self) -> bool {
        self.playing.is_some()
            && self
                .playback
                .as_ref()
                .map(|p| p.is_loaded() && !p.is_paused() && !p.finished())
                .unwrap_or(false)
    }
    /// Lets playback pull in the frame that is due; call before drawing.
    pub fn update_video(&mut self) {
        if let Some(p) = &mut self.playback {
            p.poll();
        }
    }
    /// Tells playback where pictures are wanted (`None`: nowhere).
    pub fn set_video_target(&mut self, target: Option<Target>) {
        if let Some(p) = &mut self.playback {
            p.set_target(target);
        }
    }
    /// Scroll offset for a list of `len` rows shown `height` at a time.
    ///
    /// Normally the list scrolls just enough to keep the cursor visible. When
    /// `recenter` is set (the cursor was moved by a video change, e.g. shuffle
    /// jumping somewhere far away) and the cursor is off screen, the list is
    /// scrolled so the cursor sits in the middle instead of at an edge.
    pub fn scroll_offset(
        offset: usize,
        cursor: usize,
        len: usize,
        height: usize,
        recenter: bool,
    ) -> usize {
        let height = height.max(1);
        let mut offset = offset.min(len.saturating_sub(1));
        let visible = cursor >= offset && cursor < offset + height;
        if recenter && !visible {
            offset = cursor
                .saturating_sub(height / 2)
                .min(len.saturating_sub(height));
        }
        if cursor < offset {
            offset = cursor;
        } else if cursor >= offset + height {
            offset = cursor + 1 - height;
        }
        offset
    }
    pub fn tick(&mut self) -> Result<()> {
        self.poll_index();
        self.poll_measurement();
        if self.pending_play.is_none()
            && let (Some(_), Some(p)) = (self.playing, &self.playback)
            && p.is_loaded()
            && p.finished()
            && !p.is_paused()
        {
            self.step_video(1);
        }
        Ok(())
    }
}
