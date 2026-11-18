//! Everything written to the terminal goes through one thread, so a slow
//! terminal never stalls the UI loop: cell updates are queued, pictures are
//! encoded on the thread and dropped when the previous one is still going
//! out, and playback carries on regardless.
use crate::video::{Frame, Renderer};
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::JoinHandle;
enum Job {
    Bytes(Vec<u8>),
    Image {
        renderer: Renderer,
        frame: Arc<Frame>,
        x: u16,
        y: u16,
        cols: u16,
        rows: u16,
    },
    Sync(Sender<()>),
}
/// A handle on the writer thread; clones share it.
#[derive(Clone)]
pub struct Output {
    tx: SyncSender<Job>,
    /// Pictures queued or being written: at most one, so cell updates
    /// behind it wait for one image at most.
    pending_images: Arc<AtomicUsize>,
}
/// The `Write` end ratatui draws into: bytes are collected until `flush`,
/// then handed to the thread as one job.
pub struct OutputWriter {
    out: Output,
    buf: Vec<u8>,
}
impl Output {
    pub fn start() -> (Self, OutputWriter, JoinHandle<()>) {
        let (tx, rx) = mpsc::sync_channel(16);
        let pending = Arc::new(AtomicUsize::new(0));
        let thread_pending = pending.clone();
        let handle = std::thread::spawn(move || writer(rx, thread_pending));
        let out = Self {
            tx,
            pending_images: pending,
        };
        let writer = OutputWriter {
            out: out.clone(),
            buf: Vec::with_capacity(64 * 1024),
        };
        (out, writer, handle)
    }
    /// Queues a picture unless the terminal is still taking the previous
    /// one. Returns whether it was queued.
    pub fn image(
        &self,
        renderer: Renderer,
        frame: Arc<Frame>,
        x: u16,
        y: u16,
        cols: u16,
        rows: u16,
    ) -> bool {
        if self.pending_images.load(Ordering::SeqCst) > 0 {
            return false;
        }
        self.pending_images.fetch_add(1, Ordering::SeqCst);
        let job = Job::Image {
            renderer,
            frame,
            x,
            y,
            cols,
            rows,
        };
        match self.tx.try_send(job) {
            Ok(()) => true,
            Err(_) => {
                self.pending_images.fetch_sub(1, Ordering::SeqCst);
                false
            }
        }
    }
    /// Queues raw bytes behind whatever is already waiting.
    pub fn raw(&self, bytes: Vec<u8>) {
        let _ = self.tx.send(Job::Bytes(bytes));
    }
    /// Waits until everything queued so far has reached the terminal.
    pub fn sync(&self) {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(Job::Sync(tx)).is_ok() {
            let _ = rx.recv();
        }
    }
}
impl Write for OutputWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let bytes = std::mem::replace(&mut self.buf, Vec::with_capacity(64 * 1024));
        self.out
            .tx
            .send(Job::Bytes(bytes))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "terminal writer gone"))
    }
}
fn writer(rx: Receiver<Job>, pending: Arc<AtomicUsize>) {
    // `Stdout` locks per call, so the terminal stays usable for the code
    // that restores it on exit or hands it to an editor.
    let mut out = io::stdout();
    let mut seq = 0u64;
    for job in rx {
        match job {
            Job::Bytes(b) => {
                let _ = out.write_all(&b);
            }
            Job::Image {
                renderer,
                frame,
                x,
                y,
                cols,
                rows,
            } => {
                let bytes = renderer.encode(&frame, x, y, cols, rows, seq);
                seq += 1;
                let _ = out.write_all(&bytes);
                pending.fetch_sub(1, Ordering::SeqCst);
            }
            Job::Sync(done) => {
                let _ = out.flush();
                let _ = done.send(());
                continue;
            }
        }
        let _ = out.flush();
    }
}
