mod audio_feed;
mod clock;
mod frame;
mod gain;
mod playback;
pub mod probe;
mod target;
pub mod term;
#[cfg(test)]
mod tests;
mod video_feed;
pub use frame::Frame;
pub use gain::measure_gain;
pub use playback::{Media, Playback};
pub use target::{Kind, Target};
#[cfg(test)]
use target::fit;
pub use term::{Protocol, Renderer};
/// Audio is decoded to this rate and channel count whatever the file holds,
/// so the chain after ffmpeg never has to care.
pub const AUDIO_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: u16 = 2;
/// Seconds as an ffmpeg `-ss` argument.
pub(crate) fn secs_arg(secs: f64) -> String {
    format!("{:.3}", secs.max(0.0))
}
