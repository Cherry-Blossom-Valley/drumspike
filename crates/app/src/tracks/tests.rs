use super::*;
use crate::ServiceConfig;
use drumspike_core::io::{WavFormat, WavSpec, encode_wav};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ds_tracks_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn wav(path: &Path, sr: u32, n: usize) {
    let x: Vec<f32> = (0..n).map(|i| ((i as f32) * 0.05).sin() * 0.5).collect();
    let spec = WavSpec {
        sample_rate: sr,
        channels: 1,
        format: WavFormat::Float32,
        extensible: false,
        channel_mask: 0,
    };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, encode_wav(&spec, &x, None)).unwrap();
}

fn service() -> Service {
    Service::new(ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    })
}

fn aiff(path: &Path) {
    let mut b = b"FORM".to_vec();
    b.extend_from_slice(&[0, 0, 0, 4]);
    b.extend_from_slice(b"AIFF");
    std::fs::write(path, b).unwrap();
}

#[test]
fn the_command_list_has_no_duplicates() {
    let mut v = TRACKS_COMMANDS.to_vec();
    v.sort_unstable();
    v.dedup();
    assert_eq!(v.len(), TRACKS_COMMANDS.len());
}

#[test]
fn safe_name_follows_the_server() {
    assert_eq!(safe_name("k in_02", 0), "k_in_02");
    assert_eq!(safe_name("oh_03.L", 0), "oh_03_L");
    assert_eq!(safe_name("Kick - In", 0), "Kick_-_In");
    assert_eq!(safe_name("  __x__ ", 0), "x");
    assert_eq!(safe_name("Bäss", 0), "B_ss");
    assert_eq!(safe_name("???", 7), "track7");
}

