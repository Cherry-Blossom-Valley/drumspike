//! A5 probe: the real Pro Tools sessions under the delta rule. The finding that set
//! `s_ref` to 23 (the shipped-scale reference): with the ENST-median reference (snare 37)
//! the Session A snare (mag q995 23.8) is boosted 1.55x, gains 6 hits and the tempo test
//! `tempo_detect_real` reads a false 94-bpm excursion at 31.74 s; with the reference at
//! the shipped scale (23) the session runs shipped and the test is green. The gained
//! candidates themselves are real strokes (attack 60 dB, flux 5.0 at t 34.91).
//! Run: `cargo test -p drumspike-core --test probe412_tempo -j 8 -- --nocapture --ignored`

use drumspike_core::analyze::{AnalyzeParams, Detector, analyze_track};
use drumspike_core::detect::features::N_BANDS;
use drumspike_core::detect::{Chain, prepare};
use drumspike_core::{MixMode, read_wav_mono};
use std::path::PathBuf;

fn mag_q995(path: &std::path::Path, role: &str) {
    let (x, info) = read_wav_mono(path, MixMode::Average).unwrap();
    let spec = prepare(&x, info.sample_rate, Chain::Mix).unwrap();
    let bands = drumspike_core::detect::onset::band_range(
        if role == "kick" { 20.0 } else { 100.0 },
        if role == "kick" { 4_000.0 } else { 300.0 },
    );
    let frames = spec.len() / N_BANDS;
    let mut mags: Vec<f32> = Vec::with_capacity(frames * bands.len());
    for i in 0..frames {
        for b in bands.clone() {
            mags.push(10f32.powf(spec[i * N_BANDS + b]) - 1.0);
        }
    }
    mags.sort_by(|a, b| a.total_cmp(b));
    let q = |p: f32| -> f32 { mags[((mags.len() as f32 - 1.0) * p).round() as usize] };
    println!(
        "{} {role}: mag q995 {:.4} q999 {:.4}",
        path.file_name().unwrap().to_string_lossy(),
        q(0.995),
        q(0.999)
    );
}

fn hit_times(path: &std::path::Path, role: &str) -> Vec<f64> {
    let (x, info) = read_wav_mono(path, MixMode::Average).unwrap();
    analyze_track(&x, info.sample_rate, role, &AnalyzeParams::default(), Detector::Dsp)
        .unwrap()
        .events
        .iter()
        .map(|h| h.t)
        .collect()
}

#[test]
#[ignore = "A5 probe: needs the real sessions in ~/.cache/drumspike/sessions, run with --ignored"]
fn probe() {
    let dir = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cache/drumspike/sessions/Session A/Audio Files");
    for rule in ["0", "1"] {
        unsafe {
            std::env::set_var("DRUMSPIKE_DELTA_RULE", rule);
        }
        for (file, role) in [("k in_02.wav", "kick"), ("s top_02.wav", "snare")] {
            mag_q995(&dir.join(file), role);
            let (x, info) = read_wav_mono(&dir.join(file), MixMode::Average).unwrap();
            let (a, cands, _, _) =
                drumspike_core::analyze::analyze_track_dsp_cands(&x, info.sample_rate, role, &AnalyzeParams::default())
                    .unwrap();
            println!(
                "rule {rule} {file} role {role}: {} hits, cands {}",
                a.events.len(),
                cands.len()
            );
            if rule == "1" && role == "snare" {
                for c in &cands {
                    if (29.0..35.0).contains(&c.time) {
                        println!(
                            "  t {:.4} kept {} gate {:?} flux {:.3} rel {:.1} attack {:.1}",
                            c.time, c.kept, c.gate, c.flux, c.rel_db, c.attack
                        );
                    }
                }
            }
            let a = hit_times(&dir.join(file), role);
            println!("rule {rule} {file} role {role}: {} hits", a.len());
            for t in &a {
                if (29.0..35.0).contains(t) {
                    println!("  {t:.4}");
                }
            }
        }
    }
}
