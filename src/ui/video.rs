//! The picture: text cells drawn straight into the buffer, or a
//! blank area whose position is left for the main loop to send an image
//! escape sequence to (Kitty, iTerm2, sixel).
use super::DIM;
use super::shapes::{draw_mosaic, draw_quads, draw_sextants};
use crate::app::{App, Placement};
use crate::video::term::xterm256;
use crate::video::{Frame as Picture, Protocol};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;
fn centred(f: &mut Frame, area: Rect, text: &str) {
    if area.height == 0 {
        return;
    }
    let y = area.y + area.height / 2;
    let w = (text.chars().count() as u16).min(area.width);
    let x = area.x + (area.width - w) / 2;
    f.render_widget(
        Paragraph::new(text).style(Style::default().fg(DIM)),
        Rect {
            x,
            y,
            width: w,
            height: 1,
        },
    );
}
pub(super) fn draw_video(f: &mut Frame, area: Rect, app: &mut App) {
    let dims = app
        .playing_video()
        .filter(|v| v.has_video())
        .map(|v| (v.width, v.height));
    let has_picture = dims.is_some();
    if area.width < 2 || area.height < 1 || !has_picture {
        app.set_video_target(None);
        if has_picture {
            return;
        }
        if app.playing_video().is_some() {
            centred(f, area, "audio only");
        }
        return;
    }
    let target = app.renderer.target(area.width, area.height);
    app.set_video_target(Some(target));
    let Some(playback) = &app.playback else {
        return;
    };
    let Some(frame) = playback.frame() else {
        centred(f, area, "decoding…");
        return;
    };
    let (vw, vh) = dims.unwrap_or((frame.width, frame.height));
    let (cols, rows) = target.cells(vw, vh);
    let (cols, rows) = (cols.clamp(1, area.width), rows.clamp(1, area.height));
    let x = area.x + (area.width - cols) / 2;
    let y = area.y + (area.height - rows) / 2;
    match app.renderer.protocol {
        Protocol::Blocks => {
            let truecolor = app.renderer.truecolor;
            draw_blocks(f.buffer_mut(), frame, x, y, cols, rows, truecolor);
        }
        Protocol::Quads => {
            let truecolor = app.renderer.truecolor;
            draw_quads(f.buffer_mut(), frame, x, y, cols, rows, truecolor);
        }
        Protocol::Sextants => {
            let truecolor = app.renderer.truecolor;
            draw_sextants(f.buffer_mut(), frame, x, y, cols, rows, truecolor);
        }
        Protocol::Mosaic => {
            let truecolor = app.renderer.truecolor;
            draw_mosaic(f.buffer_mut(), frame, x, y, cols, rows, truecolor);
        }
        Protocol::Ascii => {
            let truecolor = app.renderer.truecolor;
            draw_ascii(f.buffer_mut(), frame, x, y, cols, rows, truecolor);
        }
        _ => {
            let placement = Placement {
                x,
                y,
                cols,
                rows,
                frame_id: frame.id,
            };
            // The cells under the image stay blank and unchanged, so the
            // cell diff leaves the picture alone between frames.
            let buf = f.buffer_mut();
            for cy in y..y + rows {
                for cx in x..x + cols {
                    if let Some(c) = buf.cell_mut((cx, cy)) {
                        c.reset();
                    }
                }
            }
            app.image_out = Some(placement);
        }
    }
}
pub(super) fn colour(p: (u8, u8, u8), truecolor: bool) -> Color {
    if truecolor {
        Color::Rgb(p.0, p.1, p.2)
    } else {
        Color::Indexed(xterm256(p.0, p.1, p.2))
    }
}
/// Two pixel rows per cell: the upper half block in the top pixel's colour
/// on a background of the bottom pixel's colour.
pub fn draw_blocks(
    buf: &mut Buffer,
    frame: &Picture,
    x: u16,
    y: u16,
    cols: u16,
    rows: u16,
    truecolor: bool,
) {
    for cy in 0..rows {
        let py = cy as u32 * 2;
        if py >= frame.height {
            break;
        }
        for cx in 0..cols {
            let px = cx as u32;
            if px >= frame.width {
                break;
            }
            let top = frame.pixel(px, py);
            let bottom = if py + 1 < frame.height {
                frame.pixel(px, py + 1)
            } else {
                (0, 0, 0)
            };
            if let Some(cell) = buf.cell_mut((x + cx, y + cy)) {
                cell.set_char('▀');
                cell.set_fg(colour(top, truecolor));
                cell.set_bg(colour(bottom, truecolor));
            }
        }
    }
}
/// Darkest to brightest; the classic 70-step ramp.
const RAMP: &[u8] = b" .'`^\",:;Il!i><~+_-?][}{1)(|\\/tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$";
/// One character per cell, picked by the pixel's brightness, drawn in the
/// pixel's colour (lifted a little, since thin glyphs read darker than a
/// filled cell would).
pub fn draw_ascii(
    buf: &mut Buffer,
    frame: &Picture,
    x: u16,
    y: u16,
    cols: u16,
    rows: u16,
    truecolor: bool,
) {
    for cy in 0..rows {
        let py = cy as u32;
        if py >= frame.height {
            break;
        }
        for cx in 0..cols {
            let px = cx as u32;
            if px >= frame.width {
                break;
            }
            let (r, g, b) = frame.pixel(px, py);
            let luma = (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0;
            let i = ((luma * (RAMP.len() - 1) as f32).round() as usize).min(RAMP.len() - 1);
            let lift = |v: u8| (v as u32 + (255 - v as u32) / 3).min(255) as u8;
            if let Some(cell) = buf.cell_mut((x + cx, y + cy)) {
                cell.set_char(RAMP[i] as char);
                cell.set_fg(colour((lift(r), lift(g), lift(b)), truecolor));
                cell.set_bg(Color::Black);
            }
        }
    }
}
