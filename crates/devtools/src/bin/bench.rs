//! Benchmark of phase 1 (P1-8: decode, peaks, refinement, a 30-track session) and of the
//! phase 2 detection (P2-8).
//!
//! ```text
//! cargo run --release -p drumspike-devtools --bin bench --
//!     [--tracks 30] [--seconds 300] [--format pcm24|pcm16|float32] [--runs 5]
//!     [--detect-runs 3] [--dir <folder>] [--keep]
//! ```
//!
//! The session is synthetic: `tracks` mono files of `seconds` at 48 kHz with a kick-like
//! hit every 0.5 s over a low noise floor, written to a temporary folder (removed at the
//! end unless `--keep`). Writing the files is not timed; the files are then in the page
//! cache, as for a session the app has opened before. Every step runs `runs` times and the
//! table shows the minimum and the median in milliseconds:
//!
//! * `decode 1 track`: [`read_wav_mono`] of one file (mmap, conversion, mono fold);
//! * `decode 1 track (read_wav_bytes)`: the same file read into memory with
//!   [`read_wav_bytes`] and decoded (the path of the `PyO3` binding, retired in P3-12);
//! * `peaks 1 track`: [`compute_peaks`] of that signal (256 / 4096 / 65536);
//! * `refine 1 track`: [`refine_track`] of every hit (the detector's coarse times are the
//!   true onsets plus 4 ms) and [`velocity_map`] at the refined times;
//! * `session decode + peaks`: every file decoded and its pyramid built, in parallel over
//!   the tracks with rayon ([`decode_many`] then a parallel [`compute_peaks`]): the phase 1
//!   target is under 0.5 s for 30 tracks of 5 minutes;
//! * `session decode + peaks (read_wav_bytes)`: the same with every file read into memory
//!   with [`read_wav_bytes`], as the binding decodes (review round 3, finding m4).
//! * `session detect` (P2-8): the analysis of every track as the app's analysis job runs it
//!   in the Rust mode (`engine.analyze_track`, one track after the other): decode, the
//!   DSP detector ([`detect_onsets`], role kick), the 20 ms min-gap rule, the refinement
//!   ([`refine_track`], default window) with the second min-gap pass, and the velocities.
//!   This row runs `--detect-runs` times (0 skips it); a second table splits the median run by stage.

use std::path::{Path, PathBuf};
use std::time::Instant;

use drumspike_core::detect::onset::{Role, detect_onsets};
use drumspike_core::io::{WavFormat, WavSpec, write_wav};
use drumspike_core::{
    MixMode, RefineWindow, VelocityMap, compute_peaks, decode_many, read_wav_bytes, read_wav_mono, refine_track,
    velocity_map,
};
use rayon::prelude::*;

const SR: u32 = 48_000;
const HIT_EVERY_S: f64 = 0.5;

/// Read `path` into memory and decode it, as the `PyO3` binding does (no memory mapping).
fn decode_bytes(path: &Path) -> Result<Vec<f32>, String> {
    let (bytes, info) = read_wav_bytes(path).map_err(|e| e.to_string())?;
    info.decode_mono(&bytes, MixMode::Average).map_err(|e| e.to_string())
}

struct Args {
    tracks: usize,
    seconds: f64,
    format: WavFormat,
    runs: usize,
    detect_runs: usize,
    dir: Option<PathBuf>,
    keep: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        tracks: 30,
        seconds: 300.0,
        format: WavFormat::Pcm24,
        runs: 5,
        detect_runs: 3,
        dir: None,
        keep: false,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let val = || argv.get(i + 1).cloned().unwrap_or_default();
        let num = |name: &str| {
            argv.get(i + 1)
                .cloned()
                .unwrap_or_default()
                .parse::<usize>()
                .map_err(|e| format!("{name}: {e}"))
        };
        match argv[i].as_str() {
            "--tracks" => a.tracks = num("--tracks N")?,
            "--seconds" => a.seconds = val().parse::<f64>().map_err(|e| format!("--seconds S: {e}"))?,
            "--runs" => a.runs = num("--runs N")?.max(1),
            "--detect-runs" => a.detect_runs = num("--detect-runs N")?,
            "--dir" => a.dir = Some(PathBuf::from(val())),
            "--format" => {
                a.format = match val().as_str() {
                    "pcm16" => WavFormat::Pcm16,
                    "pcm24" => WavFormat::Pcm24,
                    "float32" => WavFormat::Float32,
                    other => return Err(format!("unknown format {other}")),
                }
            }
            "--keep" => {
                a.keep = true;
                i += 1;
                continue;
            }
            other => return Err(format!("unknown argument {other}")),
        }
        i += 2;
    }
    Ok(a)
}

