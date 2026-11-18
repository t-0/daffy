use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// One decoded picture: packed 8-bit RGB, `width * height * 3` bytes.
pub struct Frame {
    /// Unique per frame, so renderers can tell a new picture from a redraw.
    pub id: u64,
    pub width: u32,
    pub height: u32,
    /// Presentation time in seconds from the start of the file.
    pub pts: f64,
    pub rgb: Vec<u8>,
}
impl Frame {
    pub fn new(width: u32, height: u32, pts: f64, rgb: Vec<u8>) -> Self {
        debug_assert_eq!(rgb.len(), (width * height * 3) as usize);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            width,
            height,
            pts,
            rgb,
        }
    }
    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> (u8, u8, u8) {
        let i = ((y * self.width + x) * 3) as usize;
        (self.rgb[i], self.rgb[i + 1], self.rgb[i + 2])
    }
}
