//! Self-calibration probe (#77, package 2): the 90th percentile of the flux over the frames of
//! every ENST close mic of the three drummers, per role. The median over the items is the
//! role's nominal flux level (`onset::self_cal_ref`); a track's own q90 divided by it is its
//! [`self_cal_scale`](drumspike_core::detect::onset::self_cal_scale). Reads the ENST copy (env
//! `ENST_ROOT`). Ignored by default.
//!
//! Run: `ENST_ROOT=~/.cache/drumspike/datasets/ENST-drums-public cargo test -p
//! drumspike-core --test self_cal_probe -j 8 -- --nocapture --ignored --test-threads=4`

use drumspike_core::detect::onset::{band_range, superflux, OnsetParams, Role};
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

fn q90(v: &mut [f32]) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let pos = 0.9 * (v.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(v.len() - 1);
    let frac = (pos - lo as f64) as f32;
    Some(v[lo] + (v[hi] - v[lo]) * frac)
}

fn item_q90(wav: &Path, role: Role) -> Result<Option<f32>, DetectError> {
    let p = OnsetParams::for_role(role);
    let bands = band_range(p.band_lo_hz, p.band_hi_hz);
    let Ok((x, info)) = read_wav_mono(wav, MixMode::Average) else {
        return Ok(None);
    };
    let spec = prepare(&x, info.sample_rate, Chain::Cli)?;
    let mut flux = superflux(&spec, bands);
    Ok(q90(&mut flux))
}

#[test]
#[ignore = "#77 probe: needs ENST_ROOT, run with --ignored"]
fn enst_flux_q90_per_item() -> Result<(), DetectError> {
    let root = PathBuf::from(std::env::var("ENST_ROOT").expect("ENST_ROOT"));
    for drummer in ["drummer_1", "drummer_2", "drummer_3"] {
        for mic in ["kick", "snare", "hi-hat"] {
            let role = role_of(mic).expect("probe mics");
            let dir = root.join(drummer).join("audio").join(mic);
            let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "wav"))
                .collect();
            paths.sort();
            for wav in paths {
                if let Some(q) = item_q90(&wav, role)? {
                    let stem = wav.file_stem().unwrap().to_string_lossy().into_owned();
                    println!("SELFCAL {} {drummer}/{mic}/{stem} q90 {q:.4}", role.name());
                }
            }
        }
    }
    Ok(())
}
