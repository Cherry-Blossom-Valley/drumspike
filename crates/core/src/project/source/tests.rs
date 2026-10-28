use super::super::tests::tmp;
use super::*;
use crate::io::{write_wav, Bext, WavFormat, WavSpec};
use crate::project::{add_track, new_project};

fn spec() -> WavSpec {
    WavSpec {
        sample_rate: 48_000,
        channels: 1,
        format: WavFormat::Pcm16,
        extensible: false,
        channel_mask: 0,
    }
}

fn kick(dir: &Path, name: &str, seed: f32) -> PathBuf {
    let p = dir.join(name);
    let x: Vec<f32> = (0..4000).map(|i| ((i as f32) * 0.01 * seed).sin() * 0.5).collect();
    write_wav(&p, &spec(), &x, None).unwrap();
    p
}

#[test]
fn head_cache_answers_an_unchanged_file_and_hashes_a_changed_one_again() {
    let d = tmp("headcache");
    let a = d.join("a.bin");
    std::fs::write(&a, vec![b'x'; 4000]).unwrap();
    let zero = std::time::Duration::ZERO;
    let (h1, cached) = head_sha1_settled(&a, zero).unwrap();
    assert!(!cached, "the first hash reads the file");
    let (h2, cached) = head_sha1_settled(&a, zero).unwrap();
    assert!(cached, "the second hash of the unchanged file comes from the cache");
    assert_eq!(h1, h2);
    // the same size, other bytes: mtime and ctime change, so the key changes
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&a, vec![b'y'; 4000]).unwrap();
    let (h3, cached) = head_sha1_settled(&a, zero).unwrap();
    assert!(!cached, "a changed file is hashed again");
    assert_ne!(h3, h1);
    assert_eq!(h3, hash_head_now(&a).unwrap());
    // a file that changed within the settle time is never kept
    let (_, c1) = head_sha1_settled(&a, std::time::Duration::from_secs(3600)).unwrap();
    let (_, c2) = head_sha1_settled(&a, std::time::Duration::from_secs(3600)).unwrap();
    assert!(!c1 && !c2);
    assert!(head_sha1_settled(&d.join("none"), zero).is_err());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn head_cache_counts_grow_with_every_hash() {
    let d = tmp("headcounts");
    let a = d.join("a.bin");
    std::fs::write(&a, b"abc").unwrap();
    let before = head_cache_counts();
    head_sha1(&a).unwrap();
    head_sha1(&a).unwrap();
    let after = head_cache_counts();
    // other tests hash files at the same time: the counters only grow
    assert!(
        after.hits + after.hashed >= before.hits + before.hashed + 2,
        "{before:?} {after:?}"
    );
    assert!(after.hashed > before.hashed, "a new file is read");
    assert!(HEAD_CACHE_SETTLE >= std::time::Duration::from_secs(2) && HEAD_CACHE_MAX > 30);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn head_sha1_covers_only_the_first_mib() {
    let d = tmp("sha");
    let a = d.join("a.bin");
    let b = d.join("b.bin");
    let mut x = vec![b'x'; HEAD_BYTES + 10];
    std::fs::write(&a, &x).unwrap();
    x[HEAD_BYTES..].fill(b'y');
    std::fs::write(&b, &x).unwrap();
    assert_eq!(head_sha1(&a).unwrap(), head_sha1(&b).unwrap());
    assert_eq!(head_sha1(&a).unwrap().len(), 40);
    let mut y = vec![b'x'; HEAD_BYTES + 1];
    y[0] = b'y';
    std::fs::write(&b, &y).unwrap();
    assert_ne!(head_sha1(&a).unwrap(), head_sha1(&b).unwrap());
    // the digest of "abc" (RFC 3174)
    std::fs::write(&a, b"abc").unwrap();
    assert_eq!(head_sha1(&a).unwrap(), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert!(head_sha1(&d.join("none")).is_err());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn source_matches_checks_size_and_head_hash() {
    let d = tmp("matches");
    let p = kick(&d, "k.wav", 1.0);
    let rec = build_source(&p, &d, "referenced").unwrap();
    assert!(source_matches(&rec, &p));
    // the same bytes at another path match: the identity is size + head hash, not the path
    let q = d.join("copy.wav");
    std::fs::copy(&p, &q).unwrap();
    assert!(source_matches(&rec, &q));
    // another file of the same size: the hash differs
    let other = kick(&d, "o.wav", 2.0);
    assert_eq!(
        std::fs::metadata(&other).unwrap().len(),
        std::fs::metadata(&p).unwrap().len()
    );
    assert!(!source_matches(&rec, &other));
    // the size differs
    let mut x = std::fs::read(&p).unwrap();
    x.push(0);
    std::fs::write(&q, &x).unwrap();
    assert!(!source_matches(&rec, &q));
    // a record without size or hash, and a path that cannot be read
    assert!(!source_matches(
        &Source {
            size: rec.size,
            ..Source::default()
        },
        &p
    ));
    assert!(!source_matches(
        &Source {
            head_sha1: rec.head_sha1.clone(),
            ..Source::default()
        },
        &p
    ));
    assert!(!source_matches(&rec, &d.join("none.wav")));
    assert!(!source_matches(&rec, &d));
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn bext_origination_is_read_like_the_python_reader() {
    let d = tmp("bext");
    let mut b = Bext {
        origination_date: "2026-09-14".into(),
        origination_time: "10:31:02".into(),
        time_reference: 12345,
        ..Default::default()
    };
    let p = d.join("b.wav");
    write_wav(&p, &spec(), &[0.0; 48], Some(&b)).unwrap();
    assert_eq!(bext_origination(&p).as_deref(), Some("2026-09-14T10:31:02Z"));
    let rec = build_source(&p, &d, "referenced").unwrap();
    assert_eq!(rec.bext_origination.as_deref(), Some("2026-09-14T10:31:02Z"));
    // no bext
    let plain = kick(&d, "p.wav", 1.0);
    assert_eq!(bext_origination(&plain), None);
    assert_eq!(build_source(&plain, &d, "copied").unwrap().bext_origination, None);
    // empty date
    b.origination_date.clear();
    write_wav(&p, &spec(), &[0.0; 48], Some(&b)).unwrap();
    assert_eq!(bext_origination(&p), None);
    // truncated chunk, not a WAV, missing file
    std::fs::write(&p, b"RIFF\x10\0\0\0WAVEbext\x04\0\0\0abcd").unwrap();
    assert_eq!(bext_origination(&p), None);
    std::fs::write(&p, b"hello").unwrap();
    assert_eq!(bext_origination(&p), None);
    assert_eq!(bext_origination(&d.join("none.wav")), None);
    // a bext body of exactly 338 bytes with a date is enough; 337 is not
    let mut body = vec![0u8; 338];
    body[320..330].copy_from_slice(b"2026-01-02");
    body[330..338].copy_from_slice(b"03:04:05");
    let mut f = b"RIFF\0\0\0\0WAVEbext".to_vec();
    f.extend_from_slice(&(body.len() as u32).to_le_bytes());
    f.extend_from_slice(&body);
    std::fs::write(&p, &f).unwrap();
    assert_eq!(bext_origination(&p).as_deref(), Some("2026-01-02T03:04:05Z"));
    let mut g = b"RIFF\0\0\0\0WAVEbext".to_vec();
    g.extend_from_slice(&337u32.to_le_bytes());
    g.extend_from_slice(&body[..337]);
    std::fs::write(&p, &g).unwrap();
    assert_eq!(bext_origination(&p), None);
    assert_eq!(ascii_field(b"ab\xff\0zz"), "ab\u{FFFD}");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn build_source_record_fields() {
    let d = tmp("build");
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    let wav = kick(&d, "Kick In.wav", 1.0);
    let rec = build_source(&wav, &pdir, "referenced").unwrap();
    let st = std::fs::metadata(&wav).unwrap();
    assert_eq!(rec.mode.as_deref(), Some("referenced"));
    assert_eq!(rec.path.as_deref(), wav.to_str());
    assert_eq!(rec.relative.as_deref(), Some("../Kick In.wav"));
    assert_eq!(rec.size, Some(st.len() as i64));
    assert_eq!(rec.mtime, Some(mtime_secs(&st)));
    assert_eq!(rec.head_sha1.as_deref(), Some(head_sha1(&wav).unwrap().as_str()));
    assert!(rec.extra.is_empty() && rec.bext_origination.is_none());
    let v = serde_json::to_value(&rec).unwrap();
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["mode", "path", "relative", "size", "mtime", "head_sha1"]);
    assert_eq!(
        build_source(&wav, &pdir, "linked").unwrap_err().to_string(),
        "bad source mode: 'linked'"
    );
    assert!(build_source(&d.join("none.wav"), &pdir, "copied").is_err());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn resolve_order_absolute_relative_then_audio_files() {
    let d = tmp("resolve");
    let pdir = d.join("P");
    std::fs::create_dir_all(pdir.join(AUDIO_DIR)).unwrap();
    let wav = kick(&d, "Kick In.wav", 1.0);
    let rec = build_source(&wav, &pdir, "referenced").unwrap();
    assert_eq!(
        source_candidates(&rec, &pdir),
        vec![wav.clone(), wav.clone(), pdir.join(AUDIO_DIR).join("Kick In.wav")]
            .into_iter()
            .fold(Vec::new(), |mut acc, p| {
                if !acc.contains(&p) {
                    acc.push(p);
                }
                acc
            })
    );
    assert_eq!(resolve_source(Some(&rec), &pdir), (Some(wav.clone()), SourceStatus::Ok));
    // 1 -> 2: the whole tree moves (absolute path dead, relative path alive)
    let moved = d.join("moved");
    std::fs::create_dir_all(&moved).unwrap();
    std::fs::rename(&wav, moved.join("Kick In.wav")).unwrap();
    std::fs::rename(&pdir, moved.join("P")).unwrap();
    assert_eq!(
        resolve_source(Some(&rec), &moved.join("P")),
        (Some(moved.join("Kick In.wav")), SourceStatus::Ok)
    );
    // 2 -> 3: only a copy under Audio Files/ is left
    std::fs::rename(
        moved.join("Kick In.wav"),
        moved.join("P").join(AUDIO_DIR).join("Kick In.wav"),
    )
    .unwrap();
    assert_eq!(
        resolve_source(Some(&rec), &moved.join("P")),
        (
            Some(moved.join("P").join(AUDIO_DIR).join("Kick In.wav")),
            SourceStatus::Ok
        )
    );
    // a changed file at the absolute path loses against a matching copy further down
    let wav2 = kick(&d, "Kick In.wav", 2.0);
    let mut rec2 = build_source(&wav2, &moved.join("P"), "referenced").unwrap();
    rec2.size = rec.size;
    rec2.head_sha1 = rec.head_sha1.clone();
    assert_eq!(
        resolve_source(Some(&rec2), &moved.join("P")),
        (
            Some(moved.join("P").join(AUDIO_DIR).join("Kick In.wav")),
            SourceStatus::Ok
        )
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn resolve_reports_changed_and_missing() {
    let d = tmp("changed");
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    let wav = kick(&d, "Kick In.wav", 1.0);
    let rec = build_source(&wav, &pdir, "referenced").unwrap();
    let mut bytes = std::fs::read(&wav).unwrap();
    bytes[100..104].fill(0xff); // same size, different head
    std::fs::write(&wav, &bytes).unwrap();
    assert_eq!(
        resolve_source(Some(&rec), &pdir),
        (Some(wav.clone()), SourceStatus::Changed)
    );
    bytes.extend_from_slice(&[0; 8]); // different size
    std::fs::write(&wav, &bytes).unwrap();
    assert_eq!(
        resolve_source(Some(&rec), &pdir),
        (Some(wav.clone()), SourceStatus::Changed)
    );
    std::fs::remove_file(&wav).unwrap();
    assert_eq!(resolve_source(Some(&rec), &pdir), (None, SourceStatus::Missing));
    assert_eq!(resolve_source(None, &pdir), (None, SourceStatus::Missing));
    let bare = Source {
        mode: Some("referenced".into()),
        ..Default::default()
    };
    assert!(source_candidates(&bare, &pdir).is_empty());
    assert_eq!(resolve_source(Some(&bare), &pdir), (None, SourceStatus::Missing));
    let e = copy_into_project(&rec, &pdir, None).unwrap_err().to_string();
    assert_eq!(e, format!("cannot copy source (missing): {}", wav.display()));
    assert_eq!(SourceStatus::Ok.as_str(), "ok");
    assert_eq!(serde_json::to_string(&SourceStatus::Changed).unwrap(), "\"changed\"");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn copy_into_project_is_byte_identical_and_leaves_the_source_alone() {
    let d = tmp("copy");
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    let wav = kick(&d, "Kick In.wav", 1.0);
    let src_bytes = std::fs::read(&wav).unwrap();
    let src_mtime = std::fs::metadata(&wav).unwrap().modified().unwrap();
    let rec = build_source(&wav, &pdir, "referenced").unwrap();
    let cop = copy_into_project(&rec, &pdir, None).unwrap();
    let dst = pdir.join(AUDIO_DIR).join("Kick In.wav");
    assert_eq!(cop.mode.as_deref(), Some("copied"));
    assert_eq!(cop.path.as_deref(), dst.to_str());
    assert_eq!(cop.relative.as_deref(), Some("Audio Files/Kick In.wav"));
    assert_eq!(
        (cop.size, cop.mtime, &cop.head_sha1),
        (rec.size, rec.mtime, &rec.head_sha1)
    ); // copy2 keeps the mtime
    assert_eq!(std::fs::read(&dst).unwrap(), src_bytes);
    assert_eq!(std::fs::read(&wav).unwrap(), src_bytes);
    assert_eq!(std::fs::metadata(&wav).unwrap().modified().unwrap(), src_mtime);
    assert!(!pdir.join(AUDIO_DIR).join("Kick In.wav.tmp").exists());
    // a second, different file with the same name gets a suffix; a re-copy of a file
    // that already sits in Audio Files/ is reused; the same source again reuses the copy
    let o = d.join("o");
    std::fs::create_dir_all(&o).unwrap();
    let other = kick(&o, "Kick In.wav", 2.0);
    let cop2 = copy_into_project(&build_source(&other, &pdir, "referenced").unwrap(), &pdir, None).unwrap();
    assert_eq!(cop2.relative.as_deref(), Some("Audio Files/Kick In (2).wav"));
    assert_eq!(copy_into_project(&cop, &pdir, None).unwrap(), cop);
    assert_eq!(resolve_source(Some(&cop), &pdir), (Some(dst.clone()), SourceStatus::Ok));
    assert_eq!(copy_into_project(&rec, &pdir, None).unwrap(), cop);
    assert_eq!(
        copy_into_project(&build_source(&other, &pdir, "referenced").unwrap(), &pdir, None).unwrap(),
        cop2
    );
    let mut names: Vec<String> = std::fs::read_dir(pdir.join(AUDIO_DIR))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["Kick In (2).wav", "Kick In.wav"]);
    // the importer's explicit name follows the same rule
    assert_eq!(copy_into_project(&rec, &pdir, Some("Kick In.wav")).unwrap(), cop);
    assert_eq!(
        copy_into_project(&rec, &pdir, Some("Named.wav"))
            .unwrap()
            .relative
            .as_deref(),
        Some("Audio Files/Named.wav")
    );
    assert_eq!(
        copy_into_project(&rec, &pdir, Some("Named.wav"))
            .unwrap()
            .relative
            .as_deref(),
        Some("Audio Files/Named.wav")
    );
    assert_eq!(splitext("Kick In.wav"), ("Kick In", ".wav"));
    assert_eq!(splitext(".wav"), (".wav", ""));
    assert_eq!(splitext("noext"), ("noext", ""));
    assert_eq!(splitext("a.b.c"), ("a.b", ".c"));
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn copy_into_project_rejects_a_mismatching_record() {
    let d = tmp("mismatch");
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    let wav = kick(&d, "Kick In.wav", 1.0);
    // the record points at a file that exists but differs: resolve says changed
    let mut rec = build_source(&wav, &pdir, "referenced").unwrap();
    rec.head_sha1 = Some("0".repeat(40));
    let e = copy_into_project(&rec, &pdir, None).unwrap_err().to_string();
    assert!(e.starts_with("cannot copy source (changed): "), "{e}");
    assert!(!pdir.join(AUDIO_DIR).join("Kick In.wav").exists());
    std::fs::remove_dir_all(&d).unwrap();
}

fn audio_names(pdir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(pdir.join(AUDIO_DIR))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn copy_into_project_rejects_a_truncated_copy() {
    // test_project.py::test_copy_into_project_rejects_a_truncated_copy
    let d = tmp("truncated");
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    let wav = kick(&d, "Kick In.wav", 1.0);
    let src_bytes = std::fs::read(&wav).unwrap();
    let rec = build_source(&wav, &pdir, "referenced").unwrap();
    let short = |_: &Path, dst: &Path| std::fs::write(dst, b"short");
    let e = copy_into_project_with(&rec, &pdir, None, short)
        .unwrap_err()
        .to_string();
    assert_eq!(e, format!("copy of {} is incomplete", wav.display()));
    assert!(audio_names(&pdir).is_empty(), "the short tmp file is removed");
    assert_eq!(std::fs::read(&wav).unwrap(), src_bytes);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn copy_into_project_rejects_a_mismatching_copy() {
    // test_project.py::test_copy_into_project_rejects_a_mismatching_copy: the copy has
    // the right size but a different head, so the identity check after the rename fails
    let d = tmp("corrupt");
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    let wav = kick(&d, "Kick In.wav", 1.0);
    let src_bytes = std::fs::read(&wav).unwrap();
    let rec = build_source(&wav, &pdir, "referenced").unwrap();
    let corrupt = |src: &Path, dst: &Path| {
        copy2(src, dst)?;
        let mut b = std::fs::read(dst)?;
        b[50..52].copy_from_slice(&[0xff, 0xff]);
        std::fs::write(dst, b)
    };
    let e = copy_into_project_with(&rec, &pdir, None, corrupt)
        .unwrap_err()
        .to_string();
    assert_eq!(e, format!("copy of {} does not match the source", wav.display()));
    // as in Python, the renamed copy stays (no tmp file) and the source is untouched
    assert_eq!(audio_names(&pdir), ["Kick In.wav"]);
    assert_eq!(std::fs::read(&wav).unwrap(), src_bytes);
    // a copy that fails outright is an I/O error and leaves no file
    let fail = |_: &Path, _: &Path| Err(std::io::Error::other("disk full"));
    let e = copy_into_project_with(&rec, &pdir, Some("Other.wav"), fail).unwrap_err();
    assert!(matches!(e, ProjectError::Io(..)), "{e}");
    assert_eq!(audio_names(&pdir), ["Kick In.wav"]);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn consolidate_shares_one_copy_between_tracks_with_the_same_source() {
    let d = tmp("consolidate");
    let daw = d.join("daw");
    std::fs::create_dir_all(&daw).unwrap();
    let kick_path = kick(&daw, "Kick In.wav", 1.0);
    let mut p = new_project(&d, "Shared").unwrap();
    let rec = build_source(&kick_path, &p.dir, "referenced").unwrap();
    add_track(&mut p, "Kick In", Some(rec.clone()), "kick", 36, None).unwrap();
    add_track(&mut p, "Kick Sub", Some(rec.clone()), "kick", 35, None).unwrap();
    add_track(&mut p, "Empty", None, "other", 36, None).unwrap();
    assert_eq!(consolidate(&mut p).unwrap(), ["Kick In", "Kick Sub"]);
    let a = p.doc.track("Kick In").unwrap().source_ref().unwrap().clone();
    let b = p.doc.track("Kick Sub").unwrap().source_ref().unwrap().clone();
    assert_eq!(a, b);
    assert_eq!(a.mode.as_deref(), Some("copied"));
    assert_eq!(a.relative.as_deref(), Some("Audio Files/Kick In.wav"));
    let names: Vec<String> = std::fs::read_dir(p.subdir(AUDIO_DIR))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["Kick In.wav"]);
    assert_eq!(
        std::fs::read(p.subdir(AUDIO_DIR).join("Kick In.wav")).unwrap(),
        std::fs::read(&kick_path).unwrap()
    );
    assert!(consolidate(&mut p).unwrap().is_empty());
    assert_eq!(copy_into_project(&rec, &p.dir, Some("Kick In.wav")).unwrap(), a);
    // the cache key is the record's identity
    let key = cache_key(p.doc.track("Kick In").unwrap());
    assert_eq!(
        key,
        CacheKey {
            size: a.size,
            mtime: a.mtime,
            head_sha1: a.head_sha1.clone()
        }
    );
    assert_eq!(
        serde_json::to_string(&cache_key(p.doc.track("Empty").unwrap())).unwrap(),
        r#"{"size":null,"mtime":null,"head_sha1":null}"#
    );
    let adir = p.subdir(super::super::ANALYSIS_DIR).join("Kick In");
    assert!(!key.matches_file(&adir));
    std::fs::create_dir_all(&adir).unwrap();
    std::fs::write(adir.join("key.json"), serde_json::to_string(&key).unwrap()).unwrap();
    assert!(key.matches_file(&adir));
    std::fs::write(
        adir.join("key.json"),
        r#"{"size": 1, "mtime": 2, "head_sha1": "stale"}"#,
    )
    .unwrap();
    assert!(!key.matches_file(&adir));
    std::fs::write(adir.join("key.json"), "{nope").unwrap();
    assert!(!key.matches_file(&adir));
    std::fs::remove_dir_all(&d).unwrap();
}

// ---------- P3-4d: copy with cancel, relink search ----------

fn never() -> bool {
    false
}

fn no_bytes(_: u64) {}

#[test]
fn copy_into_project_ctl_copies_in_chunks_counts_bytes_and_keeps_the_times() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let d = tmp("ctl_copy");
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    let wav = kick(&d, "Kick In.wav", 1.0);
    let rec = build_source(&wav, &pdir, "referenced").unwrap();
    let seen = AtomicU64::new(0);
    let count = |n: u64| {
        seen.fetch_add(n, Ordering::SeqCst);
    };
    let ctl = CopyCtl {
        cancelled: &never,
        on_bytes: &count,
    };
    let out = copy_into_project_ctl(&wav, &rec, &pdir, None, &ctl).unwrap().unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), rec.size.unwrap() as u64);
    let dst = pdir.join(AUDIO_DIR).join("Kick In.wav");
    assert_eq!(out.created.as_deref(), Some(dst.as_path()));
    assert_eq!(out.source.mode.as_deref(), Some("copied"));
    assert_eq!(std::fs::read(&dst).unwrap(), std::fs::read(&wav).unwrap());
    assert_eq!(
        std::fs::metadata(&dst).unwrap().modified().unwrap(),
        std::fs::metadata(&wav).unwrap().modified().unwrap(),
        "copy2 keeps the modification time"
    );
    // the second call reuses the file: nothing created, the bytes still count for the progress
    let again = copy_into_project_ctl(&wav, &rec, &pdir, None, &ctl).unwrap().unwrap();
    assert!(again.created.is_none());
    assert_eq!(seen.load(Ordering::SeqCst), 2 * rec.size.unwrap() as u64);
    assert_eq!(audio_names(&pdir), ["Kick In.wav"]);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn copy_into_project_ctl_cancel_removes_the_temporary_file_and_leaves_nothing() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let d = tmp("ctl_cancel");
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    // a file of three chunks; the cancel comes after the first chunk was written
    let big = d.join("Big.wav");
    std::fs::write(&big, vec![7u8; 2 * COPY_CHUNK + 5]).unwrap();
    let rec = build_source(&big, &pdir, "referenced").unwrap();
    let stop = AtomicBool::new(false);
    let chunks = AtomicUsize::new(0);
    let cancelled = || stop.load(Ordering::SeqCst);
    let on_bytes = |_: u64| {
        if chunks.fetch_add(1, Ordering::SeqCst) == 0 {
            stop.store(true, Ordering::SeqCst);
        }
    };
    let ctl = CopyCtl {
        cancelled: &cancelled,
        on_bytes: &on_bytes,
    };
    let r = copy_into_project_ctl(&big, &rec, &pdir, None, &ctl).unwrap();
    assert!(r.is_none(), "a cancel answers None");
    assert_eq!(chunks.load(Ordering::SeqCst), 1, "it stopped after one chunk");
    assert!(
        audio_names(&pdir).is_empty(),
        "no .tmp file and no copy: {:?}",
        audio_names(&pdir)
    );
    assert_eq!(
        std::fs::read(&big).unwrap().len(),
        2 * COPY_CHUNK + 5,
        "the source is untouched"
    );
    // a cancel before the first chunk, too
    let r = copy_into_project_ctl(&big, &rec, &pdir, None, &ctl).unwrap();
    assert!(r.is_none());
    assert_eq!(chunks.load(Ordering::SeqCst), 1);
    assert!(audio_names(&pdir).is_empty());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn copy_chunked_reports_a_cancel_as_interrupted_and_copies_the_bytes_otherwise() {
    let d = tmp("chunked");
    let a = d.join("a.bin");
    std::fs::write(&a, vec![1u8; 100]).unwrap();
    let ctl = CopyCtl {
        cancelled: &never,
        on_bytes: &no_bytes,
    };
    copy_chunked(&a, &d.join("b.bin"), &ctl).unwrap();
    assert_eq!(std::fs::read(d.join("b.bin")).unwrap(), vec![1u8; 100]);
    let yes = || true;
    let ctl = CopyCtl {
        cancelled: &yes,
        on_bytes: &no_bytes,
    };
    let e = copy_chunked(&a, &d.join("c.bin"), &ctl).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::Interrupted);
    assert!(copy_chunked(&d.join("none"), &d.join("e.bin"), &ctl).is_err());
    std::fs::remove_dir_all(&d).unwrap();
}

fn target(track: &str, rec: &Source) -> SearchTarget {
    SearchTarget {
        track: track.to_string(),
        record: rec.clone(),
    }
}

#[test]
fn relink_search_finds_moved_files_by_name_and_by_content() {
    let d = tmp("search_found");
    let old = d.join("old");
    let new = d.join("new/deep/er");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::create_dir_all(&new).unwrap();
    let k = kick(&old, "Kick In.wav", 1.0);
    let s = kick(&old, "Snare Top.wav", 2.0);
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    let kr = build_source(&k, &pdir, "referenced").unwrap();
    let sr = build_source(&s, &pdir, "referenced").unwrap();
    // the kick moves with its name (and its case changes); the snare moves and is renamed
    std::fs::rename(&k, new.join("KICK IN.WAV")).unwrap();
    std::fs::rename(&s, new.join("Snare renamed.wav")).unwrap();
    // a decoy: same size as the snare, other content
    kick(&new, "decoy.wav", 3.0);
    let rep = relink_search(
        &d.join("new"),
        &[target("Kick_In", &kr), target("Snare_Top", &sr)],
        &never,
    )
    .unwrap();
    assert_eq!(rep.results.len(), 2);
    assert_eq!(
        rep.results[0].hit,
        SearchHit::Found {
            path: new.join("KICK IN.WAV"),
            by: "name"
        }
    );
    assert_eq!(
        rep.results[1].hit,
        SearchHit::Found {
            path: new.join("Snare renamed.wav"),
            by: "content"
        }
    );
    assert_eq!(rep.files_seen, 3);
    assert_eq!(rep.dirs_seen, 3);
    assert!(!rep.truncated && !rep.cancelled);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn relink_search_lists_ambiguous_files_and_chooses_none() {
    let d = tmp("search_ambig");
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    let orig = kick(&d, "Kick In.wav", 1.0);
    let rec = build_source(&orig, &pdir, "referenced").unwrap();
    std::fs::remove_file(&orig).unwrap();
    for sub in ["a", "b", "c"] {
        std::fs::create_dir_all(d.join("sessions").join(sub)).unwrap();
    }
    let a = kick(&d.join("sessions/a"), "Kick In.wav", 1.0);
    let b = kick(&d.join("sessions/b"), "Kick In.wav", 1.0);
    kick(&d.join("sessions/c"), "Kick In.wav", 2.0); // same name and size, other content
    let rep = relink_search(&d.join("sessions"), &[target("Kick_In", &rec)], &never).unwrap();
    let mut want = vec![a, b];
    want.sort();
    assert_eq!(rep.results[0].hit, SearchHit::Ambiguous(want));
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn relink_search_reports_missing_with_the_files_of_the_same_name_that_differ() {
    let d = tmp("search_missing");
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    let orig = kick(&d, "Kick In.wav", 1.0);
    let rec = build_source(&orig, &pdir, "referenced").unwrap();
    let none = SearchTarget {
        track: "Gone".into(),
        record: Source {
            path: Some("/x/Gone.wav".into()),
            size: Some(99),
            head_sha1: Some("00".into()),
            ..Source::default()
        },
    };
    let nosize = SearchTarget {
        track: "NoSize".into(),
        record: Source::default(),
    };
    let lib = d.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    let other = kick(&lib, "Kick In.wav", 2.0);
    std::fs::remove_file(&orig).unwrap();
    let rep = relink_search(&lib, &[target("Kick_In", &rec), none, nosize], &never).unwrap();
    assert_eq!(
        rep.results[0].hit,
        SearchHit::Missing {
            differs: vec![other.clone()]
        }
    );
    assert_eq!(rep.results[1].hit, SearchHit::Missing { differs: vec![] });
    assert_eq!(rep.results[2].hit, SearchHit::Missing { differs: vec![] });
    // a folder that does not exist is an error
    assert!(relink_search(&d.join("nope"), &[], &never).is_err());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn relink_search_skips_hidden_files_and_does_not_follow_symbolic_links() {
    let d = tmp("search_skip");
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    let orig = kick(&d, "Kick In.wav", 1.0);
    let rec = build_source(&orig, &pdir, "referenced").unwrap();
    let lib = d.join("lib");
    std::fs::create_dir_all(lib.join(".hidden")).unwrap();
    std::fs::create_dir_all(d.join("elsewhere")).unwrap();
    std::fs::copy(&orig, lib.join(".hidden/Kick In.wav")).unwrap();
    std::fs::copy(&orig, lib.join("._Kick In.wav")).unwrap();
    std::fs::copy(&orig, d.join("elsewhere/Kick In.wav")).unwrap();
    std::os::unix::fs::symlink(d.join("elsewhere"), lib.join("link")).unwrap();
    std::os::unix::fs::symlink(d.join("elsewhere/Kick In.wav"), lib.join("Kick In.wav")).unwrap();
    let rep = relink_search(&lib, &[target("Kick_In", &rec)], &never).unwrap();
    assert_eq!(rep.results[0].hit, SearchHit::Missing { differs: vec![] });
    assert_eq!(rep.files_seen, 0);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn relink_search_skips_unfinished_save_as_folders() {
    let d = tmp("search_partial");
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    let orig = kick(&d, "Kick In.wav", 1.0);
    let rec = build_source(&orig, &pdir, "referenced").unwrap();
    let lib = d.join("lib");
    std::fs::create_dir_all(lib.join("Moved.partial/Audio Files")).unwrap();
    std::fs::copy(&orig, lib.join("Moved.partial/Audio Files/Kick In.wav")).unwrap();
    let rep = relink_search(&lib, &[target("Kick_In", &rec)], &never).unwrap();
    assert_eq!(rep.results[0].hit, SearchHit::Missing { differs: vec![] });
    assert_eq!(rep.files_seen, 0);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn relink_search_stops_at_a_cancel() {
    let d = tmp("search_cancel");
    let pdir = d.join("P");
    std::fs::create_dir_all(&pdir).unwrap();
    let orig = kick(&d, "Kick In.wav", 1.0);
    let rec = build_source(&orig, &pdir, "referenced").unwrap();
    let yes = || true;
    let rep = relink_search(&d, &[target("Kick_In", &rec)], &yes).unwrap();
    assert!(rep.cancelled && rep.results.is_empty());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn relinked_source_keeps_the_identity_and_sets_path_relative_and_mode() {
    let d = tmp("relinked");
    let pdir = d.join("P");
    std::fs::create_dir_all(pdir.join(AUDIO_DIR)).unwrap();
    let orig = kick(&d, "Kick In.wav", 1.0);
    let rec = build_source(&orig, &pdir, "referenced").unwrap();
    let moved = d.join("moved");
    std::fs::create_dir_all(&moved).unwrap();
    std::fs::copy(&orig, moved.join("Kick In.wav")).unwrap();
    let r = relinked_source(&rec, &moved.join("Kick In.wav"), &pdir).unwrap();
    assert_eq!(r.mode.as_deref(), Some("referenced"));
    assert_eq!(r.path.as_deref(), Some(moved.join("Kick In.wav").to_str().unwrap()));
    assert_eq!(r.relative.as_deref(), Some("../moved/Kick In.wav"));
    assert_eq!((r.size, r.mtime, &r.head_sha1), (rec.size, rec.mtime, &rec.head_sha1));
    assert_eq!(cache_key_of(&r), cache_key_of(&rec), "the Analysis cache key stays");
    // a file inside Audio Files/ is a copied source
    std::fs::copy(&orig, pdir.join(AUDIO_DIR).join("Kick In.wav")).unwrap();
    let c = relinked_source(&rec, &pdir.join(AUDIO_DIR).join("Kick In.wav"), &pdir).unwrap();
    assert_eq!(c.mode.as_deref(), Some("copied"));
    // a record without identity is built from the file
    let bare = Source {
        path: rec.path.clone(),
        ..Source::default()
    };
    let b = relinked_source(&bare, &moved.join("Kick In.wav"), &pdir).unwrap();
    assert_eq!(b.head_sha1, rec.head_sha1);
    std::fs::remove_dir_all(&d).unwrap();
}

fn cache_key_of(r: &Source) -> (Option<i64>, Option<i64>, Option<String>) {
    (r.size, r.mtime, r.head_sha1.clone())
}
