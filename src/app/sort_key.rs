#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Folder,
    Title,
    Newest,
    Length,
}
impl SortKey {
    pub const ALL: [SortKey; 4] = [
        SortKey::Folder,
        SortKey::Title,
        SortKey::Newest,
        SortKey::Length,
    ];
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Folder => "Folder",
            SortKey::Title => "Title",
            SortKey::Newest => "Newest",
            SortKey::Length => "Length",
        }
    }
    pub fn describe(self) -> &'static str {
        match self {
            SortKey::Folder => "collection, then folder, then file name",
            SortKey::Title => "title, then folder",
            SortKey::Newest => "most recently changed first",
            SortKey::Length => "longest first",
        }
    }
    /// Key that picks this order inside the sort popup.
    pub fn shortcut(self) -> char {
        match self {
            SortKey::Folder => 'f',
            SortKey::Title => 't',
            SortKey::Newest => 'n',
            SortKey::Length => 'l',
        }
    }
}
