//! P4-8 stage A: the tempo and meter map as commands (the page shows them in stage C).
//!
//! * `tempo_map_get {sid}`: the project's map (the default map when none is stored).
//! * `tempo_map_set {sid, map}`: replace the whole map in one step; the answer carries the
//!   `previous` map, so one more call undoes it. `null` or `{}` sets the default map.
//! * `tempo_map_import_midi {sid, path}`: read the tempo and time-signature events of a MIDI file
//!   (a DAW export) and replace the map with them (one step, `previous` in the answer).
//!
//! * `tempo_map_detect {sid, tracks?, min_bars?, min_step_pct?}` (stage B): a suggested map from
//!   the kick and snare hits (`drumspike_core::tempo_detect`); read only, the caller accepts the
//!   suggestion with `tempo_map_set`.
//!
//! The map is `{bpm, steps: [{t, bpm}], meter: [num, den], changes: [{bar, meter}], bar1_s}`,
//! see `docs/migration/project-format.md`. The MIDI export of a project uses it
//! (`export::export_midi`, Export All); the default map writes the bytes of the exports before this card.

use drumspike_core::tempo::{TempoMap, import_midi};
use drumspike_core::tempo_detect::{
    DEFAULT_VELOCITY, DetectParams, Detection, Hit, ISOLATED_S, Role, detect, refit_bar1,
};
use serde_json::{Value, json};

use crate::Service;
use crate::error::AppError;
use crate::registry::lock;

/// The commands of this card.
pub const TEMPO_COMMANDS: [&str; 5] = [
    "tempo_map_get",
    "tempo_map_set",
    "tempo_map_import_midi",
    "tempo_map_detect",
    "tempo_map_refit_bar1",
];

/// The meter rule of the detection, as the answer states it.
pub const METER_RULE: &str = "per beat the kick hits minus the snare hits on the beat, and the kicks and the snares \
     between the beats; 3/4 when the pattern's correlation at 3 beats is at least 0.4 and beats the one at 4 beats by 0.5 \
     and the one at 2 beats by 0.2, else 4/4; the 4/4 confidence needs the 4-beat correlation to beat the 2-beat one \
     (feel two-beat otherwise: 2/4, or 6/8 counted in dotted quarters, fit the hits as well)";

/// Largest MIDI file the import reads, in bytes.
pub const IMPORT_LIMIT: u64 = 64 << 20;

/// The map of a document; a stored value that is not a valid map is an error with a message.
///
/// # Errors
/// 400 when the tempo map stored in the document does not parse as a valid map.
pub fn doc_map(doc: &drumspike_core::Document) -> Result<TempoMap, AppError> {
    doc.tempo_map()
        .map_err(|e| AppError::bad(format!("the stored tempo map is not valid: {e}")))
}

