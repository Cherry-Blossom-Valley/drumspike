//! P3-4c: track import through `drumspike_app::Service`, without Tauri. Test names start with the
//! command name; the last tests check the route map row 23.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use common::*;
use drumspike_app::roles::guess_role;
use drumspike_app::{Progress, Service, TRACKS_COMMANDS};
use drumspike_core::project::{load_project, SourceStatus};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn service(root: &Path) -> Service {
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    svc.set_projects_dir(root.join("projects"));
    svc
}

fn new_project(svc: &Service, name: &str) -> (String, PathBuf) {
    let r = svc.project_new(Some(name), None, None).unwrap();
    (
        r["sid"].as_str().unwrap().to_string(),
        PathBuf::from(r["folder"].as_str().unwrap()),
    )
}

fn put(root: &Path, rel: &str, sr: u32) -> String {
    let p = root.join(rel);
    write_wav(&p, sr, &kick_signal(&[0.25, 0.75], 1.5, sr));
    p.display().to_string()
}

fn collect() -> (Progress, Arc<Mutex<Vec<Value>>>) {
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let p: Progress = Arc::new(move |v: Value| sink.lock().unwrap().push(v));
    (p, seen)
}

// ---------- tracks_scan ----------

#[test]
fn tracks_scan_lists_a_folder_with_roles_and_the_rejected_files() {
    let root = tmp("scan");
    let svc = service(&root);
    for n in ["k in_02", "k out_02", "s top_02", "t2_02", "oh_03.L", "Clap_01"] {
        put(&root, &format!("drop/{n}.wav"), 48_000);
    }
    put(&root, "drop/rate.wav", 88_200);
    std::fs::write(root.join("drop/old.aiff"), b"FORM\0\0\0\x04AIFF").unwrap();
    let r = svc
        .tracks_scan(&[root.join("drop").display().to_string()], None)
        .unwrap();
    let roles: Vec<(String, String)> = r["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["stem"].as_str().unwrap().into(), f["role"].as_str().unwrap().into()))
        .collect();
    assert_eq!(
        roles,
        [
            ("Clap_01", "kick"), // the signal is a kick, so the audio suggestion fires (#406)
            ("k in_02", "kick"),
            ("k out_02", "kick"),
            ("oh_03.L", "cymbal"),
            ("s top_02", "snare"),
            ("t2_02", "tom")
        ]
        .map(|(a, b)| (a.to_string(), b.to_string()))
    );
    let rej = r["rejected"].as_array().unwrap();
    assert_eq!(rej.len(), 2);
    let why = |n: &str| {
        rej.iter().find(|x| x["name"] == n).unwrap()["reason"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(
        why("old.aiff").starts_with("AIFF files are not supported"),
        "{}",
        why("old.aiff")
    );
    assert!(why("rate.wav").contains("88200 Hz"), "{}", why("rate.wav"));
}

#[test]
fn kit_presets_list_save_and_load_round_trip_through_the_files() {
    let root = tmp("presets");
    let svc = service(&root);
    assert_eq!(svc.kit_presets_list().unwrap(), json!([]));
    svc.kit_preset_save("Studio", &json!([{"pattern": "Clap", "role": "other", "note": 39}]))
        .unwrap();
    assert_eq!(svc.kit_presets_list().unwrap(), json!([{"name": "Studio", "rules": 1}]));
    assert!(root.join("home/kit-presets/Studio.json").is_file());
    // a second service on the same home folder sees the preset (it lives in a file)
    let other = service(&root);
    assert_eq!(
        other.kit_preset_load("Studio").unwrap(),
        json!({"name": "Studio", "rules": [{"pattern": "clap", "role": "other", "note": 39}]})
    );
}

#[test]
fn kit_preset_load_rules_win_in_the_scan() {
    let root = tmp("preset_use");
    let svc = service(&root);
    svc.kit_preset_save("Mine", &json!([{"pattern": "k samp", "role": "kick", "note": 35}]))
        .unwrap();
    // steady noise audio: the core scores None, so only the preset rules decide (#406)
    let p = root.join("drop/k samp savior_01.wav");
    write_wav(&p, 48_000, &noise_signal(1.5, 48_000));
    let p = p.display().to_string();
    let r = svc.tracks_scan(std::slice::from_ref(&p), Some("Mine")).unwrap();
    assert_eq!(
        (r["files"][0]["role"].clone(), r["files"][0]["note"].clone()),
        (json!("kick"), json!(35))
    );
    let none = svc.tracks_scan(&[p], None).unwrap();
    assert_eq!(none["files"][0]["guess"], "none");
}

#[test]
fn kit_preset_save_replaces_the_preset_of_the_same_name_and_checks_the_rules() {
    let root = tmp("preset_save");
    let svc = service(&root);
    svc.kit_preset_save("Studio", &json!([{"pattern": "a", "role": "kick"}]))
        .unwrap();
    svc.kit_preset_save("Studio", &json!([])).unwrap();
    assert_eq!(svc.kit_presets_list().unwrap(), json!([{"name": "Studio", "rules": 0}]));
    assert_eq!(
        svc.kit_preset_save("Studio", &json!([{"pattern": "a", "role": "bass"}]))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        svc.kit_preset_load("Studio").unwrap()["rules"],
        json!([]),
        "a bad save leaves the file"
    );
}

// ---------- tracks_add ----------

#[test]
fn tracks_add_references_the_files_where_they_are_and_leaves_them_unchanged() {
    let root = tmp("add_ref");
    let svc = service(&root);
    let (sid, dir) = new_project(&svc, "Song");
    let a = put(&root, "daw/k in_02.wav", 48_000);
    let b = put(&root, "daw/s top_02.wav", 96_000);
    let c = put(&root, "daw/Clap_01.wav", 44_100);
    let before: Vec<String> = [&a, &b, &c]
        .iter()
        .map(|p| {
            let bytes = std::fs::read(p).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hex(hasher.finalize())
        })
        .collect();
    let (progress, seen) = collect();
    let r = svc
        .tracks_add(
            &sid,
            &[root.join("daw").display().to_string()],
            &[],
            &[],
            false,
            Some(progress),
        )
        .unwrap();
    assert_eq!(r["added"], json!(["Clap_01", "k_in_02", "s_top_02"]));
    assert_eq!(r["rejected"], json!([]));
    let t = &r["tracks"];
    assert_eq!(
        (t[1]["role"].clone(), t[1]["note"].clone(), t[1]["mode"].clone()),
        (json!("kick"), json!(36), json!("referenced"))
    );
    assert_eq!(
        (t[2]["role"].clone(), t[2]["note"].clone()),
        (json!("snare"), json!(38))
    );
    assert_eq!(
        (t[0]["role"].clone(), t[0]["note"].clone()),
        (json!("other"), json!(36))
    );
    // the source files are the same bytes
    let after: Vec<String> = [&a, &b, &c]
        .iter()
        .map(|p| {
            let bytes = std::fs::read(p).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hex(hasher.finalize())
        })
        .collect();
    assert_eq!(before, after);
    assert!(
        !dir.join("Audio Files").join("k in_02.wav").exists(),
        "nothing is copied"
    );
    // the project knows the tracks: online, dirty, rate and length from the header
    let g = svc.project_get(&sid).unwrap();
    assert_eq!(g["dirty"], true);
    assert_eq!(g["order"], json!(["Clap_01", "k_in_02", "s_top_02"]));
    assert_eq!(g["tracks"]["s_top_02"]["status"], "ok");
    assert_eq!(g["tracks"]["s_top_02"]["source"]["mode"], "referenced");
    let tr = svc.tracks_get(&sid).unwrap();
    assert_eq!(tr["tracks"]["s_top_02"]["sr"], 96_000);
    assert_eq!(tr["tracks"]["s_top_02"]["n_samples"], 144_000);
    assert_eq!(tr["tracks"]["s_top_02"]["color"], "#e0a13c");
    svc.project_save(&sid).unwrap();
    let p = load_project(&dir).unwrap();
    let rec = p.doc.track("k_in_02").unwrap().source_ref().unwrap().clone();
    assert_eq!(rec.mode.as_deref(), Some("referenced"));
    assert_eq!(rec.path.as_deref(), Some(a.as_str()));
    // progress: running before and after each file, done with the result
    let msgs = seen.lock().unwrap().clone();
    assert!(msgs.iter().all(|m| m["sid"] == sid.as_str()));
    assert_eq!(msgs.first().unwrap()["add"]["state"], "running");
    let last = msgs.last().unwrap();
    assert_eq!(
        (
            last["add"]["state"].clone(),
            last["add"]["done"].clone(),
            last["add"]["total"].clone()
        ),
        (json!("done"), json!(3), json!(3))
    );
    assert_eq!(last["add"]["result"]["added"].as_array().unwrap().len(), 3);
}

#[test]
fn tracks_add_copies_into_audio_files_when_asked_and_reports_progress() {
    let root = tmp("add_copy");
    let svc = service(&root);
    let (sid, dir) = new_project(&svc, "Song");
    let a = put(&root, "daw/k in_02.wav", 48_000);
    let b = put(&root, "daw/oh_01.wav", 48_000);
    let (progress, seen) = collect();
    let r = svc
        .tracks_add(&sid, &[a.clone(), b], &[], &[], true, Some(progress))
        .unwrap();
    assert_eq!(r["added"], json!(["k_in_02", "oh_01"]));
    assert_eq!(r["tracks"][0]["mode"], "copied");
    assert_eq!(
        std::fs::read(&a).unwrap(),
        std::fs::read(dir.join("Audio Files/k in_02.wav")).unwrap(),
        "byte-identical"
    );
    let g = svc.project_get(&sid).unwrap();
    assert_eq!(g["tracks"]["oh_01"]["source"]["mode"], "copied");
    assert_eq!(g["tracks"]["oh_01"]["source"]["relative"], "Audio Files/oh_01.wav");
    let dones: Vec<i64> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m["add"]["state"] == "running")
        .map(|m| m["add"]["done"].as_i64().unwrap())
        .collect();
    assert_eq!(dones, [0, 1, 1, 2]);
}

#[test]
fn tracks_add_uses_the_roles_and_notes_of_the_call_and_guesses_the_rest() {
    let root = tmp("add_roles");
    let svc = service(&root);
    let (sid, _) = new_project(&svc, "Song");
    let files: Vec<String> = ["a_01", "b_01", "k in_02", "hh_03"]
        .iter()
        .map(|n| put(&root, &format!("daw/{n}.wav"), 48_000))
        .collect();
    let roles = [json!("tom"), json!("bass"), Value::Null, json!("cymbal")];
    let notes = [json!(50), json!(500), json!("x"), Value::Null];
    let r = svc.tracks_add(&sid, &files, &roles, &notes, false, None).unwrap();
    let got: Vec<(String, i64)> = r["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["role"].as_str().unwrap().to_string(), t["note"].as_i64().unwrap()))
        .collect();
    // a_01: the call; b_01: unknown role and note out of range -> guess other, note 36;
    // k in_02: nothing sent -> the scheme; hh_03: role cymbal sent, note missing -> the cymbal note
    assert_eq!(
        got,
        [
            ("tom".to_string(), 50),
            ("other".to_string(), 36),
            ("kick".to_string(), 36),
            ("cymbal".to_string(), 49)
        ]
    );
}

/// 0.9.1: a kept guess records the role source and the matched words of the name; a role of the
/// call is the user's and has no matched words (the Inspector's hint tells the two apart).
#[test]
fn tracks_add_records_the_role_source_and_the_matched_words_of_a_kept_guess() {
    let root = tmp("add_role_source");
    let svc = service(&root);
    let (sid, _) = new_project(&svc, "Song");
    let files: Vec<String> = ["k in_01", "Clap_01", "Snare Top"]
        .iter()
        .map(|n| put(&root, &format!("daw/{n}.wav"), 48_000))
        .collect();
    let roles = [Value::Null, Value::Null, json!("snare")];
    let notes = [Value::Null, Value::Null, Value::Null];
    svc.tracks_add(&sid, &files, &roles, &notes, false, None).unwrap();
    let tr = |sid: &str, name: &str| svc.tracks_get(sid).unwrap()["tracks"][name].clone();
    let k = tr(&sid, "k_in_01");
    assert_eq!(k["role_source"], json!("scheme"));
    assert_eq!(k["role_match"], json!("k in"));
    let c = tr(&sid, "Clap_01");
    assert_eq!(c["role_source"], json!("none"));
    assert_eq!(c["role_match"], json!(null));
    let s = tr(&sid, "Snare_Top");
    assert_eq!(
        s["role_source"],
        json!("scheme"),
        "the role equals the guess: the name made it"
    );
    assert_eq!(s["role_match"], json!("snare"));
}

/// P4-4 (B3): `Kick.wav` and `kick.wav` from two folders were the tracks `Kick` and `kick`, one
/// file name for their exports and their `Analysis/` folder on a disk that ignores case.
#[test]
fn tracks_add_makes_names_unique_ignoring_case() {
    let root = tmp("add_case");
    let svc = service(&root);
    let (sid, _) = new_project(&svc, "Song");
    let a = put(&root, "one/Kick.wav", 48_000);
    let b = put(&root, "two/kick.wav", 48_000);
    let c = put(&root, "three/KICK.wav", 48_000);
    let r = svc.tracks_add(&sid, &[a, b, c], &[], &[], false, None).unwrap();
    assert_eq!(r["added"], json!(["Kick", "kick_2", "KICK_3"]));
}

#[test]
fn tracks_add_makes_unique_names_and_adds_to_the_tracks_already_there() {
    let root = tmp("add_names");
    let svc = service(&root);
    let (sid, _) = new_project(&svc, "Song");
    let a = put(&root, "one/k in_02.wav", 48_000);
    let b = put(&root, "two/k in_02.wav", 48_000);
    assert_eq!(
        svc.tracks_add(&sid, &[a], &[], &[], false, None).unwrap()["added"],
        json!(["k_in_02"])
    );
    assert_eq!(
        svc.tracks_add(&sid, &[b], &[], &[], false, None).unwrap()["added"],
        json!(["k_in_02_2"])
    );
    assert_eq!(svc.project_get(&sid).unwrap()["order"], json!(["k_in_02", "k_in_02_2"]));
}

#[test]
fn tracks_add_rejects_aiff_and_88_khz_with_the_reason_and_adds_the_rest() {
    let root = tmp("add_reject");
    let svc = service(&root);
    let (sid, _) = new_project(&svc, "Song");
    let ok = put(&root, "daw/k in_02.wav", 48_000);
    let rate = put(&root, "daw/rate.wav", 88_200);
    let aiff = root.join("daw/old.aif");
    std::fs::write(&aiff, b"FORM\0\0\0\x04AIFF").unwrap();
    std::fs::write(root.join("daw/broken.wav"), b"RIFFxxxxWAVE").unwrap();
    let all = [
        ok,
        rate,
        aiff.display().to_string(),
        root.join("daw/broken.wav").display().to_string(),
        root.join("daw/gone.wav").display().to_string(),
    ];
    let r = svc.tracks_add(&sid, &all, &[], &[], false, None).unwrap();
    assert_eq!(r["added"], json!(["k_in_02"]));
    let rej = r["rejected"].as_array().unwrap();
    assert_eq!(rej.len(), 4);
    let why = |n: &str| {
        rej.iter().find(|x| x["name"] == n).unwrap()["reason"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(why("rate.wav").contains("88200 Hz") && why("rate.wav").contains("44.1, 48 or 96 kHz"));
    assert!(why("old.aif").contains("AIFF"));
    assert!(why("broken.wav").contains("not supported"));
    assert_eq!(why("gone.wav"), "the file does not exist");
    assert_eq!(svc.project_get(&sid).unwrap()["order"], json!(["k_in_02"]));
    // only rejected files: not an error, nothing added, the project stays clean
    let (sid2, _) = new_project(&svc, "Other");
    let r = svc
        .tracks_add(&sid2, &[aiff.display().to_string()], &[], &[], false, None)
        .unwrap();
    assert_eq!(
        (r["added"].clone(), r["rejected"].as_array().unwrap().len()),
        (json!([]), 1)
    );
    assert_eq!(svc.project_get(&sid2).unwrap()["dirty"], false);
}

#[test]
fn tracks_add_knows_the_project() {
    let root = tmp("add_unknown");
    let svc = service(&root);
    assert_eq!(
        svc.tracks_add("nope", &[], &[], &[], false, None).unwrap_err().code,
        404
    );
}

#[test]
fn tracks_add_builds_the_peak_caches_so_the_lane_job_has_nothing_left() {
    let root = tmp("add_peaks");
    let svc = service(&root);
    let (sid, dir) = new_project(&svc, "Song");
    let files = [
        put(&root, "daw/k in_02.wav", 48_000),
        put(&root, "daw/s top_02.wav", 44_100),
        put(&root, "daw/oh_01.wav", 96_000),
    ];
    let (progress, seen) = collect();
    let r = svc.tracks_add(&sid, &files, &[], &[], false, Some(progress)).unwrap();
    assert!(r["peaks_ms"].as_f64().unwrap() > 0.0);
    for t in ["k_in_02", "s_top_02", "oh_01"] {
        assert!(dir.join("Analysis").join(t).join("peaks.bin").is_file(), "{t}");
        assert!(dir.join("Analysis").join(t).join("key.json").is_file(), "{t}");
    }
    let peaks: Vec<i64> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m["add"]["state"] == "peaks")
        .map(|m| m["add"]["done"].as_i64().unwrap())
        .collect();
    assert_eq!(peaks, [1, 2, 3]);
    assert_eq!(svc.peaks_build_start(&sid, None, None).unwrap()["started"], json!([]));
    assert_eq!(svc.peaks_info(&sid, "s_top_02").unwrap()["sr"], 44_100);
    // nothing to build: a call with only rejected files reports 0 ms
    let r = svc
        .tracks_add(
            &sid,
            &[root.join("none.wav").display().to_string()],
            &[],
            &[],
            false,
            None,
        )
        .unwrap();
    assert_eq!(r["peaks_ms"], 0.0);
}

#[test]
fn the_added_tracks_survive_save_and_open_with_their_status() {
    let root = tmp("add_reopen");
    let svc = service(&root);
    let (sid, dir) = new_project(&svc, "Song");
    let f = put(&root, "daw/oh_01.wav", 48_000);
    svc.tracks_add(&sid, &[f], &[], &[], false, None).unwrap();
    svc.project_save(&sid).unwrap();
    let again = service(&root);
    let o = again.project_open(dir.to_str().unwrap()).unwrap();
    assert_eq!(o["tracks"]["oh_01"]["status"], SourceStatus::Ok.as_str());
    assert_eq!(o["tracks"]["oh_01"]["role"], "cymbal");
    assert_eq!(o["tracks"]["oh_01"]["note"], 49);
}

// ---------- the real sessions ----------

/// Every name of the three real sessions of `~/.cache/drumspike/sessions` that follows the owner's scheme,
/// with its role. The names outside the scheme must have no role. The report lists the same table.
const SCHEME_NAMES: [(&str, &str); 39] = [
    ("hh_01", "hat"),
    ("hh_03", "hat"),
    ("k in_01", "kick"),
    ("k in_02", "kick"),
    ("k out_01", "kick"),
    ("k out_02", "kick"),
    ("k sub_01", "kick"),
    ("k sub_02", "kick"),
    ("OH_01", "cymbal"),
    ("oh_01", "cymbal"),
    ("oh_03.L", "cymbal"),
    ("oh_03.R", "cymbal"),
    ("ohh 1_01", "cymbal"),
    ("ohh 2_01", "cymbal"),
    ("ohh 3_01", "cymbal"),
    ("ohh 4_01", "cymbal"),
    ("ohh 5_01", "cymbal"),
    ("ri_03", "cymbal"),
    ("ride_01", "cymbal"),
    ("s bot_01", "snare"),
    ("s bot_02", "snare"),
    ("s top_01", "snare"),
    ("s top_02", "snare"),
    ("t 1_03", "tom"),
    ("t 2_03", "tom"),
    ("t 3_03", "tom"),
    ("t 4_03", "tom"),
    ("t1 HH_01", "tom"),
    ("t1 hh_02", "tom"),
    ("t2_01", "tom"),
    ("t2_02", "tom"),
    ("t3_01", "tom"),
    ("t3_02", "tom"),
    ("t4 Ri_01", "tom"),
    ("t4 ri_02", "tom"),
    ("t1 sampl_01", "tom"),
    ("t2 sampl_01", "tom"),
    ("t3 sampl_01", "tom"),
    ("t4 sampl_01", "tom"),
];

#[test]
fn tracks_scan_roles_of_the_scheme_names_of_the_three_real_sessions() {
    for (name, role) in SCHEME_NAMES {
        assert_eq!(guess_role(name), Some(role), "{name}");
    }
}

/// Reads the real sessions when they are on this host: `DRUMSPIKE_REAL_SESSIONS` (a folder with one
/// folder per song) or `~/.cache/drumspike/sessions` (arm64-mac). A song folder holds the files in `Audio Files/` or
/// itself (x86-mac). The names with a role are exactly names of `SCHEME_NAMES`. Prints the table. Only the
/// three songs are read (their folder names start with the song's title): other session folders there have
/// other names (P4-14: six more songs, 2026-10-13).
#[test]
fn tracks_scan_the_real_session_folders_when_present() {
    const SONGS: [&str; 3] = ["Session A", "Session B", "Session C"];
    let base = std::env::var_os("DRUMSPIKE_REAL_SESSIONS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache/drumspike/sessions"));
    let mut seen: Vec<(String, String)> = Vec::new();
    let (mut songs, mut files, mut rejected) = (0, 0, 0);
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&base)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                // the three songs SCHEME_NAMES was written for; the owner adds more sessions to the same folder
                .filter(|p| {
                    let n = p.file_name().unwrap_or_default().to_string_lossy().replace('_', " ");
                    SONGS.iter().any(|s| n.starts_with(s))
                })
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    if dirs.is_empty() {
        eprintln!("SKIP the real sessions: {} is not on this host", base.display());
    }
    for song in dirs {
        songs += 1;
        let audio = if song.join("Audio Files").is_dir() {
            song.join("Audio Files")
        } else {
            song.clone()
        };
        // the scan of the whole folder: every readable file has a role or none, and the rejected files have a reason
        let scan = service(&tmp("real"))
            .tracks_scan(&[song.display().to_string()], None)
            .unwrap();
        files += scan["files"].as_array().unwrap().len();
        rejected += scan["rejected"].as_array().unwrap().len();
        for e in std::fs::read_dir(&audio).unwrap().filter_map(Result::ok) {
            let n = e.file_name().to_string_lossy().into_owned();
            // skip macOS AppleDouble resource-fork files (._*) and other hidden entries, like the scanner does
            if n.starts_with('.') {
                continue;
            }
            let Some(stem) = n.strip_suffix(".wav") else { continue };
            if let Some(role) = guess_role(stem) {
                seen.push((stem.to_string(), role.to_string()));
            }
        }
    }
    seen.sort();
    for (n, r) in &seen {
        eprintln!("ROLE {r:7} {n}");
    }
    eprintln!(
        "REAL songs {songs}, readable files {files}, rejected {rejected}, names with a role {}",
        seen.len()
    );
    for (n, r) in &seen {
        assert!(
            SCHEME_NAMES.iter().any(|(a, b)| a == n && b == r),
            "{n} -> {r} is not in the table SCHEME_NAMES"
        );
    }
}

// ---------- import time on a generated session (ignored; the numbers of the report) ----------

/// `DRUMSPIKE_TRACKS_BENCH=<make_session.py folder>`: the folder dropped on an empty project,
/// referenced, with the peak cache; the sources keep their SHA-256. Run with
/// `cargo test --release -p drumspike-app --test tracks -- --ignored --nocapture`.
#[test]
#[ignore = "needs a generated session: DRUMSPIKE_TRACKS_BENCH=<make_session.py folder>"]
fn tracks_add_import_time_of_the_generated_session() {
    let Some(folder) = std::env::var_os("DRUMSPIKE_TRACKS_BENCH").map(PathBuf::from) else {
        panic!("DRUMSPIKE_TRACKS_BENCH is not set");
    };
    let root = tmp("bench");
    let svc = service(&root);
    let mut wavs: Vec<PathBuf> = std::fs::read_dir(&folder)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    wavs.sort();
    let before: Vec<String> = wavs
        .iter()
        .map(|p| {
            let bytes = std::fs::read(p).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hex(hasher.finalize())
        })
        .collect();
    for round in 0..3 {
        let (sid, dir) = new_project(&svc, &format!("Bench {round}"));
        let t0 = Instant::now();
        let scan = svc.tracks_scan(&[folder.display().to_string()], None).unwrap();
        let t_scan = t0.elapsed();
        let r = svc
            .tracks_add(&sid, &[folder.display().to_string()], &[], &[], false, None)
            .unwrap();
        let t_add = t0.elapsed();
        svc.peaks_build_start(&sid, None, None).unwrap(); // nothing left: tracks_add built the caches
        while svc.peaks_build_status(&sid)["state"] != "done" {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let t_all = t0.elapsed();
        let st = svc.peaks_build_status(&sid);
        let errors = st["tracks"]
            .as_object()
            .unwrap()
            .values()
            .filter(|t| t["state"] == "error")
            .count();
        let g = svc.project_get(&sid).unwrap();
        let online = g["tracks"]
            .as_object()
            .unwrap()
            .values()
            .filter(|t| t["status"] == "ok")
            .count();
        let referenced = g["tracks"]
            .as_object()
            .unwrap()
            .values()
            .filter(|t| t["source"]["mode"] == "referenced")
            .count();
        let peaks = std::fs::read_dir(dir.join("Analysis")).unwrap().count();
        eprintln!(
            "BENCH round {round}: files {} added {} online {online} referenced {referenced} peaks {peaks} build errors {errors} | scan {:.0} ms, add {:.0} ms (of which peak cache {:.0} ms), with the lane job {:.0} ms",
            scan["files"].as_array().unwrap().len(),
            r["added"].as_array().unwrap().len(),
            t_scan.as_secs_f64() * 1000.0,
            t_add.as_secs_f64() * 1000.0,
            r["peaks_ms"].as_f64().unwrap(),
            t_all.as_secs_f64() * 1000.0,
        );
        assert_eq!(errors, 0);
    }
    let after: Vec<String> = wavs.iter().map(|p| file_sha256(p).unwrap()).collect();
    assert_eq!(before, after, "the sources keep their SHA-256");
    eprintln!("BENCH sha256 unchanged for {} files", wavs.len());
}

// ---------- every command has a test ----------

#[test]
fn every_tracks_command_has_a_test_in_this_file() {
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tracks.rs")).unwrap();
    for c in TRACKS_COMMANDS {
        assert!(
            src.contains(&format!("fn {c}_")),
            "no test named {c}_... in tests/tracks.rs"
        );
    }
}

// ---------- tracks_remove and tracks_restore (P4-39a) ----------

/// A project of four tracks: `k in`, `k sub` (follows `k in`), `s top`, `t1`; every track has three hits with an extra
/// key, a marker file in `Analysis/<name>/`, and `s top` has params and a Learn object.
fn four_tracks(tag: &str) -> (Service, String, PathBuf) {
    let root = tmp(tag);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project(
        &root,
        "Remove",
        &[
            kick_spec("k in", "kick", 36),
            kick_spec("k sub", "kick", 36),
            kick_spec("s top", "snare", 38),
            kick_spec("t1", "tom", 48),
        ],
    );
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    for n in ["k in", "k sub", "s top", "t1"] {
        let hits: Vec<Value> = [0.25, 0.75, 1.25]
            .iter()
            .enumerate()
            .map(|(i, t)| json!({"t": t, "v": 90 + i, "id": format!("{n}-{i}"), "zz_extra": n}))
            .collect();
        svc.events_set(&sid, n, &json!(hits)).unwrap();
        let a = dir.join("Analysis").join(n);
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("candidates.bin"), b"marker").unwrap();
    }
    let params = json!({"min_gap_ms": 33.0});
    svc.track_params_set(&sid, "s top", params.as_object().unwrap())
        .unwrap();
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    svc.project_save(&sid).unwrap();
    (svc, sid, dir)
}

fn order_of(svc: &Service, sid: &str) -> Vec<String> {
    svc.tracks_get(sid).unwrap()["order"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn tracks_remove_takes_the_record_the_hits_the_order_and_the_analysis_folder() {
    let (svc, sid, dir) = four_tracks("rm_basic");
    let r = svc.tracks_remove(&sid, &["s top".to_string()], false).unwrap();
    assert_eq!(r["removed"], json!(["s top"]));
    assert_eq!(r["order"], json!(["k in", "k sub", "t1"]));
    assert_eq!(r["unfollowed"], json!([]));
    let names: Vec<&str> = r["tracks"].as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        names,
        ["k in", "k sub", "t1"],
        "the answer lists the tracks that are left"
    );
    assert!(r.get("snapshot").is_none(), "no snapshot unless asked for");
    assert!(!dir.join("Analysis/s top").exists(), "the track's folder is deleted");
    assert!(
        dir.join("Analysis/k in/candidates.bin").is_file(),
        "the other folders stay"
    );
    assert_eq!(svc.events_get(&sid, "s top").unwrap_err().code, 400);
    assert_eq!(svc.tracks_get(&sid).unwrap()["dirty"], true);
    // saved, the document has no trace of the track (record, hits, Learn, params, order)
    svc.project_save(&sid).unwrap();
    let doc = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "drumspike"))
        .unwrap();
    let text = std::fs::read_to_string(doc).unwrap();
    assert!(text.contains("k in") && text.contains("t1"));
    assert!(!text.contains("s top") && !text.contains("min_gap_ms"), "{text}");
}

#[test]
fn tracks_remove_can_take_several_tracks_and_ignores_a_name_given_twice() {
    let (svc, sid, dir) = four_tracks("rm_many");
    let names = ["t1", "k sub", "t1"].map(String::from);
    let r = svc.tracks_remove(&sid, &names, false).unwrap();
    assert_eq!(r["removed"], json!(["t1", "k sub"]));
    assert_eq!(order_of(&svc, &sid), ["k in", "s top"]);
    assert!(!dir.join("Analysis/t1").exists() && !dir.join("Analysis/k sub").exists());
    assert!(dir.join("Analysis/s top").is_dir());
}

#[test]
fn tracks_remove_refuses_an_unknown_name_or_an_empty_list_and_removes_nothing() {
    let (svc, sid, dir) = four_tracks("rm_bad");
    let both = ["k in", "Nope"].map(String::from);
    assert_eq!(svc.tracks_remove(&sid, &both, false).unwrap_err().code, 400);
    assert_eq!(svc.tracks_remove(&sid, &[], false).unwrap_err().code, 400);
    assert_eq!(
        svc.tracks_remove("00000000", &["k in".to_string()], false)
            .unwrap_err()
            .code,
        404
    );
    assert_eq!(order_of(&svc, &sid).len(), 4);
    assert!(dir.join("Analysis/k in").is_dir(), "a refused call deletes no folder");
}

#[test]
fn tracks_remove_makes_the_follower_of_a_removed_track_a_normal_track() {
    let (svc, sid, _dir) = four_tracks("rm_follow");
    let before = svc.events_get(&sid, "k sub").unwrap()["events"].clone();
    let r = svc.tracks_remove(&sid, &["k in".to_string()], true).unwrap();
    assert_eq!(r["unfollowed"], json!(["k sub"]));
    assert_eq!(r["tracks"]["k sub"]["follows"], Value::Null);
    assert_eq!(
        svc.events_get(&sid, "k sub").unwrap()["events"],
        before,
        "its hits stay"
    );
    assert_eq!(r["snapshot"][0]["followers"], json!(["k sub"]));
    // a follower removed together with its source is no "unfollowed" track
    let (svc, sid, _dir) = four_tracks("rm_follow_both");
    let both = ["k in", "k sub"].map(String::from);
    assert_eq!(svc.tracks_remove(&sid, &both, false).unwrap()["unfollowed"], json!([]));
}

#[test]
fn tracks_restore_puts_the_tracks_back_with_hits_params_extra_fields_and_the_follow() {
    let (svc, sid, dir) = four_tracks("rm_undo");
    let before = svc.tracks_get(&sid).unwrap();
    let hits = |n: &str| svc.events_get(&sid, n).unwrap()["events"].clone();
    let (h_in, h_sub, h_top) = (hits("k in"), hits("k sub"), hits("s top"));
    let names = ["k in", "s top"].map(String::from);
    let r = svc.tracks_remove(&sid, &names, true).unwrap();
    let snap = r["snapshot"].clone();
    assert_eq!(order_of(&svc, &sid), ["k sub", "t1"]);
    let back = svc.tracks_restore(&sid, &snap).unwrap();
    assert_eq!(back["restored"], json!(["k in", "s top"]));
    assert_eq!(back["order"], json!(["k in", "k sub", "s top", "t1"]));
    let after = svc.tracks_get(&sid).unwrap();
    assert_eq!(after["order"], before["order"]);
    assert_eq!(
        after["tracks"], before["tracks"],
        "params, Learn, follows, mix: every field"
    );
    assert_eq!(hits("k in"), h_in);
    assert_eq!(hits("s top"), h_top);
    assert_eq!(hits("k sub"), h_sub);
    assert_eq!(
        after["tracks"]["k sub"]["follows"], "k in",
        "the follower follows again"
    );
    assert_eq!(hits("s top")[0]["zz_extra"], "s top", "extra keys of the hits");
    assert_eq!(after["dirty"], true);
    // the records are back at their places in the document too (the saved file keeps the order of the map)
    svc.project_save(&sid).unwrap();
    let saved = load_project(&dir).unwrap();
    let keys: Vec<&str> = saved.doc.tracks().map(|(n, _)| n.as_str()).collect();
    assert_eq!(keys, ["k in", "k sub", "s top", "t1"]);
    // the caches the removal deleted are made again
    assert!(dir.join("Analysis/s top/peaks.bin").is_file());
    assert!(dir.join("Analysis/k in/peaks.bin").is_file());
}

#[test]
fn tracks_restore_refuses_a_taken_name_and_a_bad_snapshot() {
    let (svc, sid, _dir) = four_tracks("rm_undo_bad");
    let r = svc.tracks_remove(&sid, &["t1".to_string()], true).unwrap();
    let snap = r["snapshot"].clone();
    svc.tracks_restore(&sid, &snap).unwrap();
    assert_eq!(
        svc.tracks_restore(&sid, &snap).unwrap_err().code,
        409,
        "t1 is there again"
    );
    assert_eq!(svc.tracks_restore(&sid, &json!({"name": "x"})).unwrap_err().code, 400);
    assert_eq!(svc.tracks_restore(&sid, &json!([{"track": {}}])).unwrap_err().code, 400);
    assert_eq!(
        svc.tracks_restore(&sid, &json!([{"name": "a/b", "track": {}}]))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        svc.tracks_restore(&sid, &json!([{"name": "x", "track": 3}]))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(order_of(&svc, &sid).len(), 4, "nothing was added");
    assert_eq!(svc.tracks_restore("00000000", &json!([])).unwrap_err().code, 404);
}

// ---------- track_set_role (0.9.1) ----------

#[test]
fn track_set_role_changes_role_source_and_the_note_on_request() {
    let root = tmp("role_set");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project(&root, "Role", &[kick_spec("Other", "other", 36)]);
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();

    // the role changes, the source records the user, the note is offered (36 is the default of "other")
    let r = svc
        .track_set_role(&sid, "Other", "snare", None, None, None, None)
        .unwrap();
    assert_eq!(r["role"], json!("snare"));
    assert_eq!(r["role_source"], json!("user"));
    assert_eq!(r["note"], json!(36), "no note on request: the note stays");
    assert_eq!(r["suggest_note"], json!(38));
    assert_eq!(r["previous"]["role"], json!("other"));

    // the note offer taken: the note changes, the role source stays, no new analysis
    let r = svc
        .track_set_role(&sid, "Other", "snare", Some(38), None, None, None)
        .unwrap();
    assert_eq!(r["note"], json!(38));
    assert_eq!(r["role_source"], json!("user"));
    assert_eq!(r["suggest_note"], json!(null));
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], json!(true));

    // an explicit source goes through ("audio" is reserved for #225)
    let r = svc
        .track_set_role(&sid, "Other", "hat", Some(42), Some("audio"), None, None)
        .unwrap();
    assert_eq!(r["role_source"], json!("audio"));

    let e = svc
        .track_set_role(&sid, "Other", "duck", None, None, None, None)
        .unwrap_err();
    assert_eq!(e.code, 400);
    let e = svc
        .track_set_role(&sid, "Other", "hat", None, Some("vibes"), None, None)
        .unwrap_err();
    assert_eq!(e.code, 400);
}

