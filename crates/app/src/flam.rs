//! P4-13 stage B: the flam rule in the analysis ([`drumspike_core::detect::flam`]).
//!
//! After the per-track analysis of a job and its bleed rules (and after a stored re-pick), [`run_step`] looks for the
//! second stroke of a flam inside the lobe of each output hit of the analysed snare close mics: the tracks with role
//! `snare` whose name is no sample or trigger track ([`crate::roles::is_sample_name`]), that follow no other track and
//! have hits. The bleed gate's sources are the project's other tracks that are no snare track and no sample or trigger
//! track (an offline track, one that cannot be read or one at another rate is left out); a second stroke that one of
//! them explains is not added. The outcome names them (`sources`).
//!
//! An added hit is a normal hit with `"flam": true`: output, edits, exports, the loudness cut of the track (its peak
//! level under `vel_cut_db` gives `"cut": true`), the velocity from the track's floor and ceiling. A hit the rule would
//! add within 10 ms (the rule's guard) of a hit that is out of the output (cut or bleed) is not added: that hit is the
//! stroke. The rule runs on the hits of the step: the flam hits of an earlier run that are not edited go first.
//!
//! The late-peak gate: a snare bottom mic (the wires peak after the first stroke's 12 ms window) gets no flam hits.
//! The rule runs on a track only when its late peak ([`late_peak_db`]: the median over its hits of the 12 to 30 ms
//! attack envelope over the first 12 ms) is under -3 dB; a top mic measured -6.1 to -8.8 dB, a bottom mic +0.2 to
//! +6.4 dB on 9 sessions. A alone measures the decay (a ringy top mic with a 35 ms decay has A -3 dB), so the track is
//! skipped only when A is -3 dB or more AND the share W of its hits whose 12 to 30 ms peak beats the first 12 ms is 0.4
//! or more (top mics W 0.00 to 0.23, bottom 0.52 to 0.94). No names, no levels. Both values are recorded
//! (`late_peak_db`, `late_share`). The value goes into the outcome (`late_peak_db`); a skipped track has
//! `on: false` and the reason [`LATE_REASON`]. `sources` names only the mics with a gain.
//!
//! The version goes into the track's `detector` record (`flam_rule`) with the outcome (`flam`: `{on, reason, added,
//! gated, sources}`). A project analysed before P4-13 stage B has no `flam_rule` (version 0, no rule): its hits stay
//! until Re-analyze, and a re-pick runs the version its analysis recorded. `DRUMSPIKE_FLAM_RULE` pins the version
//! (`0`: no rule). The track param `flam_split` (`false`: the Inspector's "Split flams" off) turns the rule off for one
//! track; it applies on the next re-analyze or re-pick.

use std::path::PathBuf;

use drumspike_core::detect::flam::{
    FLAM_RULE_VERSION, FlamParams, Second, applies, attack_env, late_peak_db, late_peaking, late_share, second_onsets,
    transfer_gains,
};
use drumspike_core::project::Event;
use drumspike_core::velocity::{peak_db, velocity_of_db};
use serde_json::{Map, Value, json};

use crate::media::mix_mode;
use crate::registry::{Shared, lock};
use crate::roles::is_sample_name;
use crate::{Service, ServiceConfig};

/// The version a new analysis runs (`detector.flam_rule`).
pub const FLAM_RULE: u32 = FLAM_RULE_VERSION;

/// `DRUMSPIKE_FLAM_RULE` as a version: `0` (no rule) up to [`FLAM_RULE`]; `None` for anything else.
#[must_use]
pub fn parse_flam_rule(v: &str) -> Option<u32> {
    v.trim().parse::<u32>().ok().filter(|&n| n <= FLAM_RULE)
}

/// The version of a new analysis: the configuration's pin, else [`FLAM_RULE`].
#[must_use]
pub fn analysis_flam_rule(cfg: &ServiceConfig) -> u32 {
    cfg.flam_rule.unwrap_or(FLAM_RULE)
}

/// The version of a re-pick: the configuration's pin, else the one the track's `detector` record carries. No record
/// (an analysis from before P4-13 stage B) is version 0; a version this build does not know runs the current one.
#[must_use]
pub fn recorded_flam_rule(cfg: &ServiceConfig, detector: Option<&Value>) -> u32 {
    if let Some(v) = cfg.flam_rule {
        return v;
    }
    match detector.and_then(|d| d.get("flam_rule")) {
        None | Some(Value::Null) => 0,
        Some(v) => v
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|&v| v <= FLAM_RULE)
            .unwrap_or(FLAM_RULE),
    }
}

/// A snare close mic of the rule (a target): role `snare` and no sample or trigger track.
#[must_use]
pub fn is_snare_close_mic(name: &str, role: Option<&str>) -> bool {
    role.is_some_and(|r| applies(FLAM_RULE, r)) && !is_sample_name(name)
}

