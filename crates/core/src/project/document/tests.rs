use super::*;

const DOC: &str = r##"{"format": 1, "name": "Drum Song 03", "created": 1759950000.123, "modified": null,
    "settings": {"mix_mode": "average", "theme": "dark"}, "order": ["Kick In", "Snare Top"], "view": {},
    "future_key": [1, 2, {"x": null}],
    "tracks": {
     "Kick In": {"role": "kick", "note": 36, "sr": 48000, "n_samples": 10200000, "duration": 212.5,
      "color": "#e05555", "params": {"thresholds": [0.22, 0.24, 0.32, 0.22, 0.30], "min_gap_ms": 20},
      "mix_mode": "average", "lane_height": 120,
      "source": {"mode": "referenced", "path": "/Volumes/S/Kick In.wav", "relative": "../S/Kick In.wav",
                 "size": 61448044, "mtime": 1759950000, "head_sha1": "3f1c", "bext_origination": "2026-09-14T10:31:02Z",
                 "bookmark": "AAEC"},
      "events": [{"t": 0.30000000000000004, "v": 100, "rise": 24.1}, {"t": 0.3333333333333333, "v": 20, "added": true},
                 {"t": 2.675, "v": 127, "orig": 2.6749791666666667, "edited": true, "rise_db": 7.25, "id": "l4", "tag": "x"},
                 {"t": 1e-07, "v": 1}, {"t": 1, "v": 5}]},
     "Empty": {"role": "other", "note": 36, "mix_mode": "ffmpeg", "source": null, "events": []}
    }}"##;

#[test]
fn typed_fields_and_unknown_keys_survive_a_round_trip() {
    let doc = Document::from_json(DOC).unwrap();
    assert_eq!(doc.format, 1);
    assert_eq!(doc.modified, Some(None));
    assert_eq!(doc.extra["future_key"], serde_json::json!([1, 2, {"x": null}]));
    assert_eq!(doc.settings.as_ref().unwrap().extra["theme"], "dark");
    let k = doc.track("Kick In").unwrap();
    assert_eq!(k.extra["lane_height"], 120);
    let src = k.source_ref().unwrap();
    assert_eq!(src.extra["bookmark"], "AAEC");
    assert_eq!(src.bext_origination.as_deref(), Some("2026-09-14T10:31:02Z"));
    let ev = k.events();
    assert_eq!(ev[0].t, 0.1 + 0.2);
    assert_eq!(ev[1].t, 1.0 / 3.0);
    assert_eq!(ev[1].added, Some(true));
    assert_eq!(ev[2].orig, Some(2.6749791666666667));
    assert_eq!(ev[2].edited, Some(true));
    assert_eq!(ev[2].rise_db, Some(7.25));
    assert_eq!(ev[2].id, Some(Value::from("l4")));
    assert_eq!(ev[2].extra["tag"], "x");
    assert_eq!(ev[3].t, 1e-7);
    assert_eq!(ev[4].t, 1.0);
    let e = doc.track("Empty").unwrap();
    assert_eq!(e.source, Some(None));
    assert!(e.events().is_empty());
    // round trip: the same document, every key kept, the order kept
    let text = doc.to_json_pretty();
    let back = Document::from_json(&text).unwrap();
    assert_eq!(back, doc);
    let v: Value = serde_json::from_str(&text).unwrap();
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "format",
            "name",
            "created",
            "modified",
            "settings",
            "order",
            "tracks",
            "view",
            "future_key"
        ]
    );
    let tracks: Vec<&String> = v["tracks"].as_object().unwrap().keys().collect();
    assert_eq!(tracks, ["Kick In", "Empty"]);
    assert!(text.contains("\"source\": null"));
    assert!(text.contains("\"modified\": null"));
    assert!(text.starts_with("{\n \"format\": 1,\n \"name\": \"Drum Song 03\",\n"));
    assert!(text.contains("\n     \"t\": 0.30000000000000004,\n"), "{text}");
    assert!(text.contains("\"t\": 1e-7,"));
}

