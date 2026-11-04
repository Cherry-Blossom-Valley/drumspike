//! R16 (#40): the event policy in one place. One predicate decides "leaves the output"
//! (`is_out` / [`event_out`]), one decides "kept" ([`is_kept`]), and the tempo exception is
//! named ([`TEMPO_KEEPS_FILTERED`]). The duplicates call this module: `export::is_cut`
//! (MIDI, spikes, Export All, the click), `histogram::event_cut` (flam, loudness),
//! `confidence::is_out`, `tempo::is_cut`. The page's copies (`isManual`, `isKept`, `hgIsCut`,
//! `lnGreyHit` in static/app.js) and the Rust side read one truth table
//! (tests/fixtures/event-policy/table.json), so the two mechanisms cannot drift.

use drumspike_core::Event;
use serde_json::{Map, Value};

/// R4: a per-event mark. A mark is a key of the event (in `extra` for a typed [`Event`]) that is
/// on only as `true`: absent, `false` or any other value is off, and absent is not the same as
/// `false` in the saved bytes. [`is_marked`], [`mark`] and [`clear`] are the one place that
/// knows the key names; each call site keeps its own predicate around them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// The loudness cut (`histogram::mark_cut_events`).
    Cut,
    /// The tom bleed and cross bleed rules.
    Bleed,
    /// The old Learn filter (#281 D16).
    Filtered,
    /// #242: the user holds the hit in the output.
    Keep,
    /// #242: the user takes the hit out of the output.
    Deactivated,
    /// The second stroke the flam rule added.
    Flam,
    /// A hit the Learn Add rule added.
    Learned,
    /// A roll fill hit.
    Roll,
    /// A kick rescue hit.
    Rescue,
    /// A stage 8 tom stroke.
    Stage8,
}

impl Mark {
    /// Every mark, in the order of the enum.
    pub const ALL: [Mark; 10] = [
        Mark::Cut,
        Mark::Bleed,
        Mark::Filtered,
        Mark::Keep,
        Mark::Deactivated,
        Mark::Flam,
        Mark::Learned,
        Mark::Roll,
        Mark::Rescue,
        Mark::Stage8,
    ];

    /// The event key of the mark.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Mark::Cut => "cut",
            Mark::Bleed => "bleed",
            Mark::Filtered => "filtered",
            Mark::Keep => "keep",
            Mark::Deactivated => "deactivated",
            Mark::Flam => "flam",
            Mark::Learned => "learned",
            Mark::Roll => "roll",
            Mark::Rescue => "rescue",
            Mark::Stage8 => "stage8",
        }
    }
}

/// An event that carries marks: a typed [`Event`] (the marks live in `extra`), a JSON event
/// ([`Value`]) or its object ([`Map`]). The primitives under [`is_marked`], [`mark`], [`clear`].
pub trait Marked {
    /// The value under `key`, `None` when absent (a JSON event that is no object has none).
    fn mark_get(&self, key: &str) -> Option<&Value>;
    /// Set `key` to `true`: a present key keeps its place, a new one goes last.
    fn mark_set(&mut self, key: &str);
    /// Remove `key` and keep the order of the other keys (`shift_remove`); the old value.
    fn mark_remove(&mut self, key: &str) -> Option<Value>;
}

impl Marked for Event {
    fn mark_get(&self, key: &str) -> Option<&Value> {
        self.extra.get(key)
    }
    fn mark_set(&mut self, key: &str) {
        self.extra.insert(key.into(), Value::Bool(true));
    }
    fn mark_remove(&mut self, key: &str) -> Option<Value> {
        self.extra.shift_remove(key)
    }
}