impl Service {
    /// `tempo_map_get {sid}`: `{map, default}`.
    ///
    /// # Errors
    /// 404 when `sid` is not an open project; 400 when the stored tempo map is not valid.
    pub fn tempo_map_get(&self, sid: &str) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        let g = lock(&op);
        let map = doc_map(&g.project.doc)?;
        Ok(json!({"map": map.to_json(), "default": map.is_default()}))
    }

    /// Store `map` and answer `{map, previous, changed}`; marks the project dirty when it changed.
    fn store_map(&self, sid: &str, map: TempoMap, extra: Option<Value>) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let mut g = lock(&op);
        // A stored value that is not a valid map has no `previous` (null) and is always replaced,
        // so a broken value never blocks the MIDI exports for good.
        let previous = doc_map(&g.project.doc).ok();
        let changed = previous.as_ref() != Some(&map);
        if changed {
            g.project.doc.set_tempo_map(&map);
            g.mark_dirty();
        }
        let mut out = json!({"map": map.to_json(), "previous": previous.map(|p| p.to_json()), "changed": changed});
        if let (Some(Value::Object(x)), Some(o)) = (extra, out.as_object_mut()) {
            o.extend(x);
        }
        Ok(out)
    }

    /// `tempo_map_set {sid, map}`: replace the whole map. 400 for a map that is not valid
    /// (the message names the fault). Answers `{map, previous, changed}`; `previous` is null
    /// when the stored value was not a valid map (it is replaced in any case).
    ///
    /// # Errors
    /// 400 when `map` is not a valid tempo map; 404 when `sid` is not an open project; 409 when
    /// the project is open read-only.
    pub fn tempo_map_set(&self, sid: &str, map: &Value) -> Result<Value, AppError> {
        let m = if map.is_null() {
            TempoMap::default()
        } else {
            TempoMap::from_json(map).map_err(AppError::bad)?
        };
        self.store_map(sid, m, None)
    }

    /// `tempo_map_import_midi {sid, path}`: the map of a MIDI file replaces the project's map.
    /// 404 for a missing file, 400 for a file that is not a MIDI file, has no tempo or
    /// time-signature event, or holds a tempo outside 4 to 1000 BPM. Answers
    /// `{map, previous, changed, ppq, tempo_events, meter_events, snapped}`.
    ///
    /// # Errors
    /// 404 when `sid` is not an open project or `path` cannot be stat-ed; 409 when the project is
    /// open read-only; 400 as listed above; 500 when the file cannot be read.
    pub fn tempo_map_import_midi(&self, sid: &str, path: &str) -> Result<Value, AppError> {
        self.inner.registry.get_writable(sid)?;
        let p = std::path::Path::new(path);
        let meta = std::fs::metadata(p).map_err(|e| AppError::not_found(format!("{path}: {e}")))?;
        if !meta.is_file() || meta.len() > IMPORT_LIMIT {
            return Err(AppError::bad(format!(
                "{path} is not a MIDI file of at most {IMPORT_LIMIT} bytes"
            )));
        }
        let bytes = std::fs::read(p).map_err(|e| AppError::internal(format!("{path}: {e}")))?;
        let got = import_midi(&bytes).map_err(AppError::bad)?;
        if got.tempo_events == 0 && got.meter_events == 0 {
            return Err(AppError::bad(format!(
                "{path} has no tempo or time-signature event: the map is not changed"
            )));
        }
        self.store_map(
            sid,
            got.map,
            Some(json!({"ppq": got.ppq, "tempo_events": got.tempo_events,
                        "meter_events": got.meter_events, "snapped": got.snapped})),
        )
    }
}

/// True for an event out of the tempo detection: the one `out` predicate of
/// [`crate::policy`] (R16 #40): the cut, the bleed rule and the old Learn filter all take a
/// hit out of the tempo, as they take it out of every other output path.
fn is_cut(e: &drumspike_core::Event) -> bool {
    crate::policy::tempo_out(e)
}

/// The answer of `tempo_map_detect`.
#[must_use]
pub fn detection_json(d: &Detection, tracks: Vec<Value>, params: &DetectParams, warnings: Vec<String>) -> Value {
    let r4 = |x: f64| (x * 10_000.0).round() / 10_000.0;
    let r6 = |x: f64| (x * 1_000_000.0).round() / 1_000_000.0;
    json!({
        "suggestion": d.map.to_json(),
        "start": {"bpm": r4(d.bpm), "error_bar": r6(d.bpm_error_bar), "confidence": d.confidence,
                  "range_edge": d.range_edge,
                  "alternatives": d.alternatives.iter().map(|a| json!({"bpm": a.bpm, "score": a.score,
                                                                        "comb": a.comb, "gated": a.gated})).collect::<Vec<_>>()},
        "bar1": {"t": r4(d.bar1_s), "confidence": d.bar1_confidence, "backbeat": d.bar1_backbeat,
                 "downbeat": d.bar1_downbeat, "alternative": r4(d.bar1_alternative_s)},
        "meter": {"value": [d.meter.0, d.meter.1], "confidence": d.meter_confidence, "feel": d.meter_feel,
                  "r2": d.meter_r2, "r3": d.meter_r3, "r4": d.meter_r4, "rule": METER_RULE},
        "steps": d.steps.iter().map(|s| json!({"t": s.t, "bpm": s.bpm, "bar": s.bar, "bars": s.bars,
                                                "confidence": s.confidence})).collect::<Vec<_>>(),
        "fit": {"on_beat": d.on_beat, "jitter_ms": d.jitter_ms, "hits": d.hits, "beats": d.beats,
                "late": d.late, "isolated": d.isolated},
        "excursions": d.excursions.iter().map(|x| json!({"t": r4(x.t), "bars": x.bars, "bpm": x.bpm, "map_bpm": r4(x.map_bpm)})).collect::<Vec<_>>(),
        "tracks": tracks,
        "params": {"min_bars": params.min_bars, "min_step_pct": params.min_step_pct},
        "warnings": warnings,
        "changed": false,
    })
}

