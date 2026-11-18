/// Nearest xterm 256-colour index to an RGB value: the 6×6×6 cube or the
/// grey ramp, whichever is closer.
pub fn xterm256(r: u8, g: u8, b: u8) -> u8 {
    const CUBE: [i32; 6] = [0, 95, 135, 175, 215, 255];
    let nearest = |v: u8| -> (usize, i32) {
        let mut best = (0usize, i32::MAX);
        for (i, &c) in CUBE.iter().enumerate() {
            let d = (v as i32 - c).abs();
            if d < best.1 {
                best = (i, d);
            }
        }
        best
    };
    let (ri, rv) = nearest(r);
    let (gi, gv) = nearest(g);
    let (bi, bv) = nearest(b);
    let cube_dist = rv * rv + gv * gv + bv * bv;
    let cube_idx = 16 + 36 * ri + 6 * gi + bi;
    let avg = (r as i32 + g as i32 + b as i32) / 3;
    let gi = ((avg - 8 + 5) / 10).clamp(0, 23);
    let gv = 8 + 10 * gi;
    let grey_dist = (r as i32 - gv).pow(2) + (g as i32 - gv).pow(2) + (b as i32 - gv).pow(2);
    if grey_dist < cube_dist {
        (232 + gi) as u8
    } else {
        cube_idx as u8
    }
}
