use super::audio_feed::AudioFeed;
use super::clock::Clock;
use super::video_feed::VideoFeed;
use super::{Frame, Target};
use crate::fx::{Fx, FxParams, SharedParams};
use crate::index::Video;
use anyhow::{Context, Result};
use rodio::cpal::BufferSize;
use rodio::{DeviceSinkBuilder, MixerDeviceSink};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
/// Frames per output period, when `DAFFY_AUDIO_BUFFER` asks for a fixed one.
/// Otherwise the audio backend's default applies.
fn buffer_size() -> Option<u32> {
    std::env::var("DAFFY_AUDIO_BUFFER")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|&n| n > 0)
}
fn open_output() -> Result<MixerDeviceSink, rodio::DeviceSinkError> {
    // `from_default_device` picks a fixed 50 ms buffer, which CoreAudio
    // refuses on devices with a smaller maximum; `BufferSize::Default`
    // leaves the period to the backend.
    let first = DeviceSinkBuilder::from_default_device().and_then(|b| {
        let size = match buffer_size() {
            Some(frames) => BufferSize::Fixed(frames),
            None => BufferSize::Default,
        };
        b.with_buffer_size(size).open_stream()
    });
    match first {
        Ok(s) => Ok(s),
        Err(_) => DeviceSinkBuilder::open_default_sink(),
    }
}
/// What playback needs to know about a file, copied out of the index.
#[derive(Clone, Debug)]
pub struct Media {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_audio: bool,
    pub duration: f64,
}
impl From<&Video> for Media {
    fn from(v: &Video) -> Self {
        Self {
            path: v.path.clone(),
            width: v.width,
            height: v.height,
            fps: v.fps,
            has_audio: v.has_audio(),
            duration: v.duration_secs,
        }
    }
}
/// How long a changed video area has to stay the same before the decoder is
/// restarted for it; terminal resizes arrive as bursts.
const RETARGET_DELAY: Duration = Duration::from_millis(300);
struct Session {
    media: Media,
    /// File position the audio decoder started from; rodio's position is
    /// relative to it.
    base: f64,
    audio: Option<AudioFeed>,
    audio_started: bool,
    /// Clock for files without audio.
    clock: Clock,
    feed: Option<VideoFeed>,
    feed_target: Option<Target>,
}
/// Plays one video at a time: audio through ffmpeg → FX chain → rodio, and
/// pictures through a second ffmpeg sized for the screen, kept in step with
/// the audio clock.
pub struct Playback {
    _stream: MixerDeviceSink,
    player: rodio::Player,
    fx: SharedParams,
    ffmpeg: PathBuf,
    session: Option<Session>,
    speed: f32,
    paused: bool,
    wanted: Option<Target>,
    wanted_since: Instant,
    /// The last problem starting a decoder, for the status line.
    pub error: Option<String>,
}
impl Playback {
    pub fn new(fx: FxParams, ffmpeg: &Path) -> Result<Self> {
        let mut stream = open_output().context("opening default audio output")?;
        stream.log_on_drop(false);
        let player = rodio::Player::connect_new(stream.mixer());
        if std::env::var_os("DAFFY_MUTE").is_some() {
            player.set_volume(0.0);
        }
        Ok(Self {
            _stream: stream,
            player,
            fx: Arc::new(Mutex::new(fx)),
            ffmpeg: ffmpeg.to_path_buf(),
            session: None,
            speed: 1.0,
            paused: false,
            wanted: None,
            wanted_since: Instant::now(),
            error: None,
        })
    }
    pub fn set_fx(&self, p: FxParams) {
        if let Ok(mut g) = self.fx.lock() {
            *g = p;
        }
    }
    /// Starts `media` from `pos` seconds, playing.
    pub fn load(&mut self, media: Media, pos: f64) -> Result<()> {
        self.stop();
        self.paused = false;
        self.player.play();
        self.start_session(media, pos)
    }
    fn start_session(&mut self, media: Media, pos: f64) -> Result<()> {
        let pos = pos.max(0.0);
        let audio = if media.has_audio {
            Some(AudioFeed::start(&self.ffmpeg, &media.path, pos)?)
        } else {
            None
        };
        let mut clock = Clock::new(pos, self.speed as f64);
        if self.paused {
            clock.pause();
        }
        self.session = Some(Session {
            media,
            base: pos,
            audio,
            audio_started: false,
            clock,
            feed: None,
            feed_target: None,
        });
        self.error = None;
        Ok(())
    }
    pub fn stop(&mut self) {
        self.session = None;
        self.player.clear();
    }
    pub fn is_loaded(&self) -> bool {
        self.session.is_some()
    }
    pub fn pause(&mut self) {
        self.paused = true;
        self.player.pause();
        if let Some(s) = &mut self.session {
            s.clock.pause();
        }
    }
    pub fn resume(&mut self) {
        self.paused = false;
        self.player.play();
        if let Some(s) = &mut self.session {
            s.clock.resume();
        }
    }
    pub fn toggle_pause(&mut self) {
        if self.paused {
            self.resume();
        } else {
            self.pause();
        }
    }
    pub fn is_paused(&self) -> bool {
        self.paused
    }
    /// Current position in the file, in seconds.
    pub fn position(&self) -> f64 {
        let Some(s) = &self.session else {
            return 0.0;
        };
        match &s.audio {
            Some(a) if s.audio_started => {
                (s.base + self.player.get_pos().as_secs_f64() - a.underrun_secs()).max(s.base)
            }
            Some(_) => s.base,
            None => s.clock.now(),
        }
    }
    /// The file has played to its end.
    pub fn finished(&self) -> bool {
        let Some(s) = &self.session else {
            return false;
        };
        if s.audio.is_some() {
            return s.audio_started && self.player.empty();
        }
        let pos = s.clock.now();
        match &s.feed {
            Some(f) => f.finished(pos) || (s.media.duration > 0.0 && pos >= s.media.duration + 1.0),
            None => s.media.duration <= 0.0 || pos >= s.media.duration,
        }
    }
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
        self.player.set_speed(speed);
        if let Some(s) = &mut self.session {
            s.clock.set_speed(speed as f64);
        }
    }
    pub fn seek_to(&mut self, pos: f64) -> Result<()> {
        let Some(s) = &self.session else {
            return Ok(());
        };
        let media = s.media.clone();
        let target = s.feed_target;
        self.session = None;
        self.player.clear();
        if !self.paused {
            self.player.play();
        }
        self.start_session(media, pos)?;
        if let (Some(t), Some(s)) = (target, &mut self.session) {
            Self::start_feed(&self.ffmpeg, s, t, pos);
        }
        Ok(())
    }
    pub fn seek_by(&mut self, delta_secs: f64, total_secs: f64) -> Result<()> {
        let cur = self.position();
        let mut target = cur + delta_secs;
        if target < 0.0 {
            target = 0.0;
        }
        if total_secs > 0.0 && target > total_secs - 0.5 {
            target = (total_secs - 0.5).max(0.0);
        }
        self.seek_to(target)
    }
    /// Where the UI wants pictures drawn, or `None` while no video area is
    /// on screen. The decoder is (re)started for it after a short settle.
    pub fn set_target(&mut self, target: Option<Target>) {
        if target != self.wanted {
            self.wanted = target;
            self.wanted_since = Instant::now();
        }
    }
    fn start_feed(ffmpeg: &Path, s: &mut Session, t: Target, pos: f64) {
        let size = t.frame_size(s.media.width, s.media.height);
        let fps = if s.media.fps > 0.0 {
            s.media.fps.min(t.max_fps as f64)
        } else {
            (t.max_fps as f64).min(24.0)
        };
        s.feed = VideoFeed::start(ffmpeg, &s.media.path, pos, size, fps).ok();
        s.feed_target = Some(t);
    }
    /// Housekeeping to call every UI tick: starts the audio once enough is
    /// buffered, follows the wanted picture size and picks the frame that
    /// is due.
    pub fn poll(&mut self) {
        let Some(s) = &mut self.session else {
            return;
        };
        if let Some(a) = &s.audio
            && !s.audio_started
            && a.ready()
        {
            let src = Fx::new(a.source(), self.fx.clone());
            self.player.append(src);
            if self.paused {
                self.player.pause();
            } else {
                self.player.play();
            }
            s.audio_started = true;
        }
        if s.media.width > 0 {
            match (self.wanted, s.feed_target) {
                (Some(t), None) => {
                    let pos = self.position();
                    let s = self.session.as_mut().unwrap();
                    Self::start_feed(&self.ffmpeg, s, t, pos);
                }
                (Some(t), Some(cur)) if t != cur => {
                    let same_frames = t.frame_size(s.media.width, s.media.height)
                        == cur.frame_size(s.media.width, s.media.height)
                        && t.max_fps == cur.max_fps;
                    if same_frames {
                        s.feed_target = Some(t);
                    } else if self.wanted_since.elapsed() >= RETARGET_DELAY {
                        let pos = self.position();
                        let s = self.session.as_mut().unwrap();
                        Self::start_feed(&self.ffmpeg, s, t, pos);
                    }
                }
                (None, Some(_)) => {
                    s.feed = None;
                    s.feed_target = None;
                }
                _ => {}
            }
        }
        let pos = self.position();
        if let Some(s) = &mut self.session
            && let Some(f) = &mut s.feed
        {
            f.advance(pos);
        }
    }
    pub fn frame(&self) -> Option<&Arc<Frame>> {
        self.session.as_ref()?.feed.as_ref()?.frame()
    }
}
