use super::*;

/// A fresh temporary folder per test.
pub(crate) fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("drumspike-core-project-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn normpath_and_relpath_follow_os_path() {
    assert_eq!(normpath(Path::new("/a/b/../c/./d//")), PathBuf::from("/a/c/d"));
    assert_eq!(normpath(Path::new("/../x")), PathBuf::from("/x"));
    assert_eq!(normpath(Path::new("a/../../b")), PathBuf::from("../b"));
    assert_eq!(normpath(Path::new("a/..")), PathBuf::from("."));
    assert_eq!(
        relpath(Path::new("/x/Kick In.wav"), Path::new("/x/P")),
        "../Kick In.wav"
    );
    assert_eq!(
        relpath(Path::new("/x/P/Audio Files/K.wav"), Path::new("/x/P")),
        "Audio Files/K.wav"
    );
    assert_eq!(relpath(Path::new("/x/P"), Path::new("/x/P")), ".");
    assert_eq!(
        relpath(
            Path::new("/Volumes/S/Song03/Audio Files/K.wav"),
            Path::new("/Users/me/Projects/Song")
        ),
        "../../../../Volumes/S/Song03/Audio Files/K.wav"
    );
    assert!(abspath(Path::new("rel")).is_absolute());
    assert_eq!(
        document_path(Path::new("/x/Drum Song 03")),
        PathBuf::from("/x/Drum Song 03/Drum Song 03.drumspike")
    );
    assert_eq!(
        document_path(Path::new("/x/Drum Song 03/")),
        PathBuf::from("/x/Drum Song 03/Drum Song 03.drumspike")
    );
}

#[test]
fn names_are_checked_like_python() {
    for bad in ["", "  ", ".", "..", "a/b", "a\\b", "Song: Take 2", "a\0b"] {
        let e = check_name(bad).unwrap_err().to_string();
        assert!(e.starts_with("bad project name: '"), "{e}");
    }
    assert_eq!(check_name("  Drum Song 03 ").unwrap(), "Drum Song 03");
    for bad in ["", " ", ".", "..", "a/b", "a\\b", "../../x", "a\0b"] {
        assert!(
            check_track_name(bad)
                .unwrap_err()
                .to_string()
                .starts_with("bad track name: '")
        );
    }
    assert_eq!(check_track_name("Kick In").unwrap(), "Kick In");
    assert_eq!(check_track_name("Song: Take 2").unwrap(), "Song: Take 2"); // a colon is fine in a track name
    assert_eq!(py_repr("a'b\\c\0"), "'a\\'b\\\\c\\x00'");
}

