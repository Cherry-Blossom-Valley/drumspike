//! A5 probe: the false hit of the robustness 44k/snare case under the gained-spec rule.
//! Run: `cargo test -p drumspike-core --test false_hit_probe -j 8 -- --nocapture`

use drumspike_core::analyze::{analyze_track_dsp_cands, AnalyzeParams};

#[path = "level_calib_render.rs"]
mod kit;

#[test]
fn probe() {
    let x = kit::render("snare", -3.0);
    let (_, cands, _, _) = analyze_track_dsp_cands(&x, 48_000, "snare", &AnalyzeParams::default()).expect("no fail");
    for c in &cands {
        println!(
            "t {:.4} kept {} gate {:?} flux {:.4} rel {:.1} attack {:.1}",
            c.time, c.kept, c.gate, c.flux, c.rel_db, c.attack
        );
    }
}
