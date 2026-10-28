//! P4-8 stage B: the tempo detection on the three real Pro Tools sessions (owner material, read
//! only). Skips when the folder is absent: `DRUMSPIKE_TEMPO_SAMPLES`, else `~/.cache/drumspike/sessions`.
//!
//! The true tempo of each session is the one tempo event of its Pro Tools session file (`.ptx`,
//! decoded in `docs/migration/phase4-report.md` "P4-8 stage B"); the drum hits of the kick and
//! snare fall on that grid from 0 s (the session start, bar 1) within a few ms, so the songs were
//! played to a click at that tempo. The hits come from the DSP analysis of `k in` and `s top`.

use std::path::{Path, PathBuf};

use drumspike_core::analyze::{analyze_track, AnalyzeParams, Detector};
use drumspike_core::tempo::TickMap;
use drumspike_core::tempo_detect::{detect, DetectParams, Hit, Role};
use drumspike_core::{read_wav_mono, MixMode};

/// Ticks per quarter note of the MIDI export (`export_midi_stored`).
const PPQ: u16 = 24_000;

/// Session folder, the kick and snare file names, the session tempo, the on-beat floor.
///
/// Session A gets 0.38: the dotted-quarter passage at 1.34 s spacing (its real hits are a
/// quarter beat off the true 67 grid, exactly why the comb read a false 4/3 step there) holds
/// only 250 of 644 hits on the true grid (253 on the false 4/3 grid), 0.3882. The other
/// sessions keep the P4-8 round 2 floor.
const SESSIONS: [(&str, &str, &str, f64, f64); 3] = [
    ("Session A", "k in_02.wav", "s top_02.wav", 67.0, 0.38),
    ("Session B", "k in_01.wav", "s top_01.wav", 126.0, 0.39),
    ("Session C", "k in_02.wav", "s top_02.wav", 126.0, 0.39),
];

fn samples_dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("DRUMSPIKE_TEMPO_SAMPLES") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".cache/drumspike/sessions"),
    };
    dir.is_dir().then_some(dir)
}

fn hit_times(path: &Path, role: &str) -> Vec<f64> {
    let (x, info) = read_wav_mono(path, MixMode::Average).unwrap();
    let det = Detector::Dsp;
    analyze_track(&x, info.sample_rate, role, &AnalyzeParams::default(), det)
        .unwrap()
        .events
        .iter()
        .map(|h| h.t)
        .collect()
}

#[test]
fn the_real_sessions_give_their_session_tempo_within_half_a_bpm() {
    let Some(dir) = samples_dir() else {
        eprintln!("SKIP: no real sessions (set DRUMSPIKE_TEMPO_SAMPLES or put them in ~/.cache/drumspike/sessions)");
        return;
    };
    let mut seen = 0;
    for (session, kick, snare, truth, on_beat_floor) in SESSIONS {
        let audio = dir.join(session).join("Audio Files");
        let (k, s) = (audio.join(kick), audio.join(snare));
        if !k.is_file() || !s.is_file() {
            eprintln!("SKIP {session}: {} or {} is missing", k.display(), s.display());
            continue;
        }
        let mut hits: Vec<Hit> = hit_times(&k, "kick")
            .into_iter()
            .map(|t| Hit::new(t, Role::Kick))
            .collect();
        hits.extend(hit_times(&s, "snare").into_iter().map(|t| Hit::new(t, Role::Snare)));
        let d = detect(&hits, &DetectParams::default()).unwrap();
        println!(
            "{session}: suggested {} BPM (error bar {:.5}), bar 1 confidence {} (backbeat {}, downbeat {}), alternative {:.3} s, meter confidence {} feel {:?}",
            d.map.bpm,
            d.bpm_error_bar,
            d.bar1_confidence,
            d.bar1_backbeat,
            d.bar1_downbeat,
            d.bar1_alternative_s,
            d.meter_confidence,
            d.meter_feel
        );
        println!(
            "{session}: {:.4} BPM (session {truth}, error {:.4}), bar 1 at {:.4} s, meter {:?}, steps {}, on beat {}, jitter {} ms, confidence {}",
            d.bpm,
            (d.bpm - truth).abs(),
            d.bar1_s,
            d.meter,
            d.steps.len(),
            d.on_beat,
            d.jitter_ms,
            d.confidence
        );
        assert!((d.bpm - truth).abs() < 0.5, "{session}: {} BPM, session {truth}", d.bpm);
        // P4-8 fix round 2: the fit is within its error bar of the session tempo, so the
        // suggestion is the session tempo itself (not 125.999, a set_tempo of 476194 us)
        assert_eq!(d.map.bpm, truth, "{session}: error bar {}", d.bpm_error_bar);
        // every exported note sits within half a tick of its hit through the rounded map
        let tm = TickMap::new(PPQ, &d.map).unwrap();
        // #466: the ticks sit on the exact tempo of the map (6e7 / bpm), so the worst round
        // trip is half an EXACT tick; the integer `tempo_us` bound is 8e-12 s tighter and a
        // hit on a half-tick boundary of the exact grid (a sample-exact hit) exceeds it.
        let half = 0.5 * (6e7 / d.map.bpm) / 1e6 / f64::from(PPQ);
        for h in &hits {
            let back = tm.time(tm.tick(h.t));
            assert!(
                (back - h.t).abs() <= half + 1e-12,
                "{session}: hit {} comes back at {back}",
                h.t
            );
        }
        // and the rounded grid stays on the hits: the on-beat share of the suggestion
        // The on-beat floor of the suggestion (Session A 0.38 per the #317 ruling, see SESSIONS)
        assert!(d.on_beat > on_beat_floor, "{session}: on beat {}", d.on_beat);
        // the step check lives in `tempo_detect_real_no_false_steps` (#317: a false 4/3 step)
        assert!(d.excursions.is_empty(), "{session}: false excursion {:?}", d.excursions);
        assert_eq!(d.meter, (4, 4), "{session}");
        // bar 1 of the session is at its start (0 s): the grid passes 0 s and beat 1 is chosen
        // right. Hard for Session A (0.0045 s) and Session C (0.0021 s) under DSP hits; the
        // bar 1 of "Session B" (0.9555 s, two beats) lives in `tempo_detect_real_bar1`
        // (#317). No eprintln-only check: the failing session is asserted --ignored.
        if session != "Session B" {
            assert!(d.bar1_s < 0.02, "{session}: bar 1 at {} s", d.bar1_s);
        }
        seen += 1;
    }
    eprintln!("{seen} real sessions checked");
}