#[test]
fn a_document_with_every_policy_flag_loads_and_re_saves_byte_identical() {
    // R16 (#40): every flag of the event policy — the typed ones and the extra ones this tree
    // never wrote (`stage8`) — loads and re-saves byte-identically; an old document stays
    // losslessly readable.
    let text = r#"{"format": 1, "name": "Old Flags", "order": ["K"],
        "tracks": {"K": {"role": "kick", "note": 36, "events": [
            {"t": 0.5, "v": 100},
            {"t": 1.0, "v": 100, "cut": true},
            {"t": 1.5, "v": 100, "bleed": true},
            {"t": 2.0, "v": 100, "filtered": true},
            {"t": 2.5, "v": 100, "learned": true},
            {"t": 3.0, "v": 100, "flam": true, "cut": true},
            {"t": 3.5, "v": 100, "roll": true},
            {"t": 4.0, "v": 100, "rescue": true},
            {"t": 4.5, "v": 100, "stage8": true, "detector": {"model": "m", "conf": 0.4}},
            {"t": 5.0, "v": 64, "edited": true},
            {"t": 5.5, "v": 80, "added": true},
            {"t": 6.0, "v": 90, "orig": 5.9, "was": {"t": 6.0}}]}}}"#;
    let doc = Document::from_json(text).unwrap();
    let k = doc.track("K").unwrap();
    let ev = k.events();
    assert_eq!(ev.len(), 12);
    assert_eq!(ev[1].extra["cut"], Value::Bool(true));
    assert_eq!(ev[3].extra["filtered"], Value::Bool(true));
    assert_eq!(ev[5].extra["flam"], Value::Bool(true));
    assert_eq!(ev[8].extra["stage8"], Value::Bool(true));
    assert_eq!(ev[8].extra["detector"]["conf"], 0.4);
    assert_eq!(ev[9].edited, Some(true));
    assert_eq!(ev[11].extra["was"], serde_json::json!({"t": 6.0}));
    let once = doc.to_json_pretty();
    let back = Document::from_json(&once).unwrap();
    assert_eq!(back, doc);
    // the re-save is byte-identical: a load-save cycle changes no byte
    assert_eq!(back.to_json_pretty(), once);
    for flag in ["cut", "bleed", "filtered", "learned", "flam", "roll", "rescue", "stage8"] {
        assert!(once.contains(&format!("\"{flag}\"")), "{flag} survives the round trip");
    }
}

#[test]
fn volume_and_pan_are_optional_typed_fields_that_round_trip() {
    // P3-16: absent in an older document (format 1 stays loadable), typed when written
    let old = r#"{"format": 1, "tracks": {"K": {"role": "kick", "events": []}}}"#;
    let doc = Document::from_json(old).unwrap();
    let k = doc.track("K").unwrap();
    assert_eq!((k.volume_db, k.pan), (None, None));
    assert!(!doc.to_json_pretty().contains("volume_db"), "absent stays absent");
    let new = r#"{"format": 1, "tracks": {"K": {"role": "kick", "volume_db": -6.5, "pan": -0.25},
        "S": {"volume_db": "loud", "pan": null}}}"#;
    let doc = Document::from_json(new).unwrap();
    let k = doc.track("K").unwrap();
    assert_eq!((k.volume_db, k.pan), (Some(-6.5), Some(-0.25)));
    let s = doc.track("S").unwrap();
    assert_eq!((s.volume_db, s.pan), (None, None));
    assert_eq!(s.extra["volume_db"], "loud", "an ill-typed value is kept as written");
    let got: Value = serde_json::from_str(&doc.to_json_pretty()).unwrap();
    let want: Value = serde_json::from_str(new).unwrap();
    assert_eq!(got, want);
}

#[test]
fn follows_is_an_optional_typed_field_that_round_trips() {
    // P3-18: absent in an older document (format 1 stays loadable), typed when written
    let old = r#"{"format": 1, "tracks": {"K": {"role": "kick", "events": []}}}"#;
    let doc = Document::from_json(old).unwrap();
    assert_eq!(doc.track("K").unwrap().follows, None);
    assert!(!doc.to_json_pretty().contains("follows"), "absent stays absent");
    let new = r#"{"format": 1, "tracks": {"K": {"role": "kick"}, "Sub": {"role": "kick", "follows": "K"},
        "S": {"follows": 3}}}"#;
    let doc = Document::from_json(new).unwrap();
    assert_eq!(doc.track("Sub").unwrap().follows.as_deref(), Some("K"));
    let s = doc.track("S").unwrap();
    assert_eq!(s.follows, None);
    assert_eq!(s.extra["follows"], 3, "an ill-typed value is kept as written");
    let got: Value = serde_json::from_str(&doc.to_json_pretty()).unwrap();
    let want: Value = serde_json::from_str(new).unwrap();
    assert_eq!(got, want);
}

