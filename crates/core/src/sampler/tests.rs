use super::*;
use crate::tempo::{MeterChange, TempoStep};
use midly::{MidiMessage, Smf, TrackEventKind};

/// A drum-like transient: `pre` samples of silence, then a decaying 180 Hz tone plus a decaying
/// noise burst (a fixed LCG), `len` samples in all.
fn shot(sr: u32, pre: usize, len: usize, seed: u64) -> Vec<f32> {
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

/// The note-ons of a file as (track, tick, key, velocity).
fn note_ons(bytes: &[u8]) -> Vec<(usize, u64, u8, u8)> {
    let smf = Smf::parse(bytes).unwrap();
    let mut out = Vec::new();
    for (i, tr) in smf.tracks.iter().enumerate() {
        let mut tick = 0u64;
        for e in tr {
            tick += e.delta.as_int() as u64;
            if let TrackEventKind::Midi {
                message: MidiMessage::NoteOn { key, vel },
                ..
            } = e.kind
                && vel.as_int() > 0
            {
                out.push((i, tick, key.as_int(), vel.as_int()));
            }
        }
    }
    out
}

#[test]
fn the_calibration_note_is_on_the_start_of_bar_2() {
    let (tick, t) = calibration_time(&TempoMap::default(), 24_000).unwrap();
    assert_eq!((tick, t), (96_000, 2.0));
    // 90 BPM, 3/4, bar 1 at 1.5 s: one bar is 3 beats of 2/3 s
    let map = TempoMap {
        bpm: 90.0,
        meter: (3, 4),
        bar1_s: 1.5,
        ..TempoMap::default()
    };
    let (tick, t) = calibration_time(&map, 24_000).unwrap();
    let tm = TickMap::new(24_000, &map).unwrap();
    assert_eq!(tick, tm.bar1_tick(&map) + 3 * 24_000);
    assert!((t - 3.5).abs() < 1e-5, "{t}");
    // a tempo step before bar 2 is followed; a meter change later does not matter
    let map = TempoMap {
        steps: vec![TempoStep { t: 1.0, bpm: 60.0 }],
        changes: vec![MeterChange { bar: 3, num: 7, den: 8 }],
        ..TempoMap::default()
    };
    let (tick, t) = calibration_time(&map, 24_000).unwrap();
    assert_eq!(tick, 96_000);
    assert!((t - 3.0).abs() < 1e-5, "1 s at 120 then 4 beats minus 2 at 60: {t}");
}

#[test]
fn the_calibration_file_holds_one_note_at_the_known_tick() {
    let (bytes, t) = calibration_midi(38, &TempoMap::default(), 24_000, 120.0).unwrap();
    assert_eq!(t, 2.0);
    assert_eq!(note_ons(&bytes), vec![(1, 96_000, 38, 100)]);
    let map = TempoMap {
        bpm: 90.0,
        bar1_s: 1.0,
        ..TempoMap::default()
    };
    let (bytes, t) = calibration_midi(36, &map, 24_000, 120.0).unwrap();
    let (tick, t2) = calibration_time(&map, 24_000).unwrap();
    assert_eq!(t, t2);
    assert_eq!(note_ons(&bytes), vec![(1, tick as u64, 36, 100)]);
    assert!(calibration_midi(200, &TempoMap::default(), 24_000, 120.0).is_err());
}

#[test]
fn the_offset_follows_a_planted_delay_exactly() {
    for sr in [44_100u32, 48_000, 96_000] {
        let note = 2.0;
        let at = |d: usize| {
            let x = shot(sr, (note * sr as f64) as usize + d, 4 * sr as usize, 5);
            measure_offset(&x, sr, note).unwrap()
        };
        let base = at(0);
        assert!(
            base.offset_samples.abs() <= 0.0005 * sr as f64,
            "{sr}: bias {}",
            base.offset_samples
        );
        for ms in [0.1, 1.0, 5.5, 12.0, 40.0] {
            let d = (ms * 1e-3 * sr as f64).round() as usize;
            let m = at(d);
            assert_eq!(m.offset_samples - base.offset_samples, d as f64, "{sr} Hz, {d} samples");
            assert_eq!(m.sr, sr);
        }
    }
}

#[test]
fn a_note_that_is_not_on_a_whole_sample_gives_a_fractional_offset() {
    let sr = 44_100;
    let note = 2.0 + 0.3 / sr as f64;
    let x = shot(sr, 2 * sr as usize + 100, 3 * sr as usize, 5);
    let m = measure_offset(&x, sr, note).unwrap();
    let m0 = measure_offset(&shot(sr, 2 * sr as usize, 3 * sr as usize, 5), sr, 2.0).unwrap();
    assert!((m.offset_samples - (m0.offset_samples + 100.0 - 0.3)).abs() < 1e-6);
    assert!(m.offset_samples.fract() != 0.0);
}

#[test]
fn a_silent_bounce_has_no_offset() {
    assert!(measure_offset(&[0.0; 4800], 48_000, 0.0).is_err());
    assert!(measure_offset(&[], 48_000, 0.0).is_err());
    assert!(measure_offset(&[0.5; 10], 0, 0.0).is_err());
}

/// A mic with hits of `shot` (one per 0.5 s, different gains) and its hit samples.
fn mic_with_hits(sr: u32, n_hits: usize) -> (Vec<f32>, Vec<i64>) {
    let n = sr as usize * (n_hits / 2 + 2);
    let mut mic = vec![0.0f32; n];
    let mut hits = Vec::new();
    let one = shot(sr, 0, sr as usize / 4, 9);
    for k in 0..n_hits {
        let at = sr as usize / 2 + k * sr as usize / 2;
        let g = 0.3 + 0.1 * (k % 5) as f32;
        for (i, v) in one.iter().enumerate() {
            mic[at + i] += g * v;
        }
        hits.push(at as i64);
    }
    (mic, hits)
}

#[test]
fn the_polarity_of_a_bounce_against_the_mic_is_found_for_both_signs() {
    let sr = 48_000;
    let (mic, hits) = mic_with_hits(sr, 12);
    let p = AlignParams::default();
    // a bounce of the same drum, its transient 1,234 samples after its note, with a different level
    let pre = 2 * sr as usize + 1_234;
    let normal: Vec<f32> = shot(sr, pre, 3 * sr as usize, 9).iter().map(|v| 0.5 * v).collect();
    let onset = measure_offset(&normal, sr, 2.0).unwrap().onset;
    let (s, r) = bounce_polarity(&normal, onset, &mic, &hits, sr, &p).unwrap();
    assert_eq!(s, 1);
    assert!(r > 0.95, "{r}");
    let inverted: Vec<f32> = normal.iter().map(|v| -v).collect();
    let (s, r) = bounce_polarity(&inverted, onset, &mic, &hits, sr, &p).unwrap();
    assert_eq!(s, -1);
    assert!(r > 0.95, "{r}");
    // no hit, or a silent mic, or a silent bounce: nothing
    assert!(bounce_polarity(&normal, onset, &mic, &[], sr, &p).is_none());
    assert!(bounce_polarity(&normal, onset, &vec![0.0; mic.len()], &hits, sr, &p).is_none());
    assert!(bounce_polarity(&vec![0.0; normal.len()], onset, &mic, &hits, sr, &p).is_none());
}

#[test]
fn the_median_shape_ignores_a_silent_hit_and_one_odd_hit() {
    let sr = 48_000;
    let (mut mic, mut hits) = mic_with_hits(sr, 9);
    // a hit where the mic is silent and a hit with another shape
    hits.push(mic.len() as i64 - 100);
    for v in &mut mic[hits[4] as usize..hits[4] as usize + 480] {
        *v = -*v;
    }
    let shape = median_shape(&mic, &hits, 48, 480, 144).unwrap();
    assert_eq!(shape.len(), 48 + 480 + 2 * 144);
    let (a, pre) = attack_template(&shot(sr, 1_000, 5_000, 9), 1_000, sr, &AlignParams::default());
    let x = xcorr_hit(&shape, (pre + 144) as i64, &a, pre, 144).unwrap();
    assert!(x.pos_r > 0.95 && x.pos_lag.abs() < 0.2, "{x:?}");
    assert!(median_shape(&mic, &[], 48, 480, 144).is_none());
}

#[test]
fn compensate_places_the_aligned_attack_minus_the_offset_on_a_sample() {
    // no lag: the hit minus the offset
    assert_eq!(compensate(1.0, 0.0, 480.0, 48_000), (47_520.0 / 48_000.0, false));
    // a lag of -3.4 samples and an offset of 12.3: 48,000 - 3.4 - 12.3 = 47,984.3 rounds to 47,984
    let (t, c) = compensate(1.0, -3.4, 12.3, 48_000);
    assert!(!c);
    assert_eq!((t * 48_000.0).round(), 47_984.0);
    assert!((t * 48_000.0 - 47_984.0).abs() < 1e-6, "on a whole sample");
    // rounding: .5 goes up, below goes down
    assert_eq!((compensate(0.5, 0.0, 0.4, 1000).0 * 1000.0).round(), 500.0);
    assert_eq!((compensate(0.5, 0.0, 0.6, 1000).0 * 1000.0).round(), 499.0);
    // a negative offset (the sampler plays early) moves the note later
    assert_eq!(compensate(1.0, 0.0, -48.0, 48_000), (1.001, false));
    // before the first sample: 0 s and flagged; exactly sample 0 is not before
    assert_eq!(compensate(0.001, 0.0, 100.0, 48_000), (0.0, true));
    assert_eq!(compensate(0.001, 0.0, 48.0, 48_000), (0.0, false));
    assert_eq!(compensate(0.001, 0.0, 48.4, 48_000), (0.0, false));
    assert_eq!(compensate(0.001, 0.0, 48.6, 48_000), (0.0, true));
}
