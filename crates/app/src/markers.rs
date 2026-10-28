//! P4-40g: review markers. The list lives in the project document under the `markers` key — a
//! pass-through key like `learn` and `tempo_map` (format 1 stays; the loader carries every key it
//! does not type, so a document the page or an agent wrote round-trips unchanged). One entry is an
//! object with a numeric `t` (seconds) and an optional string `note`; every other key of an entry
//! is carried as it is. An empty list removes the key, so the document is as it was before the
//! feature (like the default tempo map).
//!
//! * `markers_get {sid}`: `{markers, n}` — the stored entries, oldest first (the order of the list).
//! * `markers_set {sid, markers}`: replace the whole list in one step; the answer carries the
//!   `previous` list, so one more call undoes it (as `tempo_map_set` does for the map). The entries
//!   are stored sorted by `t`.

use serde_json::{json, Value};

use crate::error::AppError;
use crate::registry::lock;
use crate::Service;

/// The document key of the marker list.
pub const DOC_KEY: &str = "markers";

/// The commands of this card.
pub const MARKERS_COMMANDS: [&str; 2] = ["markers_get", "markers_set"];

/// The stored markers: the entries of the `markers` list with a numeric `t` (the rest of the list
/// stays as it is in the document; a value of another type is no list and reads as none).
pub fn doc_markers(doc: &drumspike_core::Document) -> Vec<Value> {
    match doc.extra.get(DOC_KEY) {
        Some(Value::Array(a)) => a
            .iter()
            .filter(|v| v.get("t").and_then(Value::as_f64).is_some())
            .cloned()
            .collect(),
        _ => Vec::new(),
    }
}

/// Store `markers`; an empty list removes the key. The caller marks the project dirty.
pub fn set_doc_markers(doc: &mut drumspike_core::Document, markers: &[Value]) {
    if markers.is_empty() {
        doc.extra.shift_remove(DOC_KEY);
    } else {
        doc.extra.insert(DOC_KEY.into(), Value::Array(markers.to_vec()));
    }
}

impl Service {
    /// `markers_get {sid}`: `{markers, n}`.
    pub fn markers_get(&self, sid: &str) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        let g = lock(&op);
        let markers = doc_markers(&g.project.doc);
        let n = markers.len();
        Ok(json!({ "markers": markers, "n": n }))
    }

    /// `markers_set {sid, markers}`: replace the whole list. 400 when an entry has no numeric `t`
    /// or a `note` that is no string (the message names the index). Answers
    /// `{markers, previous, changed, n}`; `previous` is the list before the call, so one more
    /// call with it undoes the change. An empty list clears the markers.
    pub fn markers_set(&self, sid: &str, markers: &[Value]) -> Result<Value, AppError> {
        for (i, v) in markers.iter().enumerate() {
            let ok = v.is_object()
                && v.get("t").and_then(Value::as_f64).is_some()
                && v.get("note").is_none_or(Value::is_string);
            if !ok {
                return Err(AppError::bad(format!(
                    "marker {i}: an object with a numeric `t` and an optional string `note`"
                )));
            }
        }
        let mut list = markers.to_vec();
        list.sort_by(|a, b| {
            a.get("t")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .partial_cmp(&b.get("t").and_then(Value::as_f64).unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let op = self.inner.registry.get_writable(sid)?;
        let mut g = lock(&op);
        let previous = doc_markers(&g.project.doc);
        let changed = previous != list;
        if changed {
            set_doc_markers(&mut g.project.doc, &list);
            g.mark_dirty();
        }
        let n = list.len();
        Ok(json!({ "markers": list, "previous": previous, "changed": changed, "n": n }))
    }
}

/// Run a command of this card for the bridge, the command line and the MCP server; `None` when
/// `cmd` is not one of them.
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    if !MARKERS_COMMANDS.contains(&cmd) {
        return None;
    }
    let run = || -> Result<Value, AppError> {
        let sid = || {
            args.get("sid")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::bad("missing or wrong argument `sid` (string)"))
        };
        match cmd {
            "markers_get" => svc.markers_get(sid()?),
            "markers_set" => {
                let list = args
                    .get("markers")
                    .and_then(Value::as_array)
                    .ok_or_else(|| AppError::bad("missing or wrong argument `markers` (list)"))?;
                svc.markers_set(sid()?, list)
            }
            _ => Err(AppError::not_found(format!("unknown command {cmd}"))),
        }
    };
    Some(run())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Gate: the `markers` document list is a pass-through key (format 1 unchanged) with a
    /// set/get round trip.
    #[test]
    fn markers_is_a_pass_through_key_of_format_1_and_set_get_round_trips() {
        // A document the agent wrote: the entries carry extra keys, the order is the stored one.
        let doc_text =
            r#"{"format":1,"name":"Song","markers":[{"id":"m1","t":2.5,"note":"check flam"},{"t":0.5}],"tracks":{}}"#;
        let doc = drumspike_core::Document::from_json(doc_text).unwrap();
        assert_eq!(doc.format, 1);
        // Pass-through: the loader reads the list and the document values round-trip unchanged
        // (the write puts the typed fields first, the `extra` keys after — the values are the same).
        let got = doc_markers(&doc);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0]["note"], "check flam");
        assert_eq!(got[0]["id"], "m1"); // an extra key of an entry is carried
        let before: serde_json::Value = serde_json::from_str(doc_text).unwrap();
        let after: serde_json::Value = serde_json::from_str(&doc.to_json()).unwrap();
        assert_eq!(before, after);
        // Set/get round trip: the same list reads back; the format and the other keys stay.
        let mut doc = doc;
        let list = vec![json!({"t": 0.5}), json!({"t": 2.5, "note": "check flam"})];
        set_doc_markers(&mut doc, &list);
        assert_eq!(doc_markers(&doc), list);
        assert_eq!(doc.format, 1);
        assert_eq!(doc.name.as_deref(), Some("Song"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&doc.to_json()).unwrap()["markers"],
            serde_json::json!([{"t": 0.5}, {"t": 2.5, "note": "check flam"}])
        );
        // A document without markers gains no key from a read, and an empty list removes the key.
        let plain = drumspike_core::Document::from_json(r#"{"format":1}"#).unwrap();
        assert!(doc_markers(&plain).is_empty());
        assert_eq!(plain.to_json(), r#"{"format":1}"#);
        let mut doc = plain;
        set_doc_markers(&mut doc, &list);
        assert!(doc.to_json().contains("markers"));
        set_doc_markers(&mut doc, &[]);
        assert!(!doc.to_json().contains("markers"));
        // A `markers` value of another type is no list (it reads as none and stays in the document).
        let odd = drumspike_core::Document::from_json(r#"{"format":1,"markers":5}"#).unwrap();
        assert!(doc_markers(&odd).is_empty());
        assert_eq!(odd.to_json(), r#"{"format":1,"markers":5}"#);
    }

    #[test]
    fn set_doc_markers_stores_the_list_as_given() {
        // The service sorts (markers_set); the document helper stores what it is given, so an
        // entry the page or an agent wrote keeps its place and its keys.
        let mut doc = drumspike_core::Document::from_json(r#"{"format":1}"#).unwrap();
        let list = vec![json!({"t": 2.5}), json!({"t": 0.5, "note": "tom 2 bleed?"})];
        set_doc_markers(&mut doc, &list);
        assert_eq!(doc_markers(&doc), list);
        assert_eq!(doc_markers(&doc)[1]["note"], "tom 2 bleed?");
    }
}