#[test]
fn learn_is_an_unknown_track_key_that_round_trips_in_order_and_format_1_stays() {
    // P4-6a: the Learn marks of a track pass through `extra`; an older app carries them
    let text = r#"{"format": 1, "tracks": {"K": {"role": "kick", "learn": {"on": true, "threshold": 0.5,
        "auto": false, "method": "patch-1", "marks": [{"id": "m1", "kind": "this", "t0": 12.05, "t1": 12.61,
        "at": [12.0841666, 12.3527083]}, {"id": "m2", "kind": "not", "t0": 12.982, "t1": 12.982, "at": [12.9823541]}]},
        "events": []}, "S": {"role": "snare"}}}"#;
    let doc = Document::from_json(text).unwrap();
    assert_eq!(doc.format, 1);
    let k = doc.track("K").unwrap();
    assert_eq!(k.extra["learn"]["marks"][1]["kind"], "not");
    assert!(doc.track("S").unwrap().extra.get("learn").is_none());
    let got: Value = serde_json::from_str(&doc.to_json_pretty()).unwrap();
    assert_eq!(got, serde_json::from_str::<Value>(text).unwrap());
    assert_eq!(
        got["tracks"]["K"].as_object().unwrap().keys().next_back().unwrap(),
        "learn",
        "the unknown key is written after the typed ones"
    );
}

