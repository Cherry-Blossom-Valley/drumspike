//! P3-18: a track that follows another track's hits (`crates/app/src/follow.rs`), and the
//! 0-hit retry of the analysis job (`jobs.rs`). Through `drumspike_app::Service`, no Tauri.

mod common;

use common::*;
use drumspike_app::{LIFECYCLE_COMMANDS, Service};
use drumspike_core::project::{load_project, save_project};
use serde_json::{Value, json};

const SR: u32 = 48_000;

/// A kick in (full level) and a kick sub (the same hits 10 ms earlier at -24 dB), referenced.
fn kick_and_sub(tag: &str) -> (Service, String, std::path::PathBuf) {
    let root = tmp(tag);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sub_onsets: Vec<f64> = ONSETS.iter().map(|t| t - 0.010).collect();
    let sub: Vec<f32> = kick_signal(&sub_onsets, 3.5, SR)
        .iter()
        .map(|v| v * 10f32.powf(-24.0 / 20.0))
        .collect();
    let dir = project(
        &root,
        "Follow",
        &[
            kick_spec("k in", "kick", 36),
            Spec {
                name: "k sub",
                role: "kick",
                note: 36,
                x: sub,
                sr: SR,
            },
            kick_spec("s top", "snare", 38),
        ],
    );
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    (svc, sid, dir)
}

