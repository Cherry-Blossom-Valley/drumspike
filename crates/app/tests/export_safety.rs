//! R01 (#237, #248): no export may overwrite a protected input. Every exporter builds its list
//! of output paths first and checks the whole list against every protected input (each track's
//! source, one-shots, sampler bounces, for every open project) before it writes anything. The
//! comparison is canonical (symlinks, `..`) and case-insensitive (APFS). One collision refuses
//! the whole export with a message that names the file; nothing is written.
use std::path::Path;

use common::file_sha256;
use drumspike_app::Service;

mod common;
use common::{kick_spec, project, service_dsp, tmp};

fn open(svc: &Service, dir: &Path) -> String {
    svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A track `kick` whose source is `<root>/daw/<file>`: built as track `<stem>`, renamed after
/// the open (the rename is the reproduced bug: the track name builds the output name).
fn renamed_track(root: &Path, stem: &'static str, to: &str) -> (Service, String) {
    let dir = project(root, "Song", &[kick_spec(stem, "kick", 36)]);
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    svc.track_rename(&sid, stem, to).unwrap();
    (svc, sid)
}

#[test]
fn a_renamed_tracks_synthetic_export_refuses_its_own_source_and_leaves_it_unchanged() {
    let root = tmp("exp_safety1");
    let (svc, sid) = renamed_track(&root, "kick_spike", "kick");
    let src = root.join("daw").join("kick_spike.wav");
    let before = file_sha256(&src).unwrap();
    let e = svc
        .export_spikes_to(&sid, "kick", None, Some(root.join("daw").to_str().unwrap()))
        .unwrap_err();
    assert_eq!(e.code, 409, "{e}");
    assert!(e.message.contains("kick_spike.wav"), "{e}");
    assert_eq!(file_sha256(&src).unwrap(), before, "the source is unchanged");
    assert!(!src.parent().unwrap().join("kick_spike_spike.wav").exists());
}

#[test]
fn an_export_that_collides_with_another_tracks_source_is_refused() {
    let root = tmp("exp_safety2");
    // `kick` writes daw/kick_spike.wav, which is the source of the other track
    let dir = project(
        &root,
        "Song",
        &[kick_spec("kick", "kick", 36), kick_spec("kick_spike", "snare", 38)],
    );
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    let src = root.join("daw").join("kick_spike.wav");
    let before = file_sha256(&src).unwrap();
    let e = svc
        .export_spikes_to(&sid, "kick", None, Some(root.join("daw").to_str().unwrap()))
        .unwrap_err();
    assert_eq!(e.code, 409, "{e}");
    assert_eq!(
        file_sha256(&src).unwrap(),
        before,
        "the other track's source is unchanged"
    );
}

#[test]
fn a_destination_reached_through_a_symlink_is_refused() {
    let root = tmp("exp_safety3");
    let dir = project(&root, "Song", &[kick_spec("kick_spike", "kick", 36)]);
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    svc.track_rename(&sid, "kick_spike", "kick").unwrap();
    // the same folder under another name: a symlink next to it
    let real = root.join("daw");
    let link = root.join("daw_link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let src = real.join("kick_spike.wav");
    let before = file_sha256(&src).unwrap();
    let e = svc
        .export_spikes_to(&sid, "kick", None, Some(link.to_str().unwrap()))
        .unwrap_err();
    assert_eq!(e.code, 409, "{e}");
    assert_eq!(
        file_sha256(&src).unwrap(),
        before,
        "the source behind the symlink is unchanged"
    );
}

#[test]
fn a_name_that_differs_only_in_case_is_refused() {
    let root = tmp("exp_safety4");
    // track `Kick_Spike` renamed to `kick`: the output `kick_spike.wav` vs source `Kick_Spike.wav`
    let (svc, sid) = renamed_track(&root, "Kick_Spike", "kick");
    let src = root.join("daw").join("Kick_Spike.wav");
    let before = file_sha256(&src).unwrap();
    let e = svc
        .export_spikes_to(&sid, "kick", None, Some(root.join("daw").to_str().unwrap()))
        .unwrap_err();
    assert_eq!(e.code, 409, "{e}");
    assert_eq!(file_sha256(&src).unwrap(), before, "the source is unchanged");
}

#[test]
fn a_bulk_export_with_one_collision_writes_no_file_at_all() {
    let root = tmp("exp_safety5");
    let dir = project(
        &root,
        "Song",
        &[kick_spec("kick", "kick", 36), kick_spec("kick_spike", "snare", 38)],
    );
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    let dest = root.join("daw");
    // the plan: kick writes daw/kick_spike.wav (the other track's source) and daw/kick_spike.mid;
    // kick_spike writes clean files. One collision refuses the whole export.
    let e = svc
        .export_all_opts(
            &sid,
            &drumspike_app::export::ExportOpts {
                dest: Some(dest.to_str().unwrap().to_string()),
                ..Default::default()
            },
            None,
        )
        .unwrap_err();
    assert_eq!(e.code, 409, "{e}");
    let listing: Vec<_> = std::fs::read_dir(&dest)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(listing.len(), 2, "nothing was written: {listing:?}");
}

#[test]
fn the_audio_spike_export_refuses_a_source_collision_too() {
    let root = tmp("exp_safety6");
    let dir = project(
        &root,
        "Song",
        &[
            kick_spec("kick", "kick", 36),
            kick_spec("kick_audio_spike", "snare", 38),
        ],
    );
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    let src = root.join("daw").join("kick_audio_spike.wav");
    let before = file_sha256(&src).unwrap();
    let e = svc
        .export_all_opts(
            &sid,
            &drumspike_app::export::ExportOpts {
                dest: Some(root.join("daw").to_str().unwrap().to_string()),
                midi: false,
                spikes: false,
                audio: true,
                ..Default::default()
            },
            None,
        )
        .unwrap_err();
    assert_eq!(e.code, 409, "{e}");
    assert_eq!(file_sha256(&src).unwrap(), before, "the source is unchanged");
}

/// The case-sensitivity of the temp file system: true when `<probe>` and `<probe>` in another
/// case name different files. The collision guards assume a case-insensitive FS (APFS).
fn fs_is_case_sensitive(dir: &Path) -> std::io::Result<bool> {
    let a = dir.join(".case_probe");
    std::fs::write(&a, b"")?;
    let insensitive = dir.join(".CASE_PROBE").exists();
    std::fs::remove_file(&a)?;
    Ok(!insensitive)
}

#[test]
fn the_layer_render_refuses_a_protected_file_spelled_in_another_case() {
    let sr = 48_000u32;
    let root = tmp("exp_safety7");
    if fs_is_case_sensitive(&root).expect("a writable temp dir is required") {
        // the guard's collision check is case-insensitive (APFS); on a case-sensitive FS
        // (most Linux FS) `ONE SHOT.wav` is simply a new file and no 409 can happen.
        println!("skipped: case-sensitive file system, the other-case collision cannot happen");
        return;
    }
    let dir = project(
        &root,
        "Song",
        &[kick_spec("k in", "kick", 36), kick_spec("One Shot", "snare", 38)],
    );
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    let ev: Vec<serde_json::Value> = [(0.25, 127), (0.75, 100)]
        .iter()
        .map(|(t, v)| serde_json::json!({"t": t, "v": v}))
        .collect();
    svc.events_set(&sid, "k in", &serde_json::json!(ev)).unwrap();
    // the one-shot of the layer
    let shot_file = root.join("shots").join("st.wav");
    let x: Vec<f32> = (0..48_000)
        .map(|i| ((std::f64::consts::TAU * 240.0 * f64::from(i) / 48_000.0).sin() * 0.2) as f32)
        .collect();
    common::write_wav(&shot_file, sr, &x);
    svc.track_layer_set(&sid, "k in", &serde_json::json!({"path": shot_file.to_str().unwrap()}))
        .unwrap();
    // the save target is another track's source with the name spelled in another case
    let other = root.join("daw").join("one shot.wav");
    let before = file_sha256(&other).unwrap();
    let e = svc
        .export_layer_to(
            &sid,
            "k in",
            Some(root.join("daw").join("ONE SHOT.wav").to_str().unwrap()),
        )
        .unwrap_err();
    assert_eq!(e.code, 409, "{e}");
    assert_eq!(
        file_sha256(&other).unwrap(),
        before,
        "the other track's source is unchanged"
    );
}

#[test]
fn an_export_into_another_open_projects_audio_folder_refuses_its_source() {
    // R01: the guard covers *every open project*: project B's source is protected while A is
    // exported, although the export runs in A's session only.
    let root = tmp("exp_safety9");
    let a = project(&root, "Song", &[kick_spec("kick", "kick", 36)]);
    let b = project(&root, "Other", &[kick_spec("kick_spike", "snare", 38)]);
    let svc = service_dsp();
    let sa = open(&svc, &a);
    let sb = open(&svc, &b);
    let src = root.join("daw").join("kick_spike.wav");
    let before = file_sha256(&src).unwrap();
    // A's track `kick` exports to B's audio folder: the output names B's source
    let e = svc
        .export_spikes_to(&sa, "kick", None, Some(root.join("daw").to_str().unwrap()))
        .unwrap_err();
    assert_eq!(e.code, 409, "{e}");
    assert!(e.message.contains("kick_spike.wav"), "{e}");
    assert_eq!(file_sha256(&src).unwrap(), before, "B's source is unchanged");
    assert!(!src.parent().unwrap().join("kick_spike_spike.wav").exists());
    // the guard is the cross-project one: with B closed the export goes through (and overwrites
    // the file, which nothing protects any more)
    svc.project_close(&sb, false).unwrap();
    svc.export_spikes_to(&sa, "kick", None, Some(root.join("daw").to_str().unwrap()))
        .unwrap();
    // the overwrite the guard had refused before: the output file exists with new content
    assert_ne!(file_sha256(&src).unwrap(), before, "the export overwrote the file");
}

#[test]
fn the_single_track_midi_export_refuses_a_protected_file() {
    // R01: the single-track MIDI export (`export_midi`, commands.rs) checks its one output
    // file `<name>_spike.mid` against the protected files too.
    let sr = 48_000u32;
    let root = tmp("exp_safety10");
    let dir = project(
        &root,
        "Song",
        &[kick_spec("kick", "kick", 36), kick_spec("kick_spike", "snare", 38)],
    );
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    // the bounce of the `kick_spike` track: a WAV that the export output would name
    let bounce = root.join("daw").join("kick_spike.mid");
    let x: Vec<f32> = (0..48_000)
        .map(|i| ((std::f64::consts::TAU * 240.0 * f64::from(i) / f64::from(sr)).sin() * 0.2) as f32)
        .collect();
    common::write_wav(&bounce, sr, &x);
    svc.track_sampler_set(
        &sid,
        "kick_spike",
        &serde_json::json!({"path": bounce.to_str().unwrap()}),
    )
    .unwrap();
    let before = file_sha256(&bounce).unwrap();
    let e = svc
        .export_midi_to(
            &sid,
            "kick",
            36,
            &[],
            480,
            120.0,
            Some(root.join("daw").to_str().unwrap()),
        )
        .unwrap_err();
    assert_eq!(e.code, 409, "{e}");
    assert!(e.message.contains("kick_spike.mid"), "{e}");
    assert_eq!(file_sha256(&bounce).unwrap(), before, "the bounce is unchanged");
    assert!(!root.join("daw").join("kick_spike_spike.mid").exists());
}

#[test]
fn a_clean_export_still_writes_its_files() {
    let root = tmp("exp_safety8");
    let dir = project(&root, "Song", &[kick_spec("kick", "kick", 36)]);
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    let dest = root.join("out");
    std::fs::create_dir_all(&dest).unwrap();
    let r = svc
        .export_all_opts(
            &sid,
            &drumspike_app::export::ExportOpts {
                dest: Some(dest.to_str().unwrap().to_string()),
                ..Default::default()
            },
            None,
        )
        .unwrap();
    assert!(r["errors"].as_array().unwrap().is_empty(), "{r}");
    assert!(dest.join("kick_spike.wav").is_file());
    assert!(dest.join("kick_spike.mid").is_file());
}
