//! Drawing pictures in a terminal, as well as the terminal allows: the
//! Kitty graphics protocol, iTerm2 inline images, sixels, or half-block
//! characters when nothing better is available.
pub mod base64;
mod blocks;
mod iterm2;
mod kitty;
pub mod jpeg;
mod query;
mod sixel;
use super::{Frame, Kind, Target};
pub use blocks::xterm256;
pub use query::{Caps, query_terminal};
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protocol {
    /// Two pixels per cell with `▀`, coloured foreground and background.
    Blocks,
    /// Four pixels per cell with the quadrant block characters, the two
    /// colours that fit them best as foreground and background.
    Quads,
    /// Six pixels per cell (2×3) with the sextant characters, likewise.
    Sextants,
    /// 8×8 pixels per cell matched against a set of block, eighth-block,
    /// quadrant, sextant and diagonal wedge shapes, likewise.
    Mosaic,
    /// One character per cell, chosen by brightness, in the pixel's colour.
    Ascii,
    Kitty,
    Iterm2,
    Sixel,
}
impl Protocol {
    pub const ALL: [Protocol; 8] = [
        Protocol::Kitty,
        Protocol::Iterm2,
        Protocol::Sixel,
        Protocol::Blocks,
        Protocol::Quads,
        Protocol::Sextants,
        Protocol::Mosaic,
        Protocol::Ascii,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Protocol::Blocks => "blocks",
            Protocol::Quads => "quads",
            Protocol::Sextants => "sextants",
            Protocol::Mosaic => "mosaic",
            Protocol::Ascii => "ascii",
            Protocol::Kitty => "kitty",
            Protocol::Iterm2 => "iterm2",
            Protocol::Sixel => "sixel",
        }
    }
    pub fn pixels(self) -> bool {
        !matches!(
            self,
            Protocol::Blocks | Protocol::Quads | Protocol::Sextants | Protocol::Mosaic | Protocol::Ascii
        )
    }
    /// Draws into the cell buffer rather than through an image protocol,
    /// so the colour depth matters.
    pub fn is_text(self) -> bool {
        !self.pixels()
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Renderer {
    pub protocol: Protocol,
    /// Whether 24-bit colour escape codes can be used (blocks only).
    pub truecolor: bool,
    /// Pixel size of a cell; a guess unless `cell_known`.
    pub cell: (u16, u16),
    pub cell_known: bool,
}
/// Which Kitty image id the next frame uses; two alternate so the new
/// picture is in place before the old one is deleted.
pub fn kitty_id(seq: u64) -> u32 {
    1 + (seq % 2) as u32
}
impl Renderer {
    /// The `video.renderer` word that selects this renderer.
    pub fn setting_name(&self) -> String {
        match self.protocol {
            p if p.is_text() && !self.truecolor => format!("{}256", p.name()),
            p => p.name().to_string(),
        }
    }
    /// The same terminal with a different way of drawing.
    pub fn with(&self, protocol: Protocol, truecolor: bool) -> Renderer {
        Renderer {
            protocol,
            truecolor,
            ..*self
        }
    }
    pub fn label(&self) -> String {
        match self.protocol {
            p if p.is_text() => format!(
                "{} {}",
                p.name(),
                if self.truecolor { "24-bit" } else { "256" }
            ),
            p => {
                if self.cell_known {
                    format!("{} {}×{}px", p.name(), self.cell.0, self.cell.1)
                } else {
                    p.name().to_string()
                }
            }
        }
    }
    pub fn target(&self, cols: u16, rows: u16) -> Target {
        let (max_width, max_fps) = match self.protocol {
            Protocol::Kitty => (1280, 24),
            Protocol::Iterm2 => (800, 24),
            // Sixels are real pixels: a cap would shrink the picture.
            Protocol::Sixel => (u32::MAX, 15),
            _ => (u32::MAX, 30),
        };
        Target {
            cols,
            rows,
            cell: if self.protocol.is_text() && !self.cell_known {
                (1, 2)
            } else {
                self.cell
            },
            kind: match self.protocol {
                Protocol::Blocks => Kind::HalfBlocks,
                Protocol::Quads => Kind::Grid(2, 2),
                Protocol::Sextants => Kind::Grid(2, 3),
                Protocol::Mosaic => Kind::Grid(8, 8),
                Protocol::Ascii => Kind::Cells,
                _ => Kind::Pixels,
            },
            max_width,
            max_fps,
        }
    }
    /// The escape sequence that draws `frame` with its top-left corner at
    /// cell (`x`, `y`), spanning `cols`×`rows` cells. Graphics protocols
    /// only; blocks are drawn into the cell buffer by the UI.
    pub fn encode(&self, frame: &Frame, x: u16, y: u16, cols: u16, rows: u16, seq: u64) -> Vec<u8> {
        let mut out = format!("\x1b[{};{}H", y + 1, x + 1).into_bytes();
        match self.protocol {
            Protocol::Kitty => kitty::encode(&mut out, frame, cols, rows, kitty_id(seq)),
            Protocol::Iterm2 => iterm2::encode(&mut out, frame, cols, rows),
            Protocol::Sixel => sixel::encode(&mut out, frame),
            _ => {}
        }
        out
    }
    /// What to send when the picture goes away, if anything: Kitty images
    /// live on until deleted, the others are just cells to overwrite.
    pub fn clear_sequence(&self) -> Option<&'static [u8]> {
        match self.protocol {
            Protocol::Kitty => Some(kitty::DELETE_ALL),
            _ => None,
        }
    }
}
/// The environment variables detection looks at, gathered so the rules can
/// be tested.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub term: String,
    pub term_program: String,
    pub colorterm: String,
    pub lc_terminal: String,
    pub kitty_window_id: bool,
    pub tmux: bool,
}
impl Env {
    pub fn current() -> Self {
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        Self {
            term: var("TERM"),
            term_program: var("TERM_PROGRAM"),
            colorterm: var("COLORTERM"),
            lc_terminal: var("LC_TERMINAL"),
            kitty_window_id: std::env::var_os("KITTY_WINDOW_ID").is_some(),
            tmux: std::env::var_os("TMUX").is_some(),
        }
    }
    fn program(&self, name: &str) -> bool {
        self.term_program.eq_ignore_ascii_case(name)
    }
    fn multiplexed(&self) -> bool {
        self.tmux || self.term.starts_with("screen") || self.term.starts_with("tmux")
    }
    fn truecolor(&self) -> bool {
        let ct = self.colorterm.to_ascii_lowercase();
        if ct == "truecolor" || ct == "24bit" {
            return true;
        }
        if self.program("Apple_Terminal") {
            return false;
        }
        self.kitty_window_id
            || self.term.contains("kitty")
            || self.term.contains("direct")
            || ["iTerm.app", "vscode", "WezTerm", "ghostty", "Hyper", "Tabby", "rio", "mintty"]
                .iter()
                .any(|p| self.program(p))
    }
    /// Terminals that speak the iTerm2 inline image protocol.
    fn iterm2(&self) -> bool {
        self.lc_terminal.eq_ignore_ascii_case("iTerm2")
            || ["iTerm.app", "vscode", "WezTerm", "mintty", "Tabby", "rio"]
                .iter()
                .any(|p| self.program(p))
    }
}
/// Picks how to draw video. `forced` is the `video.renderer` setting. The
/// second value is a warning for the status line, when a forced choice had
/// to be adjusted.
pub fn detect(forced: Option<&str>, env: &Env, caps: &Caps) -> (Renderer, Option<String>) {
    let (cell, cell_known) = match caps.cell {
        Some(c) if c.0 > 0 && c.1 > 0 => (c, true),
        _ => ((10, 20), false),
    };
    let make = |protocol, truecolor| Renderer {
        protocol,
        truecolor,
        cell,
        cell_known,
    };
    let truecolor = env.truecolor();
    if let Some(f) = forced {
        return match f {
            "blocks" => (make(Protocol::Blocks, true), None),
            "blocks256" => (make(Protocol::Blocks, false), None),
            "quads" => (make(Protocol::Quads, true), None),
            "quads256" => (make(Protocol::Quads, false), None),
            "sextants" => (make(Protocol::Sextants, true), None),
            "sextants256" => (make(Protocol::Sextants, false), None),
            "mosaic" => (make(Protocol::Mosaic, true), None),
            "mosaic256" => (make(Protocol::Mosaic, false), None),
            "ascii" => (make(Protocol::Ascii, true), None),
            "ascii256" => (make(Protocol::Ascii, false), None),
            "kitty" => (make(Protocol::Kitty, truecolor), None),
            "iterm2" | "iterm" => (make(Protocol::Iterm2, truecolor), None),
            "sixel" if cell_known => (make(Protocol::Sixel, truecolor), None),
            "sixel" => (
                make(Protocol::Blocks, truecolor),
                Some("sixel needs the terminal to report its cell size; using blocks".into()),
            ),
            other => (
                make(Protocol::Blocks, truecolor),
                Some(format!("unknown video.renderer `{other}`; using blocks")),
            ),
        };
    }
    if env.multiplexed() {
        // Graphics would need passthrough configured in the multiplexer.
        return (make(Protocol::Blocks, truecolor), None);
    }
    if caps.kitty || env.kitty_window_id || env.term == "xterm-kitty" || env.program("ghostty") {
        return (make(Protocol::Kitty, truecolor), None);
    }
    if env.iterm2() {
        return (make(Protocol::Iterm2, truecolor), None);
    }
    if caps.sixel && cell_known {
        return (make(Protocol::Sixel, truecolor), None);
    }
    (make(Protocol::Blocks, truecolor), None)
}
#[cfg(test)]
pub use query::parse_replies;
#[cfg(test)]
pub use sixel::{palette as sixel_palette, quantize as sixel_index};
