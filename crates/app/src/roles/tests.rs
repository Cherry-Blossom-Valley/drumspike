use super::*;
use crate::ServiceConfig;

fn svc(tag: &str) -> (Service, PathBuf) {
    let d = std::env::temp_dir().join(format!("ds_roles_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let s = Service::new(ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    s.set_home(&d);
    (s, d)
}

#[test]
fn normalize_drops_the_stereo_and_take_suffix() {
    assert_eq!(normalize("k in_02"), "k in");
    assert_eq!(normalize("oh_03.L"), "oh");
    assert_eq!(normalize("OH_03.R"), "oh");
    assert_eq!(normalize("  Hi-Hat_03 "), "hi hat");
    assert_eq!(normalize("t1 HH_01"), "t1 hh");
    assert_eq!(normalize("ohh 1_01"), "ohh 1");
    assert_eq!(normalize("g1"), "g1");
    assert_eq!(normalize("k_in"), "k in");
    assert_eq!(normalize("hh_"), "hh");
    assert_eq!(normalize("Darker Side  MIX 25.2_03.L"), "darker side mix 25 2");
}

#[test]
fn tom_1_tom_2_tom_3_stay_three_different_drums() {
    // #367: a `_<digits>` suffix on a bare tom name is the drum number, not a take suffix.
    assert_eq!(normalize("tom_1"), "tom 1");
    assert_eq!(normalize("tom_2"), "tom 2");
    assert_eq!(normalize("tom_3"), "tom 3");
    assert_eq!(normalize("Tom_2"), "tom 2");
    assert_eq!(normalize("Tom 3"), "tom 3");
    let d1 = drum_of("tom_1").unwrap();
    let d2 = drum_of("tom_2").unwrap();
    let d3 = drum_of("tom_3").unwrap();
    assert_ne!(d1, d2, "{d1} / {d2}");
    assert_ne!(d2, d3, "{d2} / {d3}");
    assert_ne!(d1, d3, "{d1} / {d3}");
    assert_eq!(drum_of("Tom_2"), drum_of("tom_2"));
    assert_eq!(drum_of("Tom 3"), drum_of("tom_3"));
    for (name, role) in [("tom_1", "tom"), ("tom_2", "tom"), ("tom_3", "tom"), ("Tom_2", "tom")] {
        assert_eq!(guess_role(name), Some(role), "{name}");
    }
    // The scheme names keep the take-suffix strip: `k in_02` is kick, `s top_01` snare,
    // `t1` to `t4` unchanged, and two takes of one tom map to the same drum.
    assert_eq!(normalize("k in_02"), "k in");
    assert_eq!(guess_role("k in_02"), Some("kick"));
    assert_eq!(guess_role("s top_01"), Some("snare"));
    for name in ["t1", "t2", "t3", "t4"] {
        assert_eq!(normalize(name), name, "{name}");
        assert_eq!(guess_role(name), Some("tom"), "{name}");
    }
    assert_eq!(drum_of("t1_01"), drum_of("t1_02"));
    assert_eq!(drum_of("t1_01").as_deref(), Some("t 1"));
    // `floor tom_1` and `floor tom_2` are two drums too.
    assert_ne!(drum_of("Floor Tom_1"), drum_of("Floor Tom_2"));
}

#[test]
fn the_scheme_gives_the_role_of_every_listed_name() {
    let table = [
        ("kick", "kick"),
        ("Kick", "kick"),
        ("k in_02", "kick"),
        ("k out_01", "kick"),
        ("k sub_02", "kick"),
        ("snare", "snare"),
        ("s top_02", "snare"),
        ("s bot_01", "snare"),
        ("tom", "tom"),
        ("tom1", "tom"),
        ("t1 HH_01", "tom"),
        ("t2_02", "tom"),
        ("t3_01", "tom"),
        ("t4 Ri_01", "tom"),
        ("hh_03", "hat"),
        ("hat", "hat"),
        ("oh_01", "cymbal"),
        ("OH_01", "cymbal"),
        ("oh_03.L", "cymbal"),
        ("ohh 1_01", "cymbal"),
        ("ohh", "cymbal"),
        ("ohh 5_01", "cymbal"),
        ("ride_01", "cymbal"),
        ("ri_03", "cymbal"),
        ("t 1_03", "tom"),
        ("t 4_03", "tom"),
        ("crash", "cymbal"),
    ];
    for (name, role) in table {
        assert_eq!(guess_role(name), Some(role), "{name}");
    }
}

/// #216: Drum Spike's own export files (synthetic spike, audio spike, layer render, with Pro
/// Tools' `_01` numbered-copy suffixes) get no drum role, so the scan does not offer them as
/// drum tracks. `s_top_02_spike_01` is the owner's file in `Session C`.
#[test]
fn drum_spike_export_names_get_no_role() {
    for name in [
        "s_top_02_spike_01",
        "s_top_02_spike",
        "s_top_02_audio_spike_01",
        "s_top_02_audio_spike",
        "k in_layer",
        "k in_layer_01",
        "S_Top_02_Spike_01",
        "t3_01_layer_02",
        "t4 ri_audio_spike_03",
    ] {
        assert!(is_export_name(name), "{name}");
        assert_eq!(guess_role(name), None, "{name}");
    }
    for name in ["s top_02", "k in_02", "t2_02", "oh_03.L", "hh_01"] {
        assert!(!is_export_name(name), "{name}");
        assert!(guess_role(name).is_some(), "{name}");
    }
    for name in ["spike", "layer_02", "k spike out"] {
        assert!(!is_export_name(name), "{name}");
    }
}

/// The 32 common names of the exit run
/// `Room` and `Room L` are room mics: no role is right for them.
#[test]
fn the_32_common_names_of_the_exit_run_get_their_role() {
    let table: [(&str, Option<&str>); 32] = [
        ("BD", Some("kick")),
        ("China", Some("cymbal")),
        ("Crash", Some("cymbal")),
        ("FT", Some("tom")),
        ("Floor Tom", Some("tom")),
        ("HH", Some("hat")),
        ("Hat", Some("hat")),
        ("Hi-Hat", Some("hat")),
        ("HiHat", Some("hat")),
        ("Kick In", Some("kick")),
        ("Kick Out", Some("kick")),
        ("Kick Sub", Some("kick")),
        ("Kick", Some("kick")),
        ("OH L", Some("cymbal")),
        ("OH R", Some("cymbal")),
        ("Overhead L", Some("cymbal")),
        ("Overheads", Some("cymbal")),
        ("Rack Tom", Some("tom")),
        ("Ride", Some("cymbal")),
        ("Room L", None),
        ("Room", None),
        ("SD", Some("snare")),
        ("Snare Bottom", Some("snare")),
        ("Snare Top", Some("snare")),
        ("Snare", Some("snare")),
        ("Snr Top", Some("snare")),
        ("Tom 1", Some("tom")),
        ("Tom 2", Some("tom")),
        ("Tom Hi", Some("tom")),
        ("Tom1", Some("tom")),
        ("Tom4", Some("tom")),
        ("Toms", Some("tom")),
    ];
    let right = table.iter().filter(|(n, r)| guess_role(n) == *r).count();
    let wrong: Vec<_> = table
        .iter()
        .filter(|(n, r)| guess_role(n) != *r)
        .map(|(n, r)| format!("{n}: {:?}, want {r:?}", guess_role(n)))
        .collect();
    assert_eq!(right, 32, "{wrong:?}");
}

/// X-6: other common abbreviations, with take and stereo suffixes. They count only as the first word of the
/// name, so a room mic named after what it hears (`r  toms soap`, `rev cym`) keeps no role.
#[test]
fn common_abbreviations_give_the_role_as_the_first_word() {
    let table = [
        ("Snr", "snare"),
        ("Snr Bot_02", "snare"),
        ("SN", "snare"),
        ("SD Top_01", "snare"),
        ("Kik_01", "kick"),
        ("Kck", "kick"),
        ("BD In_03.L", "kick"),
        ("Bass Drum", "kick"),
        ("FT 2", "tom"),
        ("FT2_01", "tom"),
        ("HT", "tom"),
        ("MT", "tom"),
        ("LT", "tom"),
        ("Rack", "tom"),
        ("Floor", "tom"),
        ("Toms L", "tom"),
        ("China L_01", "cymbal"),
        ("Splash", "cymbal"),
        ("Cym", "cymbal"),
        ("Cymbals_01.R", "cymbal"),
        ("OHs", "cymbal"),
        ("Hats", "hat"),
    ];
    for (name, role) in table {
        assert_eq!(guess_role(name), Some(role), "{name}");
    }
    for name in [
        "r  toms soap_01",
        "rev cym_03.L",
        "Bass_01",
        "Bass DI",
        "s samp sd3_01",
        "Room",
        "Floorboard",
        "ft0",
        "sdx",
    ] {
        assert_eq!(guess_role(name), None, "{name}");
    }
}

#[test]
fn sample_tracks_are_named_and_drums_come_from_the_name() {
    for name in [
        "t1 sampl_01",
        "T2 Samp_03.L",
        "t samp_01",
        "t trigger_01",
        "k samp savior_01",
        "Tom1Trig",
        "T1 Smpl",
        "Tom1 SSD",
        "t2 ssd5_01",
        "T3 Smp",
    ] {
        assert!(is_sample_name(name), "{name}");
    }
    assert_eq!(
        guess_role("t1 sampl_01"),
        Some("tom"),
        "the scheme keeps its role; the bleed rule skips it"
    );
    for name in [
        "t1 hh_02",
        "t 2_01",
        "t4 Ri_01",
        "k in_01",
        "Tom Hi Top",
        "Floor Tom 1",
        "t1 hh-AtA2",
    ] {
        assert!(!is_sample_name(name), "{name}");
    }
    // two mics of one drum give one drum; numbers keep drums apart
    let same = [
        ("t1 top", "t1 bot"),
        ("t1 hh_02", "T1 Top"),
        ("t1HH_01", "Tom 1"),
        ("tom4", "t4 Ri_01"),
        ("Tom Hi Top", "Tom Hi Bot"),
        ("Hi Tom Top", "Hi Tom Bottom"),
        ("Tom Lo In", "Tom Lo Out"),
        ("Tom A Top", "Tom A Reso"),
        ("Rack Tom Up", "Rack Tom Under"),
        ("FT2 Top", "FT2 Bot"),
        ("Tom 1 Top 421", "Tom 1 Bot e604"),
        ("Tom 1 Top DI", "t1 hh_02"),
        ("t4Ri", "t4 Ri_01"),
        ("Floor Tom 1 Top", "Floor Tom 1 Bot 57"),
    ];
    for (a, b) in same {
        assert!(
            drum_of(a).is_some() && drum_of(a) == drum_of(b),
            "{a} / {b}: {:?} {:?}",
            drum_of(a),
            drum_of(b)
        );
    }
    let apart = [
        ("Floor Tom 1", "Floor Tom 2"),
        ("t1 top", "t2 top"),
        ("Tom Hi Top", "Tom Lo Top"),
        ("Tom A Top", "Tom B Top"),
        ("t 1_01", "t 2_01"),
        ("Rack Tom", "Floor Tom"),
    ];
    for (a, b) in apart {
        assert_ne!(drum_of(a), drum_of(b), "{a} / {b}");
    }
    assert_eq!(drum_of("t1 hh_02").as_deref(), Some("t 1"));
    assert_eq!(drum_of("Floor Tom 2").as_deref(), Some("floor t 2"));
    for (name, drum) in [
        ("Tom 1 Top 421", "t 1"),
        ("Tom 1 Bot e604", "t 1"),
        ("Tom 1 Top DI", "t 1"),
        ("t4Ri", "t 4"),
        ("Floor Tom 1", "floor t 1"),
        ("t1 hh-AtA2", "t 1"),
        ("Tom Hi Top", "t hi"),
    ] {
        assert_eq!(drum_of(name).as_deref(), Some(drum), "{name}");
    }
    for name in ["Top", "Bot_02", "In Out", ""] {
        assert_eq!(drum_of(name), None, "{name}");
    }
    assert_eq!(tokens("t1hh ft22 ab"), ["t", "1", "hh", "ft", "22", "ab"]);
}

#[test]
fn names_outside_the_scheme_have_no_role() {
    for name in [
        "k samp savior_01",
        "k trigger_01",
        "s trigger_01",
        "s samp sd3_01",
        "t trigger_01",
        "r far_01",
        "r mono_01",
        "r k s soap_01",
        "ro_02",
        "Clap_01",
        "Bass_01",
        "GTR 1_01.L",
        "bv h1_02.L",
        "solo_01",
        "t0",
        "t 0_01",
        "t samp_03.L",
        "tomato",
        "",
    ] {
        assert_eq!(guess_role(name), None, "{name}");
    }
}

#[test]
fn the_note_table_equals_the_page() {
    let js =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../static/app.js")).unwrap();
    let line = js
        .lines()
        .find(|l| l.starts_with("const ROLE_DEFAULT_NOTE"))
        .expect("ROLE_DEFAULT_NOTE in app.js");
    for role in ROLES {
        let want = format!("{role}:{}", default_note(role));
        assert!(line.replace(' ', "").contains(&want), "{role}: {line}");
    }
    for role in ROLES {
        assert!(is_role(role));
    }
    assert!(!is_role("bass"));
    assert_eq!(role_color("kick"), "#e05555");
    assert_eq!(role_color("whatever"), "#8a93a6");
}

#[test]
fn a_preset_rule_wins_over_the_scheme_and_the_longest_pattern_wins() {
    let rules = vec![
        Rule {
            pattern: "k samp".into(),
            role: "kick".into(),
            note: 35,
        },
        Rule {
            pattern: "k samp savior".into(),
            role: "other".into(),
            note: 10,
        },
        Rule {
            pattern: "s top".into(),
            role: "tom".into(),
            note: 50,
        },
    ];
    let g = guess("k samp sd3_01", &rules);
    assert_eq!((g.role.as_str(), g.note, g.source), ("kick", 35, Source::Preset));
    let g = guess("k samp savior_01", &rules);
    assert_eq!((g.role.as_str(), g.note), ("other", 10));
    let g = guess("s top_02", &rules);
    assert_eq!((g.role.as_str(), g.note, g.source), ("tom", 50, Source::Preset));
    let g = guess("k in_02", &rules);
    assert_eq!((g.role.as_str(), g.note, g.source), ("kick", 36, Source::Scheme));
    let g = guess("Clap_01", &rules);
    assert_eq!((g.role.as_str(), g.note, g.source), ("other", 36, Source::None));
    // the matched words of the name: the hint of the Inspector shows them
    assert_eq!(guess("k in_02", &[]).matched.as_deref(), Some("k in"));
    assert_eq!(guess("Kick In", &[]).matched.as_deref(), Some("kick"));
    assert_eq!(guess("s top_02", &rules).matched.as_deref(), Some("s top"));
    assert_eq!(guess("t 1 hh", &[]).matched.as_deref(), Some("t 1"));
    assert_eq!(guess("t1 hh", &[]).matched.as_deref(), Some("t1"));
    assert_eq!(guess("bass drum", &[]).matched.as_deref(), Some("bass drum"));
    assert_eq!(guess("Snare Top", &[]).matched.as_deref(), Some("snare"));
    assert_eq!(guess("k samp sd3_01", &rules).matched.as_deref(), Some("k samp"));
    assert_eq!(guess("Clap_01", &[]).matched, None);
    assert_eq!(Source::Preset.as_str(), "preset");
    assert_eq!(Source::Scheme.as_str(), "scheme");
    assert_eq!(Source::None.as_str(), "none");
    // a pattern matches whole words: "k" does not match "kick drum"
    let r = vec![Rule {
        pattern: "k".into(),
        role: "other".into(),
        note: 1,
    }];
    assert_eq!(guess("kick drum", &r).source, Source::Scheme);
}

#[test]
fn rule_from_json_checks_role_note_and_pattern() {
    let r = Rule::from_json(&json!({"pattern": "K In_02", "role": "kick"})).unwrap();
    assert_eq!((r.pattern.as_str(), r.note), ("k in", 36));
    assert_eq!(r.to_json(), json!({"pattern": "k in", "role": "kick", "note": 36}));
    for bad in [
        json!({"pattern": "", "role": "kick"}),
        json!({"pattern": "x", "role": "bass"}),
        json!({"pattern": "x", "role": "kick", "note": 128}),
        json!({"pattern": "x", "role": "kick", "note": "a"}),
    ] {
        assert_eq!(Rule::from_json(&bad).unwrap_err().code, 400, "{bad}");
    }
}

#[test]
fn presets_are_saved_listed_loaded_and_replaced() {
    let (s, d) = svc("presets");
    assert_eq!(s.kit_presets_list().unwrap(), json!([]));
    assert_eq!(s.kit_preset_load("Studio").unwrap_err().code, 404);
    let rules = json!([{"pattern": "k samp", "role": "kick", "note": 35}, {"pattern": "Clap", "role": "other"}]);
    let r = s.kit_preset_save("Studio A/B", &rules).unwrap();
    assert_eq!((r["name"].clone(), r["rules"].clone()), (json!("Studio A/B"), json!(2)));
    assert!(d.join(PRESET_DIR).join("Studio A_B.json").is_file());
    s.kit_preset_save("alpha", &json!([])).unwrap();
    let l = s.kit_presets_list().unwrap();
    assert_eq!(
        l,
        json!([{"name": "alpha", "rules": 0}, {"name": "Studio A/B", "rules": 2}])
    );
    let back = s.kit_preset_load("Studio A/B").unwrap();
    assert_eq!(
        back["rules"][0],
        json!({"pattern": "k samp", "role": "kick", "note": 35})
    );
    assert_eq!(
        back["rules"][1],
        json!({"pattern": "clap", "role": "other", "note": 36})
    );
    s.kit_preset_save("Studio A/B", &json!([])).unwrap();
    assert_eq!(s.kit_preset_load("Studio A/B").unwrap()["rules"], json!([]));
    std::fs::write(d.join(PRESET_DIR).join("junk.json"), "not json").unwrap();
    assert_eq!(s.kit_presets_list().unwrap().as_array().unwrap().len(), 2);
    assert_eq!(s.kit_preset_save("", &json!([])).unwrap_err().code, 400);
    assert_eq!(s.kit_preset_save("x", &json!(3)).unwrap_err().code, 400);
    assert_eq!(
        s.kit_preset_save("x", &json!([{"pattern": "a", "role": "zz"}]))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(s.kit_preset_save("   ", &json!([])).unwrap_err().code, 400);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn presets_need_a_home_folder() {
    let s = Service::new(ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    // the environment of the test run may set DRUMSPIKE_HOME; only the error path is checked then
    if s.home().is_none() {
        assert_eq!(s.kit_presets_list().unwrap_err().code, 409);
    }
}
