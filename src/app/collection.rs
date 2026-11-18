/// A run of folders under the same top-level folder (the "artist").
#[derive(Clone, Debug)]
pub struct Collection {
    pub name: String,
    pub folder_start: usize,
    pub folder_end: usize,
    pub video_start: usize,
    pub video_end: usize,
}
