/// Where the UI put the picture this frame, for the graphics protocols:
/// the main loop sends the image escape sequence there after the cells.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placement {
    pub x: u16,
    pub y: u16,
    pub cols: u16,
    pub rows: u16,
    pub frame_id: u64,
}