/// A source of the bleed gate: no snare track and no sample or trigger track.
#[must_use]
pub fn is_source(name: &str, role: Option<&str>) -> bool {
    role != Some("snare") && !is_sample_name(name)
}

/// The track's switch: the param `flam_split` (absent: on).
#[must_use]
pub fn switched_on(params: Option<&Map<String, Value>>) -> bool {
    match params.and_then(|m| m.get("flam_split")) {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(v) => v.as_f64().is_none_or(|f| f != 0.0),
    }
}

/// True for an event the rule added (`"flam": true`).
#[must_use]
pub fn is_flam(e: &Event) -> bool {
    e.extra.get("flam") == Some(&Value::Bool(true))
}

/// An edited hit (added by hand, moved or restored): the rule never removes it.
fn edited(e: &Event) -> bool {
    e.added == Some(true) || e.orig.is_some_and(|o| (o - e.t).abs() > 1e-9) || e.edited == Some(true)
}

/// Out of the output: cut, marked bleed or taken out by the Learn filter ([`crate::histogram::event_cut`]).
fn out_of_output(e: &Event) -> bool {
    crate::histogram::event_cut(e)
}

/// The reason the rule does not run on a late-peaking mic (a snare bottom mic).
pub const LATE_REASON: &str = "late-peaking mic (snare bottom)";

/// The events of one target after the rule: the events without the not-edited flam hits of an earlier run, plus the
/// strokes `found` adds (sorted by time). `x`: the target's signal; `params`: its track params (velocity map and the
/// loudness cut). Returns the events and the number added.
#[must_use]
pub fn with_flams(
    events: &[Event],
    found: &[Second],
    x: &[f32],
    sr: u32,
    params: Option<&Map<String, Value>>,
    guard_s: f64,
) -> (Vec<Event>, usize) {
    let mut out: Vec<Event> = events.iter().filter(|e| !is_flam(e) || edited(e)).cloned().collect();
    let map = crate::follow::velocity_map_of(params);
    let cut = crate::histogram::cut_of(params);
    let held: Vec<f64> = out.iter().filter(|e| out_of_output(e)).map(|e| e.t).collect();
    let mut n = 0;
    for s in found.iter().filter(|s| !s.bleed) {
        if held.iter().any(|&t| (t - s.t).abs() <= guard_s) {
            continue;
        }
        let db = peak_db(x, s.t, sr);
        let mut e = Event::new(s.t, velocity_of_db(db, &map));
        e.extra.insert("flam".into(), Value::Bool(true));
        if cut.is_some_and(|c| db < c) {
            e.extra.insert("cut".into(), Value::Bool(true));
        }
        out.push(e);
        n += 1;
    }
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    (out, n)
}

struct Mic {
    name: String,
    path: PathBuf,
    mix: String,
}

struct Target {
    mic: Mic,
    version: u32,
    on: bool,
    events: Vec<Event>,
    params: Option<Map<String, Value>>,
}

