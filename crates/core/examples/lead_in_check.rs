//! Placement of kick and snare hits against the first sample over the noise floor (#239), before and after the
//! lead-in of the back-track. Aggregates only; no file names or paths are printed.
//! Run: cargo run --release -p drumspike-core --example `lead_in_check` -- <sessions dir> <label> [<label> ...]
//! A label is a session folder (`<dir>/<label>/Audio Files`). The kick file is the one starting with `k in`, the
//! snare file the one starting with `s top` (every match is analysed, the counts add up). A label can name its files:
//! `label=<kick file>,<snare file>`.
use std::path::{Path, PathBuf};

use drumspike_core::analyze::{AnalyzeParams, Detector, analyze_track, detect_hits};
use drumspike_core::refine::{RefineRule, RefineWindow, first_motion_trace, refine_reach, refine_sr, set_lead_in};
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
    // The product's rule: `AnalyzeParams::default()` is the engine reference rule, which has no back-track at all.
    let params = AnalyzeParams {
        rule: RefineRule::CURRENT,
        ..AnalyzeParams::default()
    };
    analyze_track(x, sr, role, &params, Detector::Dsp)
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
    /// Moves under 0.25 ms, 0.25 to 1 ms, 1 to 3 ms, over 3 ms.
    dist: [usize; 4],
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
            let ms = (a - b).abs() * 1000.0;
            t.largest_ms = t.largest_ms.max(ms);
            t.dist[match ms {
                m if m < 0.25 => 0,
                m if m < 1.0 => 1,
                m if m < 3.0 => 2,
                _ => 3,
            }] += 1;
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

/// Per-hit dump: for each time in `times_s` (seconds, as shown in the editor), the nearest hit of `x` without and with
/// the lead-in, its distance to the first sample over 2x floor, and the walk's floor, threshold, foot and lead-in
/// (samples before the hit's own refined sample). The walk is traced from the refined sample without the lead-in plus
/// one, so threshold and floor can differ by a sample's worth from the analysis.
fn dump(x: &[f32], sr: u32, role: &str, times_s: &[f64]) {
    let srf = f64::from(sr);
    set_lead_in(false);
    let before = times(x, sr, role);
    set_lead_in(true);
    let after = times(x, sr, role);
    println!(
        "| asked s | hit before s | hit after s | move ms | off 2x floor before/after smp | floor | thr | foot | lead-in |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    for &q in times_s {
        let Some(k) = (0..after.len()).min_by(|&i, &j| (after[i] - q).abs().total_cmp(&(after[j] - q).abs())) else {
            println!("| {q:.4} | no hit |");
            continue;
        };
        let (b, a) = (before[k.min(before.len() - 1)], after[k]);
        let dep = departure(x, a, srf);
        let off = |t: f64| dep.map_or("-".to_string(), |d| format!("{:.1}", t * srf - d as f64));
        let s = (b * srf).round() as i64 + 1;
        let tr = first_motion_trace(x, s, sr, None, None);
        let tr_txt = tr.map_or("| - | - | - | - |".to_string(), |t| {
            format!("| {:.5} | {:.5} | {} | {} |", t.floor, t.thr, t.foot - s, t.lead - s)
        });
        println!(
            "| {q:.4} | {b:.5} | {a:.5} | {:.3} | {}/{} {tr_txt}",
            (a - b) * 1000.0,
            off(b),
            off(a)
        );
    }
    println!();
    println!("Steps before the walk, per asked time (samples relative to the hit after the lead-in, `a`):");
    println!(
        "| asked s | coarse | reference | reach (step) | s into walk | 2x floor first | floor dep | floor walk | rule window max |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    let coarse = coarse_times(x, sr, role);
    for &q in times_s {
        let Some(k) = (0..after.len()).min_by(|&i, &j| (after[i] - q).abs().total_cmp(&(after[j] - q).abs())) else {
            continue;
        };
        let a = after[k];
        let asf = (a * srf).round() as i64;
        let rel = |t: f64| format!("{:+}", (t * srf).round() as i64 - asf);
        let Some(ci) = (0..coarse.len()).min_by(|&i, &j| (coarse[i] - a).abs().total_cmp(&(coarse[j] - a).abs()))
        else {
            continue;
        };
        let t0 = coarse[ci];
        let nb: Vec<f64> = [ci.checked_sub(1), Some(ci + 1)]
            .into_iter()
            .flatten()
            .filter_map(|i| coarse.get(i).copied())
            .collect();
        let win = RefineWindow::DEFAULT;
        let (rt, _) = refine_sr(x, t0, sr, &nb, win);
        let rr = refine_reach(x, t0, sr, &nb, win);
        let dep = departure(x, a, srf).map_or("-".to_string(), |d| format!("{:+}", d - asf));
        let fl_dep = floor_before(x, (a * srf).round() as usize, srf);
        let s = (rr.t * srf).round() as i64;
        let fl_walk = first_motion_trace(x, s, sr, None, None).map_or(0.0, |t| t.floor);
        let i0 = ((t0 - 0.0015) * srf).round() as usize;
        let i1 = (((t0 + 0.010) * srf).round() as usize).min(x.len());
        let mx = x[i0.min(i1)..i1].iter().fold(0f32, |m, v| m.max(v.abs()));
        println!(
            "| {q:.4} | {} | {} | {} ({:?}) | {} | {dep} | {fl_dep:.5} | {fl_walk:.5} | {mx:.5} |",
            rel(t0),
            rel(rt),
            rel(rr.t),
            rr.step,
            s - asf
        );
    }
}

/// The coarse hits of a track after the min-gap dedup: what the refinement starts from.
fn coarse_times(x: &[f32], sr: u32, role: &str) -> Vec<f64> {
    let params = AnalyzeParams::default();
    let (mut det, _) = detect_hits(x, sr, role, &params, Detector::Dsp).expect("detection");
    det.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<f64> = Vec::new();
    for (t, _) in det {
        if out.last().is_none_or(|l| t - l >= params.min_gap_ms / 1000.0) {
            out.push(t);
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() >= 5 && args[0] == "--dump" {
        // --dump <sessions dir> <label> <kick|snare> <t,t,...>
        let (role, prefix) = if args[3] == "kick" {
            ("kick", "k in")
        } else {
            ("snare", "s top")
        };
        let ts: Vec<f64> = args[4].split(',').filter_map(|v| v.trim().parse().ok()).collect();
        for p in files(&PathBuf::from(&args[1]).join(&args[2]).join("Audio Files"), prefix) {
            if let Ok((x, info)) = read_wav_mono(&p, MixMode::Average) {
                dump(&x, info.sample_rate, role, &ts);
            }
        }
        return;
    }
    if args.len() < 2 {
        eprintln!("usage: lead_in_check <sessions dir> <label>[=<kick file>,<snare file>] ...");
        eprintln!("       lead_in_check --dump <sessions dir> <label> <kick|snare> <t,t,...>");
        std::process::exit(64);
    }
    let dir = PathBuf::from(&args[0]);
    println!(
        "hits: early = more than 0.25 ms before the first sample over 2x floor, late = 2 or more samples after it"
    );
    println!(
        "| session | role | hits | early before | early after | late before | late after | moved | largest move ms | moves <0.25/0.25-1/1-3/>3 ms |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|");
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
                "| {label} | {role} | {} | {} | {} | {} | {} | {} | {:.3} | {}/{}/{}/{} |",
                moved.hits,
                off.early,
                moved.early,
                off.late,
                moved.late,
                moved.moved,
                moved.largest_ms,
                moved.dist[0],
                moved.dist[1],
                moved.dist[2],
                moved.dist[3]
            );
        }
    }
}
