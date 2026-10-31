//! P3-15d (Q5, lanes fast path): `samples_window` reads only the requested window, and the
//! lane job builds the peak caches on the workers of `tracks_add`.
//!
//! `samples_window_bytes_equal_the_full_decode_on_the_corpus`: the 18-file corpus of
//! `tools/parity/make_corpus.py` (written through `uv run --project <repo>`), every file
//! in both mix modes, windows at the start, the middle, the end, across the end, past the end,
//! before the start and at the cap. The expected bytes come from the full decode
//! (`read_wav_mono`). Without `uv` the test skips with a note, unless `DRUMSPIKE_REQUIRE_CORE=1`.
//!
//! `lanes_fast_path_bench_of_the_generated_session` (ignored): the 30-track session of
//! `make_session.py` in `DRUMSPIKE_TRACKS_BENCH`; first open without `peaks.bin` (warm), the
//! window latency at sample zoom on 30 lanes, and the resident set of this process.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::*;
use drumspike_app::Service;
use drumspike_core::io::MixMode;
use drumspike_core::project::{add_track, build_source, new_project, save_project};
use drumspike_core::read_wav_mono;
use serde_json::Value;

/// The bytes `samples_window` must answer, made from the full decode `full` at rate `sr`
/// (the clamp rules of the command: start at 0 or later, at most 262,144 samples, the end of
/// the track).
fn expected(full: &[f32], sr: u32, start: i64, end: i64) -> Vec<u8> {
    let len = full.len() as i64;
    let start = start.max(0);
    let count = (end - start).clamp(0, 262_144);
    let (s, e) = (start.min(len) as usize, (start + count).min(len) as usize);
    let mut out = Vec::with_capacity(8 + 4 * (e - s));
    out.extend_from_slice(&(s as u32).to_le_bytes());
    out.extend_from_slice(&sr.to_le_bytes());
    for x in &full[s..e] {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// The first byte where `a` and `b` differ, or the shorter length when one is a prefix.
fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    if a == b {
        return None;
    }
    Some(
        a.iter()
            .zip(b)
            .position(|(x, y)| x != y)
            .unwrap_or(a.len().min(b.len())),
    )
}

/// The windows of a track of `n` samples: start, middle, end, across the end, past the end,
/// before the start, the cap, empty and reversed.
fn windows(n: i64) -> Vec<(&'static str, i64, i64)> {
    let w = 1000.min(n.max(1));
    vec![
        ("start", 0, w),
        ("start+1", 1, 1 + w),
        ("middle", n / 2 - w / 2, n / 2 + w / 2 + 1),
        ("end", n - w, n),
        ("across the end", n - w / 3, n - w / 3 + w),
        ("last sample", n - 1, n + 5),
        ("past the end", n + 10, n + 20),
        ("before the start", -50, w / 2),
        ("cap", 0, 10_000_000),
        ("cap from the middle", n / 3, n / 3 + 300_000),
        ("empty", n / 2, n / 2),
        ("reversed", n / 2, n / 4),
    ]
}

/// Write the corpus into `dir` through `uv`; false (and a note) when `uv` is missing.
fn make_corpus(dir: &Path) -> bool {
    let root = repo_root().canonicalize().unwrap();
    let out = Command::new("uv")
        .args(["run", "--project", root.to_str().unwrap(), "python"])
        .arg("tools/parity/make_corpus.py")
        .arg(dir)
        .current_dir(&root)
        .output();
    match out {
        Ok(o) => {
            assert!(
                o.status.success(),
                "make_corpus.py failed:\n{}",
                String::from_utf8_lossy(&o.stderr)
            );
            true
        }
        Err(e) => {
            assert!(
                std::env::var_os("DRUMSPIKE_REQUIRE_CORE").is_none_or(|v| v != "1"),
                "DRUMSPIKE_REQUIRE_CORE=1 and uv is not available ({e})"
            );
            eprintln!("SKIP samples_window corpus test: uv is not available ({e})");
            false
        }
    }
}

#[test]
fn samples_window_bytes_equal_the_full_decode_on_the_corpus() {
    let root = tmp("window_corpus");
    let corpus = root.join("corpus");
    if !make_corpus(&corpus) {
        return;
    }
    let manifest: Value = serde_json::from_slice(&std::fs::read(corpus.join("manifest.json")).unwrap()).unwrap();
    let files: Vec<String> = match &manifest {
        Value::Array(a) => a.iter().map(|f| f["name"].as_str().unwrap().to_string()).collect(),
        Value::Object(m) => m["files"]
            .as_array()
            .unwrap_or_else(|| panic!("manifest keys {:?}", m.keys().collect::<Vec<_>>()))
            .iter()
            .map(|f| f["name"].as_str().unwrap().to_string())
            .collect(),
        other => panic!("manifest is not a list or an object: {other}"),
    };
    assert_eq!(files.len(), 18, "the corpus has 18 files: {files:?}");
    let modes = [("ffmpeg", MixMode::Ffmpeg), ("average", MixMode::Average)];
    let mut p = new_project(&root, "Corpus").unwrap();
    let mut tracks: Vec<(String, PathBuf, MixMode)> = Vec::new();
    for f in &files {
        let wav = corpus.join(f);
        for (word, mode) in modes {
            let name = format!(
                "{}_{word}",
                Path::new(f).file_stem().unwrap().to_str().unwrap().replace('.', "_")
            );
            let rec = build_source(&wav, &p.dir, "referenced").unwrap();
            add_track(&mut p, &name, Some(rec), "kick", 36, Some(word)).unwrap();
            tracks.push((name, wav.clone(), mode));
        }
    }
    save_project(&mut p).unwrap();
    let svc = service_dsp();
    let sid = svc.project_open(p.dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let (mut cases, mut errors, mut planted) = (0, 0, false);
    for (name, wav, mode) in &tracks {
        match read_wav_mono(wav, *mode) {
            Ok((full, info)) => {
                let n = full.len() as i64;
                for (what, a, b) in windows(n) {
                    let want = expected(&full, info.sample_rate, a, b);
                    let got = svc.samples_window(&sid, name, a, b).unwrap();
                    if let Some(i) = first_difference(&got, &want) {
                        panic!(
                            "{name} window {what} [{a}, {b}) of {n}: byte {i} differs (got {} bytes, want {})",
                            got.len(),
                            want.len()
                        );
                    }
                    // the comparison can fire: one sample changed in the expected bytes is seen
                    if !planted && want.len() > 8 {
                        let mut bad = want.clone();
                        let k = 8 + 4 * ((bad.len() - 8) / 4 / 2);
                        let v = f32::from_le_bytes(bad[k..k + 4].try_into().unwrap());
                        bad[k..k + 4].copy_from_slice(&(v + 1.0).to_le_bytes());
                        let at = first_difference(&got, &bad);
                        assert!(
                            at.is_some_and(|i| (k..k + 4).contains(&i)),
                            "a planted wrong sample at byte {k} must be found, got {at:?}"
                        );
                        planted = true;
                    }
                    cases += 1;
                }
            }
            Err(e) => {
                // a file the full decode refuses in this mode: the window refuses it too
                let got = svc.samples_window(&sid, name, 0, 10).unwrap_err();
                let want = drumspike_app::AppError::from(e);
                assert_eq!(got.code, want.code, "{name}: {got} against {want}");
                assert!(got.message.contains(&want.message), "{name}: {got} against {want}");
                errors += 1;
            }
        }
    }
    assert!(planted);
    eprintln!(
        "samples_window corpus: {} tracks ({} files x 2 modes), {cases} windows equal, {errors} tracks refused by both",
        tracks.len(),
        files.len()
    );
    assert!(cases >= 30 * windows(1).len(), "{cases} cases");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn samples_window_follows_a_source_that_changed() {
    let root = tmp("window_changed");
    let mut long = kick_spec("Kick_In", "kick", 36);
    long.x = kick_signal(&ONSETS, 3.5, 48_000);
    let dir = project(&root, "Song", &[long]);
    let svc = service_dsp();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let wav = root.join("daw/Kick_In.wav");
    let (full, _) = read_wav_mono(&wav, MixMode::Average).unwrap();
    assert_eq!(
        svc.samples_window(&sid, "Kick_In", 100, 200).unwrap(),
        expected(&full, 48_000, 100, 200)
    );
    // the same file with other samples and another length (the record no longer matches; the
    // window still reads the file that is there, as the full decode did)
    std::thread::sleep(Duration::from_millis(1100));
    let other: Vec<f32> = full.iter().take(50_000).map(|v| -v * 0.5).collect();
    write_wav(&wav, 48_000, &other);
    let (full2, _) = read_wav_mono(&wav, MixMode::Average).unwrap();
    if let Ok(b) = svc.samples_window(&sid, "Kick_In", 49_900, 50_100) {
        assert_eq!(b, expected(&full2, 48_000, 49_900, 50_100));
    }
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn samples_window_answers_parallel_requests_with_the_same_bytes() {
    let root = tmp("window_parallel");
    let dir = project(
        &root,
        "Song",
        &[kick_spec("Kick_In", "kick", 36), kick_spec("Snare_Top", "snare", 38)],
    );
    let svc = service_dsp();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let (full, _) = read_wav_mono(&root.join("daw/Kick_In.wav"), MixMode::Average).unwrap();
    std::thread::scope(|s| {
        for k in 0..16i64 {
            let (svc, sid, full) = (&svc, &sid, &full);
            s.spawn(move || {
                let a = k * 7_000;
                let b = svc.samples_window(sid, "Kick_In", a, a + 20_000).unwrap();
                assert_eq!(b, expected(full, 48_000, a, a + 20_000), "window {k}");
            });
        }
    });
    std::fs::remove_dir_all(&root).unwrap();
}

// ---------- bench (ignored) ----------

fn rss_mb() -> f64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .unwrap_or(0.0)
        / 1024.0
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    let i = ((p / 100.0) * (v.len() - 1) as f64).round() as usize;
    v[i]
}

fn open_and_build(svc: &Service, dir: &Path) -> (String, f64, f64) {
    let t0 = Instant::now();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let t_open = t0.elapsed().as_secs_f64() * 1000.0;
    svc.peaks_build_start(&sid, None, None).unwrap();
    while svc.peaks_build_status(&sid)["state"] != "done" {
        std::thread::sleep(Duration::from_millis(2));
    }
    (sid, t_open, t0.elapsed().as_secs_f64() * 1000.0)
}

#[test]
#[ignore = "bench: needs DRUMSPIKE_TRACKS_BENCH=<make_session.py folder>"]
fn lanes_fast_path_bench_of_the_generated_session() {
    let Some(folder) = std::env::var_os("DRUMSPIKE_TRACKS_BENCH").map(PathBuf::from) else {
        panic!("DRUMSPIKE_TRACKS_BENCH is not set");
    };
    let root = tmp("lanes_bench");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    svc.set_projects_dir(root.join("projects"));
    let r = svc.project_new(Some("Bench"), None, None).unwrap();
    let sid = r["sid"].as_str().unwrap().to_string();
    let dir = PathBuf::from(r["folder"].as_str().unwrap());
    let add = svc
        .tracks_add(&sid, &[folder.display().to_string()], None, &[], &[], false, None)
        .unwrap();
    let names: Vec<String> = add["added"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(names.len(), 30, "{add}");
    svc.project_save(&sid).unwrap();
    svc.project_close(&sid, true).unwrap();
    let rss0 = rss_mb();
    eprintln!("BENCH rss after add and close {rss0:.0} MB");
    // first open without peaks.bin (warm: the files were read by the add)
    let mut sid = String::new();
    for round in 0..3 {
        if !sid.is_empty() {
            svc.project_close(&sid, true).unwrap();
        }
        std::fs::remove_dir_all(dir.join("Analysis")).unwrap();
        let (s, t_open, t_all) = open_and_build(&svc, &dir);
        let st = svc.peaks_build_status(&s);
        let done = st["tracks"]
            .as_object()
            .unwrap()
            .values()
            .filter(|t| t["state"] == "done")
            .count();
        eprintln!(
            "BENCH first open round {round}: open {t_open:.0} ms, open and lane job {t_all:.0} ms, {done} tracks built, rss {:.0} MB",
            rss_mb()
        );
        assert_eq!(done, 30);
        sid = s;
    }
    let rss1 = rss_mb();
    // windows at sample zoom: a view of 2,000 samples plus 48,000 on each side (the page's
    // fetchWindow), 30 lanes, 20 view positions across the song; once one lane after the
    // other, once all 30 lanes at the same time
    let n = 96_000 * 300i64;
    let positions: Vec<i64> = (0..20).map(|k| 48_000 + k * (n - 200_000) / 19).collect();
    let mut seq = Vec::new();
    for &pos in &positions {
        for name in &names {
            let t = Instant::now();
            let b = svc
                .samples_window(&sid, name, pos - 48_000, pos + 2_000 + 48_000)
                .unwrap();
            seq.push(t.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(b.len(), 8 + 4 * 98_000);
        }
    }
    let rss2 = rss_mb();
    let mut par = Vec::new();
    for &pos in &positions {
        let lat: Vec<f64> = std::thread::scope(|s| {
            let hs: Vec<_> = names
                .iter()
                .map(|name| {
                    let (svc, sid) = (&svc, &sid);
                    s.spawn(move || {
                        let t = Instant::now();
                        svc.samples_window(sid, name, pos + 7_000 - 48_000, pos + 7_000 + 50_000)
                            .unwrap();
                        t.elapsed().as_secs_f64() * 1000.0
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        par.extend(lat);
    }
    let rss3 = rss_mb();
    eprintln!(
        "BENCH windows one after the other: {} requests, p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms",
        seq.len(),
        pct(&mut seq.clone(), 50.0),
        pct(&mut seq.clone(), 95.0),
        pct(&mut seq, 100.0)
    );
    eprintln!(
        "BENCH windows 30 lanes at once: {} requests, p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms",
        par.len(),
        pct(&mut par.clone(), 50.0),
        pct(&mut par.clone(), 95.0),
        pct(&mut par, 100.0)
    );
    eprintln!(
        "BENCH rss: after the lane jobs {rss1:.0} MB, after the serial windows {rss2:.0} MB, after the parallel windows {rss3:.0} MB; growth over the windows {:.0} MB",
        rss3.max(rss2) - rss1
    );
    svc.project_close(&sid, true).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
}
