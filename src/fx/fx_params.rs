use super::{EqParams, StereoMode};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
/// Preamp gain range in dB. It sits in front of the whole chain.
pub const PREAMP_MIN_DB: i8 = -24;
pub const PREAMP_MAX_DB: i8 = 12;
/// Where auto-normalisation tries to put a video's gated RMS level, in dBFS,
/// and how far it is allowed to move a video to get there.
pub const NORMALIZE_TARGET_DB: f32 = -18.0;
pub const NORMALIZE_RANGE_DB: (f32, f32) = (-18.0, 12.0);
fn default_width() -> u8 {
    150
}
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct FxParams {
    /// Gain in dB applied before everything else.
    #[serde(default)]
    pub preamp: i8,
    /// Add each video's measured normalisation gain to the preamp.
    #[serde(default)]
    pub auto_normalize: bool,
    /// Normalisation gain for the current video in dB, set by the app per
    /// video; it is state, not a setting, so it is never saved.
    #[serde(skip)]
    pub track_gain: f32,
    #[serde(default)]
    pub eq: EqParams,
    #[serde(default)]
    pub loudness: bool,
    #[serde(default)]
    pub stereo: StereoMode,
    #[serde(default = "default_width")]
    pub width: u8,
}
impl FxParams {
    /// User preamp plus the video's normalisation gain when that is on, in dB.
    pub fn effective_preamp_db(&self) -> f32 {
        let user = self.preamp.clamp(PREAMP_MIN_DB, PREAMP_MAX_DB) as f32;
        if self.auto_normalize {
            user + self
                .track_gain
                .clamp(NORMALIZE_RANGE_DB.0, NORMALIZE_RANGE_DB.1)
        } else {
            user
        }
    }
    pub(super) fn preamp_linear(&self) -> f32 {
        10f32.powf(self.effective_preamp_db() / 20.0)
    }
}
impl Default for FxParams {
    fn default() -> Self {
        Self {
            preamp: 0,
            auto_normalize: false,
            track_gain: 0.0,
            eq: EqParams::default(),
            loudness: false,
            stereo: StereoMode::Normal,
            width: default_width(),
        }
    }
}
pub type SharedParams = Arc<Mutex<FxParams>>;
