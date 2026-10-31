//! A5 probe on ENST: the track's own flux-contrast scale per role, to calibrate the
//! relative-delta rule (`S_REF` per role). Reads a few close-mic wavs per drummer from
//! the ENST copy (env `ENST_ROOT`), no detector run. Ignored by default.
//!
//! Run: `ENST_ROOT=~/.cache/drumspike/datasets/ENST-drums-public cargo test -p drumspike-core --test enst_contrast_probe -j 8 -- --nocapture --ignored`

use drumspike_core::detect::onset::{band_range, superflux};
use drumspike_core::detect::{Chain, DetectError, prepare};
use drumspike_core::io::read_wav_mono;
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

/// q90 of the contrast (flux minus the moving median over [i-10, i]) at the flux's local
/// maxima - the scale the relative delta is derived from.
fn contrast_scale(spec: &[f32], bands: &std::ops::Range<usize>) -> Option<f32> {
    let flux = superflux(spec, bands.clone());
    let n = flux.len();
    let mut cs: Vec<f32> = (1..n - 1)
        .filter(|&i| flux[i] > flux[i - 1] && flux[i] >= flux[i + 1])
        .map(|i| {
            let lo = i.saturating_sub(10);
            let hi = (i + 1).min(n);
            let mut v: Vec<f32> = flux[lo..hi].to_vec();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            flux[i] - v[v.len() / 2]
        })
        .collect();
    if cs.is_empty() {
        return None;
    }
    cs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Some(cs[((cs.len() as f32 - 1.0) * 0.9).round() as usize])
}

#[test]
#[ignore = "A5 probe: needs ENST_ROOT, run with --ignored"]
fn enst_contrast_per_role() -> Result<(), DetectError> {
    let root = PathBuf::from(std::env::var("ENST_ROOT").expect("ENST_ROOT"));
    let mics = ["kick", "snare", "hi-hat", "tom_1", "tom_2", "tom_3"];
    for drummer in ["drummer_1", "drummer_2", "drummer_3"] {
        let audio = root.join(drummer).join("audio");
        for mic in mics {
            let dir = audio.join(mic);
            if !dir.is_dir() {
                continue;
            }
            let mut scales = Vec::new();
            let mut n = 0;
            for entry in dir.read_dir().into_iter().flatten().flatten() {
                let wav = entry.path();
                if wav.extension().is_none_or(|e| e != "wav") {
                    continue;
                }
                if n >= 6 {
                    break;
                }
                let name = wav.file_name().unwrap().to_string_lossy();
                // the phrase items only (like the gate's corpus)
                if !name.contains("phrase") {
                    continue;
                }
                let Ok((x, info)) = read_wav_mono(&wav, drumspike_core::MixMode::Average) else {
                    continue;
                };
                let spec = prepare(&x, info.sample_rate, Chain::Mix)?;
                let bands = role_bands(if mic == "hi-hat" { "hat" } else { mic });
                if let Some(s) = contrast_scale(&spec, &bands) {
                    scales.push(s);
                }
                n += 1;
            }
            if !scales.is_empty() {
                scales.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let med = scales[scales.len() / 2];
                println!(
                    "{drummer}/{mic}: n {} median q90-contrast {:.4}  (min {:.4} max {:.4})",
                    scales.len(),
                    med,
                    scales[0],
                    scales[scales.len() - 1]
                );
            }
        }
    }
    Ok(())
}
