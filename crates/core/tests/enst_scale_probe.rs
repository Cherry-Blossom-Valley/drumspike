//! A5 probe on ENST: the track's loudness scale per role (the q995 of the linear band
//! magnitudes over the role's bands), to calibrate `s_ref` of the gained-spectrogram
//! rule. Reads a few close-mic wavs per drummer from the ENST copy (env `ENST_ROOT`).
//! Ignored by default.
//!
//! Run: `ENST_ROOT=~/.cache/drumspike/datasets/ENST-drums-public cargo test -p drumspike-core --test enst_scale_probe -j 8 -- --nocapture --ignored`

use drumspike_core::detect::features::N_BANDS;
use drumspike_core::detect::onset::band_range;
use drumspike_core::detect::{Chain, DetectError, prepare};
use drumspike_core::io::{MixMode, read_wav_mono};
use std::path::PathBuf;

fn role_bands(role: &str) -> std::ops::Range<usize> {
    let (lo, hi) = match role {
        "kick" => (20.0, 4_000.0),
        "snare" => (100.0, 300.0),
        "hat" => (2_000.0, 20_000.0),
        _ => (50.0, 600.0),
    };
    band_range(lo, hi)
}

fn mag_scale(spec: &[f32], bands: &std::ops::Range<usize>) -> Option<f32> {
    let frames = spec.len() / N_BANDS;
    let mut mags: Vec<f32> = Vec::with_capacity(frames * bands.len());
    for i in 0..frames {
        for b in bands.clone() {
            mags.push(10f32.powf(spec[i * N_BANDS + b]) - 1.0);
        }
    }
    if mags.is_empty() {
        return None;
    }
    mags.sort_by(f32::total_cmp);
    Some(mags[((mags.len() as f32 - 1.0) * 0.995).round() as usize])
}

#[test]
#[ignore = "A5 probe: needs ENST_ROOT, run with --ignored"]
fn enst_scale_per_role() -> Result<(), DetectError> {
    let root = PathBuf::from(std::env::var("ENST_ROOT").expect("ENST_ROOT"));
    let mics = ["kick", "snare", "hi-hat", "tom_1", "tom_2", "tom_3"];
    for drummer in ["drummer_1", "drummer_2", "drummer_3"] {
        for mic in mics {
            let dir = root.join(drummer).join("audio").join(mic);
            if !dir.is_dir() {
                continue;
            }
            let mut scales = Vec::new();
            let mut n = 0;
            for entry in dir.read_dir().into_iter().flatten().flatten() {
                let wav = entry.path();
                if wav.extension().is_none_or(|e| e != "wav") || n >= 8 {
                    continue;
                }
                let name = wav.file_name().unwrap().to_string_lossy();
                if !name.contains("phrase") {
                    continue;
                }
                let Ok((x, info)) = read_wav_mono(&wav, MixMode::Average) else {
                    continue;
                };
                let spec = prepare(&x, info.sample_rate, Chain::Mix)?;
                let bands = role_bands(if mic == "hi-hat" { "hat" } else { mic });
                if let Some(s) = mag_scale(&spec, &bands) {
                    scales.push(s);
                }
                n += 1;
            }
            if !scales.is_empty() {
                scales.sort_by(f32::total_cmp);
                println!(
                    "{drummer}/{mic}: n {} median q995-mag {:.4} (min {:.4} max {:.4})",
                    scales.len(),
                    scales[scales.len() / 2],
                    scales[0],
                    scales[scales.len() - 1]
                );
            }
        }
    }
    Ok(())
}
