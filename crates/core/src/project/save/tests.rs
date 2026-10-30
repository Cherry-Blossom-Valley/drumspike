use super::super::tests::tmp;
use super::*;
use crate::io::{WavFormat, WavSpec, write_wav};
use crate::project::{AUDIO_DIR, Event, add_track, load_project, new_project};
use serde_json::Value;

#[test]
fn save_project_into_writes_the_document_under_its_own_name_into_another_folder() {
    let root = tmp("into");
    let mut p = new_project(&root, "Final").unwrap();
    let other = root.join("Final.partial");
    let path = save_project_into(&mut p, &other).unwrap();
    assert_eq!(path, other.join("Final.drumspike"));
    let doc = std::fs::read_to_string(&path).unwrap();
    assert!(doc.contains("\"name\": \"Final\""), "{doc}");
    assert!(
        !p.dir.join("Backups").read_dir().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".partial"))
    );
    assert!(
        other.join("Audio Files").is_dir(),
        "the layout is made in the other folder"
    );
}

fn kick(dir: &Path, name: &str, seed: f32) -> PathBuf {
    let p = dir.join(name);
    let spec = WavSpec {
        sample_rate: 48_000,
        channels: 1,
        format: WavFormat::Pcm16,
        extensible: false,
        channel_mask: 0,
    };
    let x: Vec<f32> = (0..4000).map(|i| ((i as f32) * 0.01 * seed).sin() * 0.5).collect();
    write_wav(&p, &spec, &x, None).unwrap();
    p
}

fn events() -> Vec<Event> {
    let mut e2 = Event::new(2.675, 127);
    e2.orig = Some(1e-7);
    e2.edited = Some(true);
    let mut e3 = Event::new(123456.123456789, 64);
    e3.rise = Some(12.3);
    let mut e1 = Event::new(1.0 / 3.0, 20);
    e1.added = Some(true);
    vec![Event::new(0.1 + 0.2, 100), e1, e2, e3]
}

fn backups(bdir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(bdir)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.file_name().to_string_lossy().ends_with(DOC_EXT))
        .map(|e| (e.metadata().unwrap().modified().unwrap(), e.path()))
        .collect();
    v.sort();
    v.into_iter().map(|(_, p)| p).collect()
}

fn view_i(path: &Path) -> i64 {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["view"]["i"].as_i64().unwrap()
}

