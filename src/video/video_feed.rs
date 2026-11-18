use super::{Frame, secs_arg};
use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::thread;
/// Frames decoded ahead of the clock. Small: the terminal shows one at a
/// time and a seek throws them all away.
const QUEUE: usize = 3;
/// Decodes a file's pictures through ffmpeg, scaled to the wanted size and
/// rate, and hands out the frame due at a given time.
pub struct VideoFeed {
    rx: Receiver<Arc<Frame>>,
    pending: Option<Arc<Frame>>,
    current: Option<Arc<Frame>>,
    child: Child,
    pub fps: f64,
    eof: bool,
    last_pts: f64,
}
impl VideoFeed {
    pub fn start(
        ffmpeg: &Path,
        path: &Path,
        start_secs: f64,
        size: (u32, u32),
        fps: f64,
    ) -> Result<Self> {
        let fps = if fps.is_finite() && fps > 0.0 {
            fps
        } else {
            24.0
        };
        let filter = format!(
            "fps={fps:.3},scale={}:{}:flags=area",
            size.0.max(1),
            size.1.max(1)
        );
        let mut child = Command::new(ffmpeg)
            .args(["-nostdin", "-v", "error", "-ss", &secs_arg(start_secs), "-i"])
            .arg(path)
            .args([
                "-an",
                "-sn",
                "-dn",
                "-vf",
                &filter,
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("starting {} for video", ffmpeg.display()))?;
        let stdout = child.stdout.take().context("ffmpeg stdout")?;
        let (tx, rx) = sync_channel(QUEUE);
        thread::spawn(move || reader(stdout, tx, size, start_secs, fps));
        Ok(Self {
            rx,
            pending: None,
            current: None,
            child,
            fps,
            eof: false,
            last_pts: start_secs,
        })
    }
    /// Moves to the frame due at `pos` seconds, dropping any older ones.
    /// Returns true when the picture changed.
    pub fn advance(&mut self, pos: f64) -> bool {
        let mut changed = false;
        let slack = 0.5 / self.fps;
        loop {
            if self.pending.is_none() {
                match self.rx.try_recv() {
                    Ok(f) => self.pending = Some(f),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.eof = true;
                        break;
                    }
                }
            }
            match &self.pending {
                Some(f) if f.pts <= pos + slack => {
                    self.last_pts = f.pts;
                    self.current = self.pending.take();
                    changed = true;
                }
                _ => break,
            }
        }
        changed
    }
    pub fn frame(&self) -> Option<&Arc<Frame>> {
        self.current.as_ref()
    }
    /// The decoder has delivered its last frame and that frame has had its
    /// time on screen.
    pub fn finished(&self, pos: f64) -> bool {
        self.eof && self.pending.is_none() && pos >= self.last_pts + 1.0 / self.fps
    }
}
impl Drop for VideoFeed {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn reader(mut out: ChildStdout, tx: SyncSender<Arc<Frame>>, size: (u32, u32), start: f64, fps: f64) {
    let bytes = (size.0 * size.1 * 3) as usize;
    let mut n = 0u64;
    loop {
        let mut rgb = vec![0u8; bytes];
        if out.read_exact(&mut rgb).is_err() {
            return;
        }
        let pts = start + n as f64 / fps;
        n += 1;
        if tx.send(Arc::new(Frame::new(size.0, size.1, pts, rgb))).is_err() {
            return;
        }
    }
}
