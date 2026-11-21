//! Pictures drawn with block characters that split a cell into a shape and
//! the rest: the shape in the foreground colour, the rest in the
//! background, each the average of the pixels it covers.
use super::video::colour;
use crate::video::Frame as Picture;
use ratatui::buffer::Buffer;
use std::sync::OnceLock;
/// Sums of a group of pixels, for the averages and how well they fit.
#[derive(Clone, Copy, Default)]
struct Sum {
    rgb: [i64; 3],
    n: i64,
}
impl Sum {
    fn add(&mut self, p: [i64; 3]) {
        for k in 0..3 {
            self.rgb[k] += p[k];
        }
        self.n += 1;
    }
    fn minus(self, o: Sum) -> Sum {
        Sum {
            rgb: [0, 1, 2].map(|k| self.rgb[k] - o.rgb[k]),
            n: self.n - o.n,
        }
    }
    /// How much painting the group in its average takes away from the
    /// squared error of leaving it blank: of two splits of the same pixels,
    /// the one with the larger total fits better.
    fn fit(self) -> f64 {
        if self.n == 0 {
            return 0.0;
        }
        self.rgb.iter().map(|&s| (s * s) as f64).sum::<f64>() / self.n as f64
    }
    fn mean(self) -> (u8, u8, u8) {
        let n = self.n.max(1);
        let c = |k: usize| (self.rgb[k] / n) as u8;
        (c(0), c(1), c(2))
    }
}
/// The `gw`×`gh` pixels of cell (`cx`, `cy`), row by row.
fn cell_pixels(frame: &Picture, cx: u32, cy: u32, gw: u32, gh: u32, out: &mut Vec<[i64; 3]>) {
    out.clear();
    for sy in 0..gh {
        for sx in 0..gw {
            let px = (cx * gw + sx).min(frame.width - 1);
            let py = (cy * gh + sy).min(frame.height - 1);
            let (r, g, b) = frame.pixel(px, py);
            out.push([r as i64, g as i64, b as i64]);
        }
    }
}
/// Puts the character in the cell: `fg` over `bg`, or one colour throughout
/// when there is no split.
fn put(buf: &mut Buffer, x: u16, y: u16, ch: char, fg: Sum, bg: Sum, truecolor: bool) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        let fg = fg.mean();
        let bg = if bg.n == 0 { fg } else { bg.mean() };
        cell.set_char(ch);
        cell.set_fg(colour(fg, truecolor));
        cell.set_bg(colour(bg, truecolor));
    }
}
/// A small grid per cell (at most 6 pixels), every split of it tried: the
/// best is drawn as `glyph(mask)`, where bit `i` of the mask is pixel `i`
/// in row order and set bits are in the foreground.
#[allow(clippy::too_many_arguments)]
fn draw_grid(
    buf: &mut Buffer,
    frame: &Picture,
    x: u16,
    y: u16,
    cols: u16,
    rows: u16,
    truecolor: bool,
    (gw, gh): (u32, u32),
    glyph: fn(usize) -> char,
) {
    if frame.width == 0 || frame.height == 0 {
        return;
    }
    let n = (gw * gh) as usize;
    let full = (1usize << n) - 1;
    let mut p = Vec::with_capacity(n);
    for cy in 0..rows {
        if cy as u32 * gh >= frame.height {
            break;
        }
        for cx in 0..cols {
            if cx as u32 * gw >= frame.width {
                break;
            }
            cell_pixels(frame, cx as u32, cy as u32, gw, gh, &mut p);
            let mut all = Sum::default();
            p.iter().for_each(|&c| all.add(c));
            // A mask and its complement are the same split with the colours
            // swapped, so masks without the last pixel cover them all. One
            // colour throughout goes first so that it wins a tie.
            let mut best = (all.fit(), full, all);
            for mask in 1..1usize << (n - 1) {
                let mut fg = Sum::default();
                for (i, &c) in p.iter().enumerate() {
                    if mask >> i & 1 == 1 {
                        fg.add(c);
                    }
                }
                let fit = fg.fit() + all.minus(fg).fit();
                if fit > best.0 {
                    best = (fit, mask, fg);
                }
            }
            let (_, mask, fg) = best;
            put(buf, x + cx, y + cy, glyph(mask), fg, all.minus(fg), truecolor);
        }
    }
}
/// The quadrant characters by which quarters are drawn in the foreground
/// colour: bit 0 top left, 1 top right, 2 bottom left, 3 bottom right.
const QUADS: [char; 16] = [
    ' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█',
];
/// The sextant character for a 2×3 mask (bit 0 top left, 1 top right, 2
/// middle left, ... 5 bottom right). Unicode leaves out the four shapes
/// that already had characters: blank, left and right half, full.
pub fn sextant(mask: usize) -> char {
    match mask {
        0 => ' ',
        21 => '▌',
        42 => '▐',
        63 => '█',
        m => {
            let skipped = (m > 21) as u32 + (m > 42) as u32;
            char::from_u32(0x1FB00 + m as u32 - 1 - skipped).unwrap_or('?')
        }
    }
}
/// Four pixels per cell (2×2), drawn with quadrant characters.
pub fn draw_quads(buf: &mut Buffer, frame: &Picture, x: u16, y: u16, cols: u16, rows: u16, truecolor: bool) {
    draw_grid(buf, frame, x, y, cols, rows, truecolor, (2, 2), |m| QUADS[m]);
}
/// Six pixels per cell (2×3), drawn with sextant characters.
pub fn draw_sextants(buf: &mut Buffer, frame: &Picture, x: u16, y: u16, cols: u16, rows: u16, truecolor: bool) {
    draw_grid(buf, frame, x, y, cols, rows, truecolor, (2, 3), sextant);
}
/// Mosaic cells are sampled 8×8; bit `y * 8 + x` of a shape is that sample.
const M: u32 = 8;
/// Every shape mosaic can draw, as the samples its character covers. Only
/// one of a shape and its complement is kept (the colours swap instead),
/// and neither blank nor full, which is the no-split case.
pub fn mosaic_shapes() -> &'static [(u64, char)] {
    static SHAPES: OnceLock<Vec<(u64, char)>> = OnceLock::new();
    SHAPES.get_or_init(|| {
        let mut v: Vec<(u64, char)> = Vec::new();
        let mut push = |mask: u64, ch: char| {
            if mask != 0 && mask != !0 && !v.iter().any(|&(m, _)| m == mask || m == !mask) {
                v.push((mask, ch));
            }
        };
        let area = |f: &dyn Fn(u32, u32) -> bool| {
            let mut m = 0u64;
            for sy in 0..M {
                for sx in 0..M {
                    if f(sx, sy) {
                        m |= 1 << (sy * M + sx);
                    }
                }
            }
            m
        };
        // Lower k eighths ▁..▇ and left k eighths ▏..▉; their complements
        // are the upper and right ones.
        for k in 1..M {
            push(area(&|_, sy| sy >= M - k), char::from_u32(0x2580 + k).unwrap());
            push(area(&|sx, _| sx < k), char::from_u32(0x2590 - k).unwrap());
        }
        for (m, &ch) in QUADS.iter().enumerate() {
            push(area(&|sx, sy| m >> ((sy / 4) * 2 + sx / 4) & 1 == 1), ch);
        }
        // Sextant rows are a third of the cell: samples 0-2, 3-4, 5-7.
        let third = |sy: u32| match sy {
            0..3 => 0,
            3..5 => 1,
            _ => 2,
        };
        for m in 1..63 {
            push(area(&|sx, sy| m >> (third(sy) * 2 + sx / 4) & 1 == 1), sextant(m));
        }
        // Wedges U+1FB3C..U+1FB51: the part of the cell on one side of a
        // line between points of the sextant grid (x in halves, y in
        // thirds), on the side of the bottom corner the name gives. The
        // rest of the range, U+1FB52..U+1FB67, are their complements.
        type Point = (f64, f64);
        const L: Point = (0.0, 1.0);
        const R: Point = (1.0, 1.0);
        let t = 1.0 / 3.0;
        let wedges: [(Point, Point, Point); 22] = [
            ((0.0, 2.0 * t), (0.5, 1.0), L),
            ((0.0, 2.0 * t), (1.0, 1.0), L),
            ((0.0, t), (0.5, 1.0), L),
            ((0.0, t), (1.0, 1.0), L),
            ((0.0, 0.0), (0.5, 1.0), L),
            ((0.0, t), (0.5, 0.0), R),
            ((0.0, t), (1.0, 0.0), R),
            ((0.0, 2.0 * t), (0.5, 0.0), R),
            ((0.0, 2.0 * t), (1.0, 0.0), R),
            ((0.0, 1.0), (0.5, 0.0), R),
            ((0.0, 2.0 * t), (1.0, t), R),
            ((0.5, 1.0), (1.0, 2.0 * t), R),
            ((0.0, 1.0), (1.0, 2.0 * t), R),
            ((0.5, 1.0), (1.0, t), R),
            ((0.0, 1.0), (1.0, t), R),
            ((0.5, 1.0), (1.0, 0.0), R),
            ((0.5, 0.0), (1.0, t), L),
            ((0.0, 0.0), (1.0, t), L),
            ((0.5, 0.0), (1.0, 2.0 * t), L),
            ((0.0, 0.0), (1.0, 2.0 * t), L),
            ((0.5, 0.0), (1.0, 1.0), L),
            ((0.0, t), (1.0, 2.0 * t), L),
        ];
        for (i, &(a, b, corner)) in wedges.iter().enumerate() {
            let side = |q: (f64, f64)| (b.0 - a.0) * (q.1 - a.1) - (b.1 - a.1) * (q.0 - a.0);
            let want = side(corner).signum();
            let mask = area(&|sx, sy| {
                let q = ((sx as f64 + 0.5) / M as f64, (sy as f64 + 0.5) / M as f64);
                side(q).signum() == want
            });
            push(mask, char::from_u32(0x1FB3C + i as u32).unwrap());
        }
        v
    })
}
/// 8×8 pixels per cell matched against [`mosaic_shapes`]: the pixels are
/// split in two by the colour channel that varies most, the shapes closest
/// to that split are scored on how well their two averages fit, and the
/// best is drawn.
pub fn draw_mosaic(buf: &mut Buffer, frame: &Picture, x: u16, y: u16, cols: u16, rows: u16, truecolor: bool) {
    /// Shapes scored exactly per cell, out of the closest by bit count.
    const CANDIDATES: usize = 4;
    /// A spread this small (out of 255) is drawn as one colour.
    const FLAT: i64 = 6;
    if frame.width == 0 || frame.height == 0 {
        return;
    }
    let shapes = mosaic_shapes();
    let mut p = Vec::with_capacity((M * M) as usize);
    for cy in 0..rows {
        if cy as u32 * M >= frame.height {
            break;
        }
        for cx in 0..cols {
            if cx as u32 * M >= frame.width {
                break;
            }
            cell_pixels(frame, cx as u32, cy as u32, M, M, &mut p);
            let mut all = Sum::default();
            p.iter().for_each(|&c| all.add(c));
            let range = |k: usize| {
                let (lo, hi) = p.iter().fold((255, 0), |(lo, hi), c| (c[k].min(lo), c[k].max(hi)));
                (hi - lo, lo + hi)
            };
            let (k, (spread, mid2)) = (0..3)
                .map(|k| (k, range(k)))
                .max_by_key(|&(_, (s, _))| s)
                .unwrap();
            if spread <= FLAT {
                put(buf, x + cx, y + cy, '█', all, Sum::default(), truecolor);
                continue;
            }
            let bits = p
                .iter()
                .enumerate()
                .filter(|(_, c)| c[k] * 2 > mid2)
                .fold(0u64, |b, (i, _)| b | 1 << i);
            let mut near = [(u32::MAX, 0u64, ' '); CANDIDATES];
            for &(m, ch) in shapes {
                let d = (bits ^ m).count_ones();
                let d = d.min(64 - d);
                if let Some(w) = near.iter().position(|n| d < n.0) {
                    near.copy_within(w..CANDIDATES - 1, w + 1);
                    near[w] = (d, m, ch);
                }
            }
            let mut best = (f64::MIN, ' ', all);
            for &(d, m, ch) in &near {
                if d == u32::MAX {
                    continue;
                }
                let mut fg = Sum::default();
                for (i, &c) in p.iter().enumerate() {
                    if m >> i & 1 == 1 {
                        fg.add(c);
                    }
                }
                let fit = fg.fit() + all.minus(fg).fit();
                if fit > best.0 {
                    best = (fit, ch, fg);
                }
            }
            let (_, ch, fg) = best;
            put(buf, x + cx, y + cy, ch, fg, all.minus(fg), truecolor);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    const RED: (u8, u8, u8) = (255, 0, 0);
    const BLUE: (u8, u8, u8) = (0, 0, 255);
    /// A frame of `cells` cells side by side, each `gw`×`gh`, coloured by
    /// `f(cell, x, y)` within the cell.
    fn frame(cells: u32, gw: u32, gh: u32, f: impl Fn(u32, u32, u32) -> (u8, u8, u8)) -> Picture {
        let mut rgb = Vec::new();
        for y in 0..gh {
            for x in 0..cells * gw {
                let p = f(x / gw, x % gw, y);
                rgb.extend_from_slice(&[p.0, p.1, p.2]);
            }
        }
        Picture::new(cells * gw, gh, 0.0, rgb)
    }
    fn cells(buf: &Buffer, n: u16) -> Vec<(String, Color, Color)> {
        (0..n)
            .map(|x| {
                let c = &buf[(x, 0)];
                (c.symbol().to_string(), c.fg, c.bg)
            })
            .collect()
    }
    fn rgb(p: (u8, u8, u8)) -> Color {
        Color::Rgb(p.0, p.1, p.2)
    }
    #[test]
    fn quads_pick_the_split_and_colours_that_fit_the_four_pixels() {
        // Top left alone; the diagonal; one colour throughout.
        let f = frame(3, 2, 2, |c, x, y| match (c, x, y) {
            (0, 0, 0) | (1, 1, 0) | (1, 0, 1) | (2, _, _) => RED,
            _ => BLUE,
        });
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
        draw_quads(&mut buf, &f, 0, 0, 3, 1, true);
        assert_eq!(cells(&buf, 3), [
            ("▘".into(), rgb(RED), rgb(BLUE)),
            ("▞".into(), rgb(RED), rgb(BLUE)),
            ("█".into(), rgb(RED), rgb(RED)),
        ]);
    }
    #[test]
    fn sextant_characters_skip_the_four_older_shapes() {
        assert_eq!(sextant(1), '\u{1FB00}');
        assert_eq!(sextant(20), '\u{1FB13}');
        assert_eq!(sextant(21), '▌');
        assert_eq!(sextant(22), '\u{1FB14}');
        assert_eq!(sextant(42), '▐');
        assert_eq!(sextant(62), '\u{1FB3B}');
        assert_eq!(sextant(63), '█');
    }
    #[test]
    fn sextants_split_six_pixels() {
        // The top row; the middle right alone.
        let f = frame(2, 2, 3, |c, x, y| match (c, x, y) {
            (0, _, 0) | (1, 1, 1) => RED,
            _ => BLUE,
        });
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        draw_sextants(&mut buf, &f, 0, 0, 2, 1, true);
        assert_eq!(cells(&buf, 2), [
            ("\u{1FB02}".into(), rgb(RED), rgb(BLUE)),
            ("\u{1FB07}".into(), rgb(RED), rgb(BLUE)),
        ]);
    }
    #[test]
    fn mosaic_shapes_are_distinct_and_never_blank_or_full() {
        let s = mosaic_shapes();
        for (i, &(m, ch)) in s.iter().enumerate() {
            assert!(m != 0 && m != !0, "{ch}");
            for &(o, other) in &s[..i] {
                assert!(o != m && o != !m, "{ch} repeats {other}");
            }
        }
        assert!(s.iter().any(|&(_, c)| c == '\u{1FB3C}'));
        assert!(s.iter().any(|&(_, c)| c == '▃'));
    }
    #[test]
    fn mosaic_finds_eighths_wedges_and_flat_cells() {
        // Bottom three eighths; below the line from the upper middle left
        // to the lower right (U+1FB3F); one colour; the left quarter.
        let f = frame(4, 8, 8, |c, x, y| {
            let (fx, fy) = ((x as f64 + 0.5) / 8.0, (y as f64 + 0.5) / 8.0);
            let fg = match c {
                0 => y >= 5,
                1 => fy > 1.0 / 3.0 + fx * 2.0 / 3.0,
                2 => true,
                _ => x < 2,
            };
            if fg { RED } else { BLUE }
        });
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        draw_mosaic(&mut buf, &f, 0, 0, 4, 1, true);
        let got = cells(&buf, 4);
        assert_eq!(got[0], ("▃".into(), rgb(RED), rgb(BLUE)));
        assert_eq!(got[1], ("\u{1FB3F}".into(), rgb(RED), rgb(BLUE)));
        assert_eq!(got[2], ("█".into(), rgb(RED), rgb(RED)));
        assert_eq!(got[3], ("▎".into(), rgb(RED), rgb(BLUE)));
    }
}