/// A deterministic track: noise at about -60 dBFS (an LCG, seeded per track) and a decaying
/// 60 Hz hit every 0.5 s, its level changing per hit.
fn synth(n: usize, seed: u64) -> Vec<f32> {
    let mut s = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    let mut x: Vec<f32> = (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((s >> 33) as f32 / (1u64 << 31) as f32 - 0.5) * 0.002
        })
        .collect();
    let hit_len = (0.25 * f64::from(SR)) as usize;
    let mut k = 0usize;
    loop {
        let start = ((k as f64 + 0.1) * HIT_EVERY_S * f64::from(SR)) as usize + (seed as usize % 7);
        if start >= n {
            break;
        }
        let gain = 0.2 + 0.7 * ((k * 37 + seed as usize) % 11) as f32 / 10.0;
        for j in 0..hit_len.min(n - start) {
            let t = j as f32 / SR as f32;
            let env = (t / 0.0005).min(1.0) * (-t / 0.08).exp();
            x[start + j] += gain * env * (2.0 * std::f32::consts::PI * (60.0 + 60.0 * (-t / 0.02).exp()) * t).sin();
        }
        k += 1;
    }
    x
}

fn onsets(n: usize, seed: u64) -> Vec<f64> {
    let mut out = Vec::new();
    let mut k = 0usize;
    loop {
        let start = ((k as f64 + 0.1) * HIT_EVERY_S * f64::from(SR)) as usize + (seed as usize % 7);
        if start >= n {
            break;
        }
        out.push(start as f64 / f64::from(SR) + 0.004);
        k += 1;
    }
    out
}

/// The minimum gap of `engine.analyze_track` (`min_gap_ms` 20): keep a time when it is at
/// least this far after the previous kept time.
const MIN_GAP_S: f64 = 0.020;

/// The DSP detector for `session detect`.
struct Detector;

impl Detector {
    fn name(&self) -> &'static str {
        "rust-dsp, role kick"
    }
}

/// Seconds per stage of one `session detect` run, summed over the tracks.
#[derive(Default, Clone, Copy)]
struct Stages {
    decode: f64,
    dsp: f64,
    refine: f64,
    total: f64,
    hits: usize,
}

fn secs<T>(acc: &mut f64, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    *acc += t.elapsed().as_secs_f64();
    out
}

/// Keep a time when it is at least [`MIN_GAP_S`] after the previous kept time (`times`
/// sorted), as `engine.analyze_track` does before and after the refinement.
fn min_gap(times: &[f64]) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::with_capacity(times.len());
    for &t in times {
        if out.last().is_none_or(|&last| t - last >= MIN_GAP_S) {
            out.push(t);
        }
    }
    out
}

/// One analysis of `path` as a kick track: decode, DSP detector, role filter, min gap,
/// refinement, min gap, velocities. Adds the stage times to `st` and returns the hit count.
fn analyze(_det: &mut Detector, path: &Path, st: &mut Stages) -> Result<usize, String> {
    let t_all = Instant::now();
    let (x, info) = secs(&mut st.decode, || {
        read_wav_mono(path, MixMode::Ffmpeg)
            .map_err(|e| e.to_string())
            .map(|r| (r.0, r.1))
    })?;
    let sr = info.sample_rate;
    let coarse: Vec<f64> = secs(&mut st.dsp, || {
        detect_onsets(&x, sr, Role::Kick)
            .map_err(|e| e.to_string())
            .map(|hits| hits.iter().map(|h| h.time).collect())
    })?;
    let n = secs(&mut st.refine, || {
        let mut coarse = coarse;
        coarse.sort_by(f64::total_cmp);
        let dedup = min_gap(&coarse);
        let mut refined: Vec<f64> = refine_track(&x, sr, &dedup, RefineWindow::DEFAULT)
            .into_iter()
            .map(|h| h.0)
            .collect();
        refined.sort_by(f64::total_cmp);
        let fin = min_gap(&refined);
        velocity_map(&x, sr, &fin, &VelocityMap::default()).len()
    });
    st.total += t_all.elapsed().as_secs_f64();
    st.hits += n;
    Ok(n)
}

/// The median of `v` (the upper one of the two middle values for an even count, as [`time`]).
fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn time<T>(runs: usize, mut f: impl FnMut() -> T) -> (f64, f64) {
    let mut ms: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(f());
            t.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    ms.sort_by(f64::total_cmp);
    (ms[0], ms[ms.len() / 2])
}