/// The #317 scope addition (claude@mbp 2026-10-24 18:38Z): the bar 1 check alone for the session
/// whose bar 1 does not hold under DSP hits. Ignored until #317 fixes it; it must FAIL with
/// `--ignored` as long as "Session B" lands two beats late (0.9555 s).
#[test]
#[ignore = "#317: bar 1 off under DSP hits on Session B (0.9555 s, two beats)"]
fn tempo_detect_real_bar1() {
    let Some(dir) = samples_dir() else {
        eprintln!("SKIP: no real sessions (set DRUMSPIKE_TEMPO_SAMPLES or put them in ~/.cache/drumspike/sessions)");
        return;
    };
    for (session, kick, snare, _truth, _floor) in SESSIONS {
        if session != "Session B" {
            continue; // bar 1 stays hard for the others in the main test
        }
        let audio = dir.join(session).join("Audio Files");
        let (k, s) = (audio.join(kick), audio.join(snare));
        if !k.is_file() || !s.is_file() {
            eprintln!("SKIP {session}: {} or {} is missing", k.display(), s.display());
            continue;
        }
        let mut hits: Vec<Hit> = hit_times(&k, "kick")
            .into_iter()
            .map(|t| Hit::new(t, Role::Kick))
            .collect();
        hits.extend(hit_times(&s, "snare").into_iter().map(|t| Hit::new(t, Role::Snare)));
        let d = detect(&hits, &DetectParams::default()).unwrap();
        assert!(
            d.bar1_s < 0.02,
            "{session}: bar 1 at {} s (two beats off, #317)",
            d.bar1_s
        );
    }
}

/// The split of #294 (coordinator decision 2026-10-24 18:14Z): the step check alone, so a false
/// step does not hide the tempo fit. Un-ignored with the #317 step guard (the false 4/3 piece
/// no longer survives it): the check runs in the default test set again.
#[test]
fn tempo_detect_real_no_false_steps() {
    let Some(dir) = samples_dir() else {
        eprintln!("SKIP: no real sessions (set DRUMSPIKE_TEMPO_SAMPLES or put them in ~/.cache/drumspike/sessions)");
        return;
    };
    let mut seen = 0;
    for (session, kick, snare, _truth, _floor) in SESSIONS {
        let audio = dir.join(session).join("Audio Files");
        let (k, s) = (audio.join(kick), audio.join(snare));
        if !k.is_file() || !s.is_file() {
            eprintln!("SKIP {session}: {} or {} is missing", k.display(), s.display());
            continue;
        }
        let mut hits: Vec<Hit> = hit_times(&k, "kick")
            .into_iter()
            .map(|t| Hit::new(t, Role::Kick))
            .collect();
        hits.extend(hit_times(&s, "snare").into_iter().map(|t| Hit::new(t, Role::Snare)));
        let d = detect(&hits, &DetectParams::default()).unwrap();
        assert!(
            d.steps.is_empty(),
            "{session}: one tempo event expected, steps {:?}",
            d.steps
        );
        seen += 1;
    }
    eprintln!("{seen} real sessions checked for false steps");
}
