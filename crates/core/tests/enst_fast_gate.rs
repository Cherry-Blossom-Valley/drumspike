//! A5 fast gate probe: run the DSP detector over the ENST phrase items directly (no
//! `service_tool`) and print the pooled P/R/F1 per drummer and role at +-50 ms - the same
//! numbers the standing gate produces, for fast iteration on the delta rule. Ignored by
//! default (needs `ENST_ROOT`); the final numbers come from `tools/eval/enst_gate.py`.
//!
//! Run: `ENST_ROOT=~/.cache/drumspike/datasets/ENST-drums-public cargo test -p drumspike-core --test enst_fast_gate -j 8 -- --nocapture --ignored`

use drumspike_core::analyze::{AnalyzeParams, Detector, analyze_track};
use drumspike_core::io::{MixMode, read_wav_mono};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const WINDOW: f64 = 0.05;

fn label_role(label: &str) -> Option<&'static str> {
    match label {
        "bd" => Some("kick"),
        "sd" | "sd-" | "rs" | "cs" => Some("snare"),
        "chh" | "ohh" => Some("hat"),
        "mt" | "lt" | "lmt" | "lft" | "mtr" | "ltr" => Some("tom"),
        "cb" | "sweep" | "sticks" => None,
        l if l.starts_with("rc") || l.starts_with("cr") || l.starts_with('c') || l.starts_with("spl") => None,
        _ => None,
    }
}

fn tom_alias(lab: &str) -> &str {
    match lab {
        "mtr" => "mt",
        "ltr" => "lt",
        other => other,
    }
}

fn read_annotation(path: &Path) -> Vec<(f64, String)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                return None;
            }
            let mut it = l.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.to_string()))
        })
        .collect()
}

fn prf(refs: &[f64], est: &[f64]) -> (f64, f64, f64) {
    // one-to-one, nearest pair first, within WINDOW (mir_eval semantics)
    let mut cand: Vec<(f64, usize, usize)> = refs
        .iter()
        .enumerate()
        .flat_map(move |(i, r)| {
            est.iter()
                .enumerate()
                .filter(move |(_, e)| (**e - r).abs() <= WINDOW)
                .map(move |(j, e)| ((e - r).abs(), i, j))
        })
        .collect();
    cand.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut ur = vec![false; refs.len()];
    let mut ue = vec![false; est.len()];
    let mut n = 0usize;
    for (_, i, j) in cand {
        if ur[i] || ue[j] {
            continue;
        }
        ur[i] = true;
        ue[j] = true;
        n += 1;
    }
    let p = n as f64 / est.len() as f64;
    let r = n as f64 / refs.len() as f64;
    (p, r, 2.0 * p * r / (p + r))
}

#[derive(Default)]
struct Cell {
    refs: Vec<f64>,
    est: Vec<f64>,
}

#[test]
#[ignore = "A5 probe: needs ENST_ROOT, run with --ignored"]
fn enst_fast_gate() {
    let root = PathBuf::from(std::env::var("ENST_ROOT").expect("ENST_ROOT"));
    let tom_map: BTreeMap<String, BTreeMap<String, String>> = serde_json::from_str(
        &std::fs::read_to_string("/home/linux-arm64/.cache/drumspike/a5_tom_map.json")
            .expect("a5_tom_map.json (tools/eval/dump_tom_map.py)"),
    )
    .expect("tom map json");
    let limit: usize = std::env::var("ENST_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mics = ["kick", "snare", "hi-hat", "tom_1", "tom_2", "tom_3"];
    let mut per: BTreeMap<(String, String), Cell> = BTreeMap::new();
    for drummer in ["drummer_1", "drummer_2", "drummer_3"] {
        let audio = root.join(drummer).join("audio");
        let ann_dir = root.join(drummer).join("annotation");
        let mut anns: Vec<_> = std::fs::read_dir(&ann_dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|e| e == "txt") && p.file_name().unwrap().to_string_lossy().contains("phrase")
            })
            .collect();
        anns.sort();
        if limit > 0 {
            anns.truncate(limit);
        }
        let map = &tom_map[drummer];
        for ann in anns {
            let stem = ann.file_name().unwrap().to_string_lossy();
            let stem = stem.trim_end_matches(".txt").to_string();
            let evs = read_annotation(&ann);
            // refs per mic
            let mut refs: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
            for (t, lab) in &evs {
                match label_role(lab) {
                    Some("kick") => refs.entry("kick").or_default().push(*t),
                    Some("snare") => refs.entry("snare").or_default().push(*t),
                    Some("hat") => refs.entry("hi-hat").or_default().push(*t),
                    Some("tom") => {
                        if let Some(mic) = map.get(tom_alias(lab)) {
                            refs.entry(mic.as_str()).or_default().push(*t);
                        }
                    }
                    _ => {}
                }
            }
            for mic in mics {
                let wav = audio.join(mic).join(format!("{stem}.wav"));
                if !wav.is_file() {
                    continue;
                }
                let Ok((x, info)) = read_wav_mono(&wav, MixMode::Average) else {
                    continue;
                };
                let role = if mic == "hi-hat" { "hat" } else { mic };
                let Ok(analysis) = analyze_track(&x, info.sample_rate, role, &AnalyzeParams::default(), Detector::Dsp)
                else {
                    continue;
                };
                let est: Vec<f64> = analysis.events.iter().map(|h| h.t).collect();
                let cell = per.entry((drummer.to_string(), role.to_string())).or_default();
                cell.refs.extend(refs.get(mic).cloned().unwrap_or_default());
                cell.est.extend(est);
            }
        }
    }
    // pooled F1 per drummer/role
    let mut lines = Vec::new();
    for ((d, role), cell) in &per {
        let (p, r, f) = prf(&cell.refs, &cell.est);
        lines.push((d.clone(), role.to_string(), p, r, f, cell.refs.len(), cell.est.len()));
    }
    lines.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    println!("drummer | role | ref | est | P | R | F1");
    for (d, role, p, r, f, nr, ne) in &lines {
        println!("{d} | {role} | {nr} | {ne} | {p:.3} | {r:.3} | {f:.3}");
    }
}
