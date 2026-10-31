//! P4-14 stage 2: the tom bleed rule in the analysis (`crates/app/src/tom_bleed.rs`): the marks on a synthetic session,
//! the output without the marked hits, the restore, the off cases, the version record and the re-pick. The re-pick
//! P4-17 stage 2: version 2 with the kick and snare close mics as sources.
mod common;

use common::*;
use drumspike_app::Service;
use drumspike_app::tom_bleed::{BLEED_RULE, run_after};
use serde_json::{Map, Value, json};

const SR: u32 = 48_000;
const SECONDS: f64 = 12.0;

/// A decaying tone burst at sample `at`.
/// A drum-like hit: a pitch sweep from twice `freq` down to `freq` (0.5 ms attack, 80 ms decay) and a stick click
/// (noise, 2 ms decay; the same noise at every hit, so the bleed is a scaled copy of the source).
fn burst(x: &mut [f32], at: usize, amp: f32, freq: f64) {
    let mut phase = 0.0f64;
    let mut s = 99u64;
    for i in 0..(SR as usize / 5) {
        let k = at + i;
        if k >= x.len() {
            break;
        }
        let t = i as f64 / f64::from(SR);
        phase += 2.0 * std::f64::consts::PI * freq * (1.0 + (-t / 0.02).exp()) / f64::from(SR);
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let noise = ((s >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0;
        let env = (t / 0.0005).min(1.0) * (-t / 0.08).exp();
        x[k] += amp * (phase.sin() * env + 0.5 * noise * (-t / 0.002).exp()) as f32;
    }
}

/// The own hits of drum `d` (samples): 12 hits 0.9 s apart, every third one soft.
fn own(d: usize) -> Vec<(usize, f32)> {
    (0..12)
        .map(|i| {
            let t = 0.5 + 0.3 * d as f64 + 0.9 * i as f64;
            ((t * f64::from(SR)) as usize, if i % 3 == 2 { 0.1 } else { 0.5 })
        })
        .collect()
}

/// The three tom close mics: each hears the other drums 20 dB down and 1 ms later; a noise floor keeps every mic
/// out of digital silence. Returns the signals and, per mic, the own and the bleed hit samples.
/// The signals, the own hits and the bleed hits per mic.
type Toms = (Vec<Vec<f32>>, Vec<Vec<usize>>, Vec<Vec<usize>>);

fn toms() -> Toms {
    toms_with(0.1)
}

/// [`toms`] with the bleed at `leak` of the source's amplitude.
fn toms_with(leak: f32) -> Toms {
    let n = (SECONDS * f64::from(SR)) as usize;
    let freqs = [140.0, 110.0, 85.0];
    let mut x = vec![vec![0.0f32; n]; 3];
    let mut owns = vec![Vec::new(); 3];
    let mut bleeds = vec![Vec::new(); 3];
    for (d, &freq) in freqs.iter().enumerate() {
        for (at, a) in own(d) {
            for m in 0..3 {
                if m == d {
                    burst(&mut x[m], at, a, freq);
                    owns[m].push(at);
                } else {
                    burst(&mut x[m], at + 48, a * leak, freq);
                    bleeds[m].push(at + 48);
                }
            }
        }
    }
    let mut s = 7u64;
    for v in x.iter_mut().flatten() {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *v += 1e-4 * (((s >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0) as f32;
    }
    for v in owns.iter_mut().chain(bleeds.iter_mut()) {
        v.sort_unstable();
    }
    (x, owns, bleeds)
}

fn spec(name: &'static str, role: &'static str, x: Vec<f32>) -> Spec {
    Spec {
        name,
        role,
        note: 47,
        x,
        sr: SR,
    }
}

fn hits(samples: &[usize]) -> Value {
    let mut v: Vec<usize> = samples.to_vec();
    v.sort_unstable();
    Value::Array(
        v.iter()
            .map(|&s| json!({"t": s as f64 / f64::from(SR), "v": 100}))
            .collect(),
    )
}

fn events(svc: &Service, sid: &str, name: &str) -> Vec<Value> {
    svc.events_get(sid, name).unwrap()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn marked(svc: &Service, sid: &str, name: &str) -> Vec<usize> {
    events(svc, sid, name)
        .iter()
        .filter(|e| e["bleed"] == true)
        .map(|e| (e["t"].as_f64().unwrap() * f64::from(SR)).round() as usize)
        .collect()
}

fn detector(svc: &Service, sid: &str, name: &str) -> Value {
    svc.tracks_get(sid).unwrap()["tracks"][name]["detector"].clone()
}

fn open(svc: &Service, dir: &std::path::Path) -> String {
    svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A project of the three toms, the sample track of tom 1 (its clean signal), a snare and a free track; analysed
/// once (the DSP fallback: every track gets a detector record), then the toms get the hits a detector would list.
fn session(svc: &Service, root: &std::path::Path) -> (String, Vec<Vec<usize>>, Vec<Vec<usize>>, Value) {
    let (x, owns, bleeds) = toms();
    let mut clean = vec![0.0f32; x[0].len()];
    for (at, a) in own(0) {
        burst(&mut clean, at, a, 140.0);
    }
    let dir = project(
        root,
        "Toms",
        &[
            spec("t1", "tom", x[0].clone()),
            spec("t2", "tom", x[1].clone()),
            spec("t3", "tom", x[2].clone()),
            spec("t1 sampl", "tom", clean),
            spec("s top", "snare", kick_signal(&ONSETS, SECONDS, SR)),
            spec("fol", "other", vec![0.01; x[0].len()]),
        ],
    );
    let sid = open(svc, &dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    let st = wait_done(svc, &sid);
    for (m, name) in ["t1", "t2", "t3"].iter().enumerate() {
        let all: Vec<usize> = owns[m].iter().chain(&bleeds[m]).copied().collect();
        svc.events_set(&sid, name, &hits(&all)).unwrap();
    }
    svc.events_set(&sid, "t1 sampl", &hits(&owns[0])).unwrap();
    (sid, owns, bleeds, st)
}

fn targets(names: &[&str], v: u32) -> Vec<(String, u32)> {
    names.iter().map(|n| (n.to_string(), v)).collect()
}

#[test]
fn the_job_runs_the_rule_on_the_tom_close_mics_only() {
    let root = tmp("bleed_job");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let (sid, _, _, st) = session(&svc, &root);
    assert_eq!(st["bleed"]["rule"], BLEED_RULE, "{st}");
    for t in ["t1", "t2", "t3"] {
        assert_eq!(st["tracks"][t]["bleed"]["rule"], BLEED_RULE, "{t}: {st}");
        assert_eq!(detector(&svc, &sid, t)["bleed_rule"], BLEED_RULE, "{t}");
    }
    for t in ["t1 sampl", "s top", "fol"] {
        assert!(st["tracks"][t].get("bleed").is_none(), "{t}: {st}");
        assert!(detector(&svc, &sid, t).get("bleed_rule").is_none(), "{t}");
    }
    assert!(st["bleed"]["tracks"].get("t1 sampl").is_none());
}

#[test]
fn marked_hits_are_the_bleed_and_leave_the_output_until_restored() {
    let root = tmp("bleed_marks");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let (sid, owns, bleeds, _) = session(&svc, &root);
    let all = ["t1", "t2", "t3", "t1 sampl", "s top", "fol"];
    let r = run_after(&svc, &sid, &targets(&all, 1)).unwrap().unwrap();
    assert_eq!(r["on"], true, "{r}");
    assert_eq!(
        r["played"],
        json!(["t1", "t2", "t3"]),
        "the sample track is no close mic: {r}"
    );
    assert_eq!(r["tracks"], json!({"t1": 24, "t2": 24, "t3": 24}));
    assert!(r.get("shared").is_none(), "played pairs stay two drums: {r}");
    for (m, t) in ["t1", "t2", "t3"].iter().enumerate() {
        assert_eq!(marked(&svc, &sid, t), bleeds[m], "{t}: exactly the other drums' hits");
        let det = detector(&svc, &sid, t);
        assert_eq!(det["bleed_rule"], 1);
        assert_eq!(
            det["bleed"],
            json!({"on": true, "reason": null, "removed": 24, "played": true})
        );
    }
    assert!(marked(&svc, &sid, "t1 sampl").is_empty() && detector(&svc, &sid, "t1 sampl").get("bleed").is_none());
    // the output: MIDI, spikes, Export All and a follower take the 12 own hits of t1
    let ev = events(&svc, &sid, "t1");
    assert_eq!(
        svc.export_midi(&sid, "t1", 47, &ev, 960, 120.0).unwrap()["n_events"],
        12
    );
    assert_eq!(svc.export_spikes(&sid, "t1", None).unwrap()["n_events"], 12);
    let out = svc.export_all(&sid, None).unwrap();
    assert!(out["errors"].as_array().is_none_or(std::vec::Vec::is_empty), "{out}");
    svc.track_set_follow(&sid, "fol", Some("t1")).unwrap();
    assert_eq!(events(&svc, &sid, "fol").len(), 12);
    // a restore puts one hit back (the follower follows), a move takes the mark off another
    let idx = |svc: &Service, s: usize| {
        events(svc, &sid, "t1")
            .iter()
            .position(|e| (e["t"].as_f64().unwrap() * f64::from(SR)).round() as usize == s)
            .unwrap()
    };
    let r = svc
        .events_edit(
            &sid,
            "t1",
            &[json!({"op": "restore", "index": idx(&svc, bleeds[0][0])})],
        )
        .unwrap();
    assert_eq!(r["restored"], 1);
    assert!(!marked(&svc, &sid, "t1").contains(&bleeds[0][0]));
    assert_eq!(svc.export_spikes(&sid, "t1", None).unwrap()["n_events"], 13);
    assert_eq!(events(&svc, &sid, "fol").len(), 13);
    let to = (bleeds[0][1] + 10) as f64 / f64::from(SR);
    svc.events_edit(
        &sid,
        "t1",
        &[json!({"op": "move", "index": idx(&svc, bleeds[0][1]), "t": to})],
    )
    .unwrap();
    assert_eq!(marked(&svc, &sid, "t1").len(), 22);
    // the rule never marks an edited hit, and version 0 takes every mark off
    run_after(&svc, &sid, &targets(&["t1"], 1)).unwrap();
    assert_eq!(
        marked(&svc, &sid, "t1").len(),
        22,
        "the moved and the restored hit stay unmarked"
    );
    let r = run_after(&svc, &sid, &targets(&["t1", "t2"], 0)).unwrap().unwrap();
    assert_eq!((r["rule"].clone(), r["on"].clone()), (json!(0), json!(false)));
    assert!(marked(&svc, &sid, "t1").is_empty() && marked(&svc, &sid, "t2").is_empty());
    assert!(detector(&svc, &sid, "t1").get("bleed_rule").is_none());
    assert_eq!(marked(&svc, &sid, "t3").len(), 24, "only the targets change");
    assert_eq!(owns[0].len(), 12);
    assert!(
        run_after(&svc, &sid, &targets(&["s top", "t1 sampl"], 1))
            .unwrap()
            .is_none()
    );
}

#[test]
fn the_rule_is_off_with_one_played_tom_or_a_silent_tom_and_says_why() {
    let root = tmp("bleed_off");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let (sid, owns, _, _) = session(&svc, &root);
    // t2 and t3 without hits: only t1 is played
    for t in ["t2", "t3"] {
        svc.events_set(&sid, t, &json!([])).unwrap();
    }
    let r = run_after(&svc, &sid, &targets(&["t1", "t2", "t3"], 1))
        .unwrap()
        .unwrap();
    assert_eq!(r["on"], false);
    assert_eq!(r["reason"], "fewer than two played tom close mics (played: t1)");
    assert!(marked(&svc, &sid, "t1").is_empty());
    let det = detector(&svc, &sid, "t1");
    assert_eq!(
        (det["bleed_rule"].clone(), det["bleed"]["on"].clone()),
        (json!(1), json!(false))
    );
    assert_eq!(det["bleed"]["reason"], r["reason"]);
    // a tom edited to digital silence between its own hits
    let (mut x, owns3, bleeds3) = toms();
    for (i, v) in x[2].iter_mut().enumerate() {
        if !owns3[2].iter().any(|&s| i + 960 >= s && i < s + SR as usize / 5) {
            *v = 0.0;
        }
    }
    let dir = project(
        &root,
        "Gated",
        &[
            spec("t1", "tom", x[0].clone()),
            spec("t2", "tom", x[1].clone()),
            spec("t3", "tom", x[2].clone()),
        ],
    );
    let gid = open(&svc, &dir);
    svc.analyze_start(&gid, None, None, None).unwrap();
    wait_done(&svc, &gid);
    for (m, t) in ["t1", "t2", "t3"].iter().enumerate() {
        let list: Vec<usize> = if m == 2 {
            owns3[2].clone()
        } else {
            owns3[m].iter().chain(&bleeds3[m]).copied().collect()
        };
        svc.events_set(&gid, t, &hits(&list)).unwrap();
    }
    let r = run_after(&svc, &gid, &targets(&["t1", "t2", "t3"], 1))
        .unwrap()
        .unwrap();
    assert_eq!(r["on"], false, "{r}");
    assert!(r["reason"].as_str().unwrap().contains("digital silence (t3)"), "{r}");
    assert!(marked(&svc, &gid, "t1").is_empty());
    assert_eq!(owns.len(), 3);
}

/// Fix round: a restore lasts. The restored hit stays unmarked through a stored re-pick at another Sensitivity and a
/// Re-analyze with "keep edited" (both keep it as an edited hit, `edited: true`).
#[test]
fn a_restored_hit_stays_unmarked_through_a_repick_and_a_reanalyze_with_keep() {
    let root = tmp("bleed_restore");
    let svc = service_dsp();
    let (x, _, _) = toms_with(0.2); // bleed at -14 dB: the model lists it, the pairs stay two drums (sum -28 dB)
    let dir = project(
        &root,
        "Restore",
        &[
            spec("t1", "tom", x[0].clone()),
            spec("t2", "tom", x[1].clone()),
            spec("t3", "tom", x[2].clone()),
        ],
    );
    let sid = open(&svc, &dir);
    // a high Sensitivity: the model lists the bleed of the other toms
    let at = |v: f64| {
        let mut p = Map::new();
        p.insert("sensitivity".into(), json!(v));
        p.insert("thresholds".into(), json!(drumspike_app::sensitivity::thresholds_of(v)));
        p
    };
    let mut all = Map::new();
    for t in ["t1", "t2", "t3"] {
        all.insert(t.into(), Value::Object(at(150.0)));
    }
    svc.analyze_start(&sid, None, Some(&all), None).unwrap();
    wait_done(&svc, &sid);
    let first = marked(&svc, &sid, "t1");
    assert!(
        !first.is_empty(),
        "the model lists bleed hits of t2 and t3 on t1: {} {}",
        detector(&svc, &sid, "t1"),
        events(&svc, &sid, "t1").len()
    );
    let s = first[0];
    let index = |svc: &Service| {
        events(svc, &sid, "t1")
            .iter()
            .position(|e| (e["t"].as_f64().unwrap() * f64::from(SR)).round() as usize == s)
            .unwrap()
    };
    let r = svc
        .events_edit(&sid, "t1", &[json!({"op": "restore", "index": index(&svc)})])
        .unwrap();
    assert_eq!(r["restored"], 1);
    let kept = |svc: &Service, what: &str| {
        let ev = events(svc, &sid, "t1");
        let hit = &ev[index(svc)];
        assert!(hit.get("bleed").is_none() && hit["edited"] == true, "{what}: {hit}");
        assert!(!marked(svc, &sid, "t1").contains(&s), "{what}");
    };
    kept(&svc, "after the restore");
    // a stored re-pick at another Sensitivity, keeping the edited hits (as the page does)
    let r = svc.track_repick_keep(&sid, "t1", Some(&at(130.0)), true, true).unwrap();
    assert_eq!(r["bleed"]["rule"], BLEED_RULE, "{r}");
    kept(&svc, "after the re-pick");
    // Re-analyze with keep edited
    svc.analyze_start_keep(&sid, None, None, None, None, true).unwrap();
    wait_done(&svc, &sid);
    kept(&svc, "after Re-analyze with keep");
    assert!(
        !marked(&svc, &sid, "t1").is_empty(),
        "the other bleed hits are marked again"
    );
}

/// Fix round 2: the drum comes from the name without the position words. A top and a bottom mic of one tom never
/// explain each other (the bottom mic hears its drum from the other side, polarity turned); a name without a drum
/// switches the rule off with the reason.
#[test]
fn two_mics_of_one_drum_never_explain_each_other_and_an_unknown_drum_turns_the_rule_off() {
    let root = tmp("bleed_names");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let (x, owns, bleeds) = toms();
    let bottom: Vec<f32> = x[0].iter().map(|v| -0.5 * v).collect();
    let dir = project(
        &root,
        "Names",
        &[
            spec("Tom Hi Top", "tom", x[0].clone()),
            spec("Tom Hi Bot", "tom", bottom),
            spec("Tom Lo In", "tom", x[1].clone()),
            spec("Floor Tom 1", "tom", x[2].clone()),
        ],
    );
    let sid = open(&svc, &dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    let names = ["Tom Hi Top", "Tom Hi Bot", "Tom Lo In", "Floor Tom 1"];
    for (i, n) in names.iter().enumerate() {
        let m = if i == 0 { 0 } else { i - 1 };
        let all: Vec<usize> = owns[m].iter().chain(&bleeds[m]).copied().collect();
        svc.events_set(&sid, n, &hits(&all)).unwrap();
    }
    let r = run_after(&svc, &sid, &targets(&names, 1)).unwrap().unwrap();
    assert_eq!(r["on"], true, "{r}");
    for (i, n) in names.iter().enumerate() {
        let m = if i == 0 { 0 } else { i - 1 };
        assert_eq!(
            marked(&svc, &sid, n),
            bleeds[m],
            "{n}: its own drum's hits stay, the others' go"
        );
    }
    // a name that tells no drum
    let dir = project(
        &root,
        "Unknown",
        &[
            spec("t1", "tom", x[0].clone()),
            spec("t2", "tom", x[1].clone()),
            spec("Top", "tom", x[2].clone()),
        ],
    );
    let sid = open(&svc, &dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    for (m, n) in ["t1", "t2", "Top"].iter().enumerate() {
        let all: Vec<usize> = owns[m].iter().chain(&bleeds[m]).copied().collect();
        svc.events_set(&sid, n, &hits(&all)).unwrap();
    }
    let r = run_after(&svc, &sid, &targets(&["t1", "t2", "Top"], 1))
        .unwrap()
        .unwrap();
    assert_eq!(r["on"], false, "{r}");
    assert_eq!(r["reason"], "the drum of Top cannot be told from the track name");
    assert!(marked(&svc, &sid, "t1").is_empty());
}

/// Fix round 3: a bleed-only tom mic with a hot preamp (here a copy of t1's mic, 9 or 15 dB louder) takes no real
/// hit: the shared pair counts as one drum, says so, and the bleed-only mic keeps its false hits.
#[test]
fn a_hot_bleed_only_mic_takes_no_real_hit_and_the_pair_counts_as_one_drum() {
    for db in [9.0f32, 15.0] {
        let root = tmp(&format!("bleed_hot{db}"));
        let svc = service_dsp();
        svc.set_home(root.join("home"));
        let (x, owns, bleeds) = toms();
        let g = 10f32.powf(db / 20.0);
        let mut hot = vec![0.0f32; x[0].len()];
        for (i, v) in x[0].iter().enumerate().take(x[0].len() - 48) {
            hot[i + 48] = v * g;
        }
        let dir = project(
            &root,
            "Hot",
            &[
                spec("t1", "tom", x[0].clone()),
                spec("t2", "tom", x[1].clone()),
                spec("t3", "tom", hot),
            ],
        );
        let sid = open(&svc, &dir);
        svc.analyze_start(&sid, None, None, None).unwrap();
        wait_done(&svc, &sid);
        for (m, n) in ["t1", "t2"].iter().enumerate() {
            let all: Vec<usize> = owns[m].iter().chain(&bleeds[m]).copied().collect();
            svc.events_set(&sid, n, &hits(&all)).unwrap();
        }
        let t3: Vec<usize> = owns[0].iter().map(|s| s + 48).collect();
        svc.events_set(&sid, "t3", &hits(&t3)).unwrap();
        let r = run_after(&svc, &sid, &targets(&["t1", "t2", "t3"], 1))
            .unwrap()
            .unwrap();
        for (m, n) in ["t1", "t2"].iter().enumerate() {
            let lost: Vec<usize> = marked(&svc, &sid, n)
                .into_iter()
                .filter(|s| owns[m].contains(s))
                .collect();
            assert!(lost.is_empty(), "{db} dB: {n} lost real hits {lost:?}");
        }
        assert!(marked(&svc, &sid, "t3").is_empty(), "{db} dB: t3 keeps its false hits");
        assert_eq!(
            r["shared"],
            json!(["t1 and t3 hear one drum: counted as one"]),
            "{db} dB: {r}"
        );
        assert_eq!(
            detector(&svc, &sid, "t1")["bleed"]["shared"],
            json!(["t1 and t3 hear one drum: counted as one"])
        );
    }
}

/// P4-17 stage 2: the three toms of [`toms`] plus a snare (200 Hz, 12 hits between the toms) that reaches the tom mics
/// at 0.3 of its amplitude; the toms reach the snare mic at 0.05. Returns the tom signals, the snare signal, per tom
/// mic the hits a detector lists (own, other toms, snare) and the snare bleed hits, and the snare's own hits.
/// The tom signals, the snare signal, the listed hits and the snare bleed per tom mic, the snare's own hits.
type TomsAndSnare = (Vec<Vec<f32>>, Vec<f32>, Vec<Vec<usize>>, Vec<Vec<usize>>, Vec<usize>);

fn toms_and_snare() -> TomsAndSnare {
    let (mut x, owns, bleeds) = toms();
    let mut snare = vec![0.0f32; x[0].len()];
    let at: Vec<usize> = (0..12)
        .map(|i| ((0.65 + 0.9 * i as f64) * f64::from(SR)) as usize)
        .collect();
    let mut listed: Vec<Vec<usize>> = (0..3)
        .map(|m| owns[m].iter().chain(&bleeds[m]).copied().collect())
        .collect();
    let mut from_snare = vec![Vec::new(); 3];
    for &a in &at {
        burst(&mut snare, a, 0.5, 200.0);
        for m in 0..3 {
            burst(&mut x[m], a + 48, 0.15, 200.0);
            listed[m].push(a + 48);
            from_snare[m].push(a + 48);
        }
    }
    for (d, own_d) in owns.iter().enumerate() {
        for &a in own_d {
            burst(
                &mut snare,
                a + 48,
                if d == 0 { 0.025 } else { 0.02 },
                [140.0, 110.0, 85.0][d],
            );
        }
    }
    let mut s = 5u64;
    for v in &mut snare {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *v += 1e-4 * (((s >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0) as f32;
    }
    (x, snare, listed, from_snare, at)
}

/// A project of the toms and the snare tracks `snares` (name, role), each with the snare signal; analysed once (the
/// DSP fallback), then every track gets the hits a detector would list.
fn sk_session(svc: &Service, root: &std::path::Path, name: &str, snares: &[(&'static str, &'static str)]) -> String {
    let (x, snare, listed, _, at) = toms_and_snare();
    let mut specs = vec![
        spec("t1", "tom", x[0].clone()),
        spec("t2", "tom", x[1].clone()),
        spec("t3", "tom", x[2].clone()),
    ];
    specs.extend(snares.iter().map(|&(n, r)| spec(n, r, snare.clone())));
    let dir = project(root, name, &specs);
    let sid = open(svc, &dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(svc, &sid);
    for (m, t) in ["t1", "t2", "t3"].iter().enumerate() {
        svc.events_set(&sid, t, &hits(&listed[m])).unwrap();
    }
    for &(n, _) in snares {
        svc.events_set(&sid, n, &hits(&at)).unwrap();
    }
    sid
}

#[test]
fn version_2_marks_the_snare_bleed_and_takes_no_sample_or_trigger_track_as_a_source() {
    let root = tmp("bleed_sk");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let (_, _, _, from_snare, _) = toms_and_snare();
    let toms3 = ["t1", "t2", "t3"];
    // the snare close mic is a source; its sample and trigger copies and a kick trigger are not
    let sid = sk_session(
        &svc,
        &root,
        "SK",
        &[
            ("s top", "snare"),
            ("s sampl", "snare"),
            ("k trigger", "kick"),
            ("S Trig", "snare"),
        ],
    );
    let v1 = run_after(&svc, &sid, &targets(&toms3, 1)).unwrap().unwrap();
    assert_eq!(v1["tracks"], json!({"t1": 24, "t2": 24, "t3": 24}), "{v1}");
    assert!(v1.get("sources").is_none(), "version 1 has no sources: {v1}");
    for (m, t) in toms3.iter().enumerate() {
        assert!(
            from_snare[m].iter().all(|h| !marked(&svc, &sid, t).contains(h)),
            "{t}: v1 keeps the snare bleed"
        );
    }
    let v2 = run_after(&svc, &sid, &targets(&toms3, 2)).unwrap().unwrap();
    assert_eq!(v2["on"], true, "{v2}");
    assert_eq!(v2["sources"], json!(["s top"]), "only the close mic: {v2}");
    assert_eq!(v2["tracks"], json!({"t1": 36, "t2": 36, "t3": 36}), "{v2}");
    for (m, t) in toms3.iter().enumerate() {
        assert!(
            from_snare[m].iter().all(|h| marked(&svc, &sid, t).contains(h)),
            "{t}: the snare bleed is marked"
        );
        let det = detector(&svc, &sid, t);
        assert_eq!(det["bleed_rule"], 2);
        assert_eq!(det["bleed"]["sources"], json!(["s top"]));
    }
    // the sources are never targets
    assert!(run_after(&svc, &sid, &targets(&["s top"], 2)).unwrap().is_none());
    assert!(marked(&svc, &sid, "s top").is_empty());
    // without the close mic, the sample and trigger tracks change nothing: version 2 marks what version 1 marks
    let sid = sk_session(
        &svc,
        &root,
        "SK2",
        &[("s sampl", "snare"), ("k trigger", "kick"), ("S Trig", "snare")],
    );
    let v2 = run_after(&svc, &sid, &targets(&toms3, 2)).unwrap().unwrap();
    assert_eq!(v2["tracks"], json!({"t1": 24, "t2": 24, "t3": 24}), "{v2}");
    assert!(v2.get("sources").is_none(), "{v2}");
}

// ---- P4-21: version 3, the tom pair rule

/// The three toms of [`toms`], each gated to digital silence outside its own hits (rule CD is off: "tom tracks edited to
/// digital silence"), a snare close mic with the hits of tom 1, and the lists of every mic: the own hits and the other
/// drums' bleed hits at +1 ms (no signal there). The pair rule needs no bleed between the hits: it marks those.
fn gated_session(svc: &Service, root: &std::path::Path, name: &str) -> (String, Vec<Vec<usize>>, Vec<Vec<usize>>) {
    let (mut x, owns, _) = toms();
    for m in 0..3 {
        for (i, v) in x[m].iter_mut().enumerate() {
            if !owns[m].iter().any(|&s| i + 960 >= s && i < s + SR as usize / 5) {
                *v = 0.0;
            }
        }
    }
    let dir = project(
        root,
        name,
        &[
            spec("t1", "tom", x[0].clone()),
            spec("t2", "tom", x[1].clone()),
            spec("t3", "tom", x[2].clone()),
            spec("s top", "snare", kick_signal(&ONSETS, SECONDS, SR)),
        ],
    );
    let sid = open(svc, &dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(svc, &sid);
    // fewer bleed hits than own hits (the first 4 of each other drum): the mic's median stays an own level, so the
    // bleed hits (no signal there) lie far under it
    let few: Vec<Vec<usize>> = (0..3)
        .map(|m| {
            (0..3)
                .filter(|&d| d != m)
                .flat_map(|d| owns[d][..4].iter().map(|s| s + 48))
                .collect::<Vec<usize>>()
        })
        .map(|mut v| {
            v.sort_unstable();
            v
        })
        .collect();
    for (m, t) in ["t1", "t2", "t3"].iter().enumerate() {
        let all: Vec<usize> = owns[m].iter().chain(&few[m]).copied().collect();
        svc.events_set(&sid, t, &hits(&all)).unwrap();
    }
    // the snare plays with every hit of tom 1 (louder than the toms' bleed), and 5 ms before the bleed of tom 2
    svc.events_set(&sid, "s top", &hits(&owns[0])).unwrap();
    (sid, owns, few)
}

#[test]
fn version_3_marks_the_bleed_hits_with_rule_cd_off_and_no_real_hit_nor_a_snare_hit() {
    let root = tmp("bleed_pairs");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let (sid, owns, bleeds) = gated_session(&svc, &root, "Pairs");
    let toms3 = ["t1", "t2", "t3"];
    let all = ["t1", "t2", "t3", "s top"];
    // version 2: rule CD is off and nothing else runs
    let v2 = run_after(&svc, &sid, &targets(&all, 2)).unwrap().unwrap();
    assert_eq!(v2["on"], false, "{v2}");
    assert!(v2["reason"].as_str().unwrap().contains("digital silence"), "{v2}");
    assert!(v2.get("pairs").is_none(), "version 2 has no pair rule: {v2}");
    for t in toms3 {
        assert!(marked(&svc, &sid, t).is_empty(), "{t}");
        assert!(detector(&svc, &sid, t)["bleed"].get("pairs").is_none());
    }
    // version 3: the pair rule marks exactly the bleed hits, though rule CD is off
    let v3 = run_after(&svc, &sid, &targets(&all, 3)).unwrap().unwrap();
    assert_eq!(v3["rule"], 3);
    assert_eq!(v3["on"], false, "rule CD stays off: {v3}");
    assert_eq!(v3["pairs"]["on"], true, "{v3}");
    assert_eq!(v3["pairs"]["removed"], 24, "{v3}");
    assert_eq!(v3["pairs"]["tracks"], json!({"t1": 8, "t2": 8, "t3": 8}), "{v3}");
    assert_eq!(v3["tracks"], json!({"t1": 8, "t2": 8, "t3": 8}), "{v3}");
    for (m, t) in toms3.iter().enumerate() {
        assert_eq!(
            marked(&svc, &sid, t),
            bleeds[m],
            "{t}: the other drums' hits, no own hit"
        );
        let det = detector(&svc, &sid, t);
        assert_eq!(det["bleed_rule"], 3);
        assert_eq!(det["bleed"]["pairs"], json!({"on": true, "reason": null, "removed": 8}));
        assert_eq!(det["bleed"]["removed"], 8);
    }
    // the snare is no target and no winner: its hits carry no mark, the toms' own hits stay though it is louder
    assert!(marked(&svc, &sid, "s top").is_empty());
    assert!(detector(&svc, &sid, "s top").get("bleed_rule").is_none());
    assert_eq!(events(&svc, &sid, "s top").len(), 12);
    assert!(run_after(&svc, &sid, &targets(&["s top"], 3)).unwrap().is_none());
    // the output without the marked hits (MIDI and spikes count the 12 own hits)
    assert_eq!(svc.export_spikes(&sid, "t2", None).unwrap()["n_events"], 12);
    // a restore puts one back; the rule never marks it again (an edited hit)
    let idx = events(&svc, &sid, "t1")
        .iter()
        .position(|e| (e["t"].as_f64().unwrap() * f64::from(SR)).round() as usize == bleeds[0][0])
        .unwrap();
    svc.events_edit(&sid, "t1", &[json!({"op": "restore", "index": idx})])
        .unwrap();
    assert_eq!(marked(&svc, &sid, "t1").len(), 7);
    run_after(&svc, &sid, &targets(&toms3, 3)).unwrap();
    assert_eq!(marked(&svc, &sid, "t1").len(), 7, "the restored hit stays unmarked");
    // a target of version 2 next to targets of version 3: it takes no pair marks, the others do
    let mixed = [("t1".to_string(), 3), ("t2".to_string(), 2), ("t3".to_string(), 3)];
    run_after(&svc, &sid, &mixed).unwrap();
    assert!(marked(&svc, &sid, "t2").is_empty());
    assert_eq!(detector(&svc, &sid, "t2")["bleed_rule"], 2);
    assert!(detector(&svc, &sid, "t2")["bleed"].get("pairs").is_none());
    assert_eq!(marked(&svc, &sid, "t3"), bleeds[2]);
    assert_eq!(detector(&svc, &sid, "t3")["bleed_rule"], 3);
    // version 0 takes every mark of its target off
    run_after(&svc, &sid, &targets(&["t3"], 0)).unwrap();
    assert!(marked(&svc, &sid, "t3").is_empty());
    assert_eq!(owns[0].len(), 12);
}

#[test]
fn a_cut_hit_takes_no_part_in_the_pair_rule() {
    let root = tmp("bleed_pairs_cut");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let (sid, owns, bleeds) = gated_session(&svc, &root, "PairsCut");
    // t2's first own hit is cut (the loudness cut): out of the output, so t1's bleed hit of it has no winner
    let mut ev = events(&svc, &sid, "t2");
    let first = owns[1][0];
    for e in &mut ev {
        if (e["t"].as_f64().unwrap() * f64::from(SR)).round() as usize == first {
            e["cut"] = json!(true);
        }
    }
    svc.events_set(&sid, "t2", &Value::Array(ev)).unwrap();
    run_after(&svc, &sid, &targets(&["t1", "t2", "t3"], 3)).unwrap();
    let t1 = marked(&svc, &sid, "t1");
    let free: Vec<usize> = bleeds[0].iter().copied().filter(|b| !t1.contains(b)).collect();
    assert_eq!(
        free.len(),
        1,
        "one bleed hit of t1 stays: the one next to the cut hit ({free:?})"
    );
    assert!((free[0] as i64 - first as i64 - 48).abs() <= 0, "{free:?} {first}");
    assert_eq!(marked(&svc, &sid, "t2"), bleeds[1]);
}

#[test]
fn the_pair_rule_is_off_when_a_drum_is_unknown_and_says_why() {
    let root = tmp("bleed_pairs_off");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let (x, owns, bleeds) = toms();
    let dir = project(
        &root,
        "Unknown",
        &[spec("t1", "tom", x[0].clone()), spec("Top", "tom", x[1].clone())],
    );
    let sid = open(&svc, &dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    for (m, t) in ["t1", "Top"].iter().enumerate() {
        let all: Vec<usize> = owns[m].iter().chain(&bleeds[m]).copied().collect();
        svc.events_set(&sid, t, &hits(&all)).unwrap();
    }
    let r = run_after(&svc, &sid, &targets(&["t1", "Top"], 3)).unwrap().unwrap();
    assert_eq!(r["pairs"]["on"], false, "{r}");
    assert!(r["pairs"]["reason"].as_str().unwrap().contains("cannot be told"), "{r}");
    assert!(marked(&svc, &sid, "t1").is_empty());
    let det = detector(&svc, &sid, "t1");
    assert_eq!(det["bleed"]["pairs"]["on"], false);
}

// ---- P4-21 fix round 2: what counts as output for the pair rule

/// The three toms of [`toms`] with rule CD on (every mic lists its own hits and the other drums' bleed hits) and one real
/// coincident stroke: at `T` tom 1 is struck softly (0.2) and tom 2 hard (0.5) 2 ms later. Returns the session id, the
/// hit samples of tom 1's extra hit and tom 2's extra hit, and tom 1's bleed hits.
fn coincident_session(svc: &Service, root: &std::path::Path, name: &str) -> (String, usize, usize, Vec<usize>) {
    let (mut x, owns, bleeds) = toms();
    let t = (11.0 * f64::from(SR)) as usize;
    burst(&mut x[0], t, 0.2, 140.0);
    burst(&mut x[1], t + 96, 0.5, 110.0);
    burst(&mut x[1], t + 48, 0.02, 140.0);
    burst(&mut x[0], t + 144, 0.05, 110.0);
    burst(&mut x[2], t + 48, 0.02, 140.0);
    burst(&mut x[2], t + 144, 0.05, 110.0);
    let dir = project(
        root,
        name,
        &[
            spec("t1", "tom", x[0].clone()),
            spec("t2", "tom", x[1].clone()),
            spec("t3", "tom", x[2].clone()),
        ],
    );
    let sid = open(svc, &dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(svc, &sid);
    for (m, n) in ["t1", "t2", "t3"].iter().enumerate() {
        let mut all: Vec<usize> = owns[m].iter().chain(&bleeds[m]).copied().collect();
        if m == 0 {
            all.push(t);
        }
        if m == 1 {
            all.push(t + 96);
        }
        svc.events_set(&sid, n, &hits(&all)).unwrap();
    }
    (sid, t, t + 96, bleeds[0].clone())
}

fn index_of(svc: &Service, sid: &str, name: &str, sample: usize) -> usize {
    events(svc, sid, name)
        .iter()
        .position(|e| (e["t"].as_f64().unwrap() * f64::from(SR)).round() as usize == sample)
        .unwrap()
}

#[test]
fn with_rule_cd_on_a_cd_mark_does_not_win_and_does_not_count_for_the_median_and_a_restored_hit_does() {
    let root = tmp("bleed_pairs_cd");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let (sid, t1, t2, bleeds1) = coincident_session(&svc, &root, "PairsCd");
    let toms3 = ["t1", "t2", "t3"];
    // version 2: rule CD marks the bleed hits and neither of the coincident strokes
    let v2 = run_after(&svc, &sid, &targets(&toms3, 2)).unwrap().unwrap();
    assert_eq!(v2["on"], true, "{v2}");
    let cd1 = marked(&svc, &sid, "t1");
    assert!(
        cd1.len() >= 20 && cd1.iter().all(|b| bleeds1.contains(b)),
        "CD marks bleed hits only: {cd1:?}"
    );
    assert!(!cd1.contains(&t1));
    assert!(!marked(&svc, &sid, "t2").contains(&t2));
    // version 3: the soft stroke of tom 1 is 8 dB under its mic's median (its output hits are its own hits) and tom 2's
    // hard stroke is at its median: the soft one goes. The CD-marked bleed hits count neither for the median of tom 1
    // (they would lift tom 1's relative level above the floor) nor as winners.
    let v3 = run_after(&svc, &sid, &targets(&toms3, 3)).unwrap().unwrap();
    assert_eq!(v3["pairs"]["on"], true, "{v3}");
    // (the last bleed hit, which rule CD leaves, goes to the pair rule too: two pair marks on tom 1)
    let mut want = bleeds1.clone();
    want.push(t1);
    want.sort_unstable();
    assert_eq!(
        marked(&svc, &sid, "t1"),
        want,
        "the soft stroke is marked by the pair rule: {v3}"
    );
    assert!(!marked(&svc, &sid, "t2").contains(&t2), "the hard stroke stays");
    assert_eq!(v3["pairs"]["tracks"]["t1"], 2, "{v3}");
    // the restored (edited) bleed hits of tom 1 are in the output again: they count for its median (bleed levels), so
    // the soft stroke is no longer 3 dB under it and stays
    let ops: Vec<Value> = bleeds1
        .iter()
        .map(|&b| json!({"op": "restore", "index": index_of(&svc, &sid, "t1", b)}))
        .collect();
    let r = svc.events_edit(&sid, "t1", &ops).unwrap();
    assert_eq!(r["restored"], bleeds1.len(), "{r}");
    run_after(&svc, &sid, &targets(&toms3, 3)).unwrap();
    assert!(
        !marked(&svc, &sid, "t1").contains(&t1),
        "restored hits count for the median: {:?}",
        marked(&svc, &sid, "t1")
    );
    assert!(marked(&svc, &sid, "t1").len() <= 1, "no restored hit is marked again");
}

#[test]
fn a_repick_of_one_track_does_not_let_a_stored_mark_of_another_mic_win() {
    let root = tmp("bleed_pairs_repick");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let (mut x, owns, _) = toms();
    for m in 0..3 {
        for (i, v) in x[m].iter_mut().enumerate() {
            if !owns[m].iter().any(|&s| i + 960 >= s && i < s + SR as usize / 5) {
                *v = 0.0;
            }
        }
    }
    // a hard burst on tom 2 2 ms after tom 1's third own hit (a soft one)
    let soft = owns[0][2];
    burst(&mut x[1], soft + 96, 0.5, 110.0);
    let dir = project(
        &root,
        "PairsRepick",
        &[
            spec("t1", "tom", x[0].clone()),
            spec("t2", "tom", x[1].clone()),
            spec("t3", "tom", x[2].clone()),
        ],
    );
    let sid = open(&svc, &dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    let list = |m: usize, extra: Option<usize>| -> Value {
        let mut v: Vec<usize> = owns[m].clone();
        v.extend(extra);
        let mut ev = hits(&v);
        if let (Some(e), Some(arr)) = (extra, ev.as_array_mut()) {
            for h in arr.iter_mut() {
                if (h["t"].as_f64().unwrap() * f64::from(SR)).round() as usize == e {
                    h["bleed"] = json!(true);
                }
            }
        }
        ev
    };
    for (m, n) in ["t1", "t2", "t3"].iter().enumerate() {
        let ev = if m == 1 {
            list(1, Some(soft + 96))
        } else {
            list(m, None)
        };
        svc.events_set(&sid, n, &ev).unwrap();
    }
    // tom 2's hard burst carries a stored mark: it is out of the output, so it cannot win against tom 1's soft hit
    run_after(&svc, &sid, &targets(&["t1"], 3)).unwrap().unwrap();
    assert!(
        !marked(&svc, &sid, "t1").contains(&soft),
        "a stored mark of another mic does not win: {:?}",
        marked(&svc, &sid, "t1")
    );
    // control: without the stored mark the same hit wins and tom 1's soft hit goes
    svc.events_set(
        &sid,
        "t2",
        &list(1, Some(soft + 96))
            .as_array()
            .map(|a| {
                Value::Array(
                    a.iter()
                        .cloned()
                        .map(|mut e| {
                            e.as_object_mut().unwrap().remove("bleed");
                            e
                        })
                        .collect(),
                )
            })
            .unwrap(),
    )
    .unwrap();
    run_after(&svc, &sid, &targets(&["t1"], 3)).unwrap().unwrap();
    assert!(
        marked(&svc, &sid, "t1").contains(&soft),
        "the hard burst wins when it is in the output"
    );
}

#[test]
fn the_pair_rule_is_off_when_rule_cd_is_off_for_fewer_than_two_played_toms() {
    let root = tmp("bleed_pairs_unplayed");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let (x, owns, _) = toms();
    // tom 2 is no played mic: a quiet bleed of the snare (here at the soft hits of tom 1) with a noise floor
    let mut quiet = vec![0.0f32; x[0].len()];
    let soft_at: Vec<usize> = owns[0]
        .iter()
        .copied()
        .enumerate()
        .filter(|(i, _)| i % 3 == 2)
        .map(|(_, s)| s + 96)
        .collect();
    for &s in &soft_at {
        burst(&mut quiet, s, 0.03, 60.0);
    }
    let mut seed = 11u64;
    for v in &mut quiet {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *v += 3e-3 * (((seed >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0) as f32;
    }
    let dir = project(
        &root,
        "Unplayed",
        &[spec("t1", "tom", x[0].clone()), spec("t2", "tom", quiet)],
    );
    let sid = open(&svc, &dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    svc.events_set(&sid, "t1", &hits(&owns[0])).unwrap();
    svc.events_set(&sid, "t2", &hits(&soft_at)).unwrap();
    let r = run_after(&svc, &sid, &targets(&["t1", "t2"], 3)).unwrap().unwrap();
    assert_eq!(r["on"], false, "{r}");
    assert!(
        r["reason"].as_str().unwrap().starts_with("fewer than two played"),
        "{r}"
    );
    assert_eq!(r["pairs"]["on"], false, "{r}");
    assert!(
        r["pairs"]["reason"].as_str().unwrap().contains("fewer than two played"),
        "{r}"
    );
    assert_eq!(r["pairs"]["removed"], 0, "{r}");
    assert!(marked(&svc, &sid, "t1").is_empty() && marked(&svc, &sid, "t2").is_empty());
    let det = detector(&svc, &sid, "t1");
    assert_eq!(det["bleed"]["pairs"]["on"], false);
    assert!(
        det["bleed"]["pairs"]["reason"]
            .as_str()
            .unwrap()
            .contains("fewer than two played")
    );
}
