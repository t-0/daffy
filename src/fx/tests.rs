use super::*;
use rodio::buffer::SamplesBuffer;
use std::num::NonZero;
use std::sync::{Arc, Mutex};
fn sine(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 44_100.0).sin() * 0.5)
        .collect()
}
fn run(params: FxParams) -> Vec<f32> {
    let data = sine(44_100);
    let src = SamplesBuffer::new(
        NonZero::new(2).unwrap(),
        NonZero::new(44_100).unwrap(),
        data,
    );
    let shared = Arc::new(Mutex::new(params));
    Fx::new(src, shared).collect()
}
fn rms(v: &[f32]) -> f32 {
    (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt()
}
#[test]
fn legacy_settings_load_with_defaults() {
    let p: FxParams = serde_json::from_str(r#"{"preamp": 3}"#).unwrap();
    assert_eq!(p.preamp, 3);
    assert_eq!(p.width, 150);
    assert_eq!(p.eq.mode, EqMode::Off);
    let p: FxParams = serde_json::from_str("{}").unwrap();
    assert_eq!(p, FxParams::default());
}
#[test]
fn mono_input_is_supported() {
    let data = sine(22_050);
    let src = SamplesBuffer::new(
        NonZero::new(1).unwrap(),
        NonZero::new(22_050).unwrap(),
        data,
    );
    let mut p = FxParams {
        loudness: true,
        ..Default::default()
    };
    p.eq.mode = EqMode::Seven;
    let out: Vec<f32> = Fx::new(src, Arc::new(Mutex::new(p))).collect();
    assert_eq!(out.len(), 22_050);
    assert!(out.iter().all(|v| v.is_finite()));
}
#[test]
fn parameter_changes_apply_while_streaming() {
    let shared = Arc::new(Mutex::new(FxParams::default()));
    let data = vec![0.5f32; 44_100 * 2 * 4];
    let src = SamplesBuffer::new(
        NonZero::new(2).unwrap(),
        NonZero::new(44_100).unwrap(),
        data,
    );
    let mut fx = Fx::new(src, shared.clone());
    let first: Vec<f32> = fx.by_ref().take(44_100).collect();
    assert!(first.iter().all(|&v| v == 0.5));
    shared.lock().unwrap().preamp = -12;
    let rest: Vec<f32> = fx.collect();
    let last = *rest.last().unwrap();
    assert!(
        (last - 0.1256).abs() < 0.002,
        "preamp change should apply mid-stream: {last}"
    );
}
#[test]
fn fx_chain_runs_faster_than_realtime() {
    let secs = 20;
    let data = sine(44_100 * 2 * secs);
    let p = FxParams {
        eq: EqParams {
            mode: EqMode::Seven,
            ..Default::default()
        },
        loudness: true,
        stereo: StereoMode::Width,
        preamp: 6,
        ..Default::default()
    };
    let src = SamplesBuffer::new(
        NonZero::new(2).unwrap(),
        NonZero::new(44_100).unwrap(),
        data.clone(),
    );
    let t = std::time::Instant::now();
    let n = Fx::new(src, Arc::new(Mutex::new(p))).fold(0usize, |n, v| {
        assert!(v.is_finite());
        n + 1
    });
    let el = t.elapsed().as_secs_f64();
    assert_eq!(n, data.len());
    eprintln!(
        "fx: {secs}s of audio in {el:.3}s ({:.1}x realtime)",
        secs as f64 / el
    );
    assert!(el < secs as f64 / 3.0, "ran at under 3x realtime: {el:.2}s");
}
#[test]
#[ignore]
fn soak_fx_hour_with_random_parameter_changes() {
    let sr = 44_100u32;
    let secs = 3600usize;
    let shared = Arc::new(Mutex::new(FxParams::default()));
    let block = sine(sr as usize * 2);
    let data = block.iter().copied().cycle().take(sr as usize * 2 * secs);
    struct Cycle<I: Iterator<Item = f32>>(I);
    impl<I: Iterator<Item = f32>> Iterator for Cycle<I> {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            self.0.next()
        }
    }
    impl<I: Iterator<Item = f32>> Source for Cycle<I> {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> ChannelCount {
            NonZero::new(2).unwrap()
        }
        fn sample_rate(&self) -> SampleRate {
            NonZero::new(44_100).unwrap()
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }
    let mut fx = Fx::new(Cycle(data), shared.clone());
    let mut rng = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let started = std::time::Instant::now();
    let mut n = 0usize;
    let mut peak = 0f32;
    let half_sec = sr as usize;
    loop {
        let mut done = false;
        for _ in 0..half_sec {
            match fx.next() {
                Some(v) => {
                    assert!(v.is_finite(), "non-finite sample at {n}");
                    peak = peak.max(v.abs());
                    n += 1;
                }
                None => {
                    done = true;
                    break;
                }
            }
        }
        if done {
            break;
        }
        let mut p = shared.lock().unwrap();
        p.preamp = (next() % 37) as i8 - 24;
        p.eq.mode = EqMode::ALL[(next() % 3) as usize];
        for g in p.eq.seven.iter_mut() {
            *g = (next() % 25) as i8 - 12;
        }
        for g in p.eq.three.iter_mut() {
            *g = (next() % 25) as i8 - 12;
        }
        p.loudness = next() % 2 == 0;
        p.stereo = StereoMode::ALL[(next() % 3) as usize];
        p.width = (next() % 201) as u8;
    }
    let el = started.elapsed().as_secs_f64();
    assert_eq!(n, sr as usize * 2 * secs);
    eprintln!(
        "soak fx: {secs}s of audio in {el:.1}s ({:.1}x realtime), peak {peak:.3}",
        secs as f64 / el
    );
    assert!(peak < 2.0, "peak {peak}");
}
#[test]
fn default_is_identity() {
    let out = run(FxParams::default());
    assert_eq!(out, sine(44_100));
}
#[test]
fn preamp_scales_the_signal() {
    let clean = run(FxParams::default());
    let quiet = run(FxParams {
        preamp: -6,
        ..Default::default()
    });
    let ratio = rms(&quiet) / rms(&clean);
    assert!((ratio - 0.501).abs() < 0.01, "-6 dB gave ratio {ratio}");
    let loud = run(FxParams {
        preamp: 12,
        ..Default::default()
    });
    let peak = loud.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(
        peak <= 1.0 && peak > 0.9,
        "+12 dB peak {peak} should be limited"
    );
    let muted = run(FxParams {
        preamp: PREAMP_MIN_DB,
        ..Default::default()
    });
    let ratio = rms(&muted) / rms(&clean);
    assert!((ratio - 0.063).abs() < 0.005, "-24 dB gave ratio {ratio}");
}
#[test]
fn normalisation_adds_to_preamp_and_ramps() {
    let p = FxParams {
        preamp: 6,
        auto_normalize: true,
        track_gain: -12.0,
        ..Default::default()
    };
    assert_eq!(p.effective_preamp_db(), -6.0);
    assert_eq!(
        FxParams {
            auto_normalize: false,
            ..p
        }
        .effective_preamp_db(),
        6.0
    );
    assert_eq!(
        FxParams {
            track_gain: 40.0,
            ..p
        }
        .effective_preamp_db(),
        6.0 + NORMALIZE_RANGE_DB.1
    );
    // A gain that changes while streaming fades over a fraction of a
    // second instead of stepping.
    let shared = Arc::new(Mutex::new(FxParams {
        auto_normalize: true,
        ..Default::default()
    }));
    let data = vec![0.5f32; 44_100 * 2 * 3];
    let src = SamplesBuffer::new(
        NonZero::new(2).unwrap(),
        NonZero::new(44_100).unwrap(),
        data,
    );
    let mut fx = Fx::new(src, shared.clone());
    let first: Vec<f32> = fx.by_ref().take(2000).collect();
    assert!(first.iter().all(|&v| v == 0.5));
    shared.lock().unwrap().track_gain = -6.0;
    let rest: Vec<f32> = fx.collect();
    let at = |ms: usize| rest[ms * 44_100 * 2 / 1000 + 512];
    assert!(
        at(5) < 0.5 && at(5) > 0.45,
        "ramp should have started: {}",
        at(5)
    );
    assert!(at(100) > at(300), "ramp should keep falling");
    let last = *rest.last().unwrap();
    assert!(
        (last - 0.2505).abs() < 0.002,
        "should settle at -6 dB: {last}"
    );
    let steps = rest
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(steps < 0.002, "largest step {steps} is a click");
}
#[test]
fn eq_and_loudness_stay_bounded_and_do_something() {
    let clean = sine(44_100);
    let mut p = FxParams {
        loudness: true,
        ..Default::default()
    };
    p.eq.mode = EqMode::Seven;
    p.eq.seven = [12; 7];
    let out = run(p);
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak.is_finite() && peak <= 1.0, "peak {peak}");
    let diff: f32 = out
        .iter()
        .zip(&clean)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / clean.len() as f32;
    assert!(diff > 0.01, "eq left the signal unchanged");
    let mut flat = FxParams::default();
    flat.eq.mode = EqMode::Three;
    let out = run(flat);
    let diff: f32 = out
        .iter()
        .zip(&clean)
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / clean.len() as f32;
    assert!(diff < 1e-4, "flat eq changed the signal by {diff}");
}
#[test]
fn stereo_modes_fold_and_widen() {
    let n = 22_050;
    let mut data = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / 44_100.0;
        data.push((t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 0.3);
        data.push((t * 660.0 * 2.0 * std::f32::consts::PI).sin() * 0.3);
    }
    let shared = |p: FxParams| Arc::new(Mutex::new(p));
    let side = |out: &[f32]| {
        out.chunks(2).map(|f| (f[0] - f[1]).abs()).sum::<f32>() / (out.len() / 2) as f32
    };
    let mk = || {
        SamplesBuffer::new(
            NonZero::new(2).unwrap(),
            NonZero::new(44_100).unwrap(),
            data.clone(),
        )
    };
    let normal: Vec<f32> = Fx::new(mk(), shared(FxParams::default())).collect();
    assert_eq!(normal, data);
    let mono: Vec<f32> = Fx::new(
        mk(),
        shared(FxParams {
            stereo: StereoMode::Mono,
            ..Default::default()
        }),
    )
    .collect();
    assert!(side(&mono) < 1e-6, "mono still has side signal");
    let wide: Vec<f32> = Fx::new(
        mk(),
        shared(FxParams {
            stereo: StereoMode::Width,
            width: 200,
            ..Default::default()
        }),
    )
    .collect();
    let (s0, s1) = (side(&data), side(&wide));
    assert!(s1 > s0 * 1.5, "width 200% did not widen: {s0} -> {s1}");
    assert!(wide.iter().all(|v| v.abs() <= 1.0));
}
