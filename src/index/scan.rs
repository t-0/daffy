use super::Video;
use std::path::PathBuf;
pub struct Scan {
    pub videos: Vec<Video>,
    pub unreadable: Vec<PathBuf>,
    /// ffprobe could not be run, so the entries only know what the file
    /// system says about them.
    pub probe_failed: Option<String>,
}
