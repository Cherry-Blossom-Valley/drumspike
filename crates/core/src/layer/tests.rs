use super::*;
use crate::io::{WavFormat, WavSpec, write_wav};

/// A drum-like one-shot: `pre` samples of silence, then a decaying 180 Hz tone plus a decaying
/// noise burst (a fixed LCG), `len` samples in all.
fn shot_signal(sr: u32, pre: usize, len: usize, seed: u64) -> Vec<f32> {
    let mut s = seed;
    let mut noise = || {
        s = s
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((s >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0
    };
    (0..len)
        .map(|i| {
            if i < pre {
                return 0.0;
            }
            let t = (i - pre) as f64 / sr as f64;
            let tone = (2.0 * std::f64::consts::PI * 180.0 * t).sin() * (-t / 0.08).exp();
            let burst = noise() * (-t / 0.004).exp();
            (0.7 * tone + 0.5 * burst) as f32
        })
        .collect()
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("drumspike-layer-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// An AIFF (or AIFC with `comp`) file written by hand: `frames` big-endian samples of `bits`.
fn aiff_bytes(sr_ext: [u8; 10], ch: u16, bits: u16, comp: Option<&[u8; 4]>, data: &[u8], frames: u32) -> Vec<u8> {
    let mut comm = Vec::new();
    comm.extend_from_slice(&(ch as i16).to_be_bytes());
    comm.extend_from_slice(&frames.to_be_bytes());
    comm.extend_from_slice(&(bits as i16).to_be_bytes());
    comm.extend_from_slice(&sr_ext);
    if let Some(c) = comp {
        comm.extend_from_slice(c);
        comm.extend_from_slice(&[0, 0]); // empty pascal name
    }
    let mut ssnd = vec![0u8; 8];
    ssnd.extend_from_slice(data);
    let mut body = Vec::new();
    body.extend_from_slice(if comp.is_some() { b"AIFC" } else { b"AIFF" });
    for (id, c) in [(b"COMM", &comm), (b"SSND", &ssnd)] {
        body.extend_from_slice(id);
        body.extend_from_slice(&(c.len() as u32).to_be_bytes());
        body.extend_from_slice(c);
        if c.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// 48,000 and 44,100 as 80-bit extended numbers.
const EXT_48K: [u8; 10] = [0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0];
const EXT_44K1: [u8; 10] = [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0];

#[test]
fn ext80_reads_the_aiff_rates() {
    assert_eq!(ext80_to_f64(&EXT_48K), 48_000.0);
    assert_eq!(ext80_to_f64(&EXT_44K1), 44_100.0);
    assert_eq!(ext80_to_f64(&[0; 10]), 0.0);
    assert_eq!(ext80_to_f64(&[0; 3]), 0.0);
}

#[test]
fn read_one_shot_reads_wav_aiff_and_aifc() {
    let d = tmp("read");
    let x: Vec<f32> = vec![0.0, 0.5, -0.25, 0.125];
    // 16-bit AIFF, big-endian
    let be16: Vec<u8> = x.iter().flat_map(|v| ((v * 32768.0) as i16).to_be_bytes()).collect();
    let p = d.join("a.aif");
    std::fs::write(&p, aiff_bytes(EXT_48K, 1, 16, None, &be16, 4)).unwrap();
    let s = read_one_shot(&p).unwrap();
    assert_eq!((s.sr, s.channels(), s.frames()), (48_000, 1, 4));
    assert_eq!(s.chans[0], x);
    // 24-bit AIFF, stereo
    let be24: Vec<u8> = x
        .iter()
        .flat_map(|v| {
            let i = (v * 8_388_608.0) as i32;
            let b = i.to_be_bytes();
            [b[1], b[2], b[3], b[1], b[2], b[3]]
        })
        .collect();
    let p = d.join("b.aiff");
    std::fs::write(&p, aiff_bytes(EXT_44K1, 2, 24, None, &be24, 4)).unwrap();
    let s = read_one_shot(&p).unwrap();
    assert_eq!((s.sr, s.channels(), s.frames()), (44_100, 2, 4));
    assert_eq!(s.chans[1], x);
    // AIFC sowt (little-endian 16) and fl32
    let le16: Vec<u8> = x.iter().flat_map(|v| ((v * 32768.0) as i16).to_le_bytes()).collect();
    let p = d.join("c.aifc");
    std::fs::write(&p, aiff_bytes(EXT_48K, 1, 16, Some(b"sowt"), &le16, 4)).unwrap();
    assert_eq!(read_one_shot(&p).unwrap().chans[0], x);
    let f32be: Vec<u8> = x.iter().flat_map(|v| v.to_be_bytes()).collect();
    let p = d.join("d.aifc");
    std::fs::write(&p, aiff_bytes(EXT_48K, 1, 32, Some(b"fl32"), &f32be, 4)).unwrap();
    assert_eq!(read_one_shot(&p).unwrap().chans[0], x);
    let p = d.join("e.aifc");
    std::fs::write(&p, aiff_bytes(EXT_48K, 1, 16, Some(b"ulaw"), &le16, 4)).unwrap();
    assert!(read_one_shot(&p).unwrap_err().contains("not supported"));
    // WAV, 4 channels are folded to their mean
    let p = d.join("f.wav");
    let spec = WavSpec {
        sample_rate: 96_000,
        channels: 4,
        format: WavFormat::Float32,
        extensible: false,
        channel_mask: 0,
    };
    let inter: Vec<f32> = x.iter().flat_map(|v| [*v, *v, 0.0, 0.0]).collect();
    write_wav(&p, &spec, &inter, None).unwrap();
    let s = read_one_shot(&p).unwrap();
    assert_eq!((s.sr, s.channels(), s.frames()), (96_000, 1, 4));
    assert_eq!(s.chans[0], x.iter().map(|v| v / 2.0).collect::<Vec<_>>());
    // empty and missing files
    let p = d.join("g.wav");
    write_wav(&p, &WavSpec { channels: 1, ..spec }, &[], None).unwrap();
    assert!(read_one_shot(&p).unwrap_err().contains("no audio"));
    assert!(read_one_shot(&d.join("missing.wav")).is_err());
    std::fs::write(d.join("h.txt"), b"FORMxxxxWAVE").unwrap();
    assert!(read_one_shot(&d.join("h.txt")).is_err());
}

#[test]
fn one_shot_mono_interleaved_and_resampled() {
    let s = OneShot {
        sr: 48_000,
        chans: vec![vec![1.0, 0.0, 0.5], vec![0.0, 1.0, 0.5]],
    };
    assert_eq!(s.mono(), vec![0.5, 0.5, 0.5]);
    assert_eq!(s.interleaved(), vec![1.0, 0.0, 0.0, 1.0, 0.5, 0.5]);
    assert_eq!(s.resampled(48_000), s);
    let r = s.resampled(96_000);
    assert_eq!((r.sr, r.frames(), r.channels()), (96_000, 6, 2));
}

#[test]
fn the_onset_is_the_attack_not_the_file_start() {
    for sr in [44_100u32, 48_000, 96_000] {
        let pre = (0.012 * sr as f64) as usize;
        let x = shot_signal(sr, pre, sr as usize / 2, 7);
        let on = one_shot_onset(&x, sr);
        assert!(
            (on as i64 - pre as i64).abs() <= (0.0005 * sr as f64) as i64,
            "{sr}: onset {on} for the attack at {pre}"
        );
        // an attack at the first sample
        let y = shot_signal(sr, 0, sr as usize / 4, 3);
        let o = one_shot_onset(&y, sr);
        assert!(o as f64 <= 0.0005 * sr as f64, "{sr}: {o}");
    }
    assert_eq!(one_shot_onset(&[0.0; 100], 48_000), 0);
    assert_eq!(one_shot_onset(&[], 48_000), 0);
}

#[test]
fn sinc_resample_keeps_a_tone() {
    let (from, to) = (44_100u32, 48_000u32);
    let x: Vec<f32> = (0..4_410)
        .map(|i| (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / from as f64).sin() as f32)
        .collect();
    let y = sinc_resample(&x, from, to);
    assert_eq!(y.len(), 4_800);
    for (m, v) in y.iter().enumerate().take(4_700).skip(100) {
        let want = (2.0 * std::f64::consts::PI * 1000.0 * m as f64 / to as f64).sin();
        assert!((*v as f64 - want).abs() < 2e-3, "{m}: {v} {want}");
    }
    // downwards: a tone above the new Nyquist is removed
    let hi: Vec<f32> = (0..9_600)
        .map(|i| (2.0 * std::f64::consts::PI * 30_000.0 * i as f64 / 96_000.0).sin() as f32)
        .collect();
    let d = sinc_resample(&hi, 96_000, 48_000);
    assert_eq!(d.len(), 4_800);
    let peak = d[200..4_600].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak < 0.01, "aliased {peak}");
    assert_eq!(sinc_resample(&x, from, from), x);
}

#[test]
fn frac_delayed_shifts_by_the_fraction() {
    let f = |t: f64| (2.0 * std::f64::consts::PI * t / 37.0).sin() * (-((t - 200.0) / 80.0).powi(2)).exp();
    let x: Vec<f32> = (0..400).map(|i| f(i as f64) as f32).collect();
    for frac in [0.0, 0.25, 0.5, 0.9] {
        let y = frac_delayed(&x, frac);
        assert_eq!(y.len(), 400 + 2 * SINC_HALF);
        for n in 50..350usize {
            let want = f(n as f64 - frac);
            let got = y[n + SINC_HALF] as f64;
            assert!((got - want).abs() < 2e-3, "frac {frac} n {n}: {got} {want}");
        }
    }
}

#[test]
fn the_velocity_curves() {
    let db = Curve::default();
    assert_eq!(db, Curve::Db { range_db: 24.0 });
    assert!((db.gain(127) - 1.0).abs() < 1e-12);
    assert!((db.gain(0) - 10f64.powf(-24.0 / 20.0)).abs() < 1e-12);
    assert!(
        (20.0 * db.gain(64).log10() + 24.0 * (1.0 - 64.0 / 127.0)).abs() < 1e-9,
        "linear in dB"
    );
    assert_eq!(db.gain(200), db.gain(127));
    assert_eq!(Curve::Linear.gain(127), 1.0);
    assert!((Curve::Linear.gain(64) - 64.0 / 127.0).abs() < 1e-12);
    assert_eq!(Curve::Fixed.gain(3), 1.0);
    for c in [Curve::Db { range_db: 12.0 }, Curve::Linear, Curve::Fixed] {
        assert_eq!(Curve::from_json(&c.to_json()), Some(c));
    }
    assert_eq!(
        Curve::from_json(&serde_json::json!({"kind": "db"})),
        Some(Curve::default())
    );
    assert_eq!(
        Curve::from_json(&serde_json::json!({"kind": "db", "range_db": 200})),
        None
    );
    assert_eq!(Curve::from_json(&serde_json::json!({"kind": "exp"})), None);
    assert_eq!(Curve::from_json(&serde_json::json!(3)), None);
}

/// A close mic: a quieter noise floor, the one-shot at `onset_pos` (fractional) with `sign`, plus a
/// little of another sound.
fn mic_with(shot: &[f32], onset: usize, onset_pos: f64, sign: f32, n: usize) -> Vec<f32> {
    let mut mic: Vec<f32> = (0..n).map(|i| 0.002 * ((i as f64 * 0.37).sin() as f32)).collect();
    let r = render_layer(
        n,
        &OneShot {
            sr: 48_000,
            chans: vec![shot.to_vec()],
        },
        onset as f64,
        &[(onset_pos, sign)],
    );
    for (m, v) in mic.iter_mut().zip(&r[0]) {
        *m += v;
    }
    mic
}

#[test]
fn xcorr_finds_planted_lags_and_the_inverted_polarity() {
    let sr = 48_000u32;
    let shot = shot_signal(sr, 240, 9_600, 11);
    let onset = one_shot_onset(&shot, sr);
    let p = AlignParams::default();
    let (a, pre) = attack_template(&shot, onset, sr, &p);
    assert_eq!(pre, 48);
    assert_eq!(a.len(), 480);
    let max_lag = (p.max_lag_s * sr as f64).round() as usize;
    let hit = 20_000i64;
    for (lag, sign) in [
        (0.0, 1.0f32),
        (7.0, 1.0),
        (-12.0, -1.0),
        (3.5, 1.0),
        (-40.25, -1.0),
        (100.0, 1.0),
    ] {
        let mic = mic_with(&shot, onset, hit as f64 + lag, sign, 48_000);
        let x = xcorr_hit(&mic, hit, &a, pre, max_lag).unwrap();
        let s: i8 = if sign < 0.0 { -1 } else { 1 };
        assert_eq!(x.sign(), s, "lag {lag} sign {sign}: {x:?}");
        let (got, r) = x.for_sign(s);
        assert!((got - lag).abs() < 0.2, "lag {lag}: {got} ({x:?})");
        assert!(r > 0.97 && r <= 1.0, "lag {lag}: match {r}");
        // the other sign matches worse
        assert!(x.for_sign(-s).1 < r);
    }
    // no attack: a silent mic gives no answer, a silent attack neither
    assert!(xcorr_hit(&[0.0; 1000], 500, &a, pre, max_lag).is_none());
    assert!(xcorr_hit(&shot, 500, &[0.0; 10], 1, 4).is_none());
    // a hit at the file edge reads zeros outside
    let mic = mic_with(&shot, onset, 10.0, 1.0, 4_000);
    let x = xcorr_hit(&mic, 10, &a, pre, max_lag).unwrap();
    assert!((x.pos_lag).abs() < 0.2, "{x:?}");
}

#[test]
fn xcorr_marks_a_lag_on_the_search_edge_uncertain() {
    let sr = 48_000u32;
    let shot = shot_signal(sr, 240, 9_600, 11);
    let onset = one_shot_onset(&shot, sr);
    let p = AlignParams::default();
    let (a, pre) = attack_template(&shot, onset, sr, &p);
    let max_lag = (p.max_lag_s * sr as f64).round() as usize;
    // a lag exactly on the window's edge: the best sits on the last index
    let far = p.max_lag_s;
    let mic = mic_with(&shot, onset, 20_000.0 + far * sr as f64, 1.0, sr as usize);
    let x = xcorr_hit(&mic, 20_000, &a, pre, max_lag).unwrap();
    assert!(x.pos_edge, "{x:?}");
    // a well-inside lag is not on the edge
    let mic = mic_with(&shot, onset, 20_000.5, 1.0, sr as usize);
    let x = xcorr_hit(&mic, 20_000, &a, pre, max_lag).unwrap();
    assert!(!x.pos_edge, "{x:?}");
    assert_eq!(x.for_sign_edge(1).0, x.pos_lag);
}

#[test]
fn majority_sign_counts_the_better_sign() {
    let h = |p: f64, n: f64| HitXcorr {
        pos_lag: 0.0,
        pos_r: p,
        neg_lag: 1.0,
        neg_r: n,
        pos_edge: false,
        neg_edge: false,
    };
    assert_eq!(majority_sign(&[h(0.9, 0.2), h(0.1, 0.8), h(0.2, 0.7)]), (-1, 1, 2));
    assert_eq!(majority_sign(&[h(0.9, 0.2), h(0.1, 0.8)]), (1, 1, 1), "a tie is as is");
    assert_eq!(majority_sign(&[]), (1, 0, 0));
    assert_eq!(h(0.5, 0.5).sign(), 1);
    assert_eq!(h(0.5, 0.6).for_sign(-1), (1.0, 0.6));
}

#[test]
fn render_layer_puts_each_onset_on_its_position_and_overlaps_add() {
    let shot = OneShot {
        sr: 48_000,
        chans: vec![vec![0.0, 0.0, 1.0, 0.5, 0.25], vec![0.0, 0.0, -1.0, -0.5, -0.25]],
    };
    let out = render_layer(
        20,
        &shot,
        2.0,
        &[(5.0, 1.0), (6.0, 0.5), (0.0, 1.0), (19.0, 2.0), (f64::NAN, 1.0)],
    );
    assert_eq!(out.len(), 2);
    // hit at 5: the onset (index 2) on sample 5; hit at 6 at half gain adds
    assert_eq!(out[0][5], 1.0);
    assert_eq!(out[0][6], 0.5 + 0.5);
    assert_eq!(out[0][7], 0.25 + 0.25);
    assert_eq!(out[0][8], 0.125);
    assert_eq!(out[1][5], -1.0);
    // hit at 0: the samples before the onset fall before the file
    assert_eq!(out[0][0], 1.0);
    assert_eq!(out[0][1], 0.5);
    // hit at 19: cut at the end
    assert_eq!(out[0][19], 2.0);
    assert!(out[0][9..19].iter().all(|v| *v == 0.0));
    // fractional: the render at 100.5 equals the half-sample delayed one-shot
    let x = shot_signal(48_000, 10, 400, 5);
    let s1 = OneShot {
        sr: 48_000,
        chans: vec![x.clone()],
    };
    let r = render_layer(1_000, &s1, 10.0, &[(100.5, 1.0)]);
    let d = frac_delayed(&x, 0.5);
    for (k, v) in d.iter().enumerate() {
        let i = 90 + k as i64 - SINC_HALF as i64;
        assert_eq!(r[0][i as usize], *v, "{k}");
    }
    // a fraction that rounds up to the next sample is the whole-sample copy
    let r2 = render_layer(1_000, &s1, 10.0, &[(100.999, 1.0)]);
    let r3 = render_layer(1_000, &s1, 10.0, &[(101.0, 1.0)]);
    assert_eq!(r2, r3);
}

#[test]
fn render_layer_takes_a_fractional_onset_sub_sample_exact() {
    // P4-7a fix 7: a one-shot resampled from another rate has its attack between two samples
    // (onset 139.32); the copy starts at hit minus that onset, not minus its rounding.
    let x = shot_signal(48_000, 10, 400, 9);
    let s = OneShot {
        sr: 48_000,
        chans: vec![x],
    };
    let exact = render_layer(2_000, &s, 139.32, &[(500.0, 1.0)]);
    // the same start position given as a fractional hit with a whole onset
    let same = render_layer(2_000, &s, 139.0, &[(499.68, 1.0)]);
    assert_eq!(exact, same);
    // the planted fault (the onset rounded to 139) moves the copy by 0.32 sample
    let rounded = render_layer(2_000, &s, 139.0, &[(500.0, 1.0)]);
    assert_ne!(exact, rounded);
}