#[test]
fn save_load_round_trip_is_bit_exact() {
    let d = tmp("roundtrip");
    let daw = d.join("daw");
    std::fs::create_dir_all(&daw).unwrap();
    let k = kick(&daw, "Kick In.wav", 1.0);
    let s = kick(&daw, "Snare Top.wav", 2.0);
    let mut p = new_project(&d.join("Songs"), "Song 1").unwrap();
    let krec = build_source(&k, &p.dir, "referenced").unwrap();
    let tr = add_track(&mut p, "Kick In", Some(krec), "kick", 36, None).unwrap();
    tr.sr = Some(48_000);
    tr.n_samples = Some(72_000);
    tr.duration = Some(1.5);
    tr.params = Some(serde_json::from_str(r#"{"min_gap_ms": 20}"#).unwrap());
    tr.events = Some(events());
    let srec = copy_into_project(&build_source(&s, &p.dir, "referenced").unwrap(), &p.dir, None).unwrap();
    let tr = add_track(&mut p, "Snare Top", Some(srec), "snare", 38, Some("ffmpeg")).unwrap();
    tr.events = Some(vec![Event::new(0.5, 77)]);
    p.doc.view = Some(serde_json::from_str(r#"{"t0": 0.0, "t1": 1.5, "playhead": 0.3333333333333333}"#).unwrap());
    let path = save_project(&mut p).unwrap();
    assert_eq!(path, p.dir.join("Song 1.drumspike"));
    assert!(!p.dir.join("Song 1.drumspike.tmp").exists());
    let q = load_project(&p.dir).unwrap();
    assert_eq!(q.doc, p.doc);
    let got = q.doc.track("Kick In").unwrap().events();
    let want = events();
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(&want) {
        assert_eq!(g.t.to_bits(), w.t.to_bits());
        assert_eq!(g, w);
    }
    assert_eq!(got[1].added, Some(true));
    assert_eq!(got[2].edited, Some(true));
    assert_eq!(got[2].orig.map(f64::to_bits), Some(1e-7f64.to_bits()));
    let kt = q.doc.track("Kick In").unwrap();
    assert_eq!(
        (kt.role.as_deref(), kt.note, kt.mix_mode.as_deref()),
        (Some("kick"), Some(36), Some("average"))
    );
    let st = q.doc.track("Snare Top").unwrap();
    assert_eq!(
        (st.role.as_deref(), st.note, st.mix_mode.as_deref()),
        (Some("snare"), Some(38), Some("ffmpeg"))
    );
    assert_eq!(
        st.source_ref().unwrap().relative.as_deref(),
        Some("Audio Files/Snare Top.wav")
    );
    assert_eq!(q.doc.order, Some(vec!["Kick In".to_string(), "Snare Top".to_string()]));
    assert_eq!(q.doc.view.as_ref().unwrap()["playhead"], 1.0 / 3.0);
    assert_eq!(load_project(&p.path()).unwrap().doc, q.doc); // the document path works like the folder
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn save_is_atomic_and_rotates_backups_to_ten() {
    let d = tmp("rotate");
    let mut p = new_project(&d, "Rot").unwrap();
    let bdir = p.subdir(BACKUPS_DIR);
    assert!(backups(&bdir).is_empty()); // the first write has nothing to back up
    for i in 0..13 {
        p.doc.view = Some(serde_json::from_value(serde_json::json!({"i": i})).unwrap());
        save_project(&mut p).unwrap();
        assert!(!p.dir.join("Rot.drumspike.tmp").exists());
        assert_eq!(view_i(&p.path()), i);
        // keep the mtimes apart so the rotation order is the save order
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    let b = backups(&bdir);
    assert_eq!(b.len(), 10);
    assert!(
        b.iter()
            .all(|f| f.file_name().unwrap().to_string_lossy().starts_with("Rot 20"))
    );
    assert!(!bdir.join("x.tmp").exists());
    assert_eq!(view_i(b.last().unwrap()), 11); // newest backup = previous document
    assert_eq!(view_i(&b[0]), 2); // the oldest three were removed
    assert_eq!(load_project(&p.dir).unwrap().doc.view.unwrap()["i"], 12);
    assert!(rotate_backups(&d.join("nope"), 10).unwrap().is_empty());
    // keep = 3 removes seven more
    assert_eq!(rotate_backups(&p.dir, 3).unwrap().len(), 7);
    assert_eq!(backups(&bdir).len(), 3);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn backup_falls_back_to_copy_when_link_fails() {
    // test_project.py::test_backup_falls_back_to_copy_when_link_fails
    let d = tmp("nolink");
    let mut p = new_project(&d, "L").unwrap();
    let bdir = p.subdir(BACKUPS_DIR);
    let prev = std::fs::read(p.path()).unwrap();
    let prev_mtime = std::fs::metadata(p.path()).unwrap().modified().unwrap();
    let no_links = |_: &Path, _: &Path| Err(std::io::Error::other("no links"));
    save_project_with(&mut p, no_links).unwrap();
    let b = backups(&bdir);
    assert_eq!(b.len(), 1);
    // a copy, not a link: the previous document's bytes and modification time
    assert_eq!(std::fs::read(&b[0]).unwrap(), prev);
    assert_eq!(std::fs::metadata(&b[0]).unwrap().modified().unwrap(), prev_mtime);
    assert_ne!(std::fs::read(p.path()).unwrap(), prev);
    assert!(b[0].file_name().unwrap().to_string_lossy().starts_with("L 20"));
    // the default save links: the backup shares the file with the old document
    let before = std::fs::read(p.path()).unwrap();
    save_project(&mut p).unwrap();
    let b = backups(&bdir);
    assert_eq!(b.len(), 2);
    assert!(b.iter().any(|f| std::fs::read(f).unwrap() == before));
    std::fs::remove_dir_all(&d).unwrap();
}

/// Every file under `dir`, recursively, relative to it.
fn tree(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                todo.push(p);
            } else {
                out.push(p.strip_prefix(dir).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_failed_backup_rotation_after_the_commit_is_a_warning_not_a_failed_save() {
    // R12: the rotation runs after the rename, so its failure must not undo the commit.
    let d = tmp("rot-fail");
    let mut p = new_project(&d, "Rot").unwrap();
    let bdir = p.subdir(BACKUPS_DIR);
    // The injection: a `.drumspike` directory older than every file. The rotation
    // counts it as a document candidate and `remove_file` on a directory fails on
    // every platform.
    let stuck = bdir.join("aaa-stuck.drumspike");
    std::fs::create_dir(&stuck).unwrap();
    let old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    std::fs::File::open(&stuck)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old))
        .unwrap();
    // Ten real backup files, all newer than the stuck entry: with the pre-save backup
    // of this save the folder holds 12 candidates, so the rotation must try to remove
    // the two oldest — the stuck directory first.
    for i in 0..BACKUPS_KEEP {
        std::fs::write(bdir.join(format!("old {i} 2000-01-01 00-00-00{DOC_EXT}")), b"{}").unwrap();
    }
    p.doc
        .settings
        .as_mut()
        .unwrap()
        .extra
        .insert("probe".into(), Value::from(42));
    let report = save_project_with(&mut p, |a, b| std::fs::hard_link(a, b)).unwrap();
    let warn = report.warning.expect("the rotation failure comes back as a warning");
    assert!(warn.to_string().contains("aaa-stuck.drumspike"), "{warn}");
    assert!(report.path.is_file());
    let on_disk = std::fs::read_to_string(&report.path).unwrap();
    assert!(on_disk.contains("\"probe\": 42"), "{on_disk}");
    // The strict `save_project` keeps the old contract: the same failure is an error.
    let e = save_project(&mut p).unwrap_err();
    assert!(e.to_string().contains("aaa-stuck.drumspike"), "{e}");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_document_name_with_separators_keeps_the_backup_inside_backups() {
    // finding G1: the name '../../x' wrote the backup two folders above Backups/
    let d = tmp("escape");
    let mut p = new_project(&d, "Esc").unwrap();
    let bdir = p.subdir(BACKUPS_DIR);
    p.doc.name = Some("../../x".into());
    let before = std::fs::read(p.path()).unwrap();
    save_project(&mut p).unwrap();
    let b = backups(&bdir);
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].parent().unwrap(), bdir);
    assert!(b[0].file_name().unwrap().to_string_lossy().starts_with("..-..-x 20"));
    assert_eq!(std::fs::read(&b[0]).unwrap(), before);
    // nothing outside the project folder, nothing in it but the document and the backup
    assert_eq!(tree(&d).len(), 2, "{:?}", tree(&d));
    assert!(tree(&d).iter().all(|f| f.starts_with("Esc/")), "{:?}", tree(&d));
    for (name, want) in [("a\\b:c\0d", "a-b-c-d 20"), ("/", "- 20")] {
        assert!(backup_name(name, None).unwrap().starts_with(want), "{name:?}");
    }
    // '.' and '..' are refused before anything is written
    for bad in [".", ".."] {
        p.doc.name = Some(bad.into());
        let doc = std::fs::read(p.path()).unwrap();
        let e = save_project(&mut p).unwrap_err();
        assert_eq!(e.to_string(), format!("bad project name: '{bad}'"));
        assert_eq!(std::fs::read(p.path()).unwrap(), doc);
        assert!(backup_name(bad, Some(0.0)).is_err());
    }
    assert_eq!(tree(&d).len(), 2);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn backup_names_and_free_names() {
    let n = backup_name("Rot", Some(0.0)).unwrap();
    assert!(n.starts_with("Rot 19") || n.starts_with("Rot 1970"), "{n}");
    assert!(n.ends_with(DOC_EXT));
    // the stamp has the shape YYYY-MM-DD HH-MM-SS
    let stamp = &n[4..n.len() - DOC_EXT.len()];
    assert_eq!(stamp.len(), 19);
    assert_eq!(&stamp[4..5], "-");
    assert_eq!(&stamp[10..11], " ");
    assert_eq!(&stamp[13..14], "-");
    let local = Local.timestamp_opt(1_760_000_000, 0).single().unwrap();
    assert_eq!(
        backup_name("A", Some(1_760_000_000.7)).unwrap(),
        format!("A {}{DOC_EXT}", local.format("%Y-%m-%d %H-%M-%S"))
    );
    assert!(backup_name("A", None).unwrap().starts_with("A 20"));
    let d = tmp("free");
    assert_eq!(free_name(&d, "a.drumspike"), "a.drumspike");
    std::fs::write(d.join("a.drumspike"), "").unwrap();
    assert_eq!(free_name(&d, "a.drumspike"), "a (2).drumspike");
    std::fs::write(d.join("a (2).drumspike"), "").unwrap();
    assert_eq!(free_name(&d, "a.drumspike"), "a (3).drumspike");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn write_document_tolerates_edge_cases() {
    let d = tmp("write");
    let doc = crate::project::empty_document("W", "average");
    let path = d.join("W.drumspike");
    write_document(&doc, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("{\n \"format\": 1,\n \"name\": \"W\",\n"));
    assert!(!text.ends_with('\n'));
    assert!(!d.join("W.drumspike.tmp").exists());
    assert!(write_document(&doc, &d.join("missing").join("x.drumspike")).is_err());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_failed_commit_leaves_no_temporary_file_behind() {
    let d = tmp("commitfail");
    std::fs::create_dir_all(&d).unwrap();
    let doc = crate::project::empty_document("W", "average");
    // a destination that is a directory: the final rename must fail
    let dest = d.join("sub");
    std::fs::create_dir_all(&dest).unwrap();
    assert!(write_document(&doc, &dest).is_err());
    let left: Vec<_> = std::fs::read_dir(&d)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(left.is_empty(), "temporary file left behind: {left:?}");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn save_as_and_consolidate_keep_events() {
    let d = tmp("saveas");
    let daw = d.join("daw");
    std::fs::create_dir_all(&daw).unwrap();
    let k = kick(&daw, "Kick In.wav", 1.0);
    let s = kick(&daw, "Snare Top.wav", 2.0);
    let mut p = new_project(&d.join("Songs"), "Song 1").unwrap();
    let krec = build_source(&k, &p.dir, "referenced").unwrap();
    let tr = add_track(&mut p, "Kick In", Some(krec), "kick", 36, None).unwrap();
    tr.events = Some(events());
    let srec = copy_into_project(&build_source(&s, &p.dir, "referenced").unwrap(), &p.dir, None).unwrap();
    add_track(&mut p, "Snare Top", Some(srec), "snare", 38, Some("ffmpeg")).unwrap();
    add_track(&mut p, "Empty", None, "other", 36, None).unwrap();
    save_project(&mut p).unwrap();
    let before = p.doc.track("Kick In").unwrap().events().to_vec();
    let reference = save_as(&p, &d.join("Elsewhere"), "Song 1 ref", false).unwrap();
    let copied = save_as(&p, &d.join("Elsewhere"), "Song 1 copy", true).unwrap();
    for q in [
        load_project(&reference.dir).unwrap(),
        load_project(&copied.dir).unwrap(),
    ] {
        assert_eq!(q.doc.track("Kick In").unwrap().events(), &before[..]);
        assert_eq!(q.doc.track("Snare Top").unwrap().mix_mode.as_deref(), Some("ffmpeg"));
        assert_eq!(q.doc.track("Empty").unwrap().source, Some(None));
    }
    assert_eq!(reference.doc_path, None);
    assert_eq!(
        reference.path(),
        d.join("Elsewhere").join("Song 1 ref").join("Song 1 ref.drumspike")
    );
    // referenced save-as: both sources referenced where they are now
    let r = load_project(&reference.dir).unwrap().doc;
    let rk = r.track("Kick In").unwrap().source_ref().unwrap().clone();
    assert_eq!(
        (rk.mode.as_deref(), rk.path.as_deref()),
        (Some("referenced"), k.to_str())
    );
    let rs = r.track("Snare Top").unwrap().source_ref().unwrap().clone();
    assert_eq!(rs.mode.as_deref(), Some("referenced"));
    assert_eq!(rs.path.as_deref(), p.subdir(AUDIO_DIR).join("Snare Top.wav").to_str());
    assert_eq!(std::fs::read_dir(reference.subdir(AUDIO_DIR)).unwrap().count(), 0);
    // copied save-as: self-contained, byte-identical copies
    let c = load_project(&copied.dir).unwrap().doc;
    for n in ["Kick In", "Snare Top"] {
        let rec = c.track(n).unwrap().source_ref().unwrap();
        assert_eq!(rec.mode.as_deref(), Some("copied"));
        assert_eq!(rec.relative.as_deref().unwrap(), format!("Audio Files/{n}.wav"));
    }
    assert_eq!(
        std::fs::read(copied.subdir(AUDIO_DIR).join("Kick In.wav")).unwrap(),
        std::fs::read(&k).unwrap()
    );
    // consolidate the original: the referenced kick is copied in, the snare stays
    assert_eq!(crate::project::consolidate(&mut p).unwrap(), ["Kick In"]);
    assert_eq!(
        p.doc.track("Kick In").unwrap().source_ref().unwrap().mode.as_deref(),
        Some("copied")
    );
    assert!(crate::project::consolidate(&mut p).unwrap().is_empty());
    save_project(&mut p).unwrap();
    assert_eq!(
        load_project(&p.dir).unwrap().doc.track("Kick In").unwrap().events(),
        &before[..]
    );
    let e = save_as(&p, &d.join("Elsewhere"), "Song 1 ref", false)
        .unwrap_err()
        .to_string();
    assert!(e.starts_with("project already exists: "), "{e}");
    // an offline source refuses Save As
    std::fs::remove_file(&k).unwrap();
    std::fs::remove_file(p.subdir(AUDIO_DIR).join("Kick In.wav")).unwrap();
    let e = save_as(&p, &d.join("X"), "Y", true).unwrap_err().to_string();
    assert_eq!(e, "track Kick In: source missing; relink it before Save As");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn odd_named_document_is_saved_back_to_itself() {
    let d = tmp("odd");
    let folder = d.join("Folder");
    std::fs::create_dir_all(&folder).unwrap();
    let other = folder.join("Other.drumspike");
    std::fs::write(
        &other,
        r#"{"format": 1, "name": "Other", "tracks": {"K": {"role": "kick", "note": 36, "events": [{"t": 0.30000000000000004, "v": 100}]}}}"#,
    )
    .unwrap();
    let mut p = load_project(&folder).unwrap();
    assert_eq!(p.path(), other);
    assert_eq!(p.doc_path.as_deref(), Some(other.as_path()));
    assert_eq!(p.dir, folder);
    p.doc.view = Some(serde_json::from_value(serde_json::json!({"i": 1})).unwrap());
    assert_eq!(save_project(&mut p).unwrap(), other);
    let docs: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(DOC_EXT))
        .collect();
    assert_eq!(docs, ["Other.drumspike"]);
    assert_eq!(view_i(&other), 1);
    let b = backups(&folder.join(BACKUPS_DIR));
    assert_eq!(b.len(), 1);
    assert!(b[0].file_name().unwrap().to_string_lossy().starts_with("Other 20")); // the backup carries the name
    let q = load_project(&other).unwrap();
    assert_eq!(q.doc, p.doc);
    assert_eq!(q.doc.track("K").unwrap().events()[0].t, 0.1 + 0.2);
    std::fs::remove_dir_all(&d).unwrap();
}
