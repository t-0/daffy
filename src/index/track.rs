use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Video {
    pub path: PathBuf,
    pub mtime: u64,
    pub size: u64,
    pub title: String,
    /// The folder the file is in, relative to `video.home` and shown with
    /// `/` separators; the name of `video.home` itself for files at the top.
    #[serde(default)]
    pub folder: String,
    /// The top-level folder under `video.home` the file lives in (the name
    /// of `video.home` itself for files at the top).
    #[serde(default)]
    pub collection: String,
    pub year: Option<u32>,
    pub duration_secs: f64,
    /// Picture size as displayed (rotation applied); 0×0 when the file has
    /// no video stream.
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub fps: f64,
    #[serde(default)]
    pub vcodec: String,
    /// Audio codec, or `None` for a silent file.
    #[serde(default)]
    pub acodec: Option<String>,
    #[serde(default)]
    pub channels: u32,
    #[serde(default)]
    pub sample_rate: u32,
    pub filetype: String,
    /// Auto-normalisation gain in dB, measured the first time the video is
    /// played with normalisation on (see `video::measure_gain`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f32>,
}
impl Video {
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
    pub fn has_video(&self) -> bool {
        self.width > 0 && self.height > 0
    }
    pub fn has_audio(&self) -> bool {
        self.acodec.is_some()
    }
    /// Library order: collection, folder, then file name, so episodes in a
    /// folder stay in their numbered order whatever their titles say.
    pub fn sort_key(&self) -> (String, String, String) {
        (
            self.collection.to_lowercase(),
            self.folder.to_lowercase(),
            self.file_name().to_lowercase(),
        )
    }
    /// Fills `folder` and `collection` from the path's position under `root`.
    pub fn locate(&mut self, root: &Path) {
        let (folder, collection) = locate(root, &self.path);
        self.folder = folder;
        self.collection = collection;
    }
}
fn root_name(root: &Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| root.display().to_string())
}
/// (folder, collection) for a file under `root`; see [`Video::folder`].
pub fn locate(root: &Path, path: &Path) -> (String, String) {
    let parent = path.parent().unwrap_or(path);
    let rel = parent.strip_prefix(root).unwrap_or(parent);
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        let name = root_name(root);
        return (name.clone(), name);
    }
    (parts.join("/"), parts[0].clone())
}