fn run() -> Result<(), String> {
    let a = parse_args()?;
    let n = (a.seconds * f64::from(SR)) as usize;
    let dir = a
        .dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join(format!("drumspike-bench-{}", std::process::id())));
    std::fs::create_dir_all(&dir).map_err(|e| format!("bench folder {}: {e}", dir.display()))?;
    let spec = WavSpec {
        sample_rate: SR,
        channels: 1,
        format: a.format,
        extensible: false,
        channel_mask: 0,
    };
    let paths: Vec<PathBuf> = (0..a.tracks).map(|k| dir.join(format!("track{k:02}.wav"))).collect();
    let t = Instant::now();
    paths
        .par_iter()
        .enumerate()
        .map(|(k, p)| {
            write_wav(p, &spec, &synth(n, k as u64 + 1), None).map_err(|e| format!("write {}: {e}", p.display()))
        })
        .collect::<Result<(), _>>()?;
    // the files above were written by this run, so their metadata is there
    let bytes: u64 = paths
        .iter()
        .map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or_default())
        .sum();
    println!(
        "session: {} tracks x {:.0} s at {} Hz, {:?} mono, {:.0} MB, written in {:.1} s to {}",
        a.tracks,
        a.seconds,
        SR,
        a.format,
        bytes as f64 / 1e6,
        t.elapsed().as_secs_f64(),
        dir.display()
    );
    println!("threads: {}, runs per step: {}", rayon::current_num_threads(), a.runs);

    let first: &Path = &paths[0];
    let (x, _) = read_wav_mono(first, MixMode::Average).map_err(|e| e.to_string())?;
    let coarse = onsets(n, 1);
    let rows = [
        (
            "decode 1 track".to_string(),
            // the same file decoded above, so the error arm cannot fire
            time(a.runs, || {
                read_wav_mono(first, MixMode::Average).map_or(0, |r| r.0.len())
            }),
        ),
        (
            "decode 1 track (read_wav_bytes)".to_string(),
            time(a.runs, || decode_bytes(first).map_or(0, |x| x.len())),
        ),
        ("peaks 1 track".to_string(), time(a.runs, || compute_peaks(&x).len())),
        (
            format!("refine 1 track ({} hits) + velocities", coarse.len()),
            time(a.runs, || {
                let hits = refine_track(&x, SR, &coarse, RefineWindow::DEFAULT);
                let times: Vec<f64> = hits.iter().map(|h| h.0).collect();
                velocity_map(&x, SR, &times, &VelocityMap::default()).len()
            }),
        ),
        (
            format!("session decode + peaks ({} tracks)", a.tracks),
            time(a.runs, || {
                // the tracks decoded above, so an error cannot add to the sum
                decode_many(&paths, MixMode::Average)
                    .into_par_iter()
                    .map(|r| r.map_or(0, |(x, _)| compute_peaks(&x).len()))
                    .sum::<usize>()
            }),
        ),
        (
            format!("session decode + peaks ({} tracks, read_wav_bytes)", a.tracks),
            time(a.runs, || {
                paths
                    .par_iter()
                    .map(|p| decode_bytes(p).map_or(0, |x| compute_peaks(&x).len()))
                    .sum::<usize>()
            }),
        ),
        (
            format!("session decode only ({} tracks)", a.tracks),
            time(a.runs, || decode_many(&paths, MixMode::Average).len()),
        ),
    ];
    let mut rows = rows.to_vec();

    // P2-8: the detection of the whole session, one track after the other.
    let mut stage_runs: Vec<Stages> = Vec::new();
    let mut det = Detector;
    if a.detect_runs > 0 {
        println!("detector: {} (model-free main)", det.name());
        let mut one = Vec::new();
        for _ in 0..a.detect_runs {
            let mut s1 = Stages::default();
            analyze(&mut det, first, &mut s1)?;
            one.push(s1.total * 1e3);
            let mut st = Stages::default();
            for p in &paths {
                analyze(&mut det, p, &mut st)?;
            }
            stage_runs.push(st);
        }
        let mut one_sorted = one.clone();
        rows.push((
            format!("detect 1 track ({}): decode .. velocities", det.name()),
            (
                one_sorted.iter().copied().fold(f64::INFINITY, f64::min),
                median(&mut one_sorted),
            ),
        ));
        let mut tot: Vec<f64> = stage_runs.iter().map(|s| s.total * 1e3).collect();
        rows.push((
            format!(
                "session detect ({} tracks, {}, {} hits)",
                a.tracks,
                det.name(),
                stage_runs[0].hits
            ),
            (tot.iter().copied().fold(f64::INFINITY, f64::min), median(&mut tot)),
        ));
    }

    println!("| Step | min ms | median ms |");
    println!("|---|---:|---:|");
    for (name, (mn, med)) in &rows {
        println!("| {name} | {mn:.1} | {med:.1} |");
    }
    if !stage_runs.is_empty() {
        let med = |f: fn(&Stages) -> f64| {
            let mut v: Vec<f64> = stage_runs.iter().map(f).collect();
            median(&mut v) * 1e3
        };
        println!();
        println!(
            "session detect by stage ({} runs, median ms per stage over the runs, summed over {} tracks):",
            stage_runs.len(),
            a.tracks
        );
        println!("| Stage | median ms |");
        println!("|---|---:|");
        let mut stages: Vec<(&str, f64)> = vec![("decode", med(|s| s.decode))];
        stages.push((
            "DSP detector (resample, features, flux, picking, rise_db)",
            med(|s| s.dsp),
        ));
        stages.push(("role filter, min gap, refinement, velocities", med(|s| s.refine)));
        stages.push(("total", med(|s| s.total)));
        for (name, ms) in stages {
            println!("| {name} | {ms:.1} |");
        }
    }
    if !a.keep {
        for p in &paths {
            let _ = std::fs::remove_file(p);
        }
        let _ = std::fs::remove_dir(&dir);
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
