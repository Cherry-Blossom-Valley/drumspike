//! P3-4b: tests of the service command of the native shell (`export_existing`) through
//! `drumspike_app::Service` and through the bridge. Test names start with the command name.

mod common;

use common::*;
use drumspike_app::SHELL_COMMANDS;
use drumspike_app::bridge::Bridge;
use drumspike_app::export::{ExportOpts, MidiMode, check_dest};
use drumspike_app::reply::Reply;
use serde_json::{Value, json};

fn opened(tag: &str) -> (drumspike_app::Service, String, std::path::PathBuf, std::path::PathBuf) {
    let root = tmp(tag);
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
    (svc, sid, dir, root)
}

#[test]
fn export_existing_lists_nothing_before_the_first_export_all() {
    let (svc, sid, _dir, root) = opened("shell_none");
    let r = svc.export_existing(&sid).unwrap();
    assert_eq!((r["count"].clone(), r["files"].clone()), (json!(0), json!([])));
    assert_eq!(r["remembered"], Value::Null);
    assert!(r["dest"].as_str().unwrap().ends_with("/Exports"));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn export_existing_lists_the_files_that_export_all_would_replace() {
    let (svc, sid, dir, root) = opened("shell_some");
    svc.events_set(&sid, "Kick_In", &json!([{"t": 0.5, "v": 64}])).unwrap();
    svc.export_all(&sid, None).unwrap();
    let r = svc.export_existing(&sid).unwrap();
    assert_eq!(r["count"], 4);
    let files: Vec<&str> = r["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        files,
        [
            "Exports/Kick_In_spike.mid",
            "Exports/Kick_In_spike.wav",
            "Exports/Snare_Top_spike.mid",
            "Exports/Snare_Top_spike.wav"
        ]
    );
    // one file less: three are listed, and a file that is no export of the project does not count
    std::fs::remove_file(dir.join("Exports/Snare_Top_spike.wav")).unwrap();
    std::fs::write(dir.join("Exports/other.txt"), b"x").unwrap();
    assert_eq!(svc.export_existing(&sid).unwrap()["count"], 3);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn export_existing_checks_the_session() {
    let (svc, _sid, _dir, root) = opened("shell_err");
    assert_eq!(svc.export_existing("nope").unwrap_err().code, 404);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn export_existing_answers_through_the_bridge() {
    let (svc, sid, _dir, root) = opened("shell_bridge");
    let b = Bridge::new(svc, std::path::PathBuf::from("/nonexistent"));
    match b.invoke("export_existing", &json!({"sid": sid})).unwrap() {
        Reply::Json(v) => assert_eq!(v["count"], 0),
        Reply::Bytes(_) => panic!("bytes"),
    }
    assert_eq!(b.invoke("export_existing", &json!({})).unwrap_err().code, 400);
    assert_eq!(
        SHELL_COMMANDS,
        [
            "export_existing",
            "export_dest_set",
            "export_markers_set",
            "export_compensate_set",
            "export_drag_prepare",
            "export_drag_take",
            "export_last_get",
            "export_last_set"
        ]
    );
    std::fs::remove_dir_all(&root).unwrap();
}

// #232: the kept "Compensate" checkbox: export_compensate_set stores the document field, the
// answers of export_last_get and export_existing carry it, and an export_all without a per-call
// compensate uses it (the MIDI notes move or not — visible on the written file's timing).
#[test]
fn export_compensate_set_keeps_the_checkbox_and_export_all_uses_it() {
    let (svc, sid, _dir, root) = opened("shell_comp");
    // the default is on and the field is absent
    let r = svc.export_last_get(&sid).unwrap();
    assert_eq!(r["export_compensate"], true);
    // off: the field is kept and answered
    assert_eq!(
        svc.export_compensate_set(&sid, false).unwrap()["export_compensate"],
        false
    );
    assert_eq!(svc.export_last_get(&sid).unwrap()["export_compensate"], false);
    assert_eq!(svc.export_existing(&sid).unwrap()["export_compensate"], false);
    // on again removes the field (the default)
    assert_eq!(
        svc.export_compensate_set(&sid, true).unwrap()["export_compensate"],
        true
    );
    // a bad argument is a 400
    use drumspike_app::call::call;
    assert!(matches!(
        call(&svc, "export_compensate_set", &json!({"sid": sid, "compensate": "no"}), None),
        Err(e) if e.code == 400
    ));
    std::fs::remove_dir_all(&root).unwrap();
}

/// The in-page folder browser of the retired server is gone (P3-12): the native dialogs replace it, the page
/// has no `#dirModal`, and the page test that the app never shows a folder browser exists.
#[test]
fn the_page_has_no_folder_browser() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let html = std::fs::read_to_string(root.join("static/index.html")).unwrap();
    assert!(!html.contains("dirModal"));
    let page = std::fs::read_to_string(root.join("tests/e2e_app/test_shell.py")).unwrap();
    assert!(page.contains("def test_the_app_never_shows_the_folder_browser"));
}

// ---------- P4-30: the destination folder, the single MIDI file, the drag files ----------

fn opts(v: Value) -> ExportOpts {
    ExportOpts::from_args(&v).unwrap()
}

fn two_tracks_with_hits(svc: &drumspike_app::Service, sid: &str) {
    svc.events_set(sid, "Kick_In", &json!([{"t": 0.5, "v": 64}, {"t": 1.0, "v": 90}]))
        .unwrap();
    svc.events_set(sid, "Snare_Top", &json!([{"t": 0.75, "v": 80}]))
        .unwrap();
}

fn times_of(svc: &drumspike_app::Service, sid: &str, name: &str) -> Vec<f64> {
    svc.events_get(sid, name).unwrap()["events"]
        .as_array()
        .map(|a| a.iter().map(|e| e["t"].as_f64().unwrap()).collect())
        .unwrap_or_default()
}

#[test]
fn check_dest_wants_an_absolute_existing_writable_folder() {
    let root = tmp("dest_check");
    assert_eq!(check_dest(root.to_str().unwrap()).unwrap(), root);
    assert_eq!(check_dest("relative/folder").unwrap_err().code, 400);
    assert_eq!(
        check_dest(root.join("missing").to_str().unwrap()).unwrap_err().code,
        404
    );
    let file = root.join("f.txt");
    std::fs::write(&file, b"x").unwrap();
    assert_eq!(check_dest(file.to_str().unwrap()).unwrap_err().code, 400);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let ro = root.join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        let denied = std::fs::File::create(ro.join("probe")).is_err(); // root can write anywhere
        if denied {
            let e = check_dest(ro.to_str().unwrap()).unwrap_err();
            assert_eq!(e.code, 400);
            assert!(e.message.contains("not writable"), "{}", e.message);
        }
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(
        std::fs::read_dir(&root).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".drumspike-write"))
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn export_options_parse_the_formats_the_mode_and_refuse_nonsense() {
    let o = opts(json!({}));
    assert!(o.midi && o.spikes && o.dest.is_none() && o.midi_mode == MidiMode::PerTrack && o.tracks.is_none());
    let o = opts(json!({"formats": ["midi"], "midi_mode": "single", "tracks": ["a"], "dest": "/x"}));
    assert!(o.midi && !o.spikes && o.midi_mode == MidiMode::Single);
    assert_eq!(
        (o.tracks, o.dest),
        (Some(vec!["a".to_string()]), Some("/x".to_string()))
    );
    for bad in [
        json!({"formats": []}),
        json!({"formats": ["wav"]}),
        json!({"midi_mode": "both"}),
        json!({"midi_mode": 1}),
        json!({"tracks": "a"}),
        json!({"tracks": [1]}),
        json!({"dest": 3}),
    ] {
        assert_eq!(ExportOpts::from_args(&bad).unwrap_err().code, 400, "{bad}");
    }
}

#[test]
fn export_all_writes_into_a_dest_and_leaves_exports_alone() {
    let (svc, sid, dir, root) = opened("dest_all");
    two_tracks_with_hits(&svc, &sid);
    let out = root.join("elsewhere");
    std::fs::create_dir(&out).unwrap();
    let o = opts(json!({"dest": out.to_str().unwrap()}));
    let r = svc.export_all_opts(&sid, &o, None).unwrap();
    assert_eq!(
        (r["midi"].clone(), r["spikes"].clone(), r["dest"].as_str().unwrap()),
        (json!(2), json!(2), out.to_str().unwrap())
    );
    let mut names: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "Kick_In_spike.mid",
            "Kick_In_spike.wav",
            "Snare_Top_spike.mid",
            "Snare_Top_spike.wav"
        ]
    );
    assert!(
        std::fs::read_dir(dir.join("Exports")).map_or(true, |mut d| d.next().is_none()),
        "the default folder got nothing"
    );
    // the replace list of the same options names the files without a folder
    let e = svc.export_existing_opts(&sid, &o).unwrap();
    assert_eq!(e["count"], 4);
    assert_eq!(e["files"][0], "Kick_In_spike.mid");
    assert_eq!(
        svc.export_existing(&sid).unwrap()["count"],
        0,
        "the default folder holds none"
    );
    // a bad dest is refused and writes nothing
    let bad = opts(json!({"dest": root.join("nope").to_str().unwrap()}));
    assert_eq!(svc.export_all_opts(&sid, &bad, None).unwrap_err().code, 404);
    assert_eq!(svc.export_existing_opts(&sid, &bad).unwrap_err().code, 404);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn export_midi_and_spikes_take_a_dest_too() {
    let (svc, sid, _dir, root) = opened("dest_one");
    two_tracks_with_hits(&svc, &sid);
    let out = root.join("one");
    std::fs::create_dir(&out).unwrap();
    let d = out.to_str().unwrap();
    let m = svc
        .export_midi_stored_to(&sid, "Kick_In", None, None, None, None, Some(d))
        .unwrap();
    assert_eq!(m["path"], out.join("Kick_In_spike.mid").to_str().unwrap());
    let w = svc.export_spikes_to(&sid, "Kick_In", None, Some(d)).unwrap();
    assert_eq!(w["path"], out.join("Kick_In_spike.wav").to_str().unwrap());
    assert_eq!(
        svc.export_spikes_to(&sid, "Kick_In", None, Some("rel"))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        svc.export_midi_stored_to(&sid, "Kick_In", None, None, None, None, Some("rel"))
            .unwrap_err()
            .code,
        400
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_single_midi_file_has_one_track_per_drum_the_notes_and_the_tempo_map() {
    let (svc, sid, dir, root) = opened("single");
    two_tracks_with_hits(&svc, &sid);
    svc.tempo_map_set(&sid, &json!({"bpm": 100.0, "steps": [{"t": 8.0, "bpm": 150.0}]}))
        .unwrap();
    let o = opts(json!({"midi_mode": "single", "formats": ["midi"]}));
    let r = svc.export_all_opts(&sid, &o, None).unwrap();
    assert_eq!((r["midi"].clone(), r["spikes"].clone()), (json!(1), json!(0)));
    let file = dir.join("Exports/Song_drums.mid");
    assert_eq!(r["midi_file"], file.to_str().unwrap());
    assert!(!dir.join("Exports/Kick_In_spike.mid").exists() && !dir.join("Exports/Kick_In_spike.wav").exists());
    let bytes = std::fs::read(&file).unwrap();
    let smf = midly::Smf::parse(&bytes).unwrap();
    assert_eq!(smf.tracks.len(), 3, "the tempo track and two drum tracks");
    let tempos = smf.tracks[0]
        .iter()
        .filter(|e| matches!(e.kind, midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(_))))
        .count();
    assert_eq!(tempos, 2, "the start tempo and the step");
    let ons: Vec<Vec<(u8, u8)>> = smf.tracks[1..]
        .iter()
        .map(|t| {
            t.iter()
                .filter_map(|e| match e.kind {
                    midly::TrackEventKind::Midi {
                        message: midly::MidiMessage::NoteOn { key, vel },
                        ..
                    } if vel.as_int() > 0 => Some((key.as_int(), vel.as_int())),
                    _ => None,
                })
                .collect()
        })
        .collect();
    assert_eq!(ons, vec![vec![(36, 64), (36, 90)], vec![(38, 80)]]);
    // the replace list names this file, not the per-track ones
    let e = svc.export_existing_opts(&sid, &o).unwrap();
    assert_eq!(e["files"], json!(["Exports/Song_drums.mid"]));
    // a selection of one track
    let one = opts(json!({"midi_mode": "single", "formats": ["midi"], "tracks": ["Snare_Top"]}));
    svc.export_all_opts(&sid, &one, None).unwrap();
    assert_eq!(
        midly::Smf::parse(&std::fs::read(&file).unwrap()).unwrap().tracks.len(),
        2
    );
    assert_eq!(
        svc.export_all_opts(&sid, &opts(json!({"tracks": ["Nope"]})), None)
            .unwrap_err()
            .code,
        400
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn export_dest_set_keeps_the_folder_per_project_and_marks_the_project_dirty() {
    let (svc, sid, dir, root) = opened("dest_set");
    let out = root.join("keep");
    std::fs::create_dir(&out).unwrap();
    assert_eq!(
        svc.export_dest_set(&sid, Some(out.to_str().unwrap())).unwrap(),
        json!({"export_dest": out.to_str().unwrap()})
    );
    assert_eq!(svc.export_existing(&sid).unwrap()["remembered"], out.to_str().unwrap());
    svc.project_save(&sid).unwrap();
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("Song.drumspike")).unwrap()).unwrap();
    assert_eq!(doc["export_dest"], out.to_str().unwrap());
    assert_eq!(svc.export_dest_set(&sid, Some("rel")).unwrap_err().code, 400);
    assert_eq!(svc.export_dest_set(&sid, None).unwrap(), json!({"export_dest": null}));
    assert_eq!(svc.export_existing(&sid).unwrap()["remembered"], Value::Null);
    assert_eq!(svc.export_dest_set("nope", None).unwrap_err().code, 404);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn export_drag_prepare_stages_in_exports_drag_and_a_take_makes_the_export() {
    let (svc, sid, dir, root) = opened("drag");
    two_tracks_with_hits(&svc, &sid);
    let out = root.join("mine");
    std::fs::create_dir(&out).unwrap();
    svc.export_dest_set(&sid, Some(out.to_str().unwrap())).unwrap();
    let r = svc.export_drag_prepare(&sid, &opts(json!({}))).unwrap();
    let drag = svc.drag_dir(&sid).unwrap();
    assert_eq!(r["dir"], drag.to_str().unwrap());
    let exports = std::fs::canonicalize(dir.join("Exports")).unwrap();
    assert!(
        std::fs::canonicalize(&drag).unwrap().starts_with(&exports),
        "the drag folder lies in Exports/"
    );
    assert_eq!(drag.file_name().unwrap(), ".drag");
    assert_eq!(
        (
            r["midi"].as_array().unwrap().len(),
            r["spikes"].as_array().unwrap().len()
        ),
        (2, 2)
    );
    assert!(
        std::fs::read_dir(&out).unwrap().next().is_none(),
        "the chosen folder is untouched"
    );
    let names = |p: &std::path::Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(p)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        names(&dir.join("Exports")),
        [".drag"],
        "the render stays out of Exports/ until a drag"
    );
    // an option change replaces the files of the old render
    let r = svc
        .export_drag_prepare(&sid, &opts(json!({"midi_mode": "single", "formats": ["midi"]})))
        .unwrap();
    assert_eq!(
        (
            r["midi"].as_array().unwrap().len(),
            r["spikes"].as_array().unwrap().len()
        ),
        (1, 0)
    );
    assert!(r["midi"][0].as_str().unwrap().ends_with("Song_drums.mid"));
    assert_eq!(
        names(&drag),
        ["Song_drums.mid"],
        "the old spike files are gone with the old options"
    );
    // a drag starts: the staged file moves into Exports/ under its export name, with the export's bytes
    let staged = std::path::PathBuf::from(r["midi"][0].as_str().unwrap());
    let taken = svc.export_drag_take(&sid, &[staged.display().to_string()]).unwrap();
    let dragged = std::path::PathBuf::from(taken["files"][0].as_str().unwrap());
    assert!(!staged.exists(), "the staged file moved");
    assert_eq!(
        names(&dir.join("Exports")),
        [".drag", "Song_drums.mid"],
        "the dragged file is the export"
    );
    let single = json!({"midi_mode": "single", "formats": ["midi"]});
    svc.export_all_opts(&sid, &opts(single), None).unwrap();
    assert_eq!(
        std::fs::read(&dragged).unwrap(),
        std::fs::read(dir.join("Exports").join("Song_drums.mid")).unwrap(),
        "the drag file holds the export's bytes"
    );
    // a taken file is not there any more: a second take of it is a 404, a path outside the drag folder a 400
    assert_eq!(
        svc.export_drag_take(&sid, &[staged.display().to_string()])
            .unwrap_err()
            .code,
        404
    );
    assert_eq!(
        svc.export_drag_take(
            &sid,
            &[dir.join("Exports").join("Song_drums.mid").display().to_string()]
        )
        .unwrap_err()
        .code,
        400
    );
    // a cancelled dialog (clear) removes the staged files and leaves Exports/ with no new file
    svc.export_drag_prepare(&sid, &opts(json!({}))).unwrap();
    svc.export_drag_clear(&sid).unwrap();
    assert!(!drag.exists(), "a cancel removes the staged files");
    assert_eq!(names(&dir.join("Exports")), ["Song_drums.mid"]);
    // the dragged file outlives the project: a quit must not take it away from the DAW
    svc.project_save(&sid).unwrap();
    svc.project_close(&sid, false).unwrap();
    assert!(dragged.is_file(), "the dragged file survives the close");
    std::fs::remove_dir_all(&root).unwrap();
}

/// P4-4 regression (the stale-follower e2e failed under a gate load): the drag render runs the
/// export's derivation (`export_all_opts`), so its answer must carry the followers it derived —
/// or the page misses the new hits when the render lands before the real export, which then
/// finds everything fresh and names none.
#[test]
fn export_drag_prepare_answers_the_followers_its_render_derived() {
    let (svc, sid, dir, root) = opened("drag_followers");
    svc.events_set(&sid, "Kick_In", &json!([{"t": 0.25, "v": 100}, {"t": 1.25, "v": 100}]))
        .unwrap();
    svc.track_set_follow(&sid, "Snare_Top", Some("Kick_In")).unwrap();
    assert_eq!(times_of(&svc, &sid, "Snare_Top"), [0.25, 1.25]);
    assert_eq!(svc.export_all(&sid, None).unwrap()["errors"], json!([]));
    let wav = dir.parent().unwrap().join("daw/Snare_Top.wav");
    let away = wav.with_extension("away");
    std::fs::rename(&wav, &away).unwrap();
    svc.project_get(&sid).unwrap();
    let edited = json!([{"t": 0.25, "v": 100}, {"t": 1.25, "v": 100}, {"t": 2.25, "v": 100}]);
    assert_eq!(
        svc.events_set(&sid, "Kick_In", &edited).unwrap(),
        json!({"saved": 3, "followers": []}),
        "the refresh failed: the follower is offline"
    );
    std::fs::rename(&away, &wav).unwrap();
    // the dialog's drag render derives the follower first; its answer names it
    let r = svc.export_drag_prepare(&sid, &opts(json!({}))).unwrap();
    assert_eq!(r["followers"], json!(["Snare_Top"]));
    // the real export then finds everything fresh: the page needed the render's answer
    let all = svc.export_all(&sid, None).unwrap();
    assert_eq!(all["errors"], json!([]));
    assert_eq!(all["followers"], json!([]));
    assert_eq!(times_of(&svc, &sid, "Snare_Top"), [0.25, 1.25, 2.25]);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_new_commands_answer_through_the_bridge() {
    let (svc, sid, _dir, root) = opened("bridge_new");
    let b = Bridge::new(svc, std::path::PathBuf::from("/nonexistent"));
    let json_of = |r: Reply| match r {
        Reply::Json(v) => v,
        Reply::Bytes(_) => panic!("bytes"),
    };
    let out = root.join("b");
    std::fs::create_dir(&out).unwrap();
    let v = json_of(
        b.invoke("export_dest_set", &json!({"sid": sid, "dest": out.to_str().unwrap()}))
            .unwrap(),
    );
    assert_eq!(v["export_dest"], out.to_str().unwrap());
    assert_eq!(
        b.invoke("export_dest_set", &json!({"sid": sid, "dest": 5}))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        json_of(b.invoke("export_dest_set", &json!({"sid": sid})).unwrap())["export_dest"],
        Value::Null
    );
    let v = json_of(
        b.invoke(
            "export_existing",
            &json!({"sid": sid, "dest": out.to_str().unwrap(), "formats": ["midi"]}),
        )
        .unwrap(),
    );
    assert_eq!(v["count"], 0);
    assert_eq!(
        b.invoke("export_existing", &json!({"sid": sid, "formats": []}))
            .unwrap_err()
            .code,
        400
    );
    let v = json_of(
        b.invoke("export_drag_prepare", &json!({"sid": sid, "formats": ["spikes"]}))
            .unwrap(),
    );
    assert_eq!(v["midi"], json!([]));
    assert_eq!(b.invoke("export_drag_prepare", &json!({})).unwrap_err().code, 400);
    let v = json_of(
        b.invoke("export_drag_prepare", &json!({"sid": sid, "clear": true}))
            .unwrap(),
    );
    assert_eq!(v["midi"], json!([]));
    assert!(
        !std::path::Path::new(v["dir"].as_str().unwrap()).exists(),
        "clear removes the staged drag files (#433)"
    );
    std::fs::remove_dir_all(&root).unwrap();
}