fn events(svc: &Service, sid: &str, name: &str) -> Vec<Value> {
    svc.events_get(sid, name).unwrap()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn times(ev: &[Value]) -> Vec<f64> {
    ev.iter().map(|e| e["t"].as_f64().unwrap()).collect()
}

fn vels(ev: &[Value]) -> Vec<i64> {
    ev.iter().map(|e| e["v"].as_i64().unwrap()).collect()
}

#[test]
fn a_follower_takes_the_positions_of_the_source_and_its_own_velocities() {
    let (svc, sid, _dir) = kick_and_sub("positions");
    svc.analyze_start(&sid, Some(vec!["k in".into()]), None, None).unwrap();
    let st = wait_done(&svc, &sid);
    assert_eq!(st["tracks"]["k in"]["state"], "done", "{st}");
    let src = events(&svc, &sid, "k in");
    assert_eq!(src.len(), 6);
    let r = svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    assert_eq!(r, json!({"follows": "k in", "n_events": 6}));
    let sub = events(&svc, &sid, "k sub");
    assert_eq!(times(&sub), times(&src), "the same sample positions");
    // the velocities come from the sub's own signal: -24 dB under the kick in, so lower
    let own = svc.velocities_recompute(&sid, "k sub", Some(&sub)).unwrap();
    assert_eq!(vels(own["events"].as_array().unwrap()), vels(&sub));
    assert!(
        vels(&sub).iter().zip(vels(&src)).all(|(s, k)| s < &k),
        "{:?} {:?}",
        vels(&sub),
        vels(&src)
    );
    assert!(sub.iter().all(|e| e.get("rise").is_none()));
    let t = svc.tracks_get(&sid).unwrap();
    assert_eq!(t["tracks"]["k sub"]["follows"], "k in");
    assert_eq!(t["tracks"]["k sub"]["n_events"], 6);
    assert!(t["tracks"]["k in"].get("follows").is_none());
    // the same follow again derives again, without a second backup error; own detection keeps the hits
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    let r = svc.track_set_follow(&sid, "k sub", None).unwrap();
    assert_eq!(r, json!({"follows": null, "n_events": 6}));
    assert!(
        svc.tracks_get(&sid).unwrap()["tracks"]["k sub"]
            .get("follows")
            .is_none()
    );
}

#[test]
fn the_rules_refuse_itself_chains_sources_with_followers_and_unknown_tracks() {
    let (svc, sid, _dir) = kick_and_sub("rules");
    let e = svc.track_set_follow(&sid, "k sub", Some("k sub")).unwrap_err();
    assert_eq!((e.code, e.message.as_str()), (400, "a track cannot follow itself"));
    let e = svc.track_set_follow(&sid, "k sub", Some("nope")).unwrap_err();
    assert_eq!((e.code, e.message.as_str()), (400, "unknown track nope"));
    assert_eq!(svc.track_set_follow(&sid, "nope", Some("k in")).unwrap_err().code, 400);
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    // no chains: a follower is no source
    let e = svc.track_set_follow(&sid, "s top", Some("k sub")).unwrap_err();
    assert_eq!(e.message, "k sub follows k in: follow k in instead");
    // a source with followers cannot follow (k in has k sub); this rule comes before the chain rule
    let e = svc.track_set_follow(&sid, "k in", Some("k sub")).unwrap_err();
    assert_eq!(
        e.message,
        "k in has followers (k sub): set them to their own detection first"
    );
    let e = svc.track_set_follow(&sid, "k in", Some("s top")).unwrap_err();
    assert_eq!(
        e.message,
        "k in has followers (k sub): set them to their own detection first"
    );
    svc.track_set_follow(&sid, "s top", Some("k in")).unwrap();
    assert_eq!(svc.tracks_get(&sid).unwrap()["tracks"]["s top"]["follows"], "k in");
    // the follower's hits are read-only
    let e = svc
        .events_set(&sid, "k sub", &json!([{"t": 0.1, "v": 90}]))
        .unwrap_err();
    assert_eq!(
        (e.code, e.message.as_str()),
        (409, "track k sub follows k in: edit the hits of k in")
    );
    assert!(LIFECYCLE_COMMANDS.contains(&"track_set_follow"));
}

#[test]
fn an_edit_of_the_source_reaches_the_follower_and_an_analysis_of_the_follower_derives() {
    let (svc, sid, _dir) = kick_and_sub("edit");
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    assert!(events(&svc, &sid, "k sub").is_empty(), "the source has no hits yet");
    let r = svc
        .events_set(
            &sid,
            "k in",
            &json!([{"t": 0.25, "v": 100, "id": "a"}, {"t": 1.0, "v": 90, "id": "b", "added": true}]),
        )
        .unwrap();
    assert_eq!(r, json!({"saved": 2, "followers": ["k sub"]}));
    let sub = events(&svc, &sid, "k sub");
    assert_eq!(times(&sub), [0.25, 1.0]);
    assert!(sub.iter().all(|e| e.get("added").is_none() && e.get("id").is_none()));
    // Analyze All: the follower derives (no detector), the source detects and refreshes it
    svc.analyze_start(&sid, None, None, None).unwrap();
    let st = wait_done(&svc, &sid);
    let f = &st["tracks"]["k sub"];
    assert_eq!(
        (f["state"].as_str(), f["follows"].as_str(), f["n_events"].as_i64()),
        (Some("done"), Some("k in"), Some(6)),
        "{st}"
    );
    assert!(f["detector"].is_null());
    assert_eq!(st["tracks"]["k in"]["followers"], json!(["k sub"]));
    assert_eq!(times(&events(&svc, &sid, "k sub")), times(&events(&svc, &sid, "k in")));
    assert_eq!(events(&svc, &sid, "k sub").len(), 6);
}

#[test]
fn a_rename_of_the_source_follows_and_a_gone_source_clears_the_follower() {
    let (svc, sid, dir) = kick_and_sub("gone");
    svc.events_set(&sid, "k in", &json!([{"t": 0.5, "v": 100}])).unwrap();
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    svc.track_rename(&sid, "k in", "Kick In").unwrap();
    assert_eq!(svc.tracks_get(&sid).unwrap()["tracks"]["k sub"]["follows"], "Kick In");
    svc.events_set(&sid, "Kick In", &json!([{"t": 0.5, "v": 100}, {"t": 1.5, "v": 100}]))
        .unwrap();
    assert_eq!(times(&events(&svc, &sid, "k sub")), [0.5, 1.5]);
    // persisted: the document carries the key; a reload keeps the follow
    svc.project_save(&sid).unwrap();
    let mut proj = load_project(&dir).unwrap();
    assert_eq!(proj.doc.track("k sub").unwrap().follows.as_deref(), Some("Kick In"));
    assert!(proj.doc.track("Kick In").unwrap().follows.is_none());
    let text = std::fs::read_to_string(proj.path()).unwrap();
    assert!(text.contains("\"follows\": \"Kick In\""), "{text}");
    // the source goes away (no command removes a track yet: the record is taken out of the
    // document on disk, the project opened again): the follower's next derivation gives it
    // its own detection back, no hits
    proj.doc.tracks_mut().shift_remove("Kick In");
    save_project(&mut proj).unwrap();
    svc.project_close(&sid, true).unwrap();
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(svc.tracks_get(&sid).unwrap()["tracks"]["k sub"]["follows"], "Kick In");
    svc.analyze_start(&sid, Some(vec!["k sub".into()]), None, None).unwrap();
    let st = wait_done(&svc, &sid);
    let f = &st["tracks"]["k sub"];
    assert_eq!(
        (f["state"].as_str(), f["n_events"].as_i64(), f["source_gone"].as_bool()),
        (Some("done"), Some(0), Some(true)),
        "{st}"
    );
    assert!(f["follows"].is_null());
    assert!(events(&svc, &sid, "k sub").is_empty());
    assert!(
        svc.tracks_get(&sid).unwrap()["tracks"]["k sub"]
            .get("follows")
            .is_none()
    );
}

#[test]
fn a_silent_track_reports_no_hits_and_the_dsp_fallback_does_not_retry() {
    let root = tmp("silent");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project(
        &root,
        "Silent",
        &[Spec {
            name: "k sub",
            role: "kick",
            note: 36,
            x: vec![0.0; SR as usize * 2],
            sr: SR,
        }],
    );
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.analyze_start(&sid, None, None, None).unwrap();
    let st = wait_done(&svc, &sid);
    let t = &st["tracks"]["k sub"];
    assert_eq!(
        (t["state"].as_str(), t["n_events"].as_i64(), t["detector"].as_str()),
        (Some("done"), Some(0), Some("rust-dsp")),
        "{st}"
    );
    assert_eq!(t["weak_signal"], json!({"from": 50.0, "to": null}));
    let p = &svc.tracks_get(&sid).unwrap()["tracks"]["k sub"]["params"];
    assert_eq!(p["weak_signal"], json!({"from": 50.0, "to": null}));
    assert!(
        p.get("thresholds").is_none() && p.get("sensitivity").is_none(),
        "the DSP path changes no threshold: {p}"
    );
}

/// P4-4 (B2): the follower's audio is offline while its source is edited, so the refresh of
/// `events_set` fails and the follower keeps its old hits. Export All and the single exports derive
/// it again once its audio is back (they wrote the old hits before); while it is still offline the
/// export of that track is a 409 and writes no file. `expected_times` is what the follower must hold.
#[test]
fn a_follower_stale_after_an_offline_edit_is_derived_again_at_export() {
    use drumspike_core::midi::{MidiEvent, midi_bytes};
    let (svc, sid, dir) = kick_and_sub("stale_export");
    svc.events_set(&sid, "k in", &json!([{"t": 0.25, "v": 100}, {"t": 1.25, "v": 100}]))
        .unwrap();
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    assert_eq!(times(&events(&svc, &sid, "k sub")), [0.25, 1.25]);
    assert_eq!(
        svc.export_all(&sid, None).unwrap()["errors"],
        json!([]),
        "an earlier export"
    );
    let earlier = std::fs::read(dir.join("Exports/k sub_spike.mid")).unwrap();
    let wav = dir.parent().unwrap().join("daw/k sub.wav");
    let away = wav.with_extension("away");
    std::fs::rename(&wav, &away).unwrap();
    svc.project_get(&sid).unwrap();
    let edited = json!([{"t": 0.25, "v": 100}, {"t": 1.25, "v": 100}, {"t": 2.25, "v": 100}]);
    let r = svc.events_set(&sid, "k in", &edited).unwrap();
    assert_eq!(
        r,
        json!({"saved": 3, "followers": []}),
        "the refresh failed: the follower is offline"
    );
    assert_eq!(times(&events(&svc, &sid, "k sub")), [0.25, 1.25], "stale");
    // still offline at the export: the follower's MIDI file is not written with the old hits
    let all = svc.export_all(&sid, None).unwrap();
    let errs = all["errors"].as_array().unwrap();
    assert_eq!(errs.len(), 1, "{all}");
    assert_eq!(
        (errs[0]["name"].as_str(), errs[0]["code"].as_u64()),
        (Some("k sub"), Some(409))
    );
    let msg = errs[0]["message"].as_str().unwrap();
    assert!(msg.contains("follows k in"), "{all}");
    assert!(
        msg.ends_with(
            "Exports/k sub_spike.mid and Exports/k sub_spike.wav of an earlier export still hold its old hits"
        ),
        "the message names the old files: {msg}"
    );
    assert_eq!(
        std::fs::read(dir.join("Exports/k sub_spike.mid")).unwrap(),
        earlier,
        "not written again"
    );
    assert_eq!(
        svc.export_midi(&sid, "k sub", 36, &[], 24000, 120.0).unwrap_err().code,
        409
    );
    // back online: the export derives it again and writes the edited hits
    std::fs::rename(&away, &wav).unwrap();
    let all = svc.export_all(&sid, None).unwrap();
    assert_eq!(all["errors"], json!([]), "{all}");
    assert_eq!(all["followers"], json!(["k sub"]));
    let sub = events(&svc, &sid, "k sub");
    assert_eq!(times(&sub), [0.25, 1.25, 2.25]);
    let want: Vec<MidiEvent> = sub
        .iter()
        .map(|e| MidiEvent {
            t: e["t"].as_f64().unwrap(),
            v: e["v"].as_i64().unwrap(),
        })
        .collect();
    assert_eq!(
        std::fs::read(dir.join("Exports/k sub_spike.mid")).unwrap(),
        midi_bytes(&want, 36, 24000, 120.0).unwrap()
    );
    // fresh now: nothing is derived again, and a single export ignores the page's (stale) list
    assert_eq!(svc.export_all(&sid, None).unwrap()["followers"], json!([]));
    let stale_page = json!([{"t": 0.25, "v": 1}]);
    let m = svc
        .export_midi(&sid, "k sub", 36, stale_page.as_array().unwrap(), 24000, 120.0)
        .unwrap();
    assert_eq!(m["n_events"], 3);
    let s = svc
        .export_spikes(&sid, "k sub", stale_page.as_array().map(Vec::as_slice))
        .unwrap();
    assert_eq!(s["n_events"], 3);
    assert!(s.get("followers").is_none());
}

/// P4-4: a follower whose hits match its source's uncut hits is not derived again at an export
/// (the project stays clean), and a track that follows nothing is never derived.
#[test]
fn an_export_leaves_a_fresh_follower_and_its_project_alone() {
    let (svc, sid, _dir) = kick_and_sub("fresh");
    svc.events_set(
        &sid,
        "k in",
        &json!([{"t": 0.25, "v": 100}, {"t": 0.5, "v": 90, "cut": true}]),
    )
    .unwrap();
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    assert_eq!(
        times(&events(&svc, &sid, "k sub")),
        [0.25],
        "the cut hit is not followed"
    );
    svc.project_save(&sid).unwrap();
    let all = svc.export_all(&sid, None).unwrap();
    assert_eq!(
        (all["followers"].clone(), all["errors"].clone()),
        (json!([]), json!([]))
    );
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], false, "nothing was derived");
}

