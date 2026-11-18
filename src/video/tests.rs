use super::audio_feed::{PcmSource, Shared};
use super::clock::Clock;
use super::term::{self, Caps, Env, Protocol, base64, jpeg, xterm256};
use super::*;
use rodio::Source;
fn solid(w: u32, h: u32, rgb: (u8, u8, u8)) -> Frame {
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for _ in 0..w * h {
        data.extend_from_slice(&[rgb.0, rgb.1, rgb.2]);
    }
    Frame::new(w, h, 0.0, data)
}
#[test]
fn fit_preserves_aspect_and_never_exceeds_the_box() {
    assert_eq!(fit(1920, 1080, 192, 200), (192, 108));
    assert_eq!(fit(1080, 1920, 192, 200), (113, 200));
    assert_eq!(fit(0, 0, 50, 20), (50, 20));
    assert_eq!(fit(100, 100, 0, 0), (1, 1));
    for (w, h, bw, bh) in [(1280, 720, 7, 3), (3, 1000, 40, 40), (999, 1, 10, 10)] {
        let (fw, fh) = fit(w, h, bw, bh);
        assert!(fw >= 1 && fh >= 1 && fw <= bw && fh <= bh, "{fw}x{fh}");
    }
}
#[test]
fn block_target_decodes_one_pixel_per_half_cell() {
    let t = Target {
        cols: 80,
        rows: 20,
        cell: (1, 2),
        kind: Kind::HalfBlocks,
        max_width: u32::MAX,
        max_fps: 30,
    };
    // 16:9 into 80×40 virtual pixels: width-bound at 71×40.
    assert_eq!(t.frame_size(1920, 1080), (71, 40));
    assert_eq!(t.cells(71, 40), (71, 20));
    // Odd heights round down to whole cells.
    let (w, h) = t.frame_size(100, 33);
    assert!(h.is_multiple_of(2) && w <= 80 && h <= 40, "{w}x{h}");
    // With a measured cell size the aspect ratio accounts for it.
    let t = Target {
        cell: (10, 20),
        ..t
    };
    assert_eq!(t.frame_size(1920, 1080), (71, 40));
    let t = Target {
        cell: (8, 20),
        ..t
    };
    // A half block is 8×10 px here, so a square picture needs more columns
    // than half-rows: 400×400 px is 50 columns by 40 half-rows.
    assert_eq!(t.frame_size(1000, 1000), (50, 40));
}
#[test]
fn pixel_target_fits_cells_and_caps_width() {
    let t = Target {
        cols: 100,
        rows: 30,
        cell: (10, 20),
        kind: Kind::Pixels,
        max_width: 400,
        max_fps: 24,
    };
    let (w, h) = t.frame_size(1920, 1080);
    assert_eq!((w, h), (400, 224));
    // The cap shrinks the decode, not the picture: it still spans the
    // whole fitted area (1000×562 px of a 1000×600 box).
    assert_eq!(t.cells(1920, 1080), (100, 29));
    assert_eq!(t.cells(1000, 600), (100, 30));
    assert_eq!(t.cells(600, 600), (60, 30));
    let big = Target {
        max_width: u32::MAX,
        ..t
    };
    assert_eq!(big.frame_size(1920, 1080), (1000, 562));
    // ASCII: one pixel per cell, so the frame is the cell count.
    let text = Target {
        kind: Kind::Cells,
        ..t
    };
    assert_eq!(text.frame_size(1920, 1080), (100, 28));
    assert_eq!(text.cells(1920, 1080), (100, 28));
}
#[test]
fn probe_json_is_parsed_with_rotation_and_fallbacks() {
    let json = r#"{
      "streams": [
        {"codec_type":"video","codec_name":"mjpeg","width":600,"height":600,"disposition":{"attached_pic":1}},
        {"codec_type":"video","codec_name":"h264","width":1920,"height":1080,
         "r_frame_rate":"30000/1001","avg_frame_rate":"0/0",
         "side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}],
         "tags":{"creation_time":"2019-05-04T10:00:00.000000Z"}},
        {"codec_type":"audio","codec_name":"aac","channels":6,"sample_rate":"48000"}
      ],
      "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"5432.1","tags":{"TITLE":"Some Film"}}
    }"#;
    let p = probe::parse_probe(json).unwrap();
    assert_eq!(p.title.as_deref(), Some("Some Film"));
    assert_eq!((p.width, p.height), (1080, 1920), "rotated 90°");
    assert!((p.fps - 29.97).abs() < 0.01, "{}", p.fps);
    assert_eq!(p.vcodec.as_deref(), Some("h264"));
    assert_eq!(p.acodec.as_deref(), Some("aac"));
    assert_eq!((p.channels, p.sample_rate), (6, 48000));
    assert_eq!(p.format.as_deref(), Some("mp4"));
    assert_eq!(p.year, Some(2019));
    assert!((p.duration - 5432.1).abs() < 1e-6);
    let silent = probe::parse_probe(
        r#"{"streams":[{"codec_type":"video","codec_name":"vp9","width":640,"height":360,"avg_frame_rate":"25/1","duration":"12"}],"format":{"format_name":"matroska,webm"}}"#,
    )
    .unwrap();
    assert_eq!(silent.acodec, None);
    assert_eq!(silent.fps, 25.0);
    assert_eq!(silent.duration, 12.0);
    assert_eq!(silent.format.as_deref(), Some("mkv"));
    assert!(probe::parse_probe(r#"{"streams":[],"format":{}}"#).is_err());
    assert!(probe::parse_probe("not json").is_err());
}
#[test]
fn clock_runs_pauses_and_scales() {
    let mut c = Clock::new(10.0, 1.0);
    std::thread::sleep(std::time::Duration::from_millis(30));
    let t = c.now();
    assert!(t > 10.02 && t < 11.0, "{t}");
    c.pause();
    let p = c.now();
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert_eq!(c.now(), p, "paused clock must not move");
    c.set_speed(2.0);
    assert_eq!(c.now(), p, "setting speed while paused keeps it paused");
    c.resume();
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert!(c.now() > p + 0.05, "2x speed should gain at least 50 ms in 30 ms");
}
#[test]
fn pcm_source_plays_silence_on_underrun_and_ends_on_eof() {
    let shared = Shared::new();
    let mut src = PcmSource::new(shared.clone());
    assert_eq!(src.channels().get(), AUDIO_CHANNELS);
    assert_eq!(src.sample_rate().get(), AUDIO_RATE);
    assert!(!shared.ready());
    // Nothing queued yet: silence, counted.
    assert_eq!(src.next(), Some(0.0));
    assert_eq!(src.next(), Some(0.0));
    assert!((shared.underrun_secs() - 2.0 / 96_000.0).abs() < 1e-9);
    shared.push(&[0.25, -0.25, 0.5]);
    assert_eq!(src.next(), Some(0.25));
    assert_eq!(src.next(), Some(-0.25));
    assert_eq!(src.next(), Some(0.5));
    assert_eq!(src.next(), Some(0.0));
    shared.push(&vec![0.1; 48_000]);
    assert!(shared.ready(), "a quarter second queued is enough to start");
    shared.finish();
    let n = src.by_ref().take_while(|&s| s == 0.1).count();
    assert_eq!(n, 48_000);
    assert_eq!(src.next(), None, "eof after the queue drains");
}
#[test]
fn base64_matches_the_standard() {
    assert_eq!(base64::encode(b""), "");
    assert_eq!(base64::encode(b"f"), "Zg==");
    assert_eq!(base64::encode(b"fo"), "Zm8=");
    assert_eq!(base64::encode(b"foo"), "Zm9v");
    assert_eq!(base64::encode(b"foobar"), "Zm9vYmFy");
    assert_eq!(base64::encode(&[0, 0, 0]), "AAAA");
    assert_eq!(base64::encode(&[255, 255, 255]), "////");
}
#[test]
fn jpeg_is_well_formed() {
    let mut f = solid(37, 21, (10, 200, 30));
    for (i, b) in f.rgb.iter_mut().enumerate() {
        *b = b.wrapping_add((i % 7) as u8 * 9);
    }
    let data = jpeg::encode(&f, 82);
    assert_eq!(&data[..4], &[0xFF, 0xD8, 0xFF, 0xE0], "SOI then JFIF");
    assert_eq!(&data[6..11], b"JFIF\0");
    assert!(data.ends_with(&[0xFF, 0xD9]), "EOI");
    // Frame header: 8-bit, 21 rows, 37 columns, three components, 4:2:0.
    let sof = data.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    assert_eq!(data[sof + 4], 8);
    assert_eq!(u16::from_be_bytes([data[sof + 5], data[sof + 6]]), 21);
    assert_eq!(u16::from_be_bytes([data[sof + 7], data[sof + 8]]), 37);
    assert_eq!(data[sof + 9], 3);
    assert_eq!(data[sof + 11], 0x22);
    // Every 0xFF inside the scan is stuffed with 0x00.
    let sos = data.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    let scan = &data[sos + 14..data.len() - 2];
    for w in scan.windows(2) {
        if w[0] == 0xFF {
            assert_eq!(w[1], 0, "unstuffed 0xFF in scan data");
        }
    }
    // A flat picture is tiny, a noisy one is bigger, and quality matters.
    let flat = jpeg::encode(&solid(64, 64, (90, 90, 90)), 82).len();
    let busy = jpeg::encode(&noisy(64, 64), 82).len();
    let busy_low = jpeg::encode(&noisy(64, 64), 30).len();
    assert!(flat < 1200, "{flat}");
    assert!(busy > flat * 2, "{busy} vs {flat}");
    assert!(busy_low < busy, "{busy_low} vs {busy}");
}
fn noisy(w: u32, h: u32) -> Frame {
    let mut f = solid(w, h, (0, 0, 0));
    let mut x = 0x9E37_79B9u32;
    for b in f.rgb.iter_mut() {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *b = x as u8;
    }
    f
}
#[test]
fn kitty_frames_are_chunked_and_place_over_the_cells() {
    let r = Renderer {
        protocol: Protocol::Kitty,
        truecolor: true,
        cell: (10, 20),
        cell_known: true,
    };
    // Incompressible data so the base64 spans several chunks.
    let mut f = solid(200, 100, (0, 0, 0));
    let mut x = 0x1234_5678u32;
    for b in f.rgb.iter_mut() {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *b = x as u8;
    }
    let out = r.encode(&f, 5, 3, 20, 5, 0);
    let s = String::from_utf8(out).unwrap();
    assert!(s.starts_with("\x1b[4;6H\x1b_Ga=T,q=2,f=24,o=z,s=200,v=100,c=20,r=5,i=1,C=1,m=1;"));
    let chunks: Vec<&str> = s.split("\x1b\\").collect();
    assert!(chunks.len() > 3, "{}", chunks.len());
    for c in &chunks[..chunks.len() - 2] {
        let payload = c.rsplit(';').next().unwrap();
        assert!(payload.len() <= 4096, "{}", payload.len());
        assert!(payload.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='));
    }
    assert!(s.contains("\x1b_Gm=0;"), "last chunk ends the transmission");
    assert!(s.ends_with("\x1b_Ga=d,d=I,i=2,q=2\x1b\\"));
    let second = String::from_utf8(r.encode(&f, 0, 0, 1, 1, 1)).unwrap();
    assert!(second.contains(",i=2,"));
    assert!(second.ends_with("i=1,q=2\x1b\\"));
    assert_eq!(r.clear_sequence(), Some(&b"\x1b_Ga=d,d=A,q=2\x1b\\"[..]));
}
#[test]
fn iterm2_frames_are_inline_jpegs_sized_in_cells() {
    let r = Renderer {
        protocol: Protocol::Iterm2,
        truecolor: true,
        cell: (10, 20),
        cell_known: true,
    };
    let f = solid(4, 4, (1, 2, 3));
    let s = String::from_utf8(r.encode(&f, 2, 1, 8, 2, 0)).unwrap();
    assert!(s.starts_with("\x1b[2;3H\x1b]1337;File=inline=1;size="));
    assert!(s.contains(";width=8;height=2;preserveAspectRatio=0;"));
    assert!(s.ends_with('\x07'));
    let b64 = s.rsplit(':').next().unwrap().trim_end_matches('\x07');
    assert!(b64.starts_with("/9j/"), "JPEG signature in base64");
    assert!(r.clear_sequence().is_none());
}
/// Decodes the sixel our encoder writes, enough to check every pixel.
fn decode_sixel(s: &str, w: usize, h: usize) -> Vec<Option<usize>> {
    let mut img = vec![None; w * h];
    let body = s.split_once('q').unwrap().1;
    let body = body.strip_suffix("\x1b\\").unwrap();
    let body = &body[body.find('#').unwrap()..];
    let mut color = 0usize;
    let mut x = 0usize;
    let mut band = 0usize;
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '#' => {
                let mut num = String::new();
                while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                    num.push(*d);
                    chars.next();
                }
                color = num.parse().unwrap();
                if chars.peek() == Some(&';') {
                    // palette definition: skip 2;r;g;b
                    for _ in 0..4 {
                        chars.next();
                        while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                            let _ = d;
                            chars.next();
                        }
                    }
                }
            }
            '!' => {
                let mut num = String::new();
                while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                    num.push(*d);
                    chars.next();
                }
                let n: usize = num.parse().unwrap();
                let ch = chars.next().unwrap();
                for _ in 0..n {
                    paint(&mut img, w, h, x, band, color, ch);
                    x += 1;
                }
            }
            '$' => x = 0,
            '-' => {
                x = 0;
                band += 1;
            }
            c if ('?'..='~').contains(&c) => {
                paint(&mut img, w, h, x, band, color, c);
                x += 1;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    img
}
fn paint(img: &mut [Option<usize>], w: usize, h: usize, x: usize, band: usize, color: usize, ch: char) {
    let bits = ch as u8 - 63;
    for row in 0..6 {
        if bits & (1 << row) != 0 {
            let y = band * 6 + row;
            assert!(x < w && y < h, "pixel {x},{y} outside {w}x{h}");
            assert!(img[y * w + x].is_none(), "pixel {x},{y} painted twice");
            img[y * w + x] = Some(color);
        }
    }
}
#[test]
fn sixel_output_paints_every_pixel_once_with_the_right_palette_entry() {
    let r = Renderer {
        protocol: Protocol::Sixel,
        truecolor: true,
        cell: (10, 20),
        cell_known: true,
    };
    let (w, h) = (9u32, 8u32);
    let mut f = solid(w, h, (0, 0, 0));
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 3) as usize;
            f.rgb[i] = (x * 28) as u8;
            f.rgb[i + 1] = (y * 36) as u8;
            f.rgb[i + 2] = if (x + y) % 2 == 0 { 255 } else { 0 };
        }
    }
    let s = String::from_utf8(r.encode(&f, 0, 0, 1, 1, 0)).unwrap();
    assert!(s.starts_with("\x1b[1;1H\x1bP0;1;0q\"1;1;9;8"));
    assert!(s.ends_with("\x1b\\"));
    let img = decode_sixel(&s, w as usize, h as usize);
    for y in 0..h {
        for x in 0..w {
            let (pr, pg, pb) = f.pixel(x, y);
            let want = super::term::sixel_index(pr, pg, pb, x, y);
            assert_eq!(img[(y * w + x) as usize], Some(want), "pixel {x},{y}");
        }
    }
    // Palette entries map back to sensible colours.
    let (pr, pg, pb) = super::term::sixel_palette(super::term::sixel_index(255, 255, 255, 0, 0));
    assert_eq!((pr, pg, pb), (100, 100, 100));
    let (pr, pg, pb) = super::term::sixel_palette(super::term::sixel_index(0, 0, 0, 0, 0));
    assert_eq!((pr, pg, pb), (0, 0, 0));
}
#[test]
fn xterm256_picks_cube_or_grey() {
    assert_eq!(xterm256(0, 0, 0), 16);
    assert_eq!(xterm256(255, 255, 255), 231);
    assert_eq!(xterm256(255, 0, 0), 196);
    assert_eq!(xterm256(0, 0, 255), 21);
    assert_eq!(xterm256(128, 128, 128), 244);
    assert_eq!(xterm256(18, 18, 18), 233);
}
#[test]
fn renderer_detection_follows_the_terminal() {
    let env = |program: &str| Env {
        term: "xterm-256color".into(),
        term_program: program.into(),
        ..Default::default()
    };
    let none = Caps::default();
    let cell = Caps {
        cell: Some((9, 18)),
        ..none
    };
    let pick = |f: Option<&str>, e: &Env, c: &Caps| term::detect(f, e, c).0;
    assert_eq!(pick(None, &env("Apple_Terminal"), &none).protocol, Protocol::Blocks);
    assert!(!pick(None, &env("Apple_Terminal"), &none).truecolor);
    assert_eq!(pick(None, &env("iTerm.app"), &cell).protocol, Protocol::Iterm2);
    assert_eq!(pick(None, &env("vscode"), &none).protocol, Protocol::Iterm2);
    assert!(pick(None, &env("vscode"), &none).truecolor);
    assert_eq!(pick(None, &env("ghostty"), &none).protocol, Protocol::Kitty);
    assert_eq!(pick(None, &env("WezTerm"), &none).protocol, Protocol::Iterm2);
    let kitty = Caps {
        kitty: true,
        ..cell
    };
    assert_eq!(pick(None, &env("WezTerm"), &kitty).protocol, Protocol::Kitty);
    assert_eq!(pick(None, &env(""), &kitty).protocol, Protocol::Kitty);
    let sixel = Caps {
        sixel: true,
        ..cell
    };
    assert_eq!(pick(None, &env(""), &sixel).protocol, Protocol::Sixel);
    let sixel_no_cell = Caps {
        sixel: true,
        ..none
    };
    assert_eq!(pick(None, &env(""), &sixel_no_cell).protocol, Protocol::Blocks);
    let tmux = Env {
        tmux: true,
        ..env("iTerm.app")
    };
    assert_eq!(pick(None, &tmux, &kitty).protocol, Protocol::Blocks);
    assert_eq!(pick(Some("blocks256"), &env("ghostty"), &kitty).protocol, Protocol::Blocks);
    assert!(!pick(Some("blocks256"), &env("ghostty"), &kitty).truecolor);
    let a = pick(Some("ascii256"), &env("ghostty"), &kitty);
    assert_eq!((a.protocol, a.truecolor), (Protocol::Ascii, false));
    assert_eq!(a.setting_name(), "ascii256");
    assert_eq!(a.label(), "ascii 256");
    assert_eq!(a.with(Protocol::Kitty, true).setting_name(), "kitty");
    assert_eq!(pick(Some("sixel"), &env(""), &cell).protocol, Protocol::Sixel);
    let (r, warn) = term::detect(Some("sixel"), &env(""), &none);
    assert_eq!(r.protocol, Protocol::Blocks);
    assert!(warn.is_some());
    let r = pick(None, &env("vscode"), &cell);
    assert_eq!(r.label(), "iterm2 9×18px");
    assert_eq!(pick(None, &env("Apple_Terminal"), &none).label(), "blocks 256");
}
#[test]
fn terminal_replies_are_parsed() {
    let (caps, done) = term::parse_replies(b"\x1b[6;20;10t\x1b_Gi=31;OK\x1b\\\x1b[?62;4;22c");
    assert!(done);
    assert_eq!(caps, Caps {
        kitty: true,
        sixel: true,
        cell: Some((10, 20)),
    });
    let (caps, done) = term::parse_replies(b"\x1b[?1;2c");
    assert!(done);
    assert_eq!(caps, Caps::default());
    let (caps, done) = term::parse_replies(b"\x1b_Gi=31;EINVAL\x1b\\\x1b[6;0;0t");
    assert!(!done);
    assert_eq!(caps, Caps::default());
}