#[test]
fn null_and_ill_typed_values_round_trip_through_extra() {
    // values the Python layer loads and keeps (finding F4 of stage 3)
    let text = r#"{"format": 1, "name": null, "created": "yesterday", "modified": "soon", "view": null,
        "settings": {"mix_mode": null}, "order": [1, "K"],
        "tracks": {
         "K": {"role": "kick", "note": 36.0, "color": null, "sr": 48000.0, "params": [],
               "source": {"mode": "referenced", "path": null, "relative": "../k.wav", "size": "12"},
               "events": [{"t": 0.5, "v": 90.0}, {"t": 1.5, "v": null, "added": 1, "orig": "x", "id": null},
                          {"t": 2.0, "edited": "yes", "rise": null, "rise_db": true}]},
         "S": {"role": "snare", "source": "lost", "events": [{"t": "0.5", "v": 3}, {"v": 4}]},
         "E": {"events": null, "mix_mode": 7}}}"#;
    let doc = Document::from_json(text).unwrap();
    assert_eq!(
        (doc.name.as_deref(), doc.created.as_ref(), doc.modified),
        (None, None, None)
    );
    assert_eq!(doc.extra["created"], "yesterday");
    assert_eq!(doc.extra["modified"], "soon");
    assert!(doc.extra["name"].is_null() && doc.extra["view"].is_null());
    assert_eq!(doc.extra["order"], serde_json::json!([1, "K"]));
    assert!(doc.settings.as_ref().unwrap().extra["mix_mode"].is_null());
    let k = doc.track("K").unwrap();
    assert_eq!((k.note, k.sr, k.color.as_deref()), (None, None, None));
    assert_eq!(k.extra["note"], serde_json::json!(36.0));
    let src = k.source_ref().unwrap();
    assert_eq!((src.path.as_deref(), src.relative.as_deref()), (None, Some("../k.wav")));
    assert!(src.extra["path"].is_null());
    assert_eq!(src.extra["size"], "12");
    let ev = k.events();
    assert_eq!((ev[0].t, ev[0].v), (0.5, None));
    assert_eq!(ev[0].extra["v"], serde_json::json!(90.0));
    assert_eq!(
        (ev[1].t, ev[1].added, ev[1].orig, ev[1].id.as_ref()),
        (1.5, None, None, None)
    );
    assert_eq!(ev[1].extra["added"], 1);
    assert!(ev[1].extra["v"].is_null() && ev[1].extra["id"].is_null());
    assert_eq!(ev[2].extra["rise_db"], true);
    // an event without a numeric t: the track keeps its list as written
    let snare = doc.track("S").unwrap();
    assert!(snare.events.is_none() && snare.events().is_empty());
    assert_eq!(
        snare.extra["events"],
        serde_json::json!([{"t": "0.5", "v": 3}, {"v": 4}])
    );
    assert_eq!(snare.source, None);
    assert_eq!(snare.extra["source"], "lost");
    assert!(doc.track("E").unwrap().extra["events"].is_null());
    // every value comes back as written (key order aside), and again after a second pass
    let out = doc.to_json_pretty();
    let want: Value = serde_json::from_str(text).unwrap();
    let got: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(got, want);
    assert_eq!(Document::from_json(&out).unwrap(), doc);
    assert!(out.contains("\"v\": 90.0") && out.contains("\"note\": 36.0"), "{out}");
    assert!(Document::from_json(r#"{"name": "x"}"#).is_err());
    assert!(Document::from_json(r#"{"format": "1"}"#).is_err());
}

#[test]
fn a_field_set_by_code_replaces_an_ill_typed_value() {
    let mut doc = Document::from_json(r#"{"format": 1, "name": 5, "modified": "x", "z": 0}"#).unwrap();
    doc.name = Some("P".into());
    doc.modified = Some(Some(2.5));
    assert_eq!(doc.to_json(), r#"{"format":1,"name":"P","modified":2.5,"z":0}"#);
}

#[test]
fn absent_keys_stay_absent() {
    let doc = Document::from_json(r#"{"format": 1, "name": "x", "tracks": {"K": {"role": "kick"}}}"#).unwrap();
    assert_eq!(doc.modified, None);
    assert_eq!(doc.track("K").unwrap().source, None);
    assert_eq!(
        doc.to_json(),
        r#"{"format":1,"name":"x","tracks":{"K":{"role":"kick"}}}"#
    );
    assert_eq!(doc.default_mix_mode(), "average");
    assert_eq!(doc.track("K").unwrap().mix_mode_or_default(), "ffmpeg");
}

#[test]
fn empty_document_has_every_key_the_python_layer_writes() {
    let d = empty_document("P", "ffmpeg");
    let v: Value = serde_json::from_str(&d.to_json()).unwrap();
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        ["format", "name", "created", "modified", "settings", "order", "tracks", "view"]
    );
    assert!(v["modified"].is_null());
    assert_eq!(v["settings"]["mix_mode"], "ffmpeg");
    assert!(v["created"].as_f64().unwrap() > 1.7e9);
}

#[test]
fn track_order_puts_unlisted_tracks_last_and_drops_unknown_names() {
    let doc = Document::from_json(r#"{"format": 1, "order": ["B", "ghost"], "tracks": {"A": {}, "B": {}, "C": {}}}"#)
        .unwrap();
    assert_eq!(doc.track_order(), ["B", "A", "C"]);
    let none = Document::from_json(r#"{"format": 1}"#).unwrap();
    assert!(none.track_order().is_empty());
    assert_eq!(none.tracks().count(), 0);
}

#[test]
fn event_new_sets_time_and_velocity_only() {
    let e = Event::new(0.5, 77);
    assert_eq!(serde_json::to_string(&e).unwrap(), r#"{"t":0.5,"v":77}"#);
}

#[test]
fn tracks_mut_creates_the_map_and_track_mut_finds_one_track() {
    let mut doc = Document::from_json(r#"{"format": 1}"#).unwrap();
    assert!(doc.tracks.is_none());
    assert!(doc.track_mut("Kick").is_none());
    doc.tracks_mut().insert("Kick".into(), Track::default());
    assert!(doc.tracks.is_some());
    doc.track_mut("Kick").unwrap().note = Some(36);
    assert!(doc.track_mut("Snare").is_none());
    assert_eq!(doc.track("Kick").unwrap().note, Some(36));
    let v: Value = serde_json::from_str(&doc.to_json()).unwrap();
    assert_eq!(v["tracks"]["Kick"]["note"], 36);
    // an existing map is returned as it is, not replaced
    doc.tracks_mut().insert("Snare".into(), Track::default());
    assert_eq!(doc.track_order(), ["Kick", "Snare"]);
}

#[test]
fn path_or_relative_prefers_the_absolute_path() {
    let mut s = Source {
        path: Some("/Volumes/S/Kick.wav".into()),
        relative: Some("../S/Kick.wav".into()),
        ..Source::default()
    };
    assert_eq!(s.path_or_relative(), "/Volumes/S/Kick.wav");
    s.path = None;
    assert_eq!(s.path_or_relative(), "../S/Kick.wav");
    s.relative = None;
    assert_eq!(s.path_or_relative(), "");
}