/// P4-4 fix round (B2 at its cause): a follower that went stale while its audio was offline is
/// derived again as soon as its audio is back: by a relink of it (the answer lists it) and by
/// `tracks_get` (the page loads the project with it), not only by an export.
#[test]
fn a_stale_follower_is_derived_again_when_its_audio_comes_back() {
    let (svc, sid, dir) = kick_and_sub("stale_back");
    svc.events_set(&sid, "k in", &json!([{"t": 0.25, "v": 100}])).unwrap();
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    let wav = dir.parent().unwrap().join("daw/k sub.wav");
    let moved = dir.parent().unwrap().join("moved/k sub.wav");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&wav, &moved).unwrap();
    svc.project_get(&sid).unwrap();
    svc.events_set(&sid, "k in", &json!([{"t": 0.25, "v": 100}, {"t": 1.25, "v": 100}]))
        .unwrap();
    assert_eq!(times(&events(&svc, &sid, "k sub")), [0.25], "stale while offline");
    // a relink to the moved file derives it
    let r = svc
        .project_relink(&sid, "k sub", &moved.display().to_string(), false)
        .unwrap();
    assert_eq!(r["followers"], json!(["k sub"]), "{r}");
    assert_eq!(times(&events(&svc, &sid, "k sub")), [0.25, 1.25]);
    // offline again, edited again, then the file is back where the record points: tracks_get derives it
    std::fs::rename(&moved, moved.with_extension("away")).unwrap();
    svc.project_get(&sid).unwrap();
    svc.events_set(&sid, "k in", &json!([{"t": 2.25, "v": 100}])).unwrap();
    assert_eq!(times(&events(&svc, &sid, "k sub")), [0.25, 1.25]);
    std::fs::rename(moved.with_extension("away"), &moved).unwrap();
    let t = svc.tracks_get(&sid).unwrap();
    assert_eq!(t["tracks"]["k sub"]["offline"], false, "{t}");
    assert_eq!(times(&events(&svc, &sid, "k sub")), [2.25]);
}