#[test]
fn track_set_role_restores_the_events_of_the_undo_state_exactly() {
    let root = tmp("role_undo");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project(&root, "Role", &[kick_spec("Other", "other", 36)]);
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let before = svc.events_get(&sid, "Other").unwrap()["events"].clone();
    let edited = json!([{"t": 0.5, "v": 100, "added": true}, {"t": 1.0, "v": 40}]);

    let r = svc
        .track_set_role(&sid, "Other", "snare", None, None, None, Some(&edited))
        .unwrap();
    assert_eq!(r["reanalyzed"], json!(false), "the undo restores, nothing re-runs");
    assert_eq!(svc.events_get(&sid, "Other").unwrap()["events"], edited);

    let r = svc
        .track_set_role(&sid, "Other", "other", Some(36), Some("guess"), None, Some(&before))
        .unwrap();
    assert_eq!(r["role"], json!("other"));
    assert_eq!(r["role_source"], json!("guess"));
    assert_eq!(svc.events_get(&sid, "Other").unwrap()["events"], before);
}

#[test]
fn the_project_round_trips_the_role_source_and_an_old_file_loads() {
    let root = tmp("role_roundtrip");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project(&root, "Role", &[kick_spec("k in", "kick", 36)]);
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.track_set_role(&sid, "k in", "snare", None, None, None, None)
        .unwrap();
    svc.project_save(&sid).unwrap();
    drop(sid);

    // the saved document holds role and source; the reload answers them
    let doc = load_project(&dir).unwrap();
    let tr = doc.doc.tracks().find(|(n, _)| n.as_str() == "k in").unwrap().1;
    assert_eq!(tr.role.as_deref(), Some("snare"));
    assert_eq!(tr.role_source.as_deref(), Some("user"));
    assert_eq!(tr.role_match, None);

    // a role of the naming scheme round-trips its matched words
    svc.track_set_role(
        svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
            .as_str()
            .unwrap(),
        "k in",
        "kick",
        Some(36),
        Some("scheme"),
        Some("k in"),
        None,
    )
    .unwrap();
    svc.project_save(
        svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let doc = load_project(&dir).unwrap();
    let tr = doc.doc.tracks().find(|(n, _)| n.as_str() == "k in").unwrap().1;
    assert_eq!(tr.role.as_deref(), Some("kick"));
    assert_eq!(tr.role_source.as_deref(), Some("scheme"));
    assert_eq!(tr.role_match.as_deref(), Some("k in"));

    // an old project file without `role_source` loads with its old role
    let old = dir.join("old").join("Old.drumspike");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(
        &old,
        r#"{"format": 1, "tracks": {"K": {"role": "kick", "note": 36}}, "events": {"K": []}}"#,
    )
    .unwrap();
    let doc = load_project(&old).unwrap();
    let tr = doc.doc.tracks().find(|(n, _)| n.as_str() == "K").unwrap().1;
    assert_eq!(tr.role.as_deref(), Some("kick"));
    assert_eq!(tr.role_source, None);
}

/// R09 (#237): a busy project refuses `track_set_role` before anything changes, and a failing
/// undo restores nothing: the role fields, the document and the dirty flag stay as they were.
#[test]
fn track_set_role_rejects_and_changes_nothing() {
    let root = tmp("role_r09");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project(&root, "Role", &[kick_spec("Other", "other", 36)]);
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();

    // a held write guard refuses the whole command (no silent `reanalyzed: false`)
    let g = svc.jobs().begin_store(&sid).unwrap();
    let e = svc
        .track_set_role(&sid, "Other", "snare", None, None, None, None)
        .unwrap_err();
    assert_eq!(e.code, 409, "{e:?}");
    drop(g);
    // a queued analysis refuses too
    svc.jobs().queue(&sid, &["Other".into()]);
    let e = svc
        .track_set_role(&sid, "Other", "snare", None, None, None, None)
        .unwrap_err();
    assert_eq!(e.code, 409, "{e:?}");
    assert!(e.message.contains("analysis"), "{e:?}");
    svc.jobs().clear(&sid);

    // invalid events (the undo path): the role is not stored
    let e = svc
        .track_set_role(
            &sid,
            "Other",
            "snare",
            None,
            None,
            None,
            Some(&json!({"not": "a list"})),
        )
        .unwrap_err();
    assert_eq!(e.code, 400, "{e:?}");

    let tr = svc.tracks_get(&sid).unwrap()["tracks"]["Other"].clone();
    assert_eq!(tr["role"], json!("other"), "the role is unchanged");
    assert_eq!(tr["role_source"], json!(null), "the source is unchanged");
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], json!(false), "not dirty");
}

