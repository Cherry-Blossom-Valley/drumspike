//! `role-from-audio`: run `drumspike_core::role_from_audio` over WAV files and print one
//! JSON line per file (`{"path", "sr", "duration", "features", "role"}`). The `tools/eval`
//! role check (#281 item 10, ticket #393) shells out to this binary so the eval exercises
//! the real classifier. Not product code.
//!
//! Usage: `role-from-audio <file.wav>...`

use drumspike_core::io::{read_wav_mono, MixMode};
use drumspike_core::role::{role_features, role_from_audio, role_from_audio_confidence};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: role-from-audio <file.wav>...");
        std::process::exit(2);
    }
    let per_hit = std::env::var("ROLE_HITS").is_ok();
    for a in &args {
        let path = PathBuf::from(a);
        let Ok((x, info)) = read_wav_mono(&path, MixMode::Average) else {
            let line = serde_json::json!({"path": a, "error": "read failed"}).to_string();
            println!("{line}");
            continue;
        };
        if per_hit {
            for h in drumspike_core::role::hit_features(&x, info.sample_rate) {
                let line = serde_json::json!({
                    "path": a,
                    "t_s": h.t_s, "env_db": h.env_db,
                    "b_low": h.bands[0], "b_mid": h.bands[1], "b_upper": h.bands[2],
                    "b_high": h.bands[3], "b_top": h.bands[4],
                    "low200": h.low200, "centroid": h.centroid,
                    "t20_hi": h.t20_hi, "ratio_hi": h.ratio_hi, "tonal_lo": h.tonal_lo,
                })
                .to_string();
                println!("{line}");
            }
            continue;
        }
        let (raw_role, confidence) = role_from_audio_confidence(&x, info.sample_rate);
        let role = role_from_audio(&x, info.sample_rate);
        let f = role_features(&x, info.sample_rate);
        let line = serde_json::json!({
            "path": a,
            "sr": info.sample_rate,
            "duration": x.len() as f64 / f64::from(info.sample_rate),
            "features": {
                "b_low": f.b_low, "b_mid": f.b_mid, "b_upper": f.b_upper,
                "b_high": f.b_high, "b_top": f.b_top, "centroid": f.centroid,
                "iqr_high": f.iqr_high, "tonal_lo": f.tonal_lo,
                "w_low": f.w_low, "w_mid": f.w_mid, "w_upper": f.w_upper,
                "w_high": f.w_high, "w_top": f.w_top,
                "decay_s": f.decay_s, "hit_low200": f.hit_low200,
                "t20_hi": f.t20_hi, "ratio_hi": f.ratio_hi, "hi_sustain": f.hi_sustain,
                "onsets_per_s": f.onsets_per_s, "crest": f.crest,
            },
            "role": role.map(|r| r.as_str()),
            "raw_role": raw_role.map(|r| r.as_str()),
            "confidence": confidence,
        })
        .to_string();
        println!("{line}");
    }
}
