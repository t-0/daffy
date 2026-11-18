#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Videos,
    Folders,
    Collections,
}
impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Videos => "Videos",
            Section::Folders => "Folders",
            Section::Collections => "Collections",
        }
    }
}
