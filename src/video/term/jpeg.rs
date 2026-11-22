//! A baseline JPEG writer (4:2:0, standard Huffman tables) for the iTerm2
//! protocol, which wants an image file: a frame is a tenth of the bytes a
//! PNG would be, which is what decides how many frames a second the
//! terminal can show.
use super::super::Frame;
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];
const LUMA_Q: [u8; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113,
    92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];
const CHROMA_Q: [u8; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];
const DC_LUMA_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_CHROMA_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const DC_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_LUMA_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
const AC_LUMA_VALS: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5,
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];
const AC_CHROMA_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const AC_CHROMA_VALS: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0,
    0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5,
    0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];
/// Huffman codes indexed by symbol: (code, length).
struct Huff {
    codes: [(u16, u8); 256],
}
impl Huff {
    fn new(bits: &[u8; 16], vals: &[u8]) -> Self {
        let mut codes = [(0u16, 0u8); 256];
        let mut code = 0u16;
        let mut k = 0;
        for (len, &n) in bits.iter().enumerate() {
            for _ in 0..n {
                codes[vals[k] as usize] = (code, len as u8 + 1);
                code += 1;
                k += 1;
            }
            code <<= 1;
        }
        Self { codes }
    }
}
struct Bits<'a> {
    out: &'a mut Vec<u8>,
    acc: u32,
    n: u32,
}
impl Bits<'_> {
    fn put(&mut self, code: u32, len: u32) {
        self.acc = (self.acc << len) | (code & ((1 << len) - 1));
        self.n += len;
        while self.n >= 8 {
            let b = (self.acc >> (self.n - 8)) as u8;
            self.out.push(b);
            if b == 0xFF {
                self.out.push(0);
            }
            self.n -= 8;
        }
    }
    fn flush(&mut self) {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad);
        }
    }
}
fn scaled_table(base: &[u8; 64], quality: u8) -> [u8; 64] {
    // Above 95 the DC differences can outgrow the standard Huffman table.
    let q = quality.clamp(1, 95) as u32;
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    let mut t = [0u8; 64];
    for i in 0..64 {
        t[i] = ((base[i] as u32 * scale + 50) / 100).clamp(1, 255) as u8;
    }
    t
}
fn cos_table() -> [[f32; 8]; 8] {
    let mut t = [[0f32; 8]; 8];
    for (x, row) in t.iter_mut().enumerate() {
        for (u, v) in row.iter_mut().enumerate() {
            *v = (((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI) / 16.0).cos();
        }
    }
    t
}
/// Forward DCT of an 8×8 block of level-shifted samples, then quantised and
/// zig-zagged.
fn dct_quant(block: &[f32; 64], cos: &[[f32; 8]; 8], q: &[u8; 64], out: &mut [i32; 64]) {
    let mut tmp = [0f32; 64];
    for y in 0..8 {
        for u in 0..8 {
            let mut s = 0.0;
            for x in 0..8 {
                s += block[y * 8 + x] * cos[x][u];
            }
            tmp[y * 8 + u] = s;
        }
    }
    let c = |k: usize| {
        if k == 0 {
            std::f32::consts::FRAC_1_SQRT_2
        } else {
            1.0
        }
    };
    for v in 0..8 {
        for u in 0..8 {
            let mut s = 0.0;
            for y in 0..8 {
                s += tmp[y * 8 + u] * cos[y][v];
            }
            let coef = 0.25 * c(u) * c(v) * s;
            out[v * 8 + u] = (coef / q[v * 8 + u] as f32).round() as i32;
        }
    }
    let natural = *out;
    for (z, &n) in ZIGZAG.iter().enumerate() {
        out[z] = natural[n];
    }
}
fn category(v: i32) -> (u32, u32) {
    let a = v.unsigned_abs();
    let cat = 32 - a.leading_zeros();
    let bits = if v < 0 { (v - 1) as u32 } else { v as u32 };
    (cat, bits)
}
fn encode_block(bits: &mut Bits, coefs: &[i32; 64], prev_dc: &mut i32, dc: &Huff, ac: &Huff) {
    let diff = coefs[0] - *prev_dc;
    *prev_dc = coefs[0];
    let (cat, v) = category(diff);
    let (code, len) = dc.codes[cat as usize];
    bits.put(code as u32, len as u32);
    if cat > 0 {
        bits.put(v, cat);
    }
    let mut run = 0u32;
    let last = coefs.iter().rposition(|&c| c != 0).unwrap_or(0);
    for &coef in coefs.iter().take(last + 1).skip(1) {
        if coef == 0 {
            run += 1;
            continue;
        }
        while run >= 16 {
            let (code, len) = ac.codes[0xF0];
            bits.put(code as u32, len as u32);
            run -= 16;
        }
        let (cat, v) = category(coef);
        let (code, len) = ac.codes[((run << 4) | cat) as usize];
        bits.put(code as u32, len as u32);
        bits.put(v, cat);
        run = 0;
    }
    if last < 63 {
        let (code, len) = ac.codes[0x00];
        bits.put(code as u32, len as u32);
    }
}
fn marker(out: &mut Vec<u8>, m: u8, body: &[u8]) {
    out.extend_from_slice(&[0xFF, m]);
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(body);
}
/// Encodes `frame` as a baseline JPEG at `quality` (1..=100).
pub fn encode(frame: &Frame, quality: u8) -> Vec<u8> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let lq = scaled_table(&LUMA_Q, quality);
    let cq = scaled_table(&CHROMA_Q, quality);
    let cos = cos_table();
    let dc_l = Huff::new(&DC_LUMA_BITS, &DC_VALS);
    let dc_c = Huff::new(&DC_CHROMA_BITS, &DC_VALS);
    let ac_l = Huff::new(&AC_LUMA_BITS, &AC_LUMA_VALS);
    let ac_c = Huff::new(&AC_CHROMA_BITS, &AC_CHROMA_VALS);
    let mut out = Vec::with_capacity(w * h / 4 + 1024);
    out.extend_from_slice(&[0xFF, 0xD8]);
    marker(
        &mut out,
        0xE0,
        &[b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0],
    );
    let mut dqt = Vec::with_capacity(130);
    for (id, t) in [(0u8, &lq), (1u8, &cq)] {
        dqt.push(id);
        for &z in &ZIGZAG {
            dqt.push(t[z]);
        }
    }
    marker(&mut out, 0xDB, &dqt);
    let mut sof = vec![8];
    sof.extend_from_slice(&(h as u16).to_be_bytes());
    sof.extend_from_slice(&(w as u16).to_be_bytes());
    sof.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    marker(&mut out, 0xC0, &sof);
    let mut dht = Vec::new();
    for (class_id, bits, vals) in [
        (0x00u8, &DC_LUMA_BITS, &DC_VALS[..]),
        (0x10, &AC_LUMA_BITS, &AC_LUMA_VALS[..]),
        (0x01, &DC_CHROMA_BITS, &DC_VALS[..]),
        (0x11, &AC_CHROMA_BITS, &AC_CHROMA_VALS[..]),
    ] {
        dht.push(class_id);
        dht.extend_from_slice(bits);
        dht.extend_from_slice(vals);
    }
    marker(&mut out, 0xC4, &dht);
    marker(&mut out, 0xDA, &[3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);
    // Planes, padded to whole 16×16 macroblocks by repeating the edge.
    let (pw, ph) = (w.div_ceil(16) * 16, h.div_ceil(16) * 16);
    let mut yp = vec![0f32; pw * ph];
    let mut cb = vec![0f32; pw * ph];
    let mut cr = vec![0f32; pw * ph];
    for y in 0..ph {
        let sy = y.min(h - 1);
        for x in 0..pw {
            let sx = x.min(w - 1);
            let i = (sy * w + sx) * 3;
            let (r, g, b) = (
                frame.rgb[i] as f32,
                frame.rgb[i + 1] as f32,
                frame.rgb[i + 2] as f32,
            );
            let o = y * pw + x;
            yp[o] = 0.299 * r + 0.587 * g + 0.114 * b - 128.0;
            cb[o] = -0.168_736 * r - 0.331_264 * g + 0.5 * b;
            cr[o] = 0.5 * r - 0.418_688 * g - 0.081_312 * b;
        }
    }
    let mut bits = Bits {
        out: &mut out,
        acc: 0,
        n: 0,
    };
    let (mut pdy, mut pdb, mut pdr) = (0, 0, 0);
    let mut block = [0f32; 64];
    let mut coefs = [0i32; 64];
    for my in (0..ph).step_by(16) {
        for mx in (0..pw).step_by(16) {
            for (by, bx) in [(0, 0), (0, 8), (8, 0), (8, 8)] {
                for y in 0..8 {
                    for x in 0..8 {
                        block[y * 8 + x] = yp[(my + by + y) * pw + mx + bx + x];
                    }
                }
                dct_quant(&block, &cos, &lq, &mut coefs);
                encode_block(&mut bits, &coefs, &mut pdy, &dc_l, &ac_l);
            }
            for (plane, prev) in [(&cb, &mut pdb), (&cr, &mut pdr)] {
                for y in 0..8 {
                    for x in 0..8 {
                        let o = (my + 2 * y) * pw + mx + 2 * x;
                        block[y * 8 + x] =
                            0.25 * (plane[o] + plane[o + 1] + plane[o + pw] + plane[o + pw + 1]);
                    }
                }
                dct_quant(&block, &cos, &cq, &mut coefs);
                encode_block(&mut bits, &coefs, prev, &dc_c, &ac_c);
            }
        }
    }
    bits.flush();
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}
