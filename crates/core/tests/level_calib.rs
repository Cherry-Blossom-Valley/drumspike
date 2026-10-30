//! #281 A5: the level invariance test. The same kit rendered
//! at -20 dBFS and -40 dBFS must give the same hit count within 2 % - the detector's
//! thresholds must come from the track's own level and flux distribution, not from
//! constants tuned at one recording level.
//!
//! Today's numbers (before the self-calibration, measured 2026-10-26 on linux-arm64 with the
//! shipped absolute `delta` constants) are in
//! `docs/migration/reports/a5-self-calibration.md`. Until the relative rule lands the
//! 2 % assertion is an ignored finding test, exactly like the C14 finding in
//! `robustness.rs`.
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
/// 2 %. Ignored until the self-calibration lands; run it with --ignored for today's
/// numbers.
#[test]
#[ignore = "A5 finding: the shipped absolute delta misses hits at -40 dBFS; run with --ignored for today's numbers"]
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