impl Marked for Map<String, Value> {
    fn mark_get(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
    fn mark_set(&mut self, key: &str) {
        self.insert(key.into(), Value::Bool(true));
    }
    fn mark_remove(&mut self, key: &str) -> Option<Value> {
        self.shift_remove(key)
    }
}

/// A JSON event. [`mark`] writes as `e[key] = true` does (it panics for a value that is neither
/// an object nor `null`); [`clear`] does nothing to a value that is no object.
impl Marked for Value {
    fn mark_get(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
    fn mark_set(&mut self, key: &str) {
        self[key] = Value::Bool(true);
    }
    fn mark_remove(&mut self, key: &str) -> Option<Value> {
        self.as_object_mut().and_then(|o| o.shift_remove(key))
    }
}

/// True when `e` carries `m` as `true` (absent, `false` and other values are off).
#[must_use]
pub fn is_marked<E: Marked + ?Sized>(e: &E, m: Mark) -> bool {
    e.mark_get(m.key()) == Some(&Value::Bool(true))
}

/// Set `m` on `e` to `true` (a present key keeps its place).
pub fn mark<E: Marked + ?Sized>(e: &mut E, m: Mark) {
    e.mark_set(m.key());
}

/// Remove `m` from `e` (`shift_remove`: the other keys keep their order); the old value.
pub fn clear<E: Marked + ?Sized>(e: &mut E, m: Mark) -> Option<Value> {
    e.mark_remove(m.key())
}

/// The flags that take a hit out of the output: the loudness cut (`"cut": true`;
/// `histogram::mark_cut_events`), the tom bleed rule (`"bleed": true`; [`crate::tom_bleed`])
/// and the old Learn filter (`"filtered": true`; #281 D16).
const OUT_KEYS: [&str; 3] = ["cut", "bleed", "filtered"];

/// #242: the per-hit overrides of `Toggle Hit Active`, carried in `extra` like the marks.
/// `"deactivated": true` takes a hit out of the output whatever the rules say; `"keep": true`
/// holds a deactivated hit in the output. The user's choice wins over every automatic mark
/// (the predicates below), survives a re-analyze (it is a kept hit of [`is_kept`], so
/// `edit::merge_edited` preserves it) and a change of the cut or bleed threshold or of the
/// Learn (the marking rules never touch the flags, and [`crate::histogram`] skips kept hits).
const USER_OUT_KEY: &str = "deactivated";
const USER_KEEP_KEY: &str = "keep";

/// A hand-edited hit: added, or moved away from its detected time. Structural: it reads only
/// `added` and `orig`, never the reserved `edited` flag (the page's `isManual`).
pub fn is_manual(e: &Value) -> bool {
    if e.get("added") == Some(&Value::Bool(true)) {
        return true;
    }
    match (
        e.get("orig").and_then(Value::as_f64),
        e.get("t").and_then(Value::as_f64),
    ) {
        (Some(o), Some(t)) => (t - o).abs() > 1e-9,
        _ => false,
    }
}

/// A hit the engineer keeps as it is: a hand-edited hit, one with the document's reserved
/// edit flag `edited: true` (a hand velocity or a restore), or one of the user's per-hit
/// overrides of #242 (`keep`/`deactivated` — a re-pick or Re-analyze keeps those too). The
/// page's `isKept` is this rule.
#[must_use]
pub fn is_kept(e: &Value) -> bool {
    is_manual(e)
        || e.get("edited") == Some(&Value::Bool(true))
        || e.get(USER_KEEP_KEY) == Some(&Value::Bool(true))
        || e.get(USER_OUT_KEY) == Some(&Value::Bool(true))
}

/// True for a hit out of the output (MIDI, spikes, Export All, the click, Learn scoring).
/// A kept hit never carries an out mark: Rust strips the marks on edit (edit.rs), the page
/// guards its predicates with `!isManual` — one policy, two mechanisms (R1). The user's
/// `deactivated` wins over everything; the user's `keep` wins over the automatic marks (#242).
#[must_use]
pub fn is_out(e: &Value) -> bool {
    e.get(USER_OUT_KEY) == Some(&Value::Bool(true))
        || (e.get(USER_KEEP_KEY) != Some(&Value::Bool(true))
            && OUT_KEYS.iter().any(|k| e.get(k) == Some(&Value::Bool(true))))
}

/// [`is_out`] for the typed events of a document (the marks live in `extra`).
#[must_use]
pub fn event_out(e: &Event) -> bool {
    event_user_out(e) || (!event_user_kept(e) && OUT_KEYS.iter().any(|k| e.extra.get(*k) == Some(&Value::Bool(true))))
}

/// The user's `keep` of #242 for the typed events: the marking rules read it to leave a kept
/// hit alone.
#[must_use]
pub fn event_user_kept(e: &Event) -> bool {
    e.extra.get(USER_KEEP_KEY) == Some(&Value::Bool(true))
}

/// The user's `deactivated` of #242 for the typed events.
#[must_use]
pub fn event_user_out(e: &Event) -> bool {
    e.extra.get(USER_OUT_KEY) == Some(&Value::Bool(true))
}

/// R2 in this tree: tempo detection has no predicate of its own — it calls the same one
/// predicate as every other output path, so a `filtered` hit leaves the tempo too. (The plan
/// draft described an older tree; the behaviour is unchanged either way, see the done note.)
pub const TEMPO_USES_ONE_PREDICATE: &str =
    "tempo detection uses the one `out` predicate: a cut, bled or filtered hit is no tempo candidate";

/// [`is_out`] for the tempo detection: the same one predicate, no exception.
#[must_use]
pub fn tempo_out(e: &Event) -> bool {
    event_out(e)
}

/// [`tempo_out`] for the JSON events of the page and the tests.
#[must_use]
pub fn tempo_out_json(e: &Value) -> bool {
    is_out(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn table() -> Value {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/event-policy/table.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// R4: the mark truth table. For every mark and every state of its key (absent, `true`,
    /// `false`, `null`, a number, a string, an object), on a typed event, a JSON event and its
    /// object: `is_marked` is on only for `true`, `mark` writes what a raw `insert` writes
    /// (a present key keeps its place) and `clear` what a raw `shift_remove` writes.
    #[test]
    fn the_marks_match_the_raw_key_reads_and_writes() {
        let keys: Vec<&str> = Mark::ALL.iter().map(|m| m.key()).collect();
        assert_eq!(
            keys,
            [
                "cut",
                "bleed",
                "filtered",
                "keep",
                "deactivated",
                "flam",
                "learned",
                "roll",
                "rescue",
                "stage8"
            ]
        );
        let states = [
            None,
            Some(json!(true)),
            Some(json!(false)),
            Some(Value::Null),
            Some(json!(1)),
            Some(json!("true")),
            Some(json!({})),
        ];
        let mut rows = 0;
        for m in Mark::ALL {
            let k = m.key();
            for st in &states {
                let mut obj = Map::new();
                obj.insert("t".into(), json!(1.0));
                if let Some(s) = st {
                    obj.insert(k.into(), s.clone());
                }
                obj.insert("y".into(), json!(3));
                obj.insert("z".into(), json!(2));
                let on = st.as_ref() == Some(&json!(true));
                let val = Value::Object(obj.clone());
                let ev: Event = serde_json::from_value(val.clone()).unwrap();
                assert_eq!(is_marked(&obj, m), on, "{k} {st:?} map");
                assert_eq!(is_marked(&val, m), on, "{k} {st:?} value");
                assert_eq!(is_marked(&ev, m), on, "{k} {st:?} event");

                let mut raw = obj.clone();
                raw.insert(k.into(), Value::Bool(true));
                let want = serde_json::to_string(&raw).unwrap();
                let (mut o, mut v, mut e) = (obj.clone(), val.clone(), ev.clone());
                mark(&mut o, m);
                mark(&mut v, m);
                mark(&mut e, m);
                assert_eq!(serde_json::to_string(&o).unwrap(), want, "{k} {st:?} mark map");
                assert_eq!(serde_json::to_string(&v).unwrap(), want, "{k} {st:?} mark value");
                assert_eq!(serde_json::to_string(&e).unwrap(), want, "{k} {st:?} mark event");

                let mut raw = obj.clone();
                let old = raw.shift_remove(k);
                let want = serde_json::to_string(&raw).unwrap();
                let (mut o, mut v, mut e) = (obj.clone(), val.clone(), ev.clone());
                assert_eq!(clear(&mut o, m), old, "{k} {st:?} clear map");
                assert_eq!(clear(&mut v, m), old, "{k} {st:?} clear value");
                assert_eq!(clear(&mut e, m), old, "{k} {st:?} clear event");
                assert_eq!(serde_json::to_string(&o).unwrap(), want, "{k} {st:?} clear map");
                assert_eq!(serde_json::to_string(&v).unwrap(), want, "{k} {st:?} clear value");
                assert_eq!(serde_json::to_string(&e).unwrap(), want, "{k} {st:?} clear event");
                rows += 1;
            }
        }
        assert_eq!(rows, 70, "10 marks x 7 states");
        // the order is part of the bytes: a present key keeps its place, a new one goes last,
        // a cleared one leaves the others in order
        let mut e = json!({"t": 1.0, "cut": false, "z": 2});
        mark(&mut e, Mark::Cut);
        mark(&mut e, Mark::Bleed);
        assert_eq!(e.to_string(), r#"{"t":1.0,"cut":true,"z":2,"bleed":true}"#);
        assert_eq!(clear(&mut e, Mark::Cut), Some(json!(true)));
        assert_eq!(e.to_string(), r#"{"t":1.0,"z":2,"bleed":true}"#);
        // a JSON event that is no object carries no mark and clears nothing; `null` becomes one
        let mut n = json!([1, 2]);
        assert!(!is_marked(&n, Mark::Cut));
        assert_eq!(clear(&mut n, Mark::Cut), None);
        assert_eq!(n, json!([1, 2]));
        let mut n = Value::Null;
        mark(&mut n, Mark::Flam);
        assert_eq!(n, json!({"flam": true}));
    }

    /// edit.rs strips the out marks from a kept hit on every edit; the truth table rows for a
    /// kept hit that still carries a mark go through this mechanism before the predicate, so
    /// the table proves the two mechanisms (strip in Rust, `!isManual` guard on the page) agree.
    fn strip_marks_if_kept(e: &Value) -> Value {
        let mut e = e.clone();
        if is_kept(&e) {
            for k in OUT_KEYS {
                if e.get(k) == Some(&Value::Bool(true)) {
                    e.as_object_mut().unwrap().insert(k.into(), Value::Bool(false));
                }
            }
        }
        e
    }

    #[test]
    fn the_policy_matches_the_truth_table_row_by_row() {
        let fx = table();
        for row in fx["rows"].as_array().unwrap() {
            let e = &row["event"];
            let out = row["out"].as_bool().unwrap();
            let kept = row["kept"].as_bool().unwrap();
            let tempo = row["tempo"].as_bool().unwrap();
            let stripped = strip_marks_if_kept(e);
            assert_eq!(is_out(&stripped), out, "{}", row["name"]);
            assert_eq!(is_kept(e), kept, "{}", row["name"]);
            // audible and exportable are the same gate: a hit out of the output plays no click
            // and lands in no MIDI or spike file.
            assert_eq!(!is_out(&stripped), row["audible"].as_bool().unwrap(), "{}", row["name"]);
            assert_eq!(
                !is_out(&stripped),
                row["exportable"].as_bool().unwrap(),
                "{}",
                row["name"]
            );
            // the tempo gate: the same one predicate (TEMPO_USES_ONE_PREDICATE)
            assert_eq!(
                !tempo_out_json(&stripped),
                tempo,
                "{} (tempo: {})",
                row["name"],
                TEMPO_USES_ONE_PREDICATE
            );
        }
        let rows = fx["rows"].as_array().unwrap();
        assert!(rows.len() >= 10, "the table has the combinations of the policy doc");
    }

    #[test]
    fn the_typed_predicate_reads_extra() {
        let mk = |extra: Value| Event {
            t: 1.0,
            extra: extra.as_object().unwrap().clone(),
            ..Default::default()
        };
        assert!(event_out(&mk(json!({"cut": true}))));
        assert!(event_out(&mk(json!({"bleed": true, "filtered": true}))));
        assert!(!event_out(&mk(json!({"filtered": false}))));
        assert!(!event_out(&mk(json!({}))));
        let tempo_e = mk(json!({"filtered": true}));
        assert!(tempo_out(&tempo_e), "{}", TEMPO_USES_ONE_PREDICATE);
        assert!(tempo_out(&mk(json!({"bleed": true}))));
    }

    /// R4 (refactor 3.2): the serialized events of the pure mark writers, pinned byte for byte
    /// before the mark keys move behind `Mark`. Key order and absent against `false` are part of
    /// the output (a `shift_remove` keeps the order of the other keys, an `insert` of a present
    /// key keeps its place).
    mod golden {
        use super::*;
        use drumspike_core::detect::flam::Second;

        fn lines<T: serde::Serialize>(events: &[T]) -> String {
            events
                .iter()
                .map(|e| serde_json::to_string(e).unwrap())
                .collect::<Vec<_>>()
                .join("\n")
        }

        fn event(v: Value) -> Event {
            serde_json::from_value(v).unwrap()
        }

        /// sr 1000: a loud burst around 0.1 s (-6 dB), silence everywhere else.
        fn signal() -> Vec<f32> {
            let mut x = vec![0.0f32; 2000];
            for s in &mut x[95..106] {
                *s = 0.5;
            }
            x
        }

        fn cut_inputs() -> Vec<Value> {
            vec![
                json!({"t": 0.1, "v": 90, "bleed": true}),
                json!({"t": 0.5, "v": 40, "cut": false, "z": 1}),
                json!({"t": 0.6, "v": 40, "z": 2, "cut": true, "y": 3}),
                json!({"t": 0.7, "v": 40, "added": true, "cut": true, "w": 0}),
                json!({"t": 0.8, "v": 40, "cut": true, "keep": true, "q": 1}),
                json!({"t": 0.9, "v": 40, "orig": 0.85, "cut": true, "filtered": false}),
                json!({"t": 1.0, "v": 40, "deactivated": true, "cut": "yes"}),
            ]
        }

        const CUT_ON: &str = r#"{"t":0.1,"v":90,"bleed":true}
{"t":0.5,"v":40,"cut":true,"z":1}
{"t":0.6,"v":40,"z":2,"cut":true,"y":3}
{"t":0.7,"v":40,"added":true,"w":0}
{"t":0.8,"v":40,"keep":true,"q":1}
{"t":0.9,"v":40,"orig":0.85,"filtered":false}
{"t":1.0,"v":40,"deactivated":true,"cut":true}"#;

        const CUT_OFF: &str = r#"{"t":0.1,"v":90,"bleed":true}
{"t":0.5,"v":40,"z":1}
{"t":0.6,"v":40,"z":2,"y":3}
{"t":0.7,"v":40,"added":true,"w":0}
{"t":0.8,"v":40,"keep":true,"q":1}
{"t":0.9,"v":40,"orig":0.85,"filtered":false}
{"t":1.0,"v":40,"deactivated":true}"#;

        #[test]
        fn the_loudness_cut_writes_the_same_events() {
            let x = signal();
            for (cut, want) in [(Some(-30.0), CUT_ON), (None, CUT_OFF)] {
                let mut evs: Vec<Event> = cut_inputs().into_iter().map(event).collect();
                crate::histogram::mark_cut_events(&mut evs, &x, 1000, cut);
                assert_eq!(lines(&evs), want, "typed events, cut {cut:?}");
                let mut vals = cut_inputs();
                crate::histogram::mark_cut_values(&mut vals, &x, 1000, cut).unwrap();
                assert_eq!(lines(&vals), want, "JSON events, cut {cut:?}");
            }
        }

        fn second(t: f64, bleed: bool) -> Second {
            Second {
                t,
                first: t - 0.03,
                gap_ms: 30.0,
                rise_db: 12.0,
                level_db: 0.0,
                bleed,
                dip: 0,
            }
        }

        const FLAMS: &str = r#"{"t":1.0,"v":100}
{"t":1.03,"v":127,"flam":true}
{"t":1.5,"v":100,"flam":false,"z":1}
{"t":1.53,"v":20,"flam":true,"cut":true}
{"t":1.7,"v":90,"orig":1.69,"flam":true}
{"t":1.805,"v":30,"cut":true}"#;

        #[test]
        fn the_flam_rule_writes_the_same_events() {
            let sr = 48_000;
            let mut x = vec![0.0f32; sr as usize * 2];
            x[(1.03 * f64::from(sr)) as usize + 10] = 0.5;
            x[(1.53 * f64::from(sr)) as usize + 10] = 0.001;
            let events = vec![
                Event::new(1.0, 100),
                event(json!({"t": 1.2, "v": 90, "flam": true})),
                event(json!({"t": 1.5, "v": 100, "flam": false, "z": 1})),
                event(json!({"t": 1.7, "v": 90, "orig": 1.69, "flam": true})),
                event(json!({"t": 1.805, "v": 30, "cut": true})),
            ];
            let found = [
                second(1.03, false),
                second(1.53, false),
                second(1.6, true),
                second(1.8, false),
            ];
            let params = json!({"vel_lo_db": -40.0, "vel_hi_db": -6.0, "vel_cut_db": -50.0});
            let (out, _) = crate::flam::with_flams(&events, &found, &x, sr, params.as_object(), 0.010);
            assert_eq!(lines(&out), FLAMS);
        }

        const REFILTER: &str = r#"{"t":0.5,"orig":0.45,"z":1}
{"t":1.0,"v":80,"filtered":true,"y":2}
{"t":1.02,"v":60,"flam":true,"q":3,"filtered":true}
{"t":2.0,"v":80,"y":4}
{"t":2.02,"v":60,"flam":true}
{"t":3.0,"v":70,"learned":true}
{"t":4.0,"v":70,"forced":true}
{"t":5.0,"v":50,"flam":true,"keep":true}"#;

        #[test]
        fn the_learn_refilter_writes_the_same_events() {
            let current = [
                json!({"t": 0.5, "orig": 0.45, "filtered": true, "z": 1}),
                json!({"t": 1.0, "v": 80, "filtered": false, "y": 2}),
                json!({"t": 1.02, "v": 60, "flam": true, "q": 3}),
                json!({"t": 2.0, "v": 80, "filtered": true, "y": 4}),
                json!({"t": 2.02, "v": 60, "flam": true, "filtered": true}),
                json!({"t": 2.5, "v": 50}),
                json!({"t": 5.0, "v": 50, "flam": true, "filtered": true, "keep": true}),
            ];
            let base = [
                json!({"t": 1.0, "filtered": true}),
                json!({"t": 2.0, "filtered": false}),
                json!({"t": 3.0, "v": 70, "learned": true}),
                json!({"t": 3.5, "v": 70, "learned": true, "filtered": true}),
            ];
            let forced = [json!({"t": 4.0, "v": 70, "forced": true})];
            let out = crate::learn_filter::refilter_merge(&current, &base, &forced, &[]);
            assert_eq!(lines(&out), REFILTER);
        }
    }

    /// #242: the user's overrides win over the automatic marks, `deactivated` over `keep`.
    #[test]
    fn is_out_follows_the_user_override() {
        // a deactivated hit is out whatever the marks say, also with none
        assert!(is_out(&json!({"t": 1.0, "deactivated": true})));
        assert!(is_out(&json!({"t": 1.0, "deactivated": true, "cut": true})));
        assert!(is_out(&json!({"t": 1.0, "deactivated": true, "v": 100})));
        // a kept hit is in the output under every mark
        assert!(!is_out(&json!({"t": 1.0, "keep": true})));
        assert!(!is_out(&json!({"t": 1.0, "keep": true, "cut": true})));
        assert!(!is_out(
            &json!({"t": 1.0, "keep": true, "bleed": true, "filtered": true})
        ));
        // deactivated wins over keep (a state the toggle never writes, but the order is fixed)
        assert!(is_out(&json!({"t": 1.0, "keep": true, "deactivated": true})));
        // the typed predicate reads the same flags from `extra`
        let mk = |extra: Value| Event {
            t: 1.0,
            extra: extra.as_object().unwrap().clone(),
            ..Default::default()
        };
        assert!(event_out(&mk(json!({"deactivated": true}))));
        assert!(!event_out(&mk(json!({"keep": true, "cut": true}))));
        assert!(event_out(&mk(json!({"keep": true, "deactivated": true}))));
        // a kept hit is no tempo candidate when it is deactivated, and one when it is kept in
        assert!(tempo_out(&mk(json!({"deactivated": true}))));
        assert!(!tempo_out(&mk(json!({"keep": true, "filtered": true}))));
    }
}
