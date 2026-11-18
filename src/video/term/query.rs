//! Asking the terminal what it can do. The replies arrive on stdin, so this
//! runs in raw mode before the event loop starts reading keys.
/// What the terminal said about itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Caps {
    /// Answered a Kitty graphics query.
    pub kitty: bool,
    /// Lists sixel support among its device attributes.
    pub sixel: bool,
    /// Pixel size of a cell (width, height).
    pub cell: Option<(u16, u16)>,
}
/// Cell size in pixels, Kitty graphics probe, then primary device
/// attributes, which every terminal answers, so its reply marks the end.
const QUERY: &[u8] = b"\x1b[16t\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c";
pub fn parse_replies(buf: &[u8]) -> (Caps, bool) {
    let s = String::from_utf8_lossy(buf);
    let mut caps = Caps::default();
    let mut done = false;
    if s.contains("\x1b_Gi=31;OK") {
        caps.kitty = true;
    }
    // CSI 6 ; height ; width t
    if let Some(i) = s.find("\x1b[6;") {
        let rest = &s[i + 4..];
        if let Some(end) = rest.find('t') {
            let mut it = rest[..end].split(';');
            if let (Some(h), Some(w)) = (
                it.next().and_then(|v| v.parse::<u16>().ok()),
                it.next().and_then(|v| v.parse::<u16>().ok()),
            ) && w > 0
                && h > 0
            {
                caps.cell = Some((w, h));
            }
        }
    }
    // CSI ? 6x ; ... c
    let mut search: &str = &s;
    while let Some(i) = search.find("\x1b[?") {
        let rest = &search[i + 3..];
        if let Some(end) = rest.find('c') {
            let body = &rest[..end];
            if body.chars().all(|c| c.is_ascii_digit() || c == ';') {
                done = true;
                if body.split(';').any(|a| a == "4") {
                    caps.sixel = true;
                }
            }
            search = &rest[end + 1..];
        } else {
            break;
        }
    }
    (caps, done)
}
#[cfg(unix)]
pub fn query_terminal() -> Caps {
    use std::io::Write;
    use std::time::{Duration, Instant};
    unsafe {
        if libc::isatty(0) == 0 || libc::isatty(1) == 0 {
            return Caps::default();
        }
    }
    let mut out = std::io::stdout();
    if out.write_all(QUERY).and_then(|_| out.flush()).is_err() {
        return Caps::default();
    }
    let deadline = Instant::now() + Duration::from_millis(700);
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let mut pfd = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        let r = unsafe { libc::poll(&mut pfd, 1, left.as_millis() as i32) };
        if r <= 0 {
            break;
        }
        let n = unsafe { libc::read(0, tmp.as_mut_ptr() as *mut libc::c_void, tmp.len()) };
        if n <= 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n as usize]);
        let (_, done) = parse_replies(&buf);
        if done {
            // Anything that trails the final reply would otherwise be read
            // as keystrokes; give stragglers a moment and swallow them.
            let mut pfd = libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            };
            while unsafe { libc::poll(&mut pfd, 1, 60) } > 0 {
                let n = unsafe { libc::read(0, tmp.as_mut_ptr() as *mut libc::c_void, tmp.len()) };
                if n <= 0 {
                    break;
                }
            }
            break;
        }
    }
    parse_replies(&buf).0
}
#[cfg(not(unix))]
pub fn query_terminal() -> Caps {
    Caps::default()
}