/// P4-4 fix round 2: `tracks_get` refreshes a stale follower under the store guard: while a store
/// (a `track_velocity_set`) holds it, the refresh does nothing, so it cannot overwrite the store's work.
#[test]
fn a_refresh_does_nothing_while_a_store_holds_the_guard() {
    let (svc, sid, dir) = kick_and_sub("refresh_guard");
    svc.events_set(&sid, "k in", &json!([{"t": 0.25, "v": 100}])).unwrap();
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    let wav = dir.parent().unwrap().join("daw/k sub.wav");
    let away = wav.with_extension("away");
    std::fs::rename(&wav, &away).unwrap();
    svc.project_get(&sid).unwrap();
    svc.events_set(&sid, "k in", &json!([{"t": 2.25, "v": 100}])).unwrap();
    std::fs::rename(&away, &wav).unwrap();
    {
        let _store = svc.jobs().begin_store(&sid).expect("free");
        svc.tracks_get(&sid).unwrap();
        assert_eq!(
            times(&events(&svc, &sid, "k sub")),
            [0.25],
            "no refresh under the guard"
        );
    }
    svc.tracks_get(&sid).unwrap();
    assert_eq!(
        times(&events(&svc, &sid, "k sub")),
        [2.25],
        "refreshed once the guard is free"
    );
    assert!(svc.jobs().begin_store(&sid).is_some(), "the refresh released the guard");
}
