use super::{AUDIO_CHANNELS, AUDIO_RATE};
use crate::fx::{NORMALIZE_RANGE_DB, NORMALIZE_TARGET_DB};
use anyhow::{Context, Result, anyhow};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
/// Decodes the whole file's audio and returns the gain in dB that would
/// bring its level to `NORMALIZE_TARGET_DB`. Level is the RMS over 400 ms
/// blocks, ignoring blocks quieter than -70 dBFS so silence at the ends
/// and in pauses does not make a video look quieter than it is.
pub fn measure_gain(ffmpeg: &Path, path: &Path) -> Result<f32> {
    let mut child = Command::new(ffmpeg)
        .args(["-nostdin", "-v", "error", "-i"])
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
        .with_context(|| format!("starting {}", ffmpeg.display()))?;
    let mut out = child.stdout.take().context("ffmpeg stdout")?;
    let block = (AUDIO_RATE as usize * AUDIO_CHANNELS as usize * 2) / 5;
    let mut levels = Vec::new();
    let (mut sum, mut n) = (0.0f64, 0usize);
    let mut buf = vec![0u8; 64 * 1024];
    let mut carry: Vec<u8> = Vec::new();
    loop {
        let got = match out.read(&mut buf) {
            Ok(0) => break,
            Ok(g) => g,
            Err(e) => return Err(e.into()),
        };
        carry.extend_from_slice(&buf[..got]);
        let whole = carry.len() / 4 * 4;
        for b in carry[..whole].as_chunks::<4>().0 {
            let v = f32::from_le_bytes(*b) as f64;
            sum += v * v;
            n += 1;
            if n == block {
                levels.push((sum / n as f64) as f32);
                sum = 0.0;
                n = 0;
            }
        }
        carry.drain(..whole);
    }
    if n > block / 2 {
        levels.push((sum / n as f64) as f32);
    }
    let status = child.wait()?;
    if !status.success() && levels.is_empty() {
        return Err(anyhow!("ffmpeg exited with {status}"));
    }
    Ok(gain_for(levels))
}
/// Gain that brings the mean of the loud-enough blocks to the target.
pub fn gain_for(levels: Vec<f32>) -> f32 {
    const GATE: f32 = 1e-7; // -70 dBFS as mean square
    let loud: Vec<f32> = levels.into_iter().filter(|&m| m > GATE).collect();
    if loud.is_empty() {
        return 0.0;
    }
    let mean = loud.iter().sum::<f32>() / loud.len() as f32;
    let level_db = 10.0 * mean.log10();
    (NORMALIZE_TARGET_DB - level_db).clamp(NORMALIZE_RANGE_DB.0, NORMALIZE_RANGE_DB.1)
}
