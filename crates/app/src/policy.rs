//! R16 (#40): the event policy in one place. One predicate decides "leaves the output"
//! (`is_out` / [`event_out`]), one decides "kept" ([`is_kept`]), and the tempo exception is
//! named ([`TEMPO_KEEPS_FILTERED`]). The duplicates call this module: `export::is_cut`
//! (MIDI, spikes, Export All, the click), `histogram::event_cut` (flam, loudness),
//! `confidence::is_out`, `tempo::is_cut`. The page's copies (`isManual`, `isKept`, `hgIsCut`,
//! `lnGreyHit` in static/app.js) and the Rust side read one truth table
//! (tests/fixtures/event-policy/table.json), so the two mechanisms cannot drift.

use drumspike_core::Event;
use serde_json::Value;

/// The flags that take a hit out of the output: the loudness cut (`"cut": true`;
/// `histogram::mark_cut_events`), the tom bleed rule (`"bleed": true`; [`crate::tom_bleed`])
/// and the old Learn filter (`"filtered": true`; #281 D16).
const OUT_KEYS: [&str; 3] = ["cut", "bleed", "filtered"];

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

/// A hit the engineer keeps as it is: a hand-edited hit, or one with the document's reserved
/// edit flag `edited: true` (a hand velocity or a restore). The page's `isKept` is this rule.
pub fn is_kept(e: &Value) -> bool {
    is_manual(e) || e.get("edited") == Some(&Value::Bool(true))
}

/// True for a hit out of the output (MIDI, spikes, Export All, the click, Learn scoring).
/// A kept hit never carries an out mark: Rust strips the marks on edit (edit.rs), the page
/// guards its predicates with `!isManual` — one policy, two mechanisms (R1).
pub fn is_out(e: &Value) -> bool {
    OUT_KEYS.iter().any(|k| e.get(k) == Some(&Value::Bool(true)))
}

/// [`is_out`] for the typed events of a document (the marks live in `extra`).
pub fn event_out(e: &Event) -> bool {
    OUT_KEYS.iter().any(|k| e.extra.get(*k) == Some(&Value::Bool(true)))
}

/// R2 in this tree: tempo detection has no predicate of its own — it calls the same one
/// predicate as every other output path, so a `filtered` hit leaves the tempo too. (The plan
/// draft described an older tree; the behaviour is unchanged either way, see the done note.)
pub const TEMPO_USES_ONE_PREDICATE: &str =
    "tempo detection uses the one `out` predicate: a cut, bled or filtered hit is no tempo candidate";

/// [`is_out`] for the tempo detection: the same one predicate, no exception.
pub fn tempo_out(e: &Event) -> bool {
    event_out(e)
}

/// [`tempo_out`] for the JSON events of the page and the tests.
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
}
