use super::{AUDIO_CHANNELS, AUDIO_RATE, secs_arg};
use anyhow::{Context, Result};
use rodio::{ChannelCount, Sample, SampleRate, Source};
use std::collections::VecDeque;
use std::io::Read;
use std::num::NonZero;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
/// How much decoded audio to keep ahead of the output; ffmpeg is held back
/// (its pipe fills) beyond this.
const CAP_SECS: f64 = 2.0;
/// Decoded audio needed before playback starts, so ffmpeg's start-up does
/// not play as silence and put the clock ahead of the sound.
const PREBUFFER_SECS: f64 = 0.25;
pub(super) struct Shared {
    queue: Mutex<VecDeque<f32>>,
    eof: AtomicBool,
    /// Samples of silence handed out while the queue was empty; the clock
    /// subtracts them because the file did not advance during that time.
    underrun: AtomicU64,
}
impl Shared {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(VecDeque::with_capacity(
                (CAP_SECS * AUDIO_RATE as f64) as usize * AUDIO_CHANNELS as usize + 65_536,
            )),
            eof: AtomicBool::new(false),
            underrun: AtomicU64::new(0),
        })
    }
    pub(super) fn push(&self, samples: &[f32]) {
        self.queue.lock().unwrap().extend(samples.iter().copied());
    }
    pub(super) fn finish(&self) {
        self.eof.store(true, Ordering::SeqCst);
    }
    pub(super) fn queued(&self) -> usize {
        self.queue.lock().map(|q| q.len()).unwrap_or(0)
    }
    pub(super) fn underrun_secs(&self) -> f64 {
        self.underrun.load(Ordering::Relaxed) as f64 / (AUDIO_RATE as f64 * AUDIO_CHANNELS as f64)
    }
    pub(super) fn ready(&self) -> bool {
        self.eof.load(Ordering::SeqCst)
            || self.queued() >= (PREBUFFER_SECS * AUDIO_RATE as f64) as usize * AUDIO_CHANNELS as usize
    }
}
/// Decodes a file's audio through ffmpeg into a queue a [`PcmSource`] plays.
pub struct AudioFeed {
    shared: Arc<Shared>,
    child: Child,
}
impl AudioFeed {
    pub fn start(ffmpeg: &Path, path: &Path, start_secs: f64) -> Result<Self> {
        let mut child = Command::new(ffmpeg)
            .args(["-nostdin", "-v", "error", "-ss", &secs_arg(start_secs), "-i"])
            .arg(path)
            .args(["-vn", "-sn", "-dn", "-f", "f32le", "-ac"])
            .arg(AUDIO_CHANNELS.to_string())
            .arg("-ar")
            .arg(AUDIO_RATE.to_string())
            .arg("pipe:1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("starting {} for audio", ffmpeg.display()))?;
        let stdout = child.stdout.take().context("ffmpeg stdout")?;
        let shared = Shared::new();
        let reader_shared = shared.clone();
        thread::spawn(move || reader(stdout, reader_shared));
        Ok(Self { shared, child })
    }
    pub fn ready(&self) -> bool {
        self.shared.ready()
    }
    pub fn underrun_secs(&self) -> f64 {
        self.shared.underrun_secs()
    }
    pub fn source(&self) -> PcmSource {
        PcmSource::new(self.shared.clone())
    }
}
impl Drop for AudioFeed {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn reader(mut out: ChildStdout, shared: Arc<Shared>) {
    let cap = (CAP_SECS * AUDIO_RATE as f64) as usize * AUDIO_CHANNELS as usize;
    let mut buf = vec![0u8; 64 * 1024];
    let mut carry: Vec<u8> = Vec::with_capacity(buf.len() + 4);
    let mut samples: Vec<f32> = Vec::with_capacity(buf.len() / 4 + 1);
    loop {
        if shared.queued() > cap {
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        let n = match out.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        carry.extend_from_slice(&buf[..n]);
        let whole = carry.len() / 4 * 4;
        samples.clear();
        samples.extend(
            carry[..whole]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b)),
        );
        shared.push(&samples);
        carry.drain(..whole);
    }
    shared.finish();
}
/// A rodio source fed from an [`AudioFeed`]'s queue. It never blocks the
/// audio thread: an empty queue yields silence (counted as underrun) until
/// ffmpeg catches up or signals the end of the file.
pub struct PcmSource {
    shared: Arc<Shared>,
    local: Vec<f32>,
    pos: usize,
}
const CHUNK: usize = 4096;
/// Samples per span (50 ms). Playback speed works by changing the sample
/// rate a source reports, and rodio's resampler only looks at the rate at
/// the start of a span: an endless span would keep the speed it started at.
const SPAN: usize = (AUDIO_RATE / 20) as usize * AUDIO_CHANNELS as usize;
impl PcmSource {
    pub(super) fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            local: Vec::with_capacity(CHUNK),
            pos: 0,
        }
    }
}
impl Iterator for PcmSource {
    type Item = Sample;
    #[inline]
    fn next(&mut self) -> Option<Sample> {
        if self.pos < self.local.len() {
            let s = self.local[self.pos];
            self.pos += 1;
            return Some(s);
        }
        self.local.clear();
        self.pos = 0;
        // The reader holds the lock only while appending a chunk, so waiting
        // for it is microseconds; a sample of silence instead would click.
        if let Ok(mut q) = self.shared.queue.lock() {
            let n = q.len().min(CHUNK);
            self.local.extend(q.drain(..n));
        }
        if self.local.is_empty() {
            if self.shared.eof.load(Ordering::SeqCst) && self.shared.queued() == 0 {
                return None;
            }
            self.shared.underrun.fetch_add(1, Ordering::Relaxed);
            return Some(0.0);
        }
        self.pos = 1;
        Some(self.local[0])
    }
}
impl Source for PcmSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(SPAN)
    }
    fn channels(&self) -> ChannelCount {
        NonZero::new(AUDIO_CHANNELS).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        NonZero::new(AUDIO_RATE).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