/// Run the rule after an analysis: `targets` are the analysed tracks with the version each gets (the job: the
/// analysis version; a re-pick: the recorded one). Adds the second strokes to the targets that are snare close mics,
/// records the version and the outcome in their `detector`, refreshes their followers. Returns `{rule, tracks: {name:
/// added}, sources}`, or `None` when no target is a snare close mic or every target has version 0.
pub(crate) fn run_step(svc: &Service, op: &Shared, targets: &[(String, u32)]) -> Option<Value> {
    let (tgt, srcs) = {
        let g = lock(op);
        let mut tgt = Vec::new();
        let mut srcs = Vec::new();
        for name in g.project.doc.track_order() {
            let Ok(tr) = g.track(&name) else { continue };
            let role = tr.role.as_deref();
            if g.offline(&name) {
                continue;
            }
            let Ok(path) = g.source(&name) else { continue };
            let mic = Mic {
                name: name.clone(),
                path,
                mix: tr.mix_mode_or_default().to_string(),
            };
            let version = targets.iter().find(|(n, _)| *n == name).map(|&(_, v)| v);
            if is_snare_close_mic(&name, role) {
                if let (Some(version), None, Some(events)) = (version, &tr.follows, &tr.events) {
                    tgt.push(Target {
                        mic,
                        version,
                        on: switched_on(tr.params.as_ref()),
                        events: events.clone(),
                        params: tr.params.clone(),
                    });
                }
            } else if is_source(&name, role) {
                srcs.push(mic);
            }
        }
        (tgt, srcs)
    };
    let version = tgt.iter().map(|t| t.version).max().unwrap_or(0);
    if version == 0 {
        return None; // no rule (a store wrote a new record without `flam_rule`)
    }
    let p = FlamParams::default();
    // the sources' envelopes, once for every target (only when a target runs the rule)
    let run_any = tgt.iter().any(|t| t.version >= 1 && t.on);
    let mut src_env: Vec<(String, u32, Vec<f64>)> = Vec::new();
    if run_any {
        for m in &srcs {
            if let Ok(d) = svc.inner.media.decode_cached(&m.path, mix_mode(&m.mix)) {
                src_env.push((m.name.clone(), d.sr, attack_env(&d.samples, d.sr, &p).0));
            }
        }
    }
    let mut tracks = Map::new();
    let mut used_all: Vec<String> = Vec::new();
    let mut changed = Vec::new();
    for t in &tgt {
        // the outcome of this target: new events and the record, or the reason it is off
        let (events, rec): (Option<Vec<Event>>, Value) = if t.version == 0 {
            (None, Value::Null)
        } else if !t.on {
            let kept: Vec<Event> = t.events.iter().filter(|e| !is_flam(e) || edited(e)).cloned().collect();
            (
                Some(kept),
                json!({"on": false, "reason": "Split flams is off for this track", "added": 0, "gated": 0}),
            )
        } else {
            match svc.inner.media.decode_cached(&t.mic.path, mix_mode(&t.mic.mix)) {
                Err(e) => (
                    None,
                    json!({"on": false, "reason": format!("{} cannot be read: {}", t.mic.name, e.message),
                           "added": 0, "gated": 0}),
                ),
                Ok(d) => {
                    let base: Vec<&Event> = t.events.iter().filter(|e| !is_flam(e) || edited(e)).collect();
                    let hits: Vec<f64> = base.iter().filter(|e| !out_of_output(e)).map(|e| e.t).collect();
                    let (env, hop) = attack_env(&d.samples, d.sr, &p);
                    let same_rate: Vec<&(String, u32, Vec<f64>)> =
                        src_env.iter().filter(|(_, sr, _)| *sr == d.sr).collect();
                    let refs: Vec<&[f64]> = same_rate.iter().map(|(_, _, e)| e.as_slice()).collect();
                    let gains = transfer_gains(&env, &refs, &hits, d.sr, hop);
                    let used: Vec<(&[f64], f64)> = refs
                        .iter()
                        .zip(&gains)
                        .filter_map(|(e, g)| g.map(|g| (&e[..e.len().min(env.len())], g)))
                        .collect();
                    // the sources with a gain (the others took no part in the gate)
                    let names: Vec<String> = same_rate
                        .iter()
                        .zip(&gains)
                        .filter(|(_, g)| g.is_some())
                        .map(|((n, _, _), _)| n.clone())
                        .collect();
                    // the late-peak gate: a snare bottom mic peaks after the first stroke's window
                    let late = late_peak_db(&env, &hits, d.sr, hop, &p);
                    let share = late_share(&env, &hits, d.sr, hop, &p);
                    if late_peaking(late, share) {
                        let kept: Vec<Event> = t.events.iter().filter(|e| !is_flam(e) || edited(e)).cloned().collect();
                        (
                            Some(kept),
                            json!({"on": false, "reason": LATE_REASON, "late_peak_db": late,
                                   "late_share": share, "added": 0, "gated": 0}),
                        )
                    } else {
                        let found = second_onsets(&d.samples, d.sr, &hits, &p, &env, &used);
                        let (events, n) = with_flams(&t.events, &found, &d.samples, d.sr, t.params.as_ref(), p.guard_s);
                        let gated = found.iter().filter(|s| s.bleed).count();
                        for nm in &names {
                            if !used_all.contains(nm) {
                                used_all.push(nm.clone());
                            }
                        }
                        (
                            Some(events),
                            json!({"on": true, "reason": Value::Null, "late_peak_db": late, "late_share": share,
                                   "added": n,
                                   "gated": gated, "sources": names}),
                        )
                    }
                }
            }
        };
        let mut g = lock(op);
        let Ok(tr) = g.track_mut(&t.mic.name) else { continue };
        let same = tr.events().len() == t.events.len() && tr.events().iter().zip(&t.events).all(|(a, b)| a.t == b.t);
        if !same {
            continue; // the hits changed while the rule ran: they keep what they have
        }
        let added = rec.get("added").and_then(Value::as_u64).unwrap_or(0);
        if let Some(ev) = events {
            tr.events = Some(ev);
        }
        if let Some(Value::Object(det)) = tr.extra.get_mut("detector") {
            if t.version == 0 {
                det.shift_remove("flam_rule");
                det.shift_remove("flam");
            } else {
                det.insert("flam_rule".into(), json!(t.version));
                det.insert("flam".into(), rec);
            }
        }
        g.mark_dirty();
        drop(g);
        if t.version > 0 {
            tracks.insert(t.mic.name.clone(), json!(added));
            changed.push(t.mic.name.clone());
        }
    }
    for name in &changed {
        crate::follow::refresh_followers(svc, op, name);
    }
    Some(json!({"rule": version, "tracks": tracks, "sources": used_all}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use drumspike_core::detect::flam::Second;

    fn cfg(pin: Option<u32>) -> ServiceConfig {
        ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: pin,
            cross_bleed: None,
            ghost_gate: None,
        }
    }

    #[test]
    fn versions_of_an_analysis_and_of_a_repick() {
        assert_eq!(parse_flam_rule(" 1 "), Some(1));
        assert_eq!(parse_flam_rule("0"), Some(0));
        assert_eq!(parse_flam_rule("2"), None);
        assert_eq!(parse_flam_rule("on"), None);
        assert_eq!(analysis_flam_rule(&cfg(None)), FLAM_RULE);
        assert_eq!(analysis_flam_rule(&cfg(Some(0))), 0);
        let det = json!({"detector": "rust-dsp", "refine": 3, "flam_rule": 1});
        assert_eq!(recorded_flam_rule(&cfg(None), Some(&det)), 1);
        assert_eq!(
            recorded_flam_rule(&cfg(None), Some(&json!({"refine": 3}))),
            0,
            "before stage B: no rule"
        );
        assert_eq!(recorded_flam_rule(&cfg(None), None), 0);
        assert_eq!(
            recorded_flam_rule(&cfg(None), Some(&json!({"flam_rule": 9}))),
            FLAM_RULE
        );
        assert_eq!(recorded_flam_rule(&cfg(Some(0)), Some(&det)), 0, "the pin wins");
    }

    #[test]
    fn targets_sources_and_the_switch() {
        assert!(is_snare_close_mic("s top", Some("snare")));
        assert!(is_snare_close_mic("s bot", Some("snare")));
        assert!(!is_snare_close_mic("s samp savior", Some("snare")), "a sample layer");
        assert!(!is_snare_close_mic("k in", Some("kick")));
        assert!(!is_snare_close_mic("s top", None));
        assert!(is_source("k in", Some("kick")) && is_source("hh", Some("hat")) && is_source("room", None));
        assert!(!is_source("s bot", Some("snare")), "the same drum");
        assert!(!is_source("k samp", Some("kick")), "a sample layer");
        assert!(switched_on(None));
        let m = |v: Value| json!({"flam_split": v}).as_object().cloned();
        assert!(switched_on(m(Value::Null).as_ref()));
        assert!(switched_on(m(json!(true)).as_ref()));
        assert!(!switched_on(m(json!(false)).as_ref()));
        assert!(!switched_on(m(json!(0)).as_ref()));
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

    #[test]
    fn the_added_hits_are_tagged_sorted_and_take_the_tracks_velocity_and_cut() {
        let sr = 48_000;
        let mut x = vec![0.0f32; sr as usize * 2];
        x[(1.03 * f64::from(sr)) as usize + 10] = 0.5; // -6 dB at the second stroke
        x[(1.53 * f64::from(sr)) as usize + 10] = 0.001; // -60 dB: under a cut at -50
        let mut old = Event::new(1.2, 90);
        old.extra.insert("flam".into(), Value::Bool(true));
        let mut moved = Event::new(1.7, 90);
        moved.extra.insert("flam".into(), Value::Bool(true));
        moved.orig = Some(1.69);
        let mut cut = Event::new(1.805, 30);
        cut.extra.insert("cut".into(), Value::Bool(true));
        let events = vec![Event::new(1.0, 100), old, Event::new(1.5, 100), moved, cut];
        let found = [
            second(1.03, false),
            second(1.53, false),
            second(1.6, true),
            second(1.8, false),
        ];
        let params = json!({"vel_lo_db": -40.0, "vel_hi_db": -6.0, "vel_cut_db": -50.0});
        let (out, n) = with_flams(&events, &found, &x, sr, params.as_object(), 0.010);
        assert_eq!(n, 2, "the bleed stroke and the one at the cut hit are not added");
        let times: Vec<f64> = out.iter().map(|e| e.t).collect();
        assert_eq!(
            times,
            vec![1.0, 1.03, 1.5, 1.53, 1.7, 1.805],
            "the old flam hit goes, the moved one stays"
        );
        assert_eq!(out[1].v, Some(127));
        assert!(is_flam(&out[1]) && is_flam(&out[3]));
        assert!(out[1].extra.get("cut").is_none());
        assert_eq!(out[3].extra.get("cut"), Some(&Value::Bool(true)));
        assert!(!is_flam(&out[0]));
    }
}
