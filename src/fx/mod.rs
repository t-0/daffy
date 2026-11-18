mod biquad;
mod channel;
mod eq_mode;
mod eq_params;
mod fx_params;
mod stereo_mode;
#[cfg(test)]
mod tests;
use biquad::Biquad;
use channel::Channel;
pub use eq_mode::{EqMode, SEVEN_BANDS, THREE_BANDS};
pub use eq_params::{EQ_RANGE_DB, EqParams};
pub use fx_params::{
    FxParams, NORMALIZE_RANGE_DB, NORMALIZE_TARGET_DB, PREAMP_MAX_DB, PREAMP_MIN_DB, SharedParams,
};
use rodio::{ChannelCount, Sample, SampleRate, Source};
use std::time::Duration;
pub use stereo_mode::{StereoMode, WIDTH_MAX};
/// Time constant of the ramp between preamp gains, so a measurement that
/// arrives mid-video fades in rather than steps.
const GAIN_RAMP_SECS: f32 = 0.3;
/// How often (in frames) the shared parameters are re-read.
const REFRESH_FRAMES: u64 = 256;
#[inline]
fn soft_limit(y: f32) -> f32 {
    const KNEE: f32 = 0.8;
    let a = y.abs();
    if a <= KNEE {
        y
    } else {
        y.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh())
    }
}
/// Parameters derived from `FxParams`, recomputed on every refresh.
#[derive(Clone, Copy, Default)]
struct Derived {
    /// Linear preamp gain; 1.0 is 0 dB.
    preamp: f32,
    eq_bands: usize,
    loudness: bool,
    side_gain: Option<f32>,
}
/// The audio chain: preamp, equaliser, loudness, stereo processing and a
/// soft limiter, applied to an interleaved source of any channel count.
pub struct Fx<S: Source> {
    inner: S,
    params: SharedParams,
    d: Derived,
    channels: usize,
    sr: f32,
    chans: Vec<Channel>,
    out: Vec<f32>,
    out_pos: usize,
    frames: u64,
    /// Current preamp gain, ramping toward `d.preamp`.
    gain: f32,
    gain_coef: f32,
}
impl<S: Source> Fx<S> {
    pub fn new(inner: S, params: SharedParams) -> Self {
        let channels = inner.channels().get() as usize;
        let sr = inner.sample_rate().get() as f32;
        let chans = (0..channels)
            .map(|_| Channel {
                eq: [Biquad::default(); 7],
                loud: [Biquad::default(); 2],
            })
            .collect();
        let gain = params.lock().map(|p| p.preamp_linear()).unwrap_or(1.0);
        let mut fx = Self {
            inner,
            params,
            d: Derived::default(),
            channels,
            sr,
            chans,
            out: Vec::with_capacity(channels),
            out_pos: 0,
            frames: 0,
            gain,
            gain_coef: 1.0 - (-1.0 / (sr * GAIN_RAMP_SECS)).exp(),
        };
        fx.refresh();
        fx
    }
    fn refresh(&mut self) {
        let p = match self.params.try_lock() {
            Ok(p) => *p,
            Err(_) => return,
        };
        let d = Derived {
            preamp: p.preamp_linear(),
            eq_bands: p.eq.mode.bands(),
            loudness: p.loudness,
            side_gain: match p.stereo {
                StereoMode::Normal => None,
                StereoMode::Mono => Some(0.0),
                StereoMode::Width => Some(p.width.min(WIDTH_MAX) as f32 / 100.0),
            },
        };
        let sr = self.sr;
        for c in &mut self.chans {
            match p.eq.mode {
                EqMode::Off => {}
                EqMode::Three => {
                    c.eq[0].set_lowshelf(sr, THREE_BANDS[0].1, p.eq.three[0] as f32);
                    c.eq[1].set_peaking(sr, THREE_BANDS[1].1, 0.7, p.eq.three[1] as f32);
                    c.eq[2].set_highshelf(sr, THREE_BANDS[2].1, p.eq.three[2] as f32);
                }
                EqMode::Seven => {
                    for (i, f) in SEVEN_BANDS.iter().enumerate() {
                        c.eq[i].set_peaking(sr, *f, 1.1, p.eq.seven[i] as f32);
                    }
                }
            }
            if p.loudness {
                c.loud[0].set_lowshelf(sr, 100.0, 7.0);
                c.loud[1].set_highshelf(sr, 10_000.0, 2.5);
            }
        }
        self.d = d;
    }
    fn advance_frame(&mut self) {
        let target = self.d.preamp;
        if self.gain != target {
            self.gain += (target - self.gain) * self.gain_coef;
            if (target - self.gain).abs() < 1e-4 {
                self.gain = target;
            }
        }
    }
    #[inline]
    fn process(&mut self, x: Sample, ch: usize) -> Sample {
        let d = self.d;
        let shape = d.eq_bands > 0 || d.loudness;
        let gain = self.gain;
        if !shape && gain == 1.0 {
            return x;
        }
        let mut y = x * gain;
        if shape {
            let c = &mut self.chans[ch];
            for f in c.eq.iter_mut().take(d.eq_bands) {
                y = f.process(y);
            }
            if d.loudness {
                let bass = c.loud[0].process(y * 0.75);
                y = c.loud[1].process(bass);
            }
        }
        if shape || gain > 1.0 {
            y = soft_limit(y);
        }
        y
    }
}
impl<S: Source> Iterator for Fx<S> {
    type Item = Sample;
    #[inline]
    fn next(&mut self) -> Option<Sample> {
        if self.out_pos < self.out.len() {
            let y = self.out[self.out_pos];
            self.out_pos += 1;
            return Some(y);
        }
        self.out.clear();
        self.out_pos = 0;
        for _ in 0..self.channels {
            match self.inner.next() {
                Some(x) => self.out.push(x),
                None => break,
            }
        }
        if self.out.is_empty() {
            return None;
        }
        if self.frames.is_multiple_of(REFRESH_FRAMES) {
            self.refresh();
        }
        self.frames += 1;
        self.advance_frame();
        for ch in 0..self.out.len() {
            let x = self.out[ch];
            self.out[ch] = self.process(x, ch);
        }
        if let Some(sg) = self.d.side_gain
            && self.out.len() == 2
        {
            let (l, r) = (self.out[0], self.out[1]);
            let mid = 0.5 * (l + r);
            let side = 0.5 * (l - r) * sg;
            self.out[0] = soft_limit(mid + side);
            self.out[1] = soft_limit(mid - side);
        }
        self.out_pos = 1;
        Some(self.out[0])
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}
impl<S: Source> Source for Fx<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(pos)
    }
}
