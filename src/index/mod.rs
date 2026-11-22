mod index_msg;
mod scan;
#[cfg(test)]
mod tests;
mod track;
use crate::video::probe;
use anyhow::{Context, Result};
pub use index_msg::IndexMsg;
pub use scan::Scan;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::UNIX_EPOCH;
pub use track::Video;
#[cfg(test)]
use track::locate;
const VIDEO_EXTS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "mpg", "mpeg", "ts", "m2ts", "mts", "wmv", "flv",
    "3gp", "ogv", "vob", "divx",
];
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Index {
    pub videos: Vec<Video>,
}
pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| VIDEO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}
fn load_existing(index_file: &Path) -> HashMap<PathBuf, Video> {
    let Ok(text) = fs::read_to_string(index_file) else {
        return HashMap::new();
    };
    let Ok(idx) = serde_json::from_str::<Index>(&text) else {
        return HashMap::new();
    };
    idx.videos
        .into_iter()
        .map(|t| (t.path.clone(), t))
        .collect()
}
fn fallback_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string()
}
/// What the file system alone says about a video; ffprobe fills in the rest.
fn bare_video(path: &Path, mtime: u64, size: u64) -> Video {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    Video {
        path: path.to_path_buf(),
        mtime,
        size,
        title: fallback_title(path),
        folder: String::new(),
        collection: String::new(),
        year: None,
        duration_secs: 0.0,
        width: 0,
        height: 0,
        fps: 0.0,
        vcodec: String::new(),
        acodec: None,
        channels: 0,
        sample_rate: 0,
        filetype: ext,
        gain_db: None,
    }
}
fn read_video(path: &Path, mtime: u64, size: u64, ffprobe: Option<&Path>) -> Video {
    let mut v = bare_video(path, mtime, size);
    if let Some(ffprobe) = ffprobe
        && let Ok(p) = probe::probe(ffprobe, path)
    {
        if let Some(t) = p.title {
            v.title = t;
        }
        v.year = p.year;
        v.duration_secs = p.duration;
        v.width = p.width;
        v.height = p.height;
        v.fps = p.fps;
        v.vcodec = p.vcodec.unwrap_or_default();
        v.acodec = p.acodec;
        v.channels = p.channels;
        v.sample_rate = p.sample_rate;
        if let Some(f) = p.format {
            v.filetype = f;
        }
    }
    v
}
/// Every video file under each of `roots`, following symlinks and skipping entries
/// whose name starts with a dot. A directory reached twice (through a symlink
/// or a loop) is read once; anything that cannot be read goes in `unreadable`.
/// Overlapping roots are fine: the shared directories are read once.
fn walk(roots: &[PathBuf], files: &mut Vec<PathBuf>, unreadable: &mut Vec<PathBuf>) {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut stack: Vec<PathBuf> = roots.iter().rev().cloned().collect();
    while let Some(dir) = stack.pop() {
        match fs::canonicalize(&dir) {
            Ok(real) => {
                if !seen.insert(real) {
                    continue;
                }
            }
            Err(_) => {
                unreadable.push(dir);
                continue;
            }
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => {
                unreadable.push(dir);
                continue;
            }
        };
        for entry in entries {
            let Ok(entry) = entry else {
                unreadable.push(dir.clone());
                continue;
            };
            let hidden = entry
                .file_name()
                .to_str()
                .map(|n| n.starts_with('.'))
                .unwrap_or(false);
            if hidden {
                continue;
            }
            let path = entry.path();
            match fs::metadata(&path) {
                Ok(m) if m.is_dir() => stack.push(path),
                Ok(m) if m.is_file() => {
                    if is_video(&path) {
                        files.push(path);
                    }
                }
                Ok(_) => {}
                Err(_) => unreadable.push(path),
            }
        }
    }
}
pub fn save(index_file: &Path, videos: &[Video]) -> Result<()> {
    let json = serde_json::to_string(&Index {
        videos: videos.to_vec(),
    })?;
    fs::write(index_file, json)
        .with_context(|| format!("writing index to {}", index_file.display()))
}
/// Checks that ffprobe runs at all, so a missing install is reported once
/// instead of failing silently on every file.
fn check_probe(ffprobe: &Path) -> Result<(), String> {
    match std::process::Command::new(ffprobe)
        .arg("-version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("{} exited with {s}", ffprobe.display())),
        Err(e) => Err(format!("cannot run {}: {e}", ffprobe.display())),
    }
}
/// The index entry for one file: the cached one when the file is unchanged
/// (and was probed), otherwise freshly read.
fn read_entry(old: &HashMap<PathBuf, Video>, path: &Path, prober: Option<&Path>) -> Video {
    let (mtime, size) = fs::metadata(path)
        .map(|m| {
            let mt = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (mt, m.len())
        })
        .unwrap_or((0, 0));
    match old.get(path) {
        // An entry read without ffprobe is worth another look once ffprobe
        // is available.
        Some(t) if t.mtime == mtime && t.size == size && (t.fps > 0.0 || prober.is_none()) => {
            t.clone()
        }
        _ => read_video(path, mtime, size, prober),
    }
}
pub fn build(
    roots: Vec<PathBuf>,
    index_file: PathBuf,
    ffprobe: PathBuf,
    force: bool,
    tx: Sender<IndexMsg>,
) {
    let result = (|| -> Result<Scan> {
        let old = if force {
            HashMap::new()
        } else {
            load_existing(&index_file)
        };
        let probe_failed = check_probe(&ffprobe).err();
        let prober = probe_failed.is_none().then_some(ffprobe.as_path());
        let mut unreadable: Vec<PathBuf> = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();
        walk(&roots, &mut files, &mut unreadable);
        let total = files.len();
        let _ = tx.send(IndexMsg::Progress { done: 0, total });
        // ffprobe is one process per file; several at once keeps a large
        // library from taking minutes.
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(1, 8)
            .min(total.max(1));
        let next = std::sync::atomic::AtomicUsize::new(0);
        let done = std::sync::atomic::AtomicUsize::new(0);
        let mut slots: Vec<Option<Video>> = (0..total).map(|_| None).collect();
        std::thread::scope(|scope| {
            let (old, files, roots, tx, next, done) = (&old, &files, &roots, &tx, &next, &done);
            let handles: Vec<_> = (0..workers)
                .map(|_| {
                    scope.spawn(move || {
                        let mut mine = Vec::new();
                        loop {
                            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some(path) = files.get(i) else { break };
                            let mut video = read_entry(old, path, prober);
                            let root = crate::config::root_of(roots, path).unwrap_or(&roots[0]);
                            video.locate(root);
                            mine.push((i, video));
                            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                            if n % 5 == 0 || n == total {
                                let _ = tx.send(IndexMsg::Progress { done: n, total });
                            }
                        }
                        mine
                    })
                })
                .collect();
            for h in handles {
                for (i, v) in h.join().unwrap_or_default() {
                    slots[i] = Some(v);
                }
            }
        });
        let mut videos: Vec<Video> = slots.into_iter().flatten().collect();
        videos.sort_by_cached_key(|t| t.sort_key());
        save(&index_file, &videos)?;
        Ok(Scan {
            videos,
            unreadable,
            probe_failed,
        })
    })();
    let _ = tx.send(IndexMsg::Done(result));
}