impl Service {
    /// `tempo_map_detect {sid, tracks?, min_bars?, min_step_pct?}`: a suggested tempo map from
    /// the hits of the kick and snare tracks (default: every track with the role `kick` or
    /// `snare` that does not follow another; `tracks` names the tracks instead, any role, toms
    /// and cymbals with half weight; a named cymbal track's hits on a beat vote for beat 1).
    /// Each hit carries its velocity (ghost notes weigh little). Cut hits are left out, and hits
    /// after the end of the longest used track (they are dropped with a warning). Read only: the
    /// project's map does not change. 404 for an unknown track, 400 for bad parameters or too
    /// few hits.
    ///
    /// # Errors
    /// 400 when `min_bars` is outside the valid range or `DetectParams::validate` rejects the
    /// parameters, or when `detect` finds too few hits; 404 when `sid` is not an open project or a
    /// name in `tracks` is not a track; 400 when the stored tempo map is not valid.
    pub fn tempo_map_detect(
        &self,
        sid: &str,
        tracks: Option<&[String]>,
        min_bars: Option<i64>,
        min_step_pct: Option<f64>,
    ) -> Result<Value, AppError> {
        let params = DetectParams {
            min_bars: match min_bars {
                Some(n) => u32::try_from(n).map_err(|_| AppError::bad(format!("min_bars must be 1 to 64, not {n}")))?,
                None => DetectParams::default().min_bars,
            },
            min_step_pct: min_step_pct.unwrap_or(DetectParams::default().min_step_pct),
            ..DetectParams::default()
        };
        params.validate().map_err(AppError::bad)?;
        let op = self.inner.registry.get(sid)?;
        let (hits, used, end) = {
            let g = lock(&op);
            let doc = &g.project.doc;
            if let Some(names) = tracks {
                for n in names {
                    if doc.track(n).is_none() {
                        return Err(AppError::not_found(format!("no track named {n}")));
                    }
                }
            }
            let mut hits = Vec::new();
            let mut used = Vec::new();
            let mut end: Option<f64> = None;
            for (name, t) in doc.tracks() {
                let role = t.role.as_deref().unwrap_or("other");
                let take = match tracks {
                    Some(names) => names.iter().any(|n| n == name),
                    None => matches!(role, "kick" | "snare") && t.follows.is_none(),
                };
                if !take {
                    continue;
                }
                let r = Role::from_name(role);
                let before = hits.len();
                hits.extend(t.events().iter().filter(|e| !is_cut(e)).map(|e| Hit {
                    t: e.t,
                    role: r,
                    v: e.v.map_or(DEFAULT_VELOCITY, |v| v as f64),
                }));
                used.push(json!({"name": name, "role": role, "hits": hits.len() - before}));
                if let Some(e) = crate::edit::track_end(t) {
                    end = Some(end.map_or(e, |x: f64| x.max(e)));
                }
            }
            (hits, used, end)
        };
        let params = DetectParams {
            duration_s: end,
            ..params
        };
        let count = |r: Role| hits.iter().filter(|h| h.role == r).count();
        let mut warnings = Vec::new();
        for (r, what) in [(Role::Kick, "kick"), (Role::Snare, "snare")] {
            if count(r) < drumspike_core::tempo_detect::MIN_HITS {
                warnings.push(format!(
                    "fewer than {} {what} hits: half or double the tempo is as likely",
                    drumspike_core::tempo_detect::MIN_HITS
                ));
            }
        }
        let mut d = detect(&hits, &params).map_err(AppError::bad)?;
        let few_hits = !warnings.is_empty();
        for x in &d.excursions {
            warnings.push(format!(
                "about {} bars at {:.1} BPM from {:.2} s are shorter than min_bars ({}) and not a step: the tempo {:.1} BPM there is pulled off. Ask the user whether the tempo changes there, then run again with a lower min_bars",
                x.bars, x.bpm, x.t, params.min_bars, x.map_bpm
            ));
        }
        if d.late > 0 {
            warnings.push(format!(
                "{} hits after the end of the tracks ({:.2} s) are left out: delete or move them (events_edit)",
                d.late,
                end.unwrap_or(0.0)
            ));
        }
        if d.isolated > 0 {
            warnings.push(format!(
                "{} hits with no other hit within {ISOLATED_S} s are left out (a stray hit carries no tempo)",
                d.isolated
            ));
        }
        if d.range_edge {
            warnings.push(format!(
                "the tempo {:.1} BPM lies at the edge of the search range ({} to {} BPM): the beat may be twice or half as fast; check it",
                d.bpm, params.bpm_range.0, params.bpm_range.1
            ));
        }
        if d.meter_feel.is_some() {
            warnings.push(
                "the kick and snare pattern repeats every 2 beats: the bar may be 2/4, or 6/8 counted in dotted quarters (compound feel); check the meter".into(),
            );
        }
        if few_hits {
            d.confidence = (d.confidence * 0.25 * 1000.0).round() / 1000.0;
        }
        Ok(detection_json(&d, used, &params, warnings))
    }