/// R14: `track_set_role` validates a supplied note like `track_set_note` (0..=127).
#[test]
fn track_set_role_rejects_a_note_outside_0_127() {
    let root = tmp("role_r14_note");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project(&root, "Note", &[kick_spec("Kick", "kick", 36)]);
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    for note in [200i64, -1, 128] {
        let e = svc
            .track_set_role(&sid, "Kick", "kick", Some(note), None, None, None)
            .unwrap_err();
        assert_eq!(e.code, 400, "note {note}: {e:?}");
        assert!(e.message.contains("0 to 127"), "{e:?}");
    }
    let tr = svc.tracks_get(&sid).unwrap()["tracks"]["Kick"].clone();
    assert_eq!(tr["note"], json!(36), "the note is unchanged");
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], json!(false), "not dirty");
}

/// R09: the documented re-pick skips keep the role change and answer `reanalyzed: false`
/// (a follower, and a track without activations: the DSP service has no model).
#[test]
fn track_set_role_reports_the_documented_skips_as_reanalyzed_false() {
    let root = tmp("role_skips");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project(
        &root,
        "Role",
        &[kick_spec("K", "kick", 36), kick_spec("Sub", "kick", 36)],
    );
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.track_set_follow(&sid, "Sub", Some("K")).unwrap();
    svc.project_save(&sid).unwrap();

    let r = svc
        .track_set_role(&sid, "Sub", "snare", None, None, None, None)
        .unwrap();
    assert_eq!(r["reanalyzed"], json!(false), "a follower has no detection of its own");
    assert_eq!(svc.tracks_get(&sid).unwrap()["tracks"]["Sub"]["role"], json!("snare"));
    let r = svc.track_set_role(&sid, "K", "snare", None, None, None, None).unwrap();
    assert_eq!(r["reanalyzed"], json!(false), "no activations without the model");
    assert_eq!(svc.tracks_get(&sid).unwrap()["tracks"]["K"]["role"], json!("snare"));
}
