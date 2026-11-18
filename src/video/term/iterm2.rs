//! iTerm2 inline images: a base64 JPEG, told to span `cols`×`rows` cells.
use super::super::Frame;
use super::{base64, jpeg};
pub fn encode(out: &mut Vec<u8>, frame: &Frame, cols: u16, rows: u16) {
    let png = jpeg::encode(frame, 80);
    out.extend_from_slice(
        format!(
            "\x1b]1337;File=inline=1;size={};width={cols};height={rows};preserveAspectRatio=0;doNotMoveCursor=1:",
            png.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(base64::encode(&png).as_bytes());
    out.push(0x07);
}
