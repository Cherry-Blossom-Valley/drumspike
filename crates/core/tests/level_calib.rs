//! #281 A5: the level invariance test. The same kit rendered
//! at -20 dBFS and -40 dBFS must give the same hit count within 2 % - the detector's
//! thresholds must come from the track's own level and flux distribution, not from
//! constants tuned at one recording level.
//!
//! The numbers before the self-calibration (measured 2026-10-26 with the absolute `delta`
//! constants) are in the A5 self-calibration report.
//! The test was an ignored finding test until the relative rule landed; it passed with
//! --ignored on 2026-11-03 (kick 20/20, snare 8/8 hits) and runs by default since then.
//!
//! Run: `cargo test -p drumspike-core --test level_calib -j 8 -- --nocapture`

use drumspike_core::analyze::{AnalyzeParams, Detector, analyze_track};

#[path = "level_calib_render.rs"]
mod kit;

fn hits(role: &str, peak_db: f64) -> usize {
    let x = kit::render(role, peak_db);
    let analysis =
        analyze_track(&x, 48_000, role, &AnalyzeParams::default(), Detector::Dsp).expect("analyze_track must not fail");
    analysis.events.len()
}

/// The ticket's level test: the same kit at -20 dB and -40 dB, the same hit count within
/// 2 %.
#[test]
fn level_minus20_vs_minus40_same_hit_count() {
    for role in ["kick", "snare"] {
        let n20 = hits(role, -20.0);
        let n40 = hits(role, -40.0);
        println!("| {role} | -20 dBFS | {n20} | -40 dBFS | {n40} |");
        let lo = n20.min(n40) as f64;
        let hi = n20.max(n40) as f64;
        assert!(
            lo >= 0.98 * hi,
            "{role}: hit count {n20} at -20 dB vs {n40} at -40 dB, outside 2 %"
        );
    }
}
