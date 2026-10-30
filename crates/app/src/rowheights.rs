//! #167: per-track lane heights. The map lives in the project document under the `row_heights`
//! key — a pass-through key like `markers` (format 1 stays; the loader carries every key it does
//! not type, so a document the page or an agent wrote round-trips unchanged). One entry maps a
//! track name to a positive factor: the wave lane of that row is the global wave height (the
//! Track Height setting) times the factor, so the global Track Height zoom still scales every
//! lane, the dragged one included. An empty map removes the key, so the document is as it was
//! before the feature.
//!
//! * `row_heights_get {sid}`: `{heights, n}` — the stored factors (the order of the object).
//! * `row_heights_set {sid, heights}`: replace the whole map in one step. 400 when an entry has a
//!   factor that is no positive finite number. Answers `{heights, changed, n}`.

use serde_json::{Map, Value, json};

use crate::Service;
use crate::error::AppError;
use crate::registry::lock;

/// The document key of the row-height map.
pub const DOC_KEY: &str = "row_heights";

/// The commands of this card.
pub const ROW_HEIGHTS_COMMANDS: [&str; 2] = ["row_heights_get", "row_heights_set"];

/// The stored row heights: the entries of the `row_heights` object whose value is a positive
/// finite number (the rest of the object stays as it is in the document; a value of another type
/// reads as no override, a non-object as none).
pub fn doc_row_heights(doc: &drumspike_core::Document) -> Value {
    match doc.extra.get(DOC_KEY) {
        Some(Value::Object(m)) => {
            let out: Map<String, Value> = m
                .iter()
                .filter(|(_, v)| v.as_f64().is_some_and(|f| f.is_finite() && f > 0.0))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            Value::Object(out)
        }
        _ => Value::Object(Map::new()),
    }
}

/// Store `heights`; an empty map removes the key. The caller marks the project dirty.
pub fn set_doc_row_heights(doc: &mut drumspike_core::Document, heights: &Value) {
    let empty = heights.as_object().is_none_or(Map::is_empty);
    if empty {
        doc.extra.shift_remove(DOC_KEY);
    } else {
        doc.extra.insert(DOC_KEY.into(), heights.clone());
    }
}

impl Service {
    /// `row_heights_get {sid}`: `{heights, n}`.
    pub fn row_heights_get(&self, sid: &str) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        let g = lock(&op);
        let heights = doc_row_heights(&g.project.doc);
        let n = heights.as_object().map_or(0, |m| m.len());
        Ok(json!({ "heights": heights, "n": n }))
    }

    /// `row_heights_set {sid, heights}`: replace the whole map. 400 when an entry has a factor
    /// that is no positive finite number (the message names the track). Answers
    /// `{heights, changed, n}`. An empty map clears the overrides.
    pub fn row_heights_set(&self, sid: &str, heights: &Value) -> Result<Value, AppError> {
        let obj = match heights {
            Value::Object(m) => m.clone(),
            _ => {
                return Err(AppError::bad(
                    "missing or wrong argument `heights` (object of track name -> factor)",
                ));
            }
        };
        for (name, v) in &obj {
            let ok = v.as_f64().is_some_and(|f| f.is_finite() && f > 0.0);
            if !ok {
                return Err(AppError::bad(format!(
                    "row height of {name}: a positive number (the factor of the global wave height)"
                )));
            }
        }
        let op = self.inner.registry.get_writable(sid)?;
        let mut g = lock(&op);
        let previous = doc_row_heights(&g.project.doc);
        let changed = previous != Value::Object(obj.clone());
        if changed {
            set_doc_row_heights(&mut g.project.doc, &Value::Object(obj.clone()));
            g.mark_dirty();
        }
        let n = obj.len();
        Ok(json!({ "heights": Value::Object(obj), "changed": changed, "n": n }))
    }
}

/// Run a command of this card for the bridge, the command line and the MCP server; `None` when
/// `cmd` is not one of them.
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    if !ROW_HEIGHTS_COMMANDS.contains(&cmd) {
        return None;
    }
    let run = || -> Result<Value, AppError> {
        let sid = || {
            args.get("sid")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::bad("missing or wrong argument `sid` (string)"))
        };
        match cmd {
            "row_heights_get" => svc.row_heights_get(sid()?),
            "row_heights_set" => svc.row_heights_set(sid()?, args.get("heights").unwrap_or(&Value::Null)),
            _ => Err(AppError::not_found(format!("unknown command {cmd}"))),
        }
    };
    Some(run())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Gate: the `row_heights` document object is a pass-through key (format 1 unchanged)
    /// with a set/get round trip and a positive-factor validation.
    #[test]
    fn row_heights_is_a_pass_through_key_of_format_1_and_set_get_round_trips() {
        // A document the page wrote: the factors ride as they are.
        let doc_text = r#"{"format":1,"name":"Song","row_heights":{"Kick In":1.5,"Snare Top":2},"tracks":{}}"#;
        let doc = drumspike_core::Document::from_json(doc_text).unwrap();
        assert_eq!(doc.format, 1);
        let got = doc_row_heights(&doc);
        assert_eq!(got["Kick In"], 1.5);
        assert_eq!(got["Snare Top"], 2.0);
        // A value of another type reads as no override.
        let bad = drumspike_core::Document::from_json(r#"{"format":1,"row_heights":{"Kick In":"tall"}}"#).unwrap();
        assert_eq!(doc_row_heights(&bad).as_object().unwrap().len(), 0);
        // Round trip: the write carries the values unchanged.
        let before: Value = serde_json::from_str(doc_text).unwrap();
        let after: Value = serde_json::from_str(&doc.to_json()).unwrap();
        assert_eq!(before["row_heights"], after["row_heights"]);
    }

    #[test]
    fn set_row_heights_validates_and_an_empty_map_removes_the_key() {
        let mut doc = drumspike_core::Document::from_json(r#"{"format":1,"tracks":{}}"#).unwrap();
        assert!(doc_row_heights(&doc).as_object().unwrap().is_empty());
        set_doc_row_heights(&mut doc, &json!({"Kick In": 1.5}));
        assert_eq!(doc_row_heights(&doc)["Kick In"], 1.5);
        set_doc_row_heights(&mut doc, &json!({}));
        assert!(doc.extra.get(DOC_KEY).is_none()); // the document is as it was before
    }
}
