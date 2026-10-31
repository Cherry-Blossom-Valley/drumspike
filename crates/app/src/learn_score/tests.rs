use super::*;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("drumspike-learn-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    d
}

#[test]
fn frames_hash_differs_per_list() {
    assert_eq!(frames_hash(&[1, 2, 3]), frames_hash(&[1, 2, 3]));
    assert_ne!(frames_hash(&[1, 2, 3]), frames_hash(&[1, 2, 4]));
    assert_ne!(frames_hash(&[]), frames_hash(&[0]));
    assert_eq!(frames_hash(&[]).len(), 16);
}

#[test]
fn learn_key_names_its_parts() {
    let k = learn_key(&json!({"source": 1}), &[5, 6], "patch-1");
    assert_eq!(k["source"], json!({"source": 1}));
    assert_eq!(k["method"], "patch-1");
    assert_eq!(k["candidates"], frames_hash(&[5, 6]));
}

#[test]
fn write_learn_and_read_learn_round_trip_under_the_same_key() {
    let d = tmp("rt").join("Analysis").join("t1");
    let key = learn_key(&json!({"a": 1}), &[3, 9], "patch-1");
    let feats: Vec<f32> = (0..6).map(|i| i as f32 / 3.0).collect();
    write_learn(&d, &[3, 9], &feats, 3, &key).unwrap();
    assert_eq!(read_learn(&d, &key, 3), Some((vec![3, 9], feats.clone())));
    // another key, another dimension: stale
    assert_eq!(
        read_learn(&d, &learn_key(&json!({"a": 2}), &[3, 9], "patch-1"), 3),
        None
    );
    assert_eq!(read_learn(&d, &learn_key(&json!({"a": 1}), &[3, 9], "mfcc-1"), 3), None);
    assert_eq!(read_learn(&d, &key, 4), None);
    // a cut file
    let b = fs::read(d.join(LEARN_BIN)).unwrap();
    fs::write(d.join(LEARN_BIN), &b[..b.len() - 4]).unwrap();
    assert_eq!(read_learn(&d, &key, 3), None);
    // the key is written last: a missing key is a miss
    write_learn(&d, &[3, 9], &feats, 3, &key).unwrap();
    fs::remove_file(d.join(LEARN_KEY)).unwrap();
    assert_eq!(read_learn(&d, &key, 3), None);
    assert!(write_learn(&d, &[3], &feats, 3, &key).is_err(), "a ragged input");
    let _ = fs::remove_dir_all(tmp("rt"));
}

#[test]
fn candidate_frames_are_unique_and_in_order() {
    let c = vec![
        json!([0.1, 0, 0.5, 10]),
        json!([0.1, 2, 0.4, 20]),
        json!([0.25, 2, 0.4, 20]),
    ];
    assert_eq!(candidate_frames(&c), vec![10, 25]);
}

#[test]
fn score_all_takes_the_automatic_threshold_from_the_solid_frames_only() {
    let r = Recipe::Patch;
    let dim = r.len();
    // 20 hits (attack dimension 0) and 40 bleed candidates of many kinds (one of 30 other dimensions each)
    let mut feats = Vec::new();
    for i in 0..60 {
        let mut v = vec![0f32; dim];
        if i < 20 {
            v[0] = 1.0;
            v[1] = 0.1 * (i % 5) as f32;
        } else {
            v[3 + i % 30] = 1.0;
            v[0] = 0.3;
        }
        feats.extend(v);
    }
    let frames: Vec<u32> = (0..60).map(|i| 10 + 10 * i).collect();
    let this = [0.1, 0.2, 0.3];
    let none = |_: f64| None;
    let all = score_all(
        &frames,
        &feats,
        r,
        &Weights::DEFAULT,
        Centre::Track,
        &[],
        &this,
        &[],
        &none,
    );
    assert!(
        all.auto > similarity::AUTO_LO,
        "the bleed sets a threshold: {}",
        all.auto
    );
    // the bleed does not pass the Sensitivity: nothing under the hits, every candidate stays
    let solid: Vec<bool> = (0..60).map(|i| i < 20).collect();
    let hits = score_all(
        &frames,
        &feats,
        r,
        &Weights::DEFAULT,
        Centre::Track,
        &solid,
        &this,
        &[],
        &none,
    );
    assert_eq!(hits.auto, similarity::AUTO_LO);
    assert_eq!(hits.scores, all.scores, "the scores do not depend on the Sensitivity");
}

#[test]
fn frame_values_take_the_lowest_value_of_a_frame() {
    let c = vec![
        json!([0.1, 0, 0.5, 30]),
        json!([0.1, 2, 0.4, 20]),
        json!([0.25, 2, 0.4, 90]),
    ];
    assert_eq!(frame_values(&c, &[10, 25]), vec![20, 90]);
    assert_eq!(
        frame_values(&c, &[10, 25, 40]),
        vec![20, 90, u64::MAX],
        "a frame without a row never passes"
    );
}