    /// `tempo_map_refit_bar1 {sid, bpm}`: the beat phase of `bpm` that the kick and snare hits
    /// support, as the new time of bar 1 (`refit_bar1`, #468; read only). The page commits it
    /// with `tempo_map_set` together with the typed tempo, so one undo step restores both.
    /// 400 for a bpm that is not a positive number of at most 1000, or fewer than 8 kick and
    /// snare hits. Answers `{bar1_s, support, hits, previous_bar1_s, bpm, changed}`.
    ///
    /// # Errors
    /// 404 when no project has the id `sid`; 400 when `bpm` is not in (0, 1000], when the stored
    /// tempo map is not valid, or when `refit_bar1` finds too few hits.
    pub fn tempo_map_refit_bar1(&self, sid: &str, bpm: f64) -> Result<Value, AppError> {
        if !(bpm.is_finite() && bpm > 0.0 && bpm <= 1000.0) {
            return Err(AppError::bad(format!(
                "wrong argument `bpm` ({bpm}): a positive number of BPM, at most 1000"
            )));
        }
        let op = self.inner.registry.get(sid)?;
        let g = lock(&op);
        let map = doc_map(&g.project.doc)?;
        let mut hits = Vec::new();
        for (_, t) in g.project.doc.tracks() {
            let role = t.role.as_deref().unwrap_or("other");
            if !matches!(role, "kick" | "snare") || t.follows.is_some() {
                continue;
            }
            let r = Role::from_name(role);
            hits.extend(t.events().iter().filter(|e| !is_cut(e)).map(|e| Hit {
                t: e.t,
                role: r,
                v: e.v.map_or(DEFAULT_VELOCITY, |v| v as f64),
            }));
        }
        let n = hits.len();
        let refit = refit_bar1(&hits, bpm, map.bar1_s).map_err(AppError::bad)?;
        let r4 = |x: f64| (x * 10_000.0).round() / 10_000.0;
        Ok(json!({
            "bar1_s": r4(refit.bar1_s),
            "support": r4(refit.support),
            "hits": n,
            "previous_bar1_s": map.bar1_s,
            "bpm": bpm,
            "changed": false,
        }))
    }
}