#[test]
fn probe_accepts_the_three_rates_and_says_why_it_rejects() {
    let d = tmp("probe");
    for sr in RATES {
        wav(&d.join(format!("ok{sr}.wav")), sr, 1000);
        let p = probe(&d.join(format!("ok{sr}.wav"))).unwrap();
        assert_eq!((p.sr, p.n_frames, p.channels), (sr, 1000, 1));
        assert!((p.duration() - 1000.0 / f64::from(sr)).abs() < 1e-12);
        assert_eq!(p.stem, format!("ok{sr}"));
    }
    wav(&d.join("rate.wav"), 88_200, 1000);
    let r = probe(&d.join("rate.wav")).unwrap_err();
    assert!(
        r.reason.contains("88200 Hz") && r.reason.contains("44.1, 48 or 96 kHz"),
        "{}",
        r.reason
    );
    assert_eq!(r.name, "rate.wav");
    aiff(&d.join("a.aif"));
    assert!(
        probe(&d.join("a.aif"))
            .unwrap_err()
            .reason
            .starts_with("AIFF files are not supported")
    );
    aiff(&d.join("liar.wav")); // the first bytes decide, not the extension
    assert!(probe(&d.join("liar.wav")).unwrap_err().reason.starts_with("AIFF"));
    std::fs::write(d.join("a.flac"), b"fLaC....").unwrap();
    assert!(probe(&d.join("a.flac")).unwrap_err().reason.starts_with("FLAC files"));
    std::fs::write(d.join("notes.txt"), b"hello").unwrap();
    assert!(
        probe(&d.join("notes.txt"))
            .unwrap_err()
            .reason
            .contains("no audio file")
    );
    std::fs::write(d.join("junk.wav"), b"this is not a wave file at all").unwrap();
    assert!(probe(&d.join("junk.wav")).unwrap_err().reason.contains("not supported"));
    std::fs::write(d.join("empty.wav"), b"").unwrap();
    assert!(probe(&d.join("empty.wav")).is_err());
    wav(&d.join("none.wav"), 48_000, 0);
    assert_eq!(
        probe(&d.join("none.wav")).unwrap_err().reason,
        "the file holds no audio samples"
    );
    assert_eq!(
        probe(&d.join("missing.wav")).unwrap_err().reason,
        "the file does not exist"
    );
    assert_eq!(probe(&d).unwrap_err().reason, "this is not a file");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn expand_skips_unfinished_save_as_folders() {
    let d = tmp("expand_partial");
    wav(&d.join("Moved.partial/Audio Files/k in_01.wav"), 48_000, 100);
    wav(&d.join("Moved.partial/k.wav"), 48_000, 100);
    wav(&d.join("drop/a.wav"), 48_000, 100);
    let p = |s: &str| d.join(s).display().to_string();
    wav(&d.join("Moved.partial/Audio Files/k.wav"), 48_000, 100);
    assert!(expand(&[p("Moved.partial")]).is_empty(), "the folder");
    assert!(expand(&[p("Moved.partial/Audio Files")]).is_empty(), "its audio folder");
    assert!(
        expand(&[p("Moved.partial/Audio Files/k.wav")]).is_empty(),
        "a file in the audio folder"
    );
    assert!(expand(&[p("Moved.partial/k.wav")]).is_empty(), "a file in it");
    assert_eq!(expand(&[p("drop")]).len(), 1, "other folders are read");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn expand_lists_a_folder_a_session_folder_and_files_once() {
    let d = tmp("expand");
    for n in ["b.wav", "A.wav", "c.WAV", ".hidden.wav", "x.ptx"] {
        wav(&d.join("drop").join(n), 48_000, 100);
    }
    aiff(&d.join("drop/z.aiff"));
    let names = |v: Vec<PathBuf>| v.iter().map(|p| file_name(p)).collect::<Vec<_>>();
    let drop = d.join("drop").display().to_string();
    assert_eq!(
        names(expand(std::slice::from_ref(&drop))),
        ["A.wav", "b.wav", "c.WAV", "z.aiff"]
    );
    // a session folder: no audio in it, an Audio Files folder
    wav(&d.join("sess/Audio Files/k in_01.wav"), 48_000, 100);
    wav(&d.join("sess/Audio Files/s top_01.wav"), 48_000, 100);
    std::fs::write(d.join("sess/s.ptx"), b"x").unwrap();
    let sess = d.join("sess").display().to_string();
    assert_eq!(names(expand(&[sess])), ["k in_01.wav", "s top_01.wav"]);
    // a file twice, and a folder with its file
    let one = d.join("drop/b.wav").display().to_string();
    assert_eq!(names(expand(&[one.clone(), one, drop])).len(), 4);
    assert_eq!(
        expand(&["/no/such/file.wav".into()]).len(),
        1,
        "a missing file is rejected later"
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn tracks_scan_guesses_roles_and_lists_rejected_files() {
    let d = tmp("scan");
    let names = ["k in_02", "s top_02", "t1 HH_01", "hh_03", "oh_01", "Clap_01"];
    for n in names {
        wav(&d.join(format!("drop/{n}.wav")), 48_000, 4800);
    }
    wav(&d.join("drop/rate.wav"), 88_200, 100);
    aiff(&d.join("drop/old.aif"));
    let r = service()
        .tracks_scan(&[d.join("drop").display().to_string()], None)
        .unwrap();
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 6);
    let by = |n: &str| files.iter().find(|f| f["stem"] == n).unwrap();
    assert_eq!(
        (by("k in_02")["role"].clone(), by("k in_02")["note"].clone()),
        (json!("kick"), json!(36))
    );
    assert_eq!(by("s top_02")["role"], "snare");
    assert_eq!(by("t1 HH_01")["role"], "tom");
    assert_eq!(by("hh_03")["role"], "hat");
    assert_eq!(by("oh_01")["role"], "cymbal");
    assert_eq!(
        (by("Clap_01")["role"].clone(), by("Clap_01")["guess"].clone()),
        (json!("other"), json!("none"))
    );
    assert_eq!(by("k in_02")["track"], "k_in_02");
    assert_eq!(by("k in_02")["n_samples"], 4800);
    assert_eq!(r["guessed"], 5);
    let rej = r["rejected"].as_array().unwrap();
    assert_eq!(rej.len(), 2);
    assert!(
        rej.iter()
            .any(|x| x["name"] == "old.aif" && x["reason"].as_str().unwrap().contains("AIFF"))
    );
    assert!(
        rej.iter()
            .any(|x| x["name"] == "rate.wav" && x["reason"].as_str().unwrap().contains("88200"))
    );
    assert_eq!(service().tracks_scan(&[], None).unwrap()["files"], json!([]));
    let s = service();
    s.set_home(d.join("home"));
    assert_eq!(s.tracks_scan(&[], Some("nope")).unwrap_err().code, 404);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn tracks_scan_applies_the_rules_of_a_preset() {
    let d = tmp("scan_preset");
    wav(&d.join("drop/Clap_01.wav"), 48_000, 480);
    wav(&d.join("drop/k in_02.wav"), 48_000, 480);
    let s = service();
    s.set_home(d.join("home"));
    s.kit_preset_save("Mine", &json!([{"pattern": "clap", "role": "cymbal", "note": 39}]))
        .unwrap();
    let r = s
        .tracks_scan(&[d.join("drop").display().to_string()], Some("Mine"))
        .unwrap();
    let f = r["files"].as_array().unwrap();
    assert_eq!(
        (f[0]["role"].clone(), f[0]["note"].clone(), f[0]["guess"].clone()),
        (json!("cymbal"), json!(39), json!("preset"))
    );
    assert_eq!(f[1]["guess"], "scheme");
    assert_eq!(s.tracks_scan(&[], Some("nope")).unwrap_err().code, 404);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn dispatch_checks_arguments_and_knows_its_commands() {
    let s = service();
    s.set_home(tmp("dispatch_home"));
    let ch = Arc::new(Channels::default());
    assert!(dispatch(&s, &ch, "nope", &json!({})).is_none());
    for c in TRACKS_COMMANDS {
        assert!(dispatch(&s, &ch, c, &json!({"paths": 3})).is_some(), "{c}");
    }
    assert_eq!(
        dispatch(&s, &ch, "tracks_scan", &json!({"paths": 3}))
            .unwrap()
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        dispatch(&s, &ch, "tracks_scan", &json!({"paths": [1]}))
            .unwrap()
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        dispatch(&s, &ch, "tracks_add", &json!({})).unwrap().unwrap_err().code,
        400
    );
    assert_eq!(
        dispatch(&s, &ch, "tracks_add", &json!({"sid": "x"}))
            .unwrap()
            .unwrap_err()
            .code,
        404
    );
    assert_eq!(
        dispatch(&s, &ch, "kit_preset_save", &json!({"name": "a"}))
            .unwrap()
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        dispatch(&s, &ch, "kit_preset_load", &json!({}))
            .unwrap()
            .unwrap_err()
            .code,
        400
    );
    let r = dispatch(&s, &ch, "tracks_scan", &json!({})).unwrap().unwrap();
    assert_eq!(r["files"], json!([]));
}
