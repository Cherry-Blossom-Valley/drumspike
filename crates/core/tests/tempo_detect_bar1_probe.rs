//! #180 step 1: bar-1 offset in beats on the real sessions (owner material, read only).
//! The reference bar 1 is the session's tempo map: 0 s, as in `tempo_detect_real.rs`. The offset
//! is `bar1_s / beat` with the beat of the detected map. Output lines `BAR1 <session> ...`.
//! Skips when the folder is absent: `DRUMSPIKE_TEMPO_SAMPLES`, else `~/.cache/drumspike/sessions`.
//!
//! Run: `DRUMSPIKE_TEMPO_SAMPLES<the sessions folder> cargo test -p
//! drumspike-core --test tempo_detect_bar1_probe -- --nocapture --ignored`

use std::path::{Path, PathBuf};

use drumspike_core::analyze::{AnalyzeParams, Detector, analyze_track};
use drumspike_core::tempo_detect::{DetectParams, Hit, Role, detect};
use drumspike_core::{MixMode, read_wav_mono};

const SESSIONS: [(&str, &str, &str); 3] = [
    ("Session A", "k in_02.wav", "s top_02.wav"),
    ("Session B", "k in_01.wav", "s top_01.wav"),
    ("Session C", "k in_02.wav", "s top_02.wav"),
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
    analyze_track(&x, info.sample_rate, role, &AnalyzeParams::default(), Detector::Dsp)
        .unwrap()
        .events
        .iter()
        .map(|h| h.t)
        .collect()
}

#[test]
#[ignore = "#180 measurement probe"]
fn bar1_offsets_in_beats() {
    let Some(dir) = samples_dir() else {
        eprintln!("SKIP: no real sessions");
        return;
    };
    for (session, kick, snare) in SESSIONS {
        let audio = dir.join(session).join("Audio Files");
        let (k, s) = (audio.join(kick), audio.join(snare));
        if !k.is_file() || !s.is_file() {
            eprintln!("SKIP {session}");
            continue;
        }
        let mut hits: Vec<Hit> = hit_times(&k, "kick")
            .into_iter()
            .map(|t| Hit::new(t, Role::Kick))
            .collect();
        hits.extend(hit_times(&s, "snare").into_iter().map(|t| Hit::new(t, Role::Snare)));
        let d = detect(&hits, &DetectParams::default()).unwrap();
        let beat = 60.0 / d.map.bpm;
        // hits per beat class mod 4 against the detected bar 1 (class 0 = the suggested beat 1);
        // on-beat hits only (within 1/8 beat of a beat)
        let mut hist = [[0u32; 4]; 2];
        for h in &hits {
            let x = (h.t - d.bar1_s) / beat;
            if (x - x.round()).abs() > 0.125 {
                continue;
            }
            let c = (x.round() as i64).rem_euclid(4) as usize;
            match h.role {
                Role::Kick => hist[0][c] += 1,
                Role::Snare => hist[1][c] += 1,
                _ => {}
            }
        }
        println!(
            "BAR1 {session}: kick by class {:?} snare by class {:?}",
            hist[0], hist[1]
        );
        println!(
            "BAR1 {session}: bpm {} bar1_s {:.4} offset {:.2} beats, backbeat {} downbeat {}",
            d.map.bpm,
            d.bar1_s,
            d.bar1_s / beat,
            d.bar1_backbeat,
            d.bar1_downbeat
        );
    }
}