#[test]
fn ensure_layout_creates_the_subfolders_and_keeps_their_contents() {
    let d = tmp("layout");
    ensure_layout(&d).unwrap();
    for s in SUBDIRS {
        assert!(d.join(s).is_dir(), "{s}");
    }
    std::fs::write(d.join(AUDIO_DIR).join("a.wav"), b"x").unwrap();
    ensure_layout(&d).unwrap(); // idempotent: an existing folder and its files stay
    assert_eq!(std::fs::read(d.join(AUDIO_DIR).join("a.wav")).unwrap(), b"x");
    std::fs::write(d.join("file"), b"").unwrap();
    let e = ensure_layout(&d.join("file")).unwrap_err();
    assert!(
        matches!(&e, ProjectError::Io(what, _) if what.starts_with("mkdir ")),
        "{e}"
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn new_project_creates_layout_and_document() {
    let d = tmp("new");
    let p = new_project(&d, "Drum Song 03").unwrap();
    assert_eq!(p.dir, d.join("Drum Song 03"));
    assert_eq!(p.path(), d.join("Drum Song 03").join("Drum Song 03.drumspike"));
    for s in SUBDIRS {
        assert!(p.dir.join(s).is_dir());
    }
    let q = load_project(&p.dir).unwrap();
    assert_eq!(q.doc.format, 1);
    assert_eq!(q.doc.name.as_deref(), Some("Drum Song 03"));
    assert_eq!(q.doc.tracks().count(), 0);
    assert_eq!(q.doc.default_mix_mode(), "average");
    assert!(q.doc.modified.unwrap().unwrap() > 1.7e9);
    assert_eq!(q.doc_path, Some(p.path()));
    assert_eq!(load_project(&p.path()).unwrap().doc, q.doc);
    let e = new_project(&d, "Drum Song 03").unwrap_err().to_string();
    assert!(e.starts_with("project already exists: "), "{e}");
    assert!(
        new_project(&d, "a/b")
            .unwrap_err()
            .to_string()
            .starts_with("bad project name")
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn add_track_defaults_and_validation() {
    let d = tmp("add");
    let mut p = new_project(&d, "P").unwrap();
    let tr = add_track(&mut p, "K", None, "kick", 36, None).unwrap().clone();
    assert_eq!(
        serde_json::to_string(&tr).unwrap(),
        r#"{"role":"kick","note":36,"mix_mode":"average","source":null,"events":[]}"#
    );
    assert_eq!(p.doc.order, Some(vec!["K".to_string()]));
    assert_eq!(
        add_track(&mut p, "K", None, "kick", 36, None).unwrap_err().to_string(),
        "track exists: K"
    );
    assert_eq!(
        add_track(&mut p, "S", None, "snare", 38, Some("sum"))
            .unwrap_err()
            .to_string(),
        "bad mix_mode: 'sum'"
    );
    add_track(&mut p, "S", None, "snare", 38, Some("ffmpeg")).unwrap();
    assert_eq!(p.doc.track("S").unwrap().mix_mode.as_deref(), Some("ffmpeg"));
    assert_eq!(p.doc.track_order(), ["K", "S"]);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn migrate_forward_from_format_0_values() {
    let doc = migrate_forward(serde_json::json!({
        "tracks": {"A": {"role": "kick"}}, "events": {"A": [{"t": 0.5, "v": 9}]}, "order": ["ghost", "A"]
    }))
    .unwrap();
    assert_eq!(doc.name.as_deref(), Some("Imported project"));
    assert_eq!(doc.order, Some(vec!["A".to_string()]));
    assert_eq!(doc.format, 1);
    assert_eq!(doc.modified, Some(None));
    assert_eq!(doc.settings.unwrap().mix_mode.as_deref(), Some("ffmpeg"));
    let a = doc.tracks.as_ref().unwrap().get("A").unwrap();
    assert_eq!(a.events(), &[Event::new(0.5, 9)]);
    assert_eq!(a.mix_mode.as_deref(), Some("ffmpeg"));
    assert_eq!(a.source, Some(None));
    // {"events": [...]} and a track without events; a stored order that lists every track
    let doc = migrate_forward(serde_json::json!({
        "format": 0, "name": "Song: Take 2", "created": 1_759_950_000.5, "order": ["Empty", "Kick_In"],
        "tracks": {"Kick_In": {"role": "kick", "note": 36, "params": {"min_gap_ms": 20}, "color": "#e05555"},
                   "Empty": {"role": "other", "note": 36}},
        "events": {"Kick_In": {"sr": 48000, "events": [{"t": 1.0 / 3.0, "v": 20, "added": true}]}}
    }))
    .unwrap();
    assert_eq!(doc.name.as_deref(), Some("Song: Take 2"));
    assert_eq!(doc.created.as_ref().unwrap().as_f64(), Some(1_759_950_000.5));
    assert_eq!(doc.order, Some(vec!["Empty".to_string(), "Kick_In".to_string()]));
    let k = doc.track("Kick_In").unwrap();
    assert_eq!(k.events()[0].t, 1.0 / 3.0);
    assert_eq!(k.events()[0].added, Some(true));
    assert_eq!(k.params.as_ref().unwrap()["min_gap_ms"], 20);
    let e = doc.track("Empty").unwrap();
    assert_eq!(
        serde_json::to_string(e).unwrap(),
        r#"{"role":"other","note":36,"mix_mode":"ffmpeg","source":null,"events":[]}"#
    );
    // a format-1 document passes through unchanged
    let one = migrate_forward(serde_json::json!({"format": 1, "name": "x", "tracks": {}})).unwrap();
    assert_eq!(one.to_json(), r#"{"format":1,"name":"x","tracks":{}}"#);
}

#[test]
fn migrate_forward_rejects_bad_documents() {
    let err = |v: Value| migrate_forward(v).unwrap_err();
    assert_eq!(
        err(serde_json::json!({"tracks": []})).to_string(),
        "format 0: tracks must be an object"
    );
    assert_eq!(
        err(serde_json::json!({"format": 1, "tracks": []})).to_string(),
        "tracks must be an object"
    );
    assert_eq!(
        err(serde_json::json!({"format": "one"})).to_string(),
        "bad format field: 'one'"
    );
    assert_eq!(
        err(serde_json::json!({"format": true, "tracks": {}})).to_string(),
        "bad format field: True"
    );
    assert_eq!(
        err(serde_json::json!({"format": 1.0, "tracks": {}})).to_string(),
        "bad format field: 1.0"
    );
    assert_eq!(
        err(serde_json::json!({"format": -1, "tracks": {}})).to_string(),
        "bad format field: -1"
    );
    // integers beyond 64 bits: the literal in the text decides, as Python's json reads
    // them as int and serde_json as float (review round 3, finding m3)
    let from_text = |t: &str| migrate_forward_from_text(serde_json::from_str(t).unwrap(), t).unwrap_err();
    let big = from_text(r#"{"tracks": {}, "format": 100000000000000000000000}"#);
    assert!(big.is_format());
    assert_eq!(
        big.to_string(),
        "project format 100000000000000000000000 is newer than this app (format 1); update the app to open it"
    );
    let huge_neg = from_text(r#"{"format": -100000000000000000000000}"#);
    assert!(!huge_neg.is_format());
    assert_eq!(huge_neg.to_string(), "bad format field: -100000000000000000000000");
    assert_eq!(from_text(r#"{"format": 1e23}"#).to_string(), "bad format field: 1e+23");
    assert_eq!(
        from_text(r#"{"format": -1e23}"#).to_string(),
        "bad format field: -1e+23"
    );
    let u = from_text(r#"{"format": 18446744073709551615}"#);
    assert!(
        u.is_format()
            && u.to_string()
                .starts_with("project format 18446744073709551615 is newer")
    );
    assert!(migrate_forward_from_text(serde_json::json!({"format": 1, "tracks": {}}), "{}").is_ok());
    let newer = err(serde_json::json!({"format": 2, "tracks": {}}));
    assert!(newer.is_format());
    assert_eq!(
        newer.to_string(),
        "project format 2 is newer than this app (format 1); update the app to open it"
    );
    assert!(err(serde_json::json!({"format": 1u64 << 40, "tracks": {}})).is_format());
    for bad in ["", " ", ".", "..", "a/b", "a\\b", "../../x", "a\0b"] {
        let e = err(serde_json::json!({"format": 1, "tracks": {bad: {"role": "kick"}}}));
        assert!(e.to_string().starts_with("bad track name"), "{e}");
        assert!(
            err(serde_json::json!({"tracks": {bad: {"role": "kick"}}}))
                .to_string()
                .starts_with("bad track name")
        );
    }
    assert_eq!(
        err(serde_json::json!({"tracks": {"A": 3}})).to_string(),
        "format 0: track A must be an object"
    );
    assert_eq!(
        err(serde_json::json!({"tracks": {"A": {}}, "events": {"A": 3}})).to_string(),
        "format 0: events of A must be a list"
    );
    assert_eq!(
        err(serde_json::json!({"tracks": {}, "events": [1]})).to_string(),
        "format 0: events must be an object"
    );
    assert!(migrate_forward(serde_json::json!({"tracks": {}, "events": []})).is_ok()); // falsy, like `or {}`
    // Python accepts any value in a track; the Rust layer keeps what it cannot type
    let odd = migrate_forward(serde_json::json!({"format": 1, "tracks": {"A": {"note": "x"}, "B": 3}})).unwrap();
    assert_eq!(odd.extra["tracks"], serde_json::json!({"A": {"note": "x"}, "B": 3}));
    assert!(
        err(serde_json::json!({"format": 1, "tracks": {"A": {}, "a/b": 3}}))
            .to_string()
            .starts_with("bad track name")
    );
}

#[test]
fn load_project_errors_and_find_document() {
    let d = tmp("load");
    let e = load_project(&d).unwrap_err().to_string();
    assert_eq!(e, format!("no .drumspike document at {}", d.display()));
    assert!(
        load_project(&d.join("nowhere"))
            .unwrap_err()
            .to_string()
            .starts_with("no .drumspike document at ")
    );
    let a = d.join("a.drumspike");
    std::fs::write(&a, "{nope").unwrap();
    assert!(
        load_project(&a)
            .unwrap_err()
            .to_string()
            .contains("is not valid JSON: ")
    );
    // bytes that are not UTF-8 and a byte order mark: "not valid JSON", not an I/O error
    for bad in [&b"{\"name\": \"\xff\"}"[..], &b"\xef\xbb\xbf{}"[..]] {
        std::fs::write(&a, bad).unwrap();
        let e = load_project(&a).unwrap_err();
        assert!(
            matches!(&e, ProjectError::Invalid(m) if m.starts_with(&format!("{} is not valid JSON: ", a.display()))),
            "{e}"
        );
    }
    std::fs::write(&a, "[]").unwrap();
    assert_eq!(
        load_project(&a).unwrap_err().to_string(),
        format!("{} is not a project document", a.display())
    );
    std::fs::write(d.join("b.drumspike"), "{}").unwrap();
    assert!(
        load_project(&d)
            .unwrap_err()
            .to_string()
            .starts_with("no .drumspike document")
    ); // two candidates
    std::fs::remove_file(&a).unwrap();
    assert_eq!(find_document(&d).unwrap(), d.join("b.drumspike")); // the only one
    let p = load_project(&d).unwrap(); // {} is a format-0 document without tracks
    assert_eq!(p.doc.format, 1);
    assert_eq!(p.dir, d);
    assert_eq!(p.name(), "Imported project"); // a format-0 document without a name
    let mut unnamed = p.clone();
    unnamed.doc.name = None;
    assert_eq!(unnamed.name(), basename(&d));
    // a newer format is rejected and never rewritten
    std::fs::write(d.join("b.drumspike"), r#"{"format": 99, "name": "New", "tracks": {}}"#).unwrap();
    let e = load_project(&d).unwrap_err();
    assert!(e.is_format() && e.to_string().contains("newer than this app"));
    assert!(std::fs::read_to_string(d.join("b.drumspike")).unwrap().contains("99"));
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn load_project_migrates_a_format_0_document_file() {
    let d = tmp("old");
    let pdir = d.join("Old");
    std::fs::create_dir_all(&pdir).unwrap();
    std::fs::write(
        pdir.join("Old.drumspike"),
        r#"{"name": "Old", "tracks": {"K": {"role": "kick", "note": 36}}, "events": {"K": {"events": [{"t": 0.3333333333333333, "v": 5}]}}}"#,
    )
    .unwrap();
    let mut p = load_project(&pdir).unwrap();
    assert_eq!(p.doc.format, 1);
    assert_eq!(p.doc.track("K").unwrap().events()[0].t, 1.0 / 3.0);
    assert_eq!(p.doc.track("K").unwrap().mix_mode.as_deref(), Some("ffmpeg"));
    save_project(&mut p).unwrap();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(pdir.join("Old.drumspike")).unwrap()).unwrap();
    assert_eq!(v["format"], 1);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn legacy_session_document_reads_meta_events_and_sources() {
    let d = tmp("legacy");
    let sdir = d.join("sessions").join("ab12cd34");
    let td = sdir.join("tracks").join("Kick_In");
    std::fs::create_dir_all(&td).unwrap();
    std::fs::create_dir_all(sdir.join("tracks").join("Empty")).unwrap();
    std::fs::write(td.join("source.wav"), b"RIFF\0\0\0\0WAVEjunk").unwrap();
    std::fs::write(
        td.join("events.json"),
        r#"{"sr": 48000, "events": [{"t": 2.675, "v": 127, "orig": 1e-07}]}"#,
    )
    .unwrap();
    let meta = r#"{"tracks": {"Kick_In": {"role": "kick", "note": 36}, "Empty": {"role": "other", "note": 36}},
                   "name": "Song: Take 2", "created": 1759950000.5, "order": ["Empty", "Kick_In"]}"#;
    std::fs::write(sdir.join("meta.json"), meta).unwrap();
    let doc0 = legacy_session_document(&sdir).unwrap();
    assert_eq!(doc0["format"], 0);
    assert_eq!(doc0["events"]["Kick_In"]["events"][0]["t"], 2.675);
    assert_eq!(doc0["tracks"]["Kick_In"]["source"]["mode"], "referenced");
    assert_eq!(
        doc0["tracks"]["Kick_In"]["source"]["path"],
        Value::from(td.join("source.wav").to_str().unwrap())
    );
    assert_eq!(
        doc0["tracks"]["Kick_In"]["source"]["relative"],
        "tracks/Kick_In/source.wav"
    );
    assert!(doc0["tracks"]["Empty"].get("source").is_none());
    let doc = migrate_forward(doc0).unwrap();
    assert_eq!(doc.name.as_deref(), Some("Song: Take 2"));
    assert_eq!(doc.order, Some(vec!["Empty".to_string(), "Kick_In".to_string()]));
    let k = doc.track("Kick_In").unwrap();
    assert_eq!(k.events()[0].orig, Some(1e-7));
    assert_eq!(k.source_ref().unwrap().mode.as_deref(), Some("referenced"));
    assert_eq!(doc.track("Empty").unwrap().source, Some(None));
    // the session is untouched
    assert_eq!(std::fs::read_to_string(sdir.join("meta.json")).unwrap(), meta);
    // errors
    assert_eq!(
        legacy_session_document(&d).unwrap_err().to_string(),
        format!("no meta.json in {}", d.display())
    );
    std::fs::write(sdir.join("meta.json"), r#"{"tracks": {"../x": {}}}"#).unwrap();
    assert!(
        legacy_session_document(&sdir)
            .unwrap_err()
            .to_string()
            .starts_with("bad track name")
    );
    std::fs::write(sdir.join("meta.json"), r#"{"tracks": []}"#).unwrap();
    assert_eq!(
        legacy_session_document(&sdir).unwrap_err().to_string(),
        "format 0: tracks must be an object"
    );
    std::fs::write(sdir.join("meta.json"), "[1]").unwrap();
    assert!(
        legacy_session_document(&sdir)
            .unwrap_err()
            .to_string()
            .ends_with("is not a session document")
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn project_name_falls_back_to_the_folder() {
    let mut p = Project::new(Path::new("/x/Folder"), empty_document("", "average"), None);
    assert_eq!(p.name(), "Folder");
    p.doc.name = None;
    assert_eq!(p.name(), "Folder");
    p.doc.name = Some("Other".into());
    assert_eq!(p.name(), "Other");
    assert_eq!(p.path(), PathBuf::from("/x/Folder/Folder.drumspike"));
    assert_eq!(p.subdir(BACKUPS_DIR), PathBuf::from("/x/Folder/Backups"));
    let q = Project::new(
        Path::new("/x/Folder"),
        empty_document("Other", "average"),
        Some(Path::new("/x/Folder/Other.drumspike")),
    );
    assert_eq!(q.path(), PathBuf::from("/x/Folder/Other.drumspike"));
}
