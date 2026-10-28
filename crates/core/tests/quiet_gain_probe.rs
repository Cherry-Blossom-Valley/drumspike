//! A5 round 2 probe: the implied gain factor of every ENST close mic against the role's
//! `s_ref` (the q995 of the linear band magnitudes over the role's bands), to calibrate
//! the quiet bar of the gained-spectrogram rule (round 2: a near-nominal track - a small
//! gain - stays shipped; see `onset::quiet_gain`). Reads every close-mic wav per drummer
//! from the ENST copy (env `ENST_ROOT`). Ignored by default.
//!
//! Run: `ENST_ROOT=~/.cache/drumspike/datasets/ENST-drums-public cargo test -p
//! drumspike-core --test quiet_gain_probe -j 8 -- --nocapture --ignored --test-threads=4`

use drumspike_core::detect::features::N_BANDS;
use drumspike_core::detect::onset::{band_range, OnsetParams, Role};
use drumspike_core::detect::{prepare, Chain, DetectError};
use drumspike_core::io::{read_wav_mono, MixMode};
use std::path::{Path, PathBuf};

fn role_of(mic: &str) -> Option<Role> {
    match mic {
        "kick" => Some(Role::Kick),
        "snare" => Some(Role::Snare),
        "hi-hat" => Some(Role::Hat),
        _ => None,
    }
}

fn scale_of(spec: &[f32], bands: &std::ops::Range<usize>) -> Option<f32> {
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
    mags.sort_by(|a, b| a.total_cmp(b));
    let pos = 0.995 * (mags.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(mags.len() - 1);
    let frac = (pos - lo as f64) as f32;
    Some(mags[lo] + (mags[hi] - mags[lo]) * frac)
}

#[test]
#[ignore = "A5 probe: needs ENST_ROOT, run with --ignored"]
fn enst_gain_factor_per_item() -> Result<(), DetectError> {
    let root = PathBuf::from(std::env::var("ENST_ROOT").expect("ENST_ROOT"));
    for drummer in ["drummer_1", "drummer_2", "drummer_3"] {
        for mic in ["kick", "snare", "hi-hat"] {
            let role = role_of(mic).expect("probe mics");
            let p = OnsetParams::for_role(role);
            let bands = band_range(p.band_lo_hz, p.band_hi_hz);
            let dir = root.join(drummer).join("audio").join(mic);
            let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "wav").unwrap_or(false))
                .collect();
            paths.sort();
            for wav in paths {
                if let Some(scale) = item_scale(&wav, &bands)? {
                    let g = s_ref_print(role) / scale;
                    println!("{drummer}/{}/{} scale {scale:.3} gain {g:.2}", mic, file_stem(&wav));
                }
            }
        }
    }
    Ok(())
}

fn file_stem(p: &Path) -> String {
    p.file_stem().unwrap().to_string_lossy().into_owned()
}

fn s_ref_print(role: Role) -> f32 {
    match role {
        Role::Kick | Role::Snare => 23.0,
        Role::Hat => 1.1,
        _ => 0.0,
    }
}

fn item_scale(wav: &Path, bands: &std::ops::Range<usize>) -> Result<Option<f32>, DetectError> {
    let Ok((x, info)) = read_wav_mono(wav, MixMode::Average) else {
        return Ok(None);
    };
    let spec = prepare(&x, info.sample_rate, Chain::Cli)?;
    Ok(scale_of(&spec, bands))
}
