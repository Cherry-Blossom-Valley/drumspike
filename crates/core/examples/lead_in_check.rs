//! Placement of kick and snare hits against the first sample over the noise floor (#239), before and after the
//! lead-in of the back-track. Aggregates only; no file names or paths are printed.
//! Run: cargo run --release -p drumspike-core --example `lead_in_check` -- <sessions dir> <label> [<label> ...]
//! A label is a session folder (`<dir>/<label>/Audio Files`). The kick file is the one starting with `k in`, the
//! snare file the one starting with `s top` (every match is analysed, the counts add up). A label can name its files:
//! `label=<kick file>,<snare file>`.
use std::path::{Path, PathBuf};

use drumspike_core::analyze::{AnalyzeParams, Detector, analyze_track};
use drumspike_core::refine::set_lead_in;
use drumspike_core::{MixMode, read_wav_mono};

/// The noise floor before a hit: the median of the 1 ms block maxima of `|x|` in the 60 ms to 12 ms before it.
fn floor_before(x: &[f32], at: usize, sr: f64) -> f64 {
    let b = (0.001 * sr) as usize;
    let end = at.saturating_sub((0.012 * sr) as usize);
    let from = at.saturating_sub((0.060 * sr) as usize);
    let mut m: Vec<f64> = x[from..end]
        .chunks_exact(b.max(1))
        .map(|c| c.iter().fold(0f64, |a, v| a.max(f64::from(v.abs()))))
        .collect();
    if m.is_empty() {
        return 0.0;
    }
    m.sort_by(f64::total_cmp);
    m[m.len() / 2]
}

/// The first sample in `[t - 10 ms, t + 5 ms]` over twice the floor (6 dB), in samples; `None` without a floor.
fn departure(x: &[f32], t: f64, sr: f64) -> Option<i64> {
    let at = (t * sr).round() as usize;
    if at < (0.060 * sr) as usize || at + (0.005 * sr) as usize >= x.len() {
        return None;
    }
    let floor = floor_before(x, at, sr);
    if floor <= 0.0 {
        return None;
    }
    let from = at - (0.010 * sr) as usize;
    let to = at + (0.005 * sr) as usize;
    (from..to)
        .find(|&i| f64::from(x[i].abs()) > 2.0 * floor)
        .map(|i| i as i64)
}

fn times(x: &[f32], sr: u32, role: &str) -> Vec<f64> {
    analyze_track(x, sr, role, &AnalyzeParams::default(), Detector::Dsp)
        .expect("analysis")
        .events
        .iter()
        .map(|h| h.t)
        .collect()
}

#[derive(Default)]
struct Tally {
    hits: usize,
    early: usize,
    late: usize,
    moved: usize,
    largest_ms: f64,
}

fn tally(x: &[f32], sr: u32, role: &str, t: &mut Tally) {
    let srf = f64::from(sr);
    set_lead_in(false);
    let before = times(x, sr, role);
    set_lead_in(true);
    let after = times(x, sr, role);
    if before.len() != after.len() {
        eprintln!(
            "the lead-in changed the hit count of a {role} track: {} -> {}",
            before.len(),
            after.len()
        );
    }
    for (b, a) in before.iter().zip(&after) {
        t.hits += 1;
        if (a - b).abs() > 0.5 / srf {
            t.moved += 1;
            t.largest_ms = t.largest_ms.max((a - b).abs() * 1000.0);
        }
        if let Some(d) = departure(x, *a, srf) {
            let off = *a * srf - d as f64;
            if off < -0.25e-3 * srf {
                t.early += 1;
            }
            if off >= 2.0 {
                t.late += 1;
            }
        }
    }
}

fn files(audio: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(audio)
        .map(|d| d.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    v.retain(|p| {
        let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
        n.starts_with(prefix) && n.ends_with(".wav")
    });
    v.sort();
    v
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: lead_in_check <sessions dir> <label>[=<kick file>,<snare file>] ...");
        std::process::exit(64);
    }
    let dir = PathBuf::from(&args[0]);
    println!(
        "hits: early = more than 0.25 ms before the first sample over 2x floor, late = 2 or more samples after it"
    );
    println!(
        "| session | role | hits | early before | early after | late before | late after | moved | largest move ms |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    for spec in &args[1..] {
        let (label, named) = match spec.split_once('=') {
            Some((l, f)) => (l, f.split_once(',')),
            None => (spec.as_str(), None),
        };
        let audio = dir.join(label).join("Audio Files");
        for (role, prefix, idx) in [("kick", "k in", 0), ("snare", "s top", 1)] {
            let list = match named {
                Some((k, s)) => vec![audio.join(if idx == 0 { k } else { s })],
                None => files(&audio, prefix),
            };
            let mut off = Tally::default();
            let mut moved = Tally::default();
            for p in list {
                let Ok((x, info)) = read_wav_mono(&p, MixMode::Average) else {
                    eprintln!("a {role} file of {label} cannot be read");
                    continue;
                };
                // `tally` runs both ways, counts the moves and measures the after placement; the before placement
                // is measured here with the lead-in off.
                tally(&x, info.sample_rate, role, &mut moved);
                set_lead_in(false);
                let srf = f64::from(info.sample_rate);
                for t in times(&x, info.sample_rate, role) {
                    off.hits += 1;
                    if let Some(d) = departure(&x, t, srf) {
                        let o = t * srf - d as f64;
                        off.early += usize::from(o < -0.25e-3 * srf);
                        off.late += usize::from(o >= 2.0);
                    }
                }
                set_lead_in(true);
            }
            println!(
                "| {label} | {role} | {} | {} | {} | {} | {} | {} | {:.3} |",
                moved.hits, off.early, moved.early, off.late, moved.late, moved.moved, moved.largest_ms
            );
        }
    }
}