/// Run a command of this card for the dispatcher; `None` when `cmd` is not one of them.
#[must_use]
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    if !TEMPO_COMMANDS.contains(&cmd) {
        return None;
    }
    let s = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{k}` (string)")))
    };
    Some((|| match cmd {
        "tempo_map_get" => svc.tempo_map_get(s("sid")?),
        "tempo_map_set" => {
            let map = args
                .get("map")
                .ok_or_else(|| AppError::bad("missing argument `map` (object)"))?;
            svc.tempo_map_set(s("sid")?, map)
        }
        "tempo_map_detect" => {
            let tracks: Option<Vec<String>> = match args.get("tracks") {
                None | Some(Value::Null) => None,
                Some(v) => Some(
                    serde_json::from_value(v.clone())
                        .map_err(|_| AppError::bad("wrong argument `tracks` (list of strings)"))?,
                ),
            };
            let num = |k: &str| match args.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v
                    .as_f64()
                    .map(Some)
                    .ok_or_else(|| AppError::bad(format!("wrong argument `{k}` (number)"))),
            };
            let min_bars = match args.get("min_bars") {
                None | Some(Value::Null) => None,
                Some(v) => Some(
                    v.as_i64()
                        .ok_or_else(|| AppError::bad("wrong argument `min_bars` (integer)"))?,
                ),
            };
            svc.tempo_map_detect(s("sid")?, tracks.as_deref(), min_bars, num("min_step_pct")?)
        }
        "tempo_map_refit_bar1" => {
            let bpm = args
                .get("bpm")
                .and_then(Value::as_f64)
                .ok_or_else(|| AppError::bad("missing or wrong argument `bpm` (number)"))?;
            svc.tempo_map_refit_bar1(s("sid")?, bpm)
        }
        _ => svc.tempo_map_import_midi(s("sid")?, s("path")?),
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_knows_its_commands_and_checks_the_arguments() {
        let svc = Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        assert!(dispatch(&svc, "project_list", &json!({})).is_none());
        for (cmd, args) in [
            ("tempo_map_get", json!({})),
            ("tempo_map_set", json!({"sid": "x"})),
            ("tempo_map_import_midi", json!({"sid": "x"})),
            ("tempo_map_detect", json!({})),
            ("tempo_map_detect", json!({"sid": "x", "tracks": "k"})),
            ("tempo_map_detect", json!({"sid": "x", "min_bars": 1.5})),
            ("tempo_map_detect", json!({"sid": "x", "min_bars": 0})),
            ("tempo_map_detect", json!({"sid": "x", "min_step_pct": "2"})),
            ("tempo_map_refit_bar1", json!({})),
            ("tempo_map_refit_bar1", json!({"sid": "x"})),
            ("tempo_map_refit_bar1", json!({"sid": "x", "bpm": "92"})),
        ] {
            assert_eq!(
                dispatch(&svc, cmd, &args).unwrap().unwrap_err().code,
                400,
                "{cmd} {args}"
            );
        }
        for (cmd, args) in [
            ("tempo_map_get", json!({"sid": "x"})),
            ("tempo_map_set", json!({"sid": "x", "map": {}})),
            ("tempo_map_import_midi", json!({"sid": "x", "path": "/nonexistent.mid"})),
            ("tempo_map_detect", json!({"sid": "x"})),
            ("tempo_map_refit_bar1", json!({"sid": "x", "bpm": 92.0})),
        ] {
            assert_eq!(dispatch(&svc, cmd, &args).unwrap().unwrap_err().code, 404, "{cmd}");
        }
    }
}
