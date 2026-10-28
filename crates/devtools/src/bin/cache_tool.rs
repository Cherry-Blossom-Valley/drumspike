//! Tools for the peak cache (P3-2b).
//!
//! ```text
//! cache_tool check <corpus_dir>             # round trip and tiles for every corpus file, both mix modes
//! cache_tool bench <session_dir> [--runs 3] # build peaks.bin for every WAV in the folder, in parallel
//! ```
//!
//! `check` reads `<corpus_dir>/manifest.json` (from `make_corpus.py`), decodes every file in both
//! mix modes, builds the cache and tests: the levels read back bit for bit equal to
//! [`compute_peaks`], and tiles from the file equal [`Track::peaks`]. `bench` writes each cache
//! into `<session_dir>/.cache_bench/<stem>/` and prints the time for decode, peaks and
//! write over all files.

use std::path::{Path, PathBuf};
use std::time::Instant;

use drumspike_core::project::CacheKey;
use drumspike_core::{build_cache, compute_peaks, read_wav_mono, MixMode, PeakCache, Track};
use rayon::prelude::*;

fn wavs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "wav"))
        .collect();
    v.sort();
    v
}

fn check(dir: &Path) -> bool {
    let key = CacheKey {
        size: Some(1),
        mtime: Some(2),
        head_sha1: Some("x".into()),
    };
    let tmp = std::env::temp_dir().join(format!("cache_tool_check_{}", std::process::id()));
    let (mut cases, mut bad) = (0, 0);
    for p in wavs(dir) {
        for mode in [MixMode::Ffmpeg, MixMode::Average] {
            let Ok((x, info)) = read_wav_mono(&p, mode) else {
                continue;
            };
            cases += 1;
            let want = compute_peaks(&x);
            let d = tmp.join(p.file_stem().unwrap());
            build_cache(&d, info.sample_rate, &x, &key).expect("build");
            let c = PeakCache::open(&d.join("peaks.bin")).expect("open");
            let mut problems = Vec::new();
            if c.sr() != info.sample_rate || c.n_samples() != x.len() as u64 {
                problems.push("header".to_string());
            }
            for (bs, lv) in &want {
                let got = c.read_level(*bs).expect("read").expect("level");
                let eq = |a: &[f64], b: &[f64]| {
                    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| p.to_bits() == q.to_bits())
                };
                if got.n != lv.n || !eq(&got.min, &lv.min) || !eq(&got.max, &lv.max) || !eq(&got.rms, &lv.rms) {
                    problems.push(format!("level {bs}"));
                }
            }
            let t = Track::new(info.sample_rate, x);
            for (bs, lv) in &want {
                for (f, to) in [
                    (0i64, 7i64),
                    (-3, 5),
                    (1, lv.n as i64),
                    (lv.n as i64 - 2, lv.n as i64 + 9),
                    (lv.n as i64 + 5, lv.n as i64 + 9),
                    (5, 2),
                ] {
                    let w = t.peaks(*bs, f, to).expect("level");
                    let g = c.tile(*bs, f, to).expect("tile").expect("level");
                    let up = |v: &[f32]| v.iter().map(|&q| f64::from(q)).collect::<Vec<f64>>();
                    if g.start != w.start || up(&g.min) != w.min || up(&g.max) != w.max || up(&g.rms) != w.rms {
                        problems.push(format!("tile {bs} {f}..{to}"));
                    }
                }
            }
            let name = format!("{} {:?}", p.file_name().unwrap().to_string_lossy(), mode);
            if problems.is_empty() {
                println!("ok   {name}");
            } else {
                bad += 1;
                println!("FAIL {name}: {}", problems.join(", "));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    println!("{} / {} cases pass", cases - bad, cases);
    cases > 0 && bad == 0
}

fn bench(dir: &Path, runs: usize) {
    let files = wavs(dir);
    let out = dir.join(".cache_bench");
    let key = CacheKey {
        size: None,
        mtime: None,
        head_sha1: None,
    };
    let mut times = Vec::new();
    for _ in 0..runs {
        let _ = std::fs::remove_dir_all(&out);
        let t = Instant::now();
        let n: usize = files
            .par_iter()
            .map(|p| {
                let (x, info) = read_wav_mono(p, MixMode::Average).expect("decode");
                build_cache(&out.join(p.file_stem().unwrap()), info.sample_rate, &x, &key).expect("build");
                x.len()
            })
            .sum();
        times.push((t.elapsed().as_secs_f64(), n));
    }
    // Same work with the samples already in memory: the cost of the cache alone.
    let sig: Vec<(u32, Vec<f32>)> = files
        .iter()
        .map(|p| {
            let (x, i) = read_wav_mono(p, MixMode::Average).expect("decode");
            (i.sample_rate, x)
        })
        .collect();
    let mut cache_only = Vec::new();
    for _ in 0..runs {
        let _ = std::fs::remove_dir_all(&out);
        let t = Instant::now();
        sig.par_iter().enumerate().for_each(|(i, (sr, x))| {
            build_cache(&out.join(format!("t{i}")), *sr, x, &key).expect("build");
        });
        cache_only.push(t.elapsed().as_secs_f64());
    }
    let _ = std::fs::remove_dir_all(&out);
    let fmt = |v: &[f64]| v.iter().map(|s| format!("{s:.3}")).collect::<Vec<_>>().join(" ");
    println!("{} files, {} samples", files.len(), times.first().map_or(0, |t| t.1));
    println!(
        "decode + peaks + write, s per run: {}",
        fmt(&times.iter().map(|t| t.0).collect::<Vec<_>>())
    );
    println!(
        "peaks + write only (samples in memory), s per run: {}",
        fmt(&cache_only)
    );
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match (a.get(1).map(String::as_str), a.get(2)) {
        (Some("check"), Some(d)) => {
            if !check(Path::new(d)) {
                std::process::exit(1);
            }
        }
        (Some("bench"), Some(d)) => {
            let runs = a
                .iter()
                .position(|s| s == "--runs")
                .and_then(|i| a.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(3);
            bench(Path::new(d), runs);
        }
        _ => {
            eprintln!("usage: cache_tool check <corpus_dir> | bench <session_dir> [--runs N]");
            std::process::exit(2);
        }
    }
}
