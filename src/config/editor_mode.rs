use anyhow::{Result, anyhow};
/// How `video.editor` behaves once launched (`video.editor.mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EditorMode {
    /// The editor is a desktop app: start it and carry on.
    #[default]
    Gui,
    /// The editor runs in this terminal: hand the screen over and wait for it
    /// to finish before the UI comes back.
    Cli,
}
impl EditorMode {
    pub(super) fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "gui" => Ok(Self::Gui),
            "cli" | "tui" | "terminal" => Ok(Self::Cli),
            other => Err(anyhow!(
                "video.editor.mode must be `gui` or `cli`, not `{other}`"
            )),
        }
    }
}
