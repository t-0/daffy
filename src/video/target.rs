/// What a decoded frame pixel becomes on screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A real pixel, for a graphics protocol.
    Pixels,
    /// Half a cell: a cell is one pixel wide and two tall.
    HalfBlocks,
    /// A cell is a grid of this many pixels across and down, drawn as one
    /// character whose shape splits them into two colours.
    Grid(u32, u32),
    /// A whole cell: one character per pixel.
    Cells,
}
/// Where and how big video frames are wanted: an area of the screen in
/// cells, plus what is known about the terminal, from which the size to
/// decode frames at follows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Target {
    pub cols: u16,
    pub rows: u16,
    /// Pixel size of one terminal cell (a 1:2 guess when unknown).
    pub cell: (u16, u16),
    pub kind: Kind,
    /// Widest frame worth sending, in pixels; limits the bytes per frame
    /// that go through the terminal.
    pub max_width: u32,
    pub max_fps: u32,
}
fn even(v: u32) -> u32 {
    v & !1
}
/// Largest `w:h`-shaped size that fits in `bw`×`bh`, at least 1×1.
pub fn fit(w: u32, h: u32, bw: u32, bh: u32) -> (u32, u32) {
    let (bw, bh) = (bw.max(1), bh.max(1));
    if w == 0 || h == 0 {
        return (bw, bh);
    }
    let scale = (bw as f64 / w as f64).min(bh as f64 / h as f64);
    let fw = ((w as f64 * scale).round() as u32).clamp(1, bw);
    let fh = ((h as f64 * scale).round() as u32).clamp(1, bh);
    (fw, fh)
}
impl Target {
    fn cell_px(&self) -> (u32, u32) {
        (self.cell.0.max(1) as u32, self.cell.1.max(1) as u32)
    }
    /// Pixel size to decode frames of a `vw`×`vh` video at.
    pub fn frame_size(&self, vw: u32, vh: u32) -> (u32, u32) {
        let (cw, ch) = self.cell_px();
        let (pw, ph) = fit(vw, vh, self.cols as u32 * cw, self.rows as u32 * ch);
        match self.kind {
            Kind::Pixels => {
                let (pw, ph) = if pw > self.max_width {
                    let s = self.max_width as f64 / pw as f64;
                    (self.max_width, ((ph as f64 * s).round() as u32).max(1))
                } else {
                    (pw, ph)
                };
                (even(pw).max(2), even(ph).max(2))
            }
            Kind::HalfBlocks => {
                let w = ((pw as f64 / cw as f64).round() as u32).clamp(1, self.cols.max(1) as u32);
                let h = ((ph as f64 * 2.0 / ch as f64).round() as u32)
                    .clamp(2, (self.rows.max(1) as u32) * 2);
                (w, even(h))
            }
            Kind::Grid(gw, gh) => {
                let w = ((pw as f64 * gw as f64 / cw as f64).round() as u32)
                    .clamp(gw, (self.cols.max(1) as u32) * gw);
                let h = ((ph as f64 * gh as f64 / ch as f64).round() as u32)
                    .clamp(gh, (self.rows.max(1) as u32) * gh);
                (w / gw * gw, h / gh * gh)
            }
            Kind::Cells => {
                let w = ((pw as f64 / cw as f64).round() as u32).clamp(1, self.cols.max(1) as u32);
                let h = ((ph as f64 / ch as f64).round() as u32).clamp(1, self.rows.max(1) as u32);
                (w, h)
            }
        }
    }
    /// Cells the picture of a `vw`×`vh` video covers on screen: the whole
    /// fitted area, whatever size the frames were decoded at (the terminal
    /// scales them to the cells; blocks are one frame pixel per half cell).
    pub fn cells(&self, vw: u32, vh: u32) -> (u16, u16) {
        let (cw, ch) = self.cell_px();
        match self.kind {
            Kind::Pixels => {
                let (pw, ph) = fit(vw, vh, self.cols as u32 * cw, self.rows as u32 * ch);
                (
                    (pw.div_ceil(cw) as u16).clamp(1, self.cols.max(1)),
                    (ph.div_ceil(ch) as u16).clamp(1, self.rows.max(1)),
                )
            }
            Kind::HalfBlocks => {
                let (w, h) = self.frame_size(vw, vh);
                (
                    (w as u16).min(self.cols),
                    (h.div_ceil(2) as u16).min(self.rows),
                )
            }
            Kind::Grid(gw, gh) => {
                let (w, h) = self.frame_size(vw, vh);
                (
                    ((w / gw) as u16).min(self.cols),
                    ((h / gh) as u16).min(self.rows),
                )
            }
            Kind::Cells => {
                let (w, h) = self.frame_size(vw, vh);
                ((w as u16).min(self.cols), (h as u16).min(self.rows))
            }
        }
    }
}
