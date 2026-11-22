//! Sixel graphics with a fixed 6×7×6 palette and ordered dithering. Each
//! band of six pixel rows is written once per colour present in it, with
//! runs compressed.
use super::super::Frame;
const R_LEVELS: u32 = 6;
const G_LEVELS: u32 = 7;
const B_LEVELS: u32 = 6;
pub const COLORS: usize = (R_LEVELS * G_LEVELS * B_LEVELS) as usize;
const BAYER: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
#[inline]
fn level(v: u8, levels: u32, dither: i32) -> u32 {
    let step = 255.0 / (levels - 1) as f32;
    // Push the value up to half a step either way before rounding.
    let d = (dither as f32 / 16.0 - 0.5) * step;
    ((v as f32 + d) / step)
        .round()
        .clamp(0.0, (levels - 1) as f32) as u32
}
/// Palette index of a pixel at (`x`, `y`).
#[inline]
pub fn quantize(r: u8, g: u8, b: u8, x: u32, y: u32) -> usize {
    let d = BAYER[(y & 3) as usize][(x & 3) as usize];
    let ri = level(r, R_LEVELS, d);
    let gi = level(g, G_LEVELS, d);
    let bi = level(b, B_LEVELS, d);
    ((ri * G_LEVELS + gi) * B_LEVELS + bi) as usize
}
/// RGB of a palette entry, as 0..=100 percentages the way sixel wants them.
pub fn palette(i: usize) -> (u32, u32, u32) {
    let i = i as u32;
    let b = i % B_LEVELS;
    let g = (i / B_LEVELS) % G_LEVELS;
    let r = i / (B_LEVELS * G_LEVELS);
    (
        r * 100 / (R_LEVELS - 1),
        g * 100 / (G_LEVELS - 1),
        b * 100 / (B_LEVELS - 1),
    )
}
fn push_run(out: &mut Vec<u8>, ch: u8, n: usize) {
    if n > 3 {
        out.extend_from_slice(format!("!{n}").as_bytes());
        out.push(ch);
    } else {
        for _ in 0..n {
            out.push(ch);
        }
    }
}
pub fn encode(out: &mut Vec<u8>, frame: &Frame) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    // P2=1: pixels a colour does not set are left alone, so colours in the
    // same band do not erase each other.
    out.extend_from_slice(format!("\x1bP0;1;0q\"1;1;{w};{h}").as_bytes());
    for i in 0..COLORS {
        let (r, g, b) = palette(i);
        out.extend_from_slice(format!("#{i};2;{r};{g};{b}").as_bytes());
    }
    let mut bits = vec![0u8; COLORS * w];
    let mut used = vec![false; COLORS];
    let mut y0 = 0;
    while y0 < h {
        bits.fill(0);
        used.fill(false);
        for row in 0..6.min(h - y0) {
            let y = y0 + row;
            for x in 0..w {
                let (r, g, b) = frame.pixel(x as u32, y as u32);
                let c = quantize(r, g, b, x as u32, y as u32);
                bits[c * w + x] |= 1 << row;
                used[c] = true;
            }
        }
        for c in 0..COLORS {
            if !used[c] {
                continue;
            }
            out.extend_from_slice(format!("#{c}").as_bytes());
            let line = &bits[c * w..(c + 1) * w];
            let last = line
                .iter()
                .rposition(|&b| b != 0)
                .map(|p| p + 1)
                .unwrap_or(0);
            let mut i = 0;
            while i < last {
                let ch = line[i];
                let mut j = i + 1;
                while j < last && line[j] == ch {
                    j += 1;
                }
                push_run(out, 63 + ch, j - i);
                i = j;
            }
            out.push(b'$');
        }
        out.push(b'-');
        y0 += 6;
    }
    out.extend_from_slice(b"\x1b\\");
}