#[test]
fn nearest_frame_reaches_30_ms() {
    let f = [10u32, 20, 50];
    assert_eq!(nearest_frame(&f, 0.104), Some(0));
    assert_eq!(nearest_frame(&f, 0.175), Some(1));
    assert_eq!(nearest_frame(&f, 0.23), Some(1), "30 ms is in reach");
    assert_eq!(nearest_frame(&f, 0.231), None, "31 ms");
    assert_eq!(nearest_frame(&f, 0.16), None, "40 ms from 0.2, 60 ms from 0.1");
    assert_eq!(nearest_frame(&[], 0.1), None);
}

#[test]
fn example_times_split_by_kind() {
    let l = json!({"marks": [{"kind": "this", "at": [1.0, 2.0]}, {"kind": "not", "at": [3.0]}, {"kind": "x", "at": [4.0]}]});
    assert_eq!(example_times(l.as_object().unwrap()), (vec![1.0, 2.0], vec![3.0]));
    assert_eq!(example_times(&serde_json::Map::new()), (vec![], vec![]));
}

#[test]
fn read_learn_checks_the_magic() {
    let d = tmp("magic");
    let key = learn_key(&json!({"a": 1}), &[3], "patch-1");
    write_learn(&d, &[3], &[1.0, 2.0], 2, &key).unwrap();
    assert!(read_learn(&d, &key, 2).is_some());
    let mut b = fs::read(d.join(LEARN_BIN)).unwrap();
    b[0] = b'X';
    fs::write(d.join(LEARN_BIN), &b).unwrap();
    assert_eq!(read_learn(&d, &key, 2), None, "another file of the right length");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn write_learn_writes_the_key_last_and_leaves_no_temporary_file() {
    let d = tmp("keylast");
    let key = learn_key(&json!({"a": 1}), &[3], "patch-1");
    // learn.bin cannot be replaced (a folder with a file in it): the write fails before any key exists
    fs::create_dir_all(d.join(LEARN_BIN).join("x")).unwrap();
    assert!(write_learn(&d, &[3], &[1.0, 2.0], 2, &key).is_err());
    assert!(!d.join(LEARN_KEY).exists(), "a key without its features");
    let left: Vec<_> = fs::read_dir(&d)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name())
        .collect();
    assert_eq!(left, vec![std::ffi::OsString::from(LEARN_BIN)], "{left:?}");
    let _ = fs::remove_dir_all(&d);
    // two writers at once: both succeed, the pair is whole, no temporary file stays
    let d = tmp("concurrent");
    let hs: Vec<_> = (0..8)
        .map(|i| {
            let (d, key) = (d.clone(), key.clone());
            std::thread::spawn(move || write_learn(&d, &[3], &[i as f32, 2.0], 2, &key))
        })
        .collect();
    for h in hs {
        h.join().unwrap().unwrap();
    }
    assert!(read_learn(&d, &key, 2).is_some());
    assert_eq!(fs::read_dir(&d).unwrap().count(), 2, "learn.bin and learn.json only");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn solid_frames_pass_at_the_sensitivity() {
    assert_eq!(
        solid_frames(&[49, 50, 51, u64::MAX], 50.0),
        vec![true, true, false, false]
    );
}

#[test]
fn centre_names_parse_back() {
    for c in [Centre::None, Centre::Track, Centre::Bleed, Centre::Auto] {
        assert_eq!(Centre::parse(c.name()), Some(c));
    }
    assert_eq!(Centre::parse("x"), None);
    assert_eq!(Centre::APP, Centre::Track);
}

/// 20 hits (attack dimension 0), 40 bleed candidates of many kinds, all with a shared colour.
fn toy() -> (Vec<u32>, Vec<f32>) {
    let dim = Recipe::Patch.len();
    let mut feats = Vec::new();
    for i in 0..60 {
        let mut v = vec![0f32; dim];
        v[2] = 2.0; // the mic's colour
        if i < 20 {
            v[0] = 1.0;
            v[1] = 0.1 * (i % 5) as f32;
        } else {
            v[3 + i % 30] = 1.0;
            v[0] = 0.3;
        }
        feats.extend(v);
    }
    ((0..60).map(|i| 10 + 10 * i).collect(), feats)
}

#[test]
fn a_far_example_is_centred_as_the_candidates_are() {
    let (frames, feats) = toy();
    let dim = Recipe::Patch.len();
    let none = |_: f64| None;
    let own = |_: f64| Some(feats[..dim].to_vec()); // the features of candidate 0, given for a far time
    for c in [Centre::Track, Centre::Bleed, Centre::None] {
        let near = score_all(
            &frames,
            &feats,
            Recipe::Patch,
            &Weights::DEFAULT,
            c,
            &[],
            &[0.1],
            &[],
            &none,
        );
        let far = score_all(
            &frames,
            &feats,
            Recipe::Patch,
            &Weights::DEFAULT,
            c,
            &[],
            &[9.0],
            &[],
            &own,
        );
        assert_eq!(near.scores, far.scores, "{c:?}");
    }
}

#[test]
fn the_centring_modes_take_their_means() {
    let (frames, feats) = toy();
    let r = Recipe::Patch;
    let w = Weights::DEFAULT;
    let none = |_: f64| None;
    let this = [0.1, 0.2, 0.3];
    let s = |c: Centre| score_all(&frames, &feats, r, &w, c, &[], &this, &[], &none);
    let (raw, track, bleed, auto) = (s(Centre::None), s(Centre::Track), s(Centre::Bleed), s(Centre::Auto));
    // the shared colour makes every raw score high; centring takes it off
    assert!(raw.scores[30] > 0.8, "{}", raw.scores[30]);
    assert!(track.scores[30] < 0.6 && bleed.scores[30] < 0.6);
    assert_ne!(bleed.scores, track.scores, "the bleed mean is another mean");
    // the hits are a third of the candidates: auto keeps the track centring
    assert_eq!(auto.scores, track.scores);
    // with only the hits passing the Sensitivity they dominate: auto does not centre
    let solid: Vec<bool> = (0..60).map(|i| i < 20).collect();
    let a = score_all(&frames, &feats, r, &w, Centre::Auto, &solid, &this, &[], &none);
    assert_eq!(a.scores, raw.scores);
    // fewer than BLEED_MIN candidates that are no example: no centring
    let few = &feats[..10 * r.len()];
    let b = score_all(&frames[..10], few, r, &w, Centre::Bleed, &[], &this, &[], &none);
    let n = score_all(&frames[..10], few, r, &w, Centre::None, &[], &this, &[], &none);
    assert_eq!(b.scores, n.scores);
}

#[test]
fn score_all_scores_candidates_and_examples() {
    // three candidates in a 2-d toy recipe: use the real recipe length with one hot block values
    let r = Recipe::Patch;
    let dim = r.len();
    let mut feats = vec![0f32; 3 * dim];
    feats[0] = 1.0; // candidate 0: attack dimension 0
    feats[dim] = 1.0; // candidate 1: like candidate 0
    feats[2 * dim + 1] = 1.0; // candidate 2: another dimension
    let frames = [10u32, 20, 30];
    let none = |_: f64| None;
    let s = score_all(
        &frames,
        &feats,
        r,
        &Weights::DEFAULT,
        Centre::None,
        &[],
        &[0.1],
        &[],
        &none,
    );
    assert_eq!(s.scores, vec![1.0, 1.0, 0.5]);
    assert_eq!(s.this, vec![0.5], "one example has nothing to compare with");
    // a negative like candidate 2 pulls it down; a far example without features is left out
    let s = score_all(
        &frames,
        &feats,
        r,
        &Weights::DEFAULT,
        Centre::None,
        &[],
        &[0.1, 0.9],
        &[0.3],
        &none,
    );
    assert_eq!(s.scores, vec![1.0, 1.0, 0.0]);
    assert_eq!((s.this.len(), s.not.len()), (1, 1));
    // the far example with its own features
    let own = |_: f64| {
        let mut v = vec![0f32; dim];
        v[0] = 1.0;
        Some(v)
    };
    let s = score_all(
        &frames,
        &feats,
        r,
        &Weights::DEFAULT,
        Centre::None,
        &[],
        &[0.1, 0.9],
        &[],
        &own,
    );
    assert_eq!(s.this, vec![1.0, 1.0]);
}

#[test]
fn a_forget_mark_alone_takes_only_the_candidates_closer_to_it_than_to_the_confirmed_template() {
    // #293: with no "this" example a candidate goes (score under the 0.05 rule) only when its similarity to the
    // Forget mark is over 0.9 AND above its similarity to the track's template (the mean of the candidates)
    let r = Recipe::Patch;
    let dim = r.len();
    let mut feats = vec![0f32; 5 * dim];
    feats[0] = 1.0; // candidate 0: the Forget mark
    feats[dim] = 1.0; // candidate 1: a look-alike of the mark, the same vector
    for c in [2, 3] {
        // candidates 2 and 3: 0.95 of the mark's dimension and 0.31 of another; their similarity to the mark is
        // over 0.9, but the track's other hits lie closer to them than the mark does
        feats[c * dim] = 0.95;
        feats[c * dim + 1] = 0.31;
    }
    feats[4 * dim + 1] = 1.0; // candidate 4: another dimension (far from the mark)
    let frames = [10u32, 20, 30, 40, 50];
    let none = |_: f64| None;
    let s = score_all(
        &frames,
        &feats,
        r,
        &Weights::DEFAULT,
        Centre::None,
        &[],
        &[],
        &[0.1],
        &none,
    );
    assert!(
        s.scores[0] < 0.05 && s.scores[1] < 0.05,
        "the mark and its look-alike go: {:?}",
        s.scores
    );
    assert_eq!(s.scores[2], 0.5, "near the mark but closer to the template: stays");
    assert_eq!(s.scores[4], 0.5, "far from the mark: stays");
}
