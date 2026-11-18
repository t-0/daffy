use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Stdio};
/// What ffprobe reports about a file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Probe {
    pub title: Option<String>,
    pub year: Option<u32>,
    pub duration: f64,
    /// Displayed picture size, after any rotation the container asks for.
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub channels: u32,
    pub sample_rate: u32,
    /// Short container name (`mp4`, `matroska`, ...).
    pub format: Option<String>,
}
pub fn probe(ffprobe: &Path, path: &Path) -> Result<Probe> {
    let out = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("running {}", ffprobe.display()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(anyhow!(
            "ffprobe failed on {}: {}",
            path.display(),
            if err.is_empty() {
                out.status.to_string()
            } else {
                err
            }
        ));
    }
    parse_probe(&String::from_utf8_lossy(&out.stdout))
}
fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
fn num_of(v: &Value, key: &str) -> Option<f64> {
    match v.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}
/// `30000/1001` → 29.97; `25` → 25.
fn parse_rate(s: &str) -> Option<f64> {
    let s = s.trim();
    let v = match s.split_once('/') {
        Some((a, b)) => {
            let (a, b) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
            if b == 0.0 {
                return None;
            }
            a / b
        }
        None => s.parse().ok()?,
    };
    (v.is_finite() && v > 0.0 && v < 1000.0).then_some(v)
}
fn tag<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    let tags = v.get("tags")?.as_object()?;
    tags.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}
fn tag_str(v: &Value, key: &str) -> Option<String> {
    tag(v, key)
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
/// Degrees the display matrix or the `rotate` tag turn the picture by.
fn rotation(stream: &Value) -> i32 {
    if let Some(list) = stream.get("side_data_list").and_then(|l| l.as_array()) {
        for sd in list {
            if let Some(r) = num_of(sd, "rotation") {
                return r.round() as i32;
            }
        }
    }
    tag_str(stream, "rotate")
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0)
}
fn year_of(s: &str) -> Option<u32> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits
        .parse::<u32>()
        .ok()
        .filter(|y| (1800..=2200).contains(y))
}
pub fn parse_probe(json: &str) -> Result<Probe> {
    let root: Value = serde_json::from_str(json).context("parsing ffprobe output")?;
    let mut p = Probe::default();
    let format = root.get("format").cloned().unwrap_or(Value::Null);
    p.duration = num_of(&format, "duration").unwrap_or(0.0).max(0.0);
    p.format = str_of(&format, "format_name").map(|f| {
        // "mov,mp4,m4a,3gp,3g2,mj2" is one demuxer for several containers.
        let first = f.split(',').next().unwrap_or(&f).to_string();
        match first.as_str() {
            "mov" => "mp4".to_string(),
            "matroska" => "mkv".to_string(),
            other => other.to_string(),
        }
    });
    p.title = tag_str(&format, "title");
    p.year = [
        "date",
        "year",
        "creation_time",
        "com.apple.quicktime.creationdate",
    ]
    .iter()
    .filter_map(|k| tag_str(&format, k))
    .find_map(|s| year_of(&s));
    let streams = root
        .get("streams")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    let mut video: Option<&Value> = None;
    let mut audio: Option<&Value> = None;
    for s in &streams {
        match str_of(s, "codec_type").as_deref() {
            Some("video") => {
                let cover = s
                    .get("disposition")
                    .and_then(|d| d.get("attached_pic"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    == 1;
                if !cover && video.is_none() {
                    video = Some(s);
                }
            }
            Some("audio") if audio.is_none() => audio = Some(s),
            _ => {}
        }
    }
    if let Some(v) = video {
        let w = num_of(v, "width").unwrap_or(0.0) as u32;
        let h = num_of(v, "height").unwrap_or(0.0) as u32;
        let rot = rotation(v).rem_euclid(180);
        let (w, h) = if rot == 90 { (h, w) } else { (w, h) };
        p.width = w;
        p.height = h;
        p.fps = str_of(v, "avg_frame_rate")
            .and_then(|r| parse_rate(&r))
            .or_else(|| str_of(v, "r_frame_rate").and_then(|r| parse_rate(&r)))
            .unwrap_or(0.0);
        p.vcodec = str_of(v, "codec_name");
        if p.duration == 0.0 {
            p.duration = num_of(v, "duration").unwrap_or(0.0).max(0.0);
        }
        if p.year.is_none() {
            p.year = tag_str(v, "creation_time").and_then(|s| year_of(&s));
        }
    }
    if let Some(a) = audio {
        p.acodec = str_of(a, "codec_name");
        p.channels = num_of(a, "channels").unwrap_or(0.0) as u32;
        p.sample_rate = num_of(a, "sample_rate").unwrap_or(0.0) as u32;
        if p.duration == 0.0 {
            p.duration = num_of(a, "duration").unwrap_or(0.0).max(0.0);
        }
    }
    if video.is_none() && audio.is_none() {
        return Err(anyhow!("no audio or video streams"));
    }
    Ok(p)
}
