mod editor_mode;
#[cfg(test)]
mod tests;
use anyhow::{Context, Result, anyhow};
pub use editor_mode::EditorMode;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
pub struct Config {
    pub video_home: PathBuf,
    pub index_file: PathBuf,
    /// Command that opens a video for editing (`video.editor`), split into
    /// program and arguments; the file path is appended when it runs.
    pub video_editor: Option<Vec<String>>,
    pub video_editor_mode: EditorMode,
    /// `video.renderer`: force a particular way of drawing video frames
    /// instead of detecting what the terminal supports.
    pub renderer: Option<String>,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}
/// Splits a command line into words, honouring single and double quotes
/// and backslash escapes, so `video.editor = "open -a \"Final Cut Pro\""` works.
fn split_command(raw: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') | (None, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    in_word = true;
                }
            }
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    words
}
pub fn home_dir() -> Result<PathBuf> {
    std::env::home_dir().ok_or_else(|| anyhow!("could not determine home directory"))
}
pub fn config_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("DAFFY_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    Ok(home_dir()?.join(".daffy"))
}
/// Where stderr goes while the UI is on screen (see `stderr.rs`).
pub fn log_path() -> PathBuf {
    config_path()
        .map(|p| p.with_extension("log"))
        .unwrap_or_else(|_| PathBuf::from("daffy.log"))
}
fn expand_tilde(raw: &str) -> Result<PathBuf> {
    if raw == "~" {
        return home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return Ok(home_dir()?.join(rest));
    }
    Ok(PathBuf::from(raw))
}
fn parse_kv(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let mut v = v.trim();
        if v.len() >= 2 {
            let b = v.as_bytes();
            if (b[0] == b'"' && b[b.len() - 1] == b'"')
                || (b[0] == b'\'' && b[b.len() - 1] == b'\'')
            {
                v = &v[1..v.len() - 1];
            }
        }
        map.insert(k.trim().to_string(), v.to_string());
    }
    map
}
const KNOWN_KEYS: [&str; 6] = [
    "video.home",
    "index.file",
    "video.editor",
    "video.editor.mode",
    "video.renderer",
    "ffmpeg.dir",
];
pub const RENDERERS: [&str; 14] = [
    "auto",
    "kitty",
    "iterm2",
    "sixel",
    "blocks",
    "blocks256",
    "quads",
    "quads256",
    "sextants",
    "sextants256",
    "mosaic",
    "mosaic256",
    "ascii",
    "ascii256",
];
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}
/// A misspelt key would otherwise be silently ignored and the default used,
/// which is much harder to notice than an error at startup.
fn check_keys(map: &HashMap<String, String>, path: &Path) -> Result<()> {
    let mut unknown: Vec<&String> = map
        .keys()
        .filter(|k| !KNOWN_KEYS.contains(&k.as_str()))
        .collect();
    unknown.sort();
    let Some(key) = unknown.first() else {
        return Ok(());
    };
    let hint = KNOWN_KEYS
        .iter()
        .map(|k| (edit_distance(key, k), *k))
        .min()
        .filter(|(d, _)| *d <= 4)
        .map(|(_, k)| format!(" (did you mean `{k}`?)"))
        .unwrap_or_default();
    Err(anyhow!(
        "unknown setting `{key}` in {}{hint}; known settings: {}",
        path.display(),
        KNOWN_KEYS.join(", ")
    ))
}
/// Where to find an ffmpeg tool: in `ffmpeg.dir` when set, otherwise on the
/// PATH, otherwise in the usual package-manager locations (a terminal app
/// started from the Finder may not have Homebrew on its PATH).
fn find_tool(name: &str, dir: Option<&Path>) -> PathBuf {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Some(d) = dir {
        return d.join(&exe);
    }
    if let Some(paths) = std::env::var_os("PATH")
        && std::env::split_paths(&paths).any(|p| p.join(&exe).is_file())
    {
        return PathBuf::from(exe);
    }
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin", "/usr/bin"] {
        let p = Path::new(d).join(&exe);
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from(exe)
}
pub fn load() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        let default = "# daffy configuration\n\
# Folder containing your videos (scanned recursively)\n\
video.home = ~/Movies\n\
\n\
# How video frames are drawn: auto (detect what the terminal can do),\n\
# kitty, iterm2, sixel, blocks (half-block cells), quads (quarter-block\n\
# cells), sextants (2×3 block cells), mosaic (block, wedge and eighth\n\
# shapes), ascii (coloured text), or the same with 256 on the end for\n\
# terminals with 256 colours. The r key\n\
# cycles through them while running.\n\
#video.renderer = auto\n\
\n\
# Folder holding ffmpeg and ffprobe, if they are not on the PATH.\n\
#ffmpeg.dir = /opt/homebrew/bin\n\
\n\
# Program that opens the selected video when you press E, e.g.\n\
#   video.editor = /Applications/HandBrake.app\n\
#   video.editor = losslesscut {file}\n\
# {file} stands for the video path; without it the path goes last.\n\
#video.editor =\n\
# gui: launch it and keep going. cli: the editor takes over this terminal\n\
# and daffy waits for it to finish.\n\
#video.editor.mode = gui\n";
        fs::write(&path, default)
            .with_context(|| format!("writing default config to {}", path.display()))?;
    }
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let map = parse_kv(&text);
    check_keys(&map, &path)?;
    let video_home = map
        .get("video.home")
        .map(|s| expand_tilde(s))
        .transpose()?
        .unwrap_or(home_dir()?.join("Movies"));
    if !video_home.is_dir() {
        return Err(anyhow!(
            "video.home `{}` (from {}) is not a directory",
            video_home.display(),
            path.display()
        ));
    }
    let index_file = match map.get("index.file") {
        Some(s) => expand_tilde(s)?,
        None => home_dir()?.join(".daffy-index.json"),
    };
    let video_editor = map
        .get("video.editor")
        .map(|s| split_command(s))
        .filter(|words| !words.is_empty())
        .map(|mut words| {
            words[0] = expand_tilde(&words[0])
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| words[0].clone());
            words
        });
    let video_editor_mode = map
        .get("video.editor.mode")
        .map(|s| EditorMode::parse(s))
        .transpose()?
        .unwrap_or_default();
    let renderer = map
        .get("video.renderer")
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty() && s != "auto");
    if let Some(r) = &renderer
        && !RENDERERS.contains(&r.as_str())
    {
        return Err(anyhow!(
            "video.renderer must be one of {}, not `{r}`",
            RENDERERS.join(", ")
        ));
    }
    let tool_dir = map
        .get("ffmpeg.dir")
        .map(|s| expand_tilde(s))
        .transpose()?;
    let ffmpeg = find_tool("ffmpeg", tool_dir.as_deref());
    let ffprobe = find_tool("ffprobe", tool_dir.as_deref());
    Ok(Config {
        video_home,
        index_file,
        video_editor,
        video_editor_mode,
        renderer,
        ffmpeg,
        ffprobe,
    })
}
impl Config {
    /// A video path for display: relative to `video.home` when it lives
    /// there, otherwise the same as [`display_path`].
    pub fn library_path(&self, p: &Path) -> String {
        match p.strip_prefix(&self.video_home) {
            Ok(rest) if !rest.as_os_str().is_empty() => rest.display().to_string(),
            _ => display_path(p),
        }
    }
}
pub fn display_path(p: &Path) -> String {
    if let Ok(home) = home_dir()
        && let Ok(rest) = p.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    p.display().to_string()
}
