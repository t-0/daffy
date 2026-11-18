/// What the three list cursors were on, by identity rather than index, so
/// they can be put back after the video list is rebuilt.
pub(super) struct ViewState {
    pub(super) video: Option<std::path::PathBuf>,
    pub(super) folder: Option<String>,
    pub(super) collection: Option<String>,
    pub(super) offsets: [usize; 3],
}
