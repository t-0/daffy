//! Kitty graphics protocol: the frame is sent as zlib-compressed RGB in
//! base64 chunks of at most 4096 bytes and placed to cover `cols`×`rows`
//! cells at the cursor. `q=2` keeps the terminal from answering, which
//! would otherwise arrive as keystrokes.
use super::super::Frame;
use super::base64;
pub const DELETE_ALL: &[u8] = b"\x1b_Ga=d,d=A,q=2\x1b\\";
const CHUNK: usize = 4096;
pub fn encode(out: &mut Vec<u8>, frame: &Frame, cols: u16, rows: u16, id: u32) {
    let z = miniz_oxide::deflate::compress_to_vec_zlib(&frame.rgb, 1);
    let b64 = base64::encode(&z);
    let bytes = b64.as_bytes();
    let mut first = true;
    let mut i = 0;
    while i < bytes.len() || first {
        let end = (i + CHUNK).min(bytes.len());
        let more = if end < bytes.len() { 1 } else { 0 };
        if first {
            out.extend_from_slice(
                format!(
                    "\x1b_Ga=T,q=2,f=24,o=z,s={},v={},c={cols},r={rows},i={id},C=1,m={more};",
                    frame.width, frame.height
                )
                .as_bytes(),
            );
            first = false;
        } else {
            out.extend_from_slice(format!("\x1b_Gm={more};").as_bytes());
        }
        out.extend_from_slice(&bytes[i..end]);
        out.extend_from_slice(b"\x1b\\");
        i = end;
    }
    // The previous frame used the other id; it is underneath the new one.
    let other = if id == 1 { 2 } else { 1 };
    out.extend_from_slice(format!("\x1b_Ga=d,d=I,i={other},q=2\x1b\\").as_bytes());
}
