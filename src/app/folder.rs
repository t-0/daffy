/// A run of videos in the same folder (the "album").
#[derive(Clone, Debug)]
pub struct Folder {
    pub name: String,
    pub collection: String,
    pub year: Option<u32>,
    pub start: usize,
    pub end: usize,
}
