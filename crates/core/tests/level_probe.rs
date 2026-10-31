//! A5 probe: how the flux and its peak contrast scale with the recording level, on the
//! level_calib kit. Not a gate; prints the numbers the relative-delta rule is calibrated
//! from. Run: `cargo test -p drumspike-core --test level_probe -j 8 -- --nocapture`

use drumspike_core::detect::features::N_BANDS;
use drumspike_core::detect::onset::superflux;
use drumspike_core::detect::{Chain, DetectError, prepare};

#[path = "level_calib_render.rs"]
mod kit;

fn mag_scale(role: &str, peak_db: f64) -> Result<(), DetectError> {
    let x = kit::render(role, peak_db);
    let spec = prepare(&x, 48_000, Chain::Mix)?;
    let bands = kit::role_bands(role);
    let frames = spec.len() / N_BANDS;
    let mut mags: Vec<f32> = Vec::with_capacity(frames * bands.len());
    for i in 0..frames {
        for b in bands.clone() {
            mags.push(10f32.powf(spec[i * N_BANDS + b]) - 1.0);
        }
    }
    mags.sort_by(f32::total_cmp);
    let q = |p: f32| -> f32 { mags[((mags.len() as f32 - 1.0) * p).round() as usize] };
    println!("{role} {peak_db:>5} dB: mag q995 {:.4} q999 {:.4}", q(0.995), q(0.999));
    Ok(())
}

fn flux_stats(role: &str, peak_db: f64) -> Result<(), DetectError> {
    let x = kit::render(role, peak_db);
    let spec = prepare(&x, 48_000, Chain::Mix)?;
    let bands = kit::role_bands(role);
    let flux = superflux(&spec, bands.clone());
    // local maxima of the flux
    let mut maxima: Vec<f32> = (1..flux.len() - 1)
        .filter(|&i| flux[i] > flux[i - 1] && flux[i] >= flux[i + 1])
        .map(|i| flux[i])
        .collect();
    maxima.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |p: f32| -> f32 {
        let i = ((maxima.len() as f32 - 1.0) * p).round() as usize;
        maxima.get(i).copied().unwrap_or(0.0)
    };
    let n = flux.len();
    let mut contrasts: Vec<f32> = (0..n)
        .filter(|&i| flux[i] > 0.0)
        .map(|i| {
            let lo = i.saturating_sub(10);
            let hi = (i + 1).min(n);
            let mut v: Vec<f32> = flux[lo..hi].to_vec();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            flux[i] - v[v.len() / 2]
        })
        .collect();
    contrasts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let cq = |p: f32| -> f32 {
        let i = ((contrasts.len() as f32 - 1.0) * p).round() as usize;
        contrasts.get(i).copied().unwrap_or(0.0)
    };
    println!(
        "{role} {peak_db:>5} dB: frames {n}, maxima {} q50 {:.4} q75 {:.4} q90 {:.4} q99 {:.4}; contrast q50 {:+.4} q90 {:+.4} q99 {:+.4}",
        maxima.len(),
        q(0.5),
        q(0.75),
        q(0.90),
        q(0.99),
        cq(0.5),
        cq(0.90),
        cq(0.99)
    );
    Ok(())
}

#[test]
fn probe() -> Result<(), DetectError> {
    for role in ["kick", "snare"] {
        for db in [-3.0, -20.0, -40.0] {
            mag_scale(role, db)?;
            flux_stats(role, db)?;
        }
    }
    Ok(())
}
