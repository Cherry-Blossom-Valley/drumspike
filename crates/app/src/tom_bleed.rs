//! P4-14 stage 2: the tom bleed rule CD in the analysis ([`drumspike_core::detect::tom_bleed`]).
//!
//! After the per-track analysis of a job (and after a stored re-pick), [`run_step`] runs the rule over the tom close
//! mics of the project: the tracks with role `tom` whose name is not a sample or trigger track
//! ([`crate::roles::is_sample_name`]: `t1 sampl`, `t samp`, `t trigger`), that follow no other track and have hits.
//! Two mics of one drum ([`crate::roles::drum_of`]: `t1 top` and `t1 bot`) never explain each other; when a name does
//! not tell the drum, the rule is off and says so. Only the
//! analysed tracks (the job's targets, the re-picked track) change; the other tom mics are sources as they are.
//!
//! A hit the rule marks keeps its place in the events with `"bleed": true` and leaves the output exactly as a hit of
//! the P4-3 loudness cut does ([`crate::export::is_cut`], [`crate::histogram::event_cut`]): MIDI, spikes, Export All,
//! the click, a follower, the tempo detection and Learn. The page shows it as it shows a cut hit (grey); how such hits
//! show in the end waits for the owner (checklist `tom-bleed-show`). An edited hit (added or moved) never carries the
//! mark, and the `restore` op of `events_edit` takes it off one hit.
//!
//! The rule's version goes into the track's `detector` record (`bleed_rule`, as `refine`) with the outcome (`bleed`:
//! `{on, reason, removed, played}`); the job answer carries the session's outcome (`bleed`). A project analysed before
//! P4-14 has no `bleed_rule` (version 0, no rule): its hits stay until Re-analyze, and a re-pick runs the version its
//! analysis recorded. The rule is off, and the analysis says why, when a tom mic is edited to digital silence or fewer
//! than two drums have a played tom mic.
//!
//! Version 2 (P4-17 stage 2) adds the session's kick and snare close mics as sources ([`source_kind`]: role `kick` or
//! `snare`, no sample or trigger name, no follower, with hits): a tom hit is also bleed when its snare excess is under
//! 9.2 dB or its kick excess under 4.0 dB ([`drumspike_core::detect::tom_bleed::tom_bleed_with`]). They are never
//! targets. The outcome names them (`sources`). A source that cannot be read or has another sample rate is left out.
//! Version 1 analyses keep their hits until Re-analyze; a re-pick runs the version its analysis recorded.
//!
//! Version 3 (P4-21) adds the tom pair rule ([`drumspike_core::detect::tom_bleed::tom_pairs`], winner takes all) after
//! version 2, on the hits still in the output: of two hits on tom close mics of different drums within 12 ms the one
//! 6 dB or more lower (against the median level of its mic's output hits) is marked `"bleed": true` like a rule CD hit.
//! It needs no bleed between the hits, so it runs when rule CD is off (tom tracks edited to digital silence); it is off
//! when rule CD is off for "fewer than two played" (kick and snare bleed on an unplayed tom mic could win there), and
//! when a drum cannot be told from a track name, or a mic cannot be read or has another rate. The outcome of the
//! pairs is `bleed.pairs` of the track's `detector` and of the job answer (`{on, reason, removed}`). Analyses with
//! version 2 keep their marks until Re-analyze; a re-pick runs the version its analysis recorded.
//!
//! Version 4 (#182) adds the pair rule's own-mic attack rise keep guard: a hit the pair rule would mark stays when
//! its own-mic attack rise (`drumspike_core::detect::tom_bleed::stroke_rise_db`, the P4-18 rise test) is 10 dB or
//! more. #177 measured 46 pair-rule merges on the toms of the 3 real sessions, 42 of them real strokes; the guard
//! keeps all 42 and unmarks no bleed merge. The guard applies to a mic from version 4: a recorded version 3 re-pick
//! next to a version 4 target runs the version 3 pair rule on its hits.

use std::path::PathBuf;

use drumspike_core::detect::tom_bleed::{
    tom_bleed_with, tom_pairs, Off, PairMic, SourceKind, SourceMic, TomBleedParams, TomMic, RULE_VERSION,
};
use serde_json::{json, Map, Value};

use crate::error::AppError;
use crate::media::mix_mode;
use crate::registry::{lock, Shared};
use crate::roles::{drum_of, is_sample_name};
use crate::{Service, ServiceConfig};

/// The version a new analysis runs (`detector.bleed_rule`).
pub const BLEED_RULE: u32 = RULE_VERSION;

/// `DRUMSPIKE_BLEED_RULE` as a version: `0` (no rule) up to [`BLEED_RULE`]; `None` for anything else.
pub fn parse_bleed_rule(v: &str) -> Option<u32> {
    v.trim().parse::<u32>().ok().filter(|&n| n <= BLEED_RULE)
}

/// The version of a new analysis: the configuration's pin, else [`BLEED_RULE`].
pub fn analysis_bleed_rule(cfg: &ServiceConfig) -> u32 {
    cfg.bleed_rule.unwrap_or(BLEED_RULE)
}

/// The version of a re-pick: the configuration's pin, else the one the track's `detector` record carries. No record
/// (an analysis from before P4-14) is version 0; a version this build does not know runs the current one.
pub fn recorded_bleed_rule(cfg: &ServiceConfig, detector: Option<&Value>) -> u32 {
    if let Some(v) = cfg.bleed_rule {
        return v;
    }
    match detector.and_then(|d| d.get("bleed_rule")) {
        None | Some(Value::Null) => 0,
        Some(v) => v
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|&v| v <= BLEED_RULE)
            .unwrap_or(BLEED_RULE),
    }
}

/// A tom close mic of the rule: role `tom` and no sample or trigger track.
pub fn is_tom_close_mic(name: &str, role: Option<&str>) -> bool {
    role == Some("tom") && !is_sample_name(name)
}

/// Version 2: the kind of a source mic of the rule: role `kick` or `snare` and no sample or trigger track.
pub fn source_kind(name: &str, role: Option<&str>) -> Option<SourceKind> {
    if is_sample_name(name) {
        return None;
    }
    match role {
        Some("snare") => Some(SourceKind::Snare),
        Some("kick") => Some(SourceKind::Kick),
        _ => None,
    }
}

/// The first rule version with the kick and snare sources.
const SOURCES_FROM: u32 = 2;

/// The start of the reason when rule CD is off because fewer than two tom close mics are played ([`off_reason`]).
const PLAYED_OFF: &str = "fewer than two played";

/// The first rule version with the tom pair rule.
const PAIRS_FROM: u32 = 3;

/// The first rule version with the pair rule's own-mic attack rise keep guard (#182).
const GUARD_FROM: u32 = 4;

/// `{on, reason, removed}` of the pair rule: `reason` names why it is off.
fn pair_record(pairs: &Option<Result<Vec<Vec<bool>>, String>>, removed: usize) -> Value {
    match pairs {
        Some(Err(r)) => json!({"on": false, "reason": r, "removed": 0}),
        _ => json!({"on": true, "reason": Value::Null, "removed": removed}),
    }
}

/// True for an event the rule marked (`"bleed": true`).
pub fn is_bleed(e: &Value) -> bool {
    e.get("bleed") == Some(&Value::Bool(true))
}

/// An edited hit (added, moved from `orig`, or restored: `edited: true`, as [`crate::edit::is_kept`]): the rule never
/// marks it.
fn edited(e: &drumspike_core::project::Event) -> bool {
    e.added == Some(true) || e.orig.is_some_and(|o| (o - e.t).abs() > 1e-9) || e.edited == Some(true)
}

/// The text of an off state, with the track names.
pub fn off_reason(off: &Off, names: &[String]) -> String {
    let list = |v: &[usize]| {
        if v.is_empty() {
            "none".to_string()
        } else {
            v.iter().map(|&i| names[i].as_str()).collect::<Vec<_>>().join(", ")
        }
    };
    match off {
        Off::DigitalSilence { mics } => format!(
            "tom tracks edited to digital silence ({}): the rule needs the bleed between the hits",
            list(mics)
        ),
        Off::FewerThanTwoPlayed { played } => {
            format!("{PLAYED_OFF} tom close mics (played: {})", list(played))
        }
    }
}

struct Mic {
    name: String,
    /// `None` when the name does not tell the drum ([`drum_of`]): the rule is off.
    drum: Option<String>,
    /// Version 2: a kick or snare close mic (a source only); `None` for a tom close mic.
    kind: Option<SourceKind>,
    path: PathBuf,
    mix: String,
    times: Vec<f64>,
    /// Version 3: per hit, the loudness cut took it out of the output.
    cut: Vec<bool>,
    /// Version 3: per hit, an earlier run's `bleed` mark (it counts for a mic that is no target of this run).
    bled: Vec<bool>,
    /// Version 3: per hit, an edited hit (the pair rule never marks it).
    edited: Vec<bool>,
}

/// Run the rule after an analysis: `targets` are the analysed tracks with the version each gets (the job: the
/// analysis version; a re-pick: the recorded one). Marks the hits of the targets that are tom close mics, records the
/// version and the outcome in their `detector`, refreshes their followers. Returns the session's outcome `{rule, on,
/// reason, played, tracks: {name: removed}}`, or `None` when no target is a tom close mic.
pub(crate) fn run_step(svc: &Service, op: &Shared, targets: &[(String, u32)]) -> Option<Value> {
    let (mics, srcs, tgt) = {
        let g = lock(op);
        let mut mics = Vec::new();
        let mut srcs = Vec::new();
        for name in g.project.doc.track_order() {
            let Ok(tr) = g.track(&name) else { continue };
            let role = tr.role.as_deref();
            let kind = source_kind(&name, role);
            if !(is_tom_close_mic(&name, role) || kind.is_some()) || tr.follows.is_some() || tr.events.is_none() {
                continue;
            }
            if g.offline(&name) {
                continue;
            }
            let Ok(path) = g.source(&name) else { continue };
            let evs: Vec<&drumspike_core::project::Event> = tr.events().iter().collect();
            let m = Mic {
                drum: drum_of(&name),
                kind,
                times: evs.iter().map(|e| e.t).collect(),
                cut: evs
                    .iter()
                    .map(|e| e.extra.get("cut") == Some(&Value::Bool(true)))
                    .collect(),
                bled: evs
                    .iter()
                    .map(|e| e.extra.get("bleed") == Some(&Value::Bool(true)))
                    .collect(),
                edited: evs.iter().map(|e| edited(e)).collect(),
                mix: tr.mix_mode_or_default().to_string(),
                path,
                name,
            };
            if kind.is_some() {
                srcs.push(m);
            } else {
                mics.push(m);
            }
        }
        let tgt: Vec<(usize, u32)> = targets
            .iter()
            .filter_map(|(n, v)| mics.iter().position(|m| &m.name == n).map(|i| (i, *v)))
            .collect();
        (mics, srcs, tgt)
    };
    if tgt.is_empty() {
        return None;
    }
    let version = tgt.iter().map(|&(_, v)| v).max().unwrap_or(0);
    let names: Vec<String> = mics.iter().map(|m| m.name.clone()).collect();
    // the rule's answer: per mic the marks, or the reason it is off
    // fix round 3: `shared` names the pairs of mics counted as one drum (a hot preamp on a mic that hears a drum)
    let srcs: &[Mic] = if version >= SOURCES_FROM { &srcs } else { &[] };
    let (outcome, shared, used): (Marks, Vec<String>, Vec<String>) = if version == 0 {
        (Err(OffRule::plain(String::new())), Vec::new(), Vec::new())
    } else {
        rule(svc, &mics, srcs, &names)
    };
    // version 3: the pair rule on the hits version 2 left in
    let pair_versions: Vec<u32> = (0..mics.len())
        .map(|i| tgt.iter().find(|&&(t, _)| t == i).map_or(0, |&(_, v)| v))
        .collect();
    // the pair rule is off when rule CD is off for "fewer than two played": kick and snare bleed on an unplayed tom mic
    // could win there and mark a soft real hit; it stays on for tom tracks edited to digital silence
    let pairs: Option<Result<Vec<Vec<bool>>, String>> =
        (pair_versions.iter().any(|&v| v >= PAIRS_FROM)).then(|| match &outcome {
            Err(r) if r.reason.starts_with(PLAYED_OFF) => {
                Err(format!("the pair rule is off with rule CD: {}", r.reason))
            }
            _ => pair_rule(svc, &mics, &pair_versions, outcome.as_ref().ok().map(|(b, _)| b)),
        });
    let mut tracks = Map::new();
    let mut pair_tracks = Map::new();
    let mut changed = Vec::new();
    {
        let mut g = lock(op);
        for &(i, v) in &tgt {
            let Ok(tr) = g.track_mut(&mics[i].name) else { continue };
            let same = tr.events().len() == mics[i].times.len()
                && tr.events().iter().zip(&mics[i].times).all(|(e, &t)| e.t == t);
            if !same {
                continue; // the hits changed while the rule ran: they keep what they have
            }
            let marks = match (&outcome, v) {
                (Ok((bleed, _)), 1..) => Some(&bleed[i]),
                // version 5: rule CD off for fewer than two played still applies the guarded shared kick marks
                (Err(off), 5..) => off.kick_marks.as_ref().map(|m| &m[i]),
                _ => None,
            };
            let pair_marks = match (&pairs, v >= PAIRS_FROM) {
                (Some(Ok(pm)), true) => Some(&pm[i]),
                _ => None,
            };
            let mut removed = 0;
            let mut pair_removed = 0;
            for (j, e) in tr.events.iter_mut().flatten().enumerate() {
                let by_pair = pair_marks.is_some_and(|m| m[j]);
                let on = (marks.is_some_and(|m| m[j]) || by_pair) && !edited(e);
                if on {
                    e.extra.insert("bleed".into(), Value::Bool(true));
                    removed += 1;
                    pair_removed += usize::from(by_pair && !marks.is_some_and(|m| m[j]));
                } else {
                    e.extra.shift_remove("bleed");
                }
            }
            if let Some(Value::Object(det)) = tr.extra.get_mut("detector") {
                if v == 0 {
                    det.shift_remove("bleed_rule");
                    det.shift_remove("bleed");
                } else {
                    let (on, reason, played) = match &outcome {
                        Ok((_, played)) => (true, Value::Null, played[i]),
                        Err(r) => (false, Value::from(r.reason.as_str()), false),
                    };
                    det.insert("bleed_rule".into(), json!(v));
                    let mut rec = json!({"on": on, "reason": reason, "removed": removed, "played": played});
                    if !shared.is_empty() {
                        rec["shared"] = json!(shared);
                    }
                    if v >= SOURCES_FROM && !used.is_empty() {
                        rec["sources"] = json!(used);
                    }
                    if v >= PAIRS_FROM {
                        rec["pairs"] = pair_record(&pairs, pair_removed);
                    }
                    det.insert("bleed".into(), rec);
                }
            }
            tracks.insert(mics[i].name.clone(), json!(removed));
            if v >= PAIRS_FROM {
                pair_tracks.insert(mics[i].name.clone(), json!(pair_removed));
            }
            changed.push(mics[i].name.clone());
        }
        g.mark_dirty();
    }
    for name in &changed {
        crate::follow::refresh_followers(svc, op, name);
    }
    let (on, reason, played) = match &outcome {
        Ok((_, played)) => (
            true,
            Value::Null,
            names
                .iter()
                .zip(played)
                .filter(|(_, &p)| p)
                .map(|(n, _)| Value::from(n.as_str()))
                .collect(),
        ),
        Err(r) if version > 0 => (false, Value::from(r.reason.as_str()), Vec::new()),
        Err(_) => (false, Value::Null, Vec::new()),
    };
    let mut out = json!({"rule": version, "on": on, "reason": reason, "played": played, "tracks": tracks});
    if !shared.is_empty() {
        out["shared"] = json!(shared);
    }
    if !used.is_empty() {
        out["sources"] = json!(used);
    }
    if pairs.is_some() {
        let mut rec = pair_record(
            &pairs,
            pair_tracks.values().filter_map(Value::as_u64).sum::<u64>() as usize,
        );
        rec["tracks"] = Value::Object(pair_tracks);
        out["pairs"] = rec;
    }
    Some(out)
}

/// The targets of the rule after a stored re-pick of `name` (its recorded version `v`): that track and every other
/// tom close mic of the session with the version its own analysis recorded (none recorded: left out), so the pair rule
/// judges the toms together as the analysis job does (a re-picked tom can win or lose a pair on another tom).
pub(crate) fn repick_targets(svc: &Service, op: &Shared, name: &str, v: u32) -> Vec<(String, u32)> {
    let g = lock(op);
    let mut out = vec![(name.to_string(), v)];
    for n in g.project.doc.track_order() {
        if n == name {
            continue;
        }
        let Ok(tr) = g.track(&n) else { continue };
        if !is_tom_close_mic(&n, tr.role.as_deref()) || tr.follows.is_some() || tr.events.is_none() {
            continue;
        }
        let rv = recorded_bleed_rule(&svc.inner.config, tr.extra.get("detector"));
        if rv > 0 {
            out.push((n, rv));
        }
    }
    out
}

/// [`run_step`] for an open project `sid`: the step the job and the re-pick run after they store (the tests and a
/// tool that set the hits by hand call it).
pub fn run_after(svc: &Service, sid: &str, targets: &[(String, u32)]) -> Result<Option<Value>, AppError> {
    let op = svc.inner.registry.get(sid)?;
    Ok(run_step(svc, &op, targets))
}

/// Version 3: the pair rule over the tom close mics. The output hits are those out of the cut and out of version 2's marks
/// (`v2`: the marks of this run, `None` when rule CD is off); a mic that is no target of this run brings its earlier
/// `bleed` marks. Only a target with version 3 or more can be marked, and never an edited hit. `Err`: why it is off.
fn pair_rule(
    svc: &Service,
    mics: &[Mic],
    versions: &[u32],
    v2: Option<&Vec<Vec<bool>>>,
) -> Result<Vec<Vec<bool>>, String> {
    let unknown: Vec<&str> = mics
        .iter()
        .filter(|m| m.drum.is_none())
        .map(|m| m.name.as_str())
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "the drum of {} cannot be told from the track name",
            unknown.join(", ")
        ));
    }
    let mut decoded = Vec::with_capacity(mics.len());
    for m in mics {
        let d = svc
            .inner
            .media
            .decode_cached(&m.path, mix_mode(&m.mix))
            .map_err(|e| format!("{} cannot be read: {}", m.name, e.message))?;
        decoded.push(d);
    }
    let sr = decoded.first().map_or(0, |d| d.sr);
    if decoded.iter().any(|d| d.sr != sr) {
        return Err("the tom tracks have different sample rates".into());
    }
    let output: Vec<Vec<bool>> = mics
        .iter()
        .enumerate()
        .map(|(i, m)| {
            (0..m.times.len())
                .map(|j| {
                    let marked = match (versions[i], v2) {
                        (0, _) => m.bled[j],
                        (_, Some(b)) => b[i][j],
                        (_, None) => false,
                    };
                    // a hit rule CD would mark but that is edited (restored) is in the output
                    !m.cut[j] && !(marked && !m.edited[j])
                })
                .collect()
        })
        .collect();
    let target: Vec<Vec<bool>> = mics
        .iter()
        .enumerate()
        .map(|(i, m)| {
            (0..m.times.len())
                .map(|j| versions[i] >= PAIRS_FROM && !m.edited[j])
                .collect()
        })
        .collect();
    let input: Vec<PairMic> = mics
        .iter()
        .zip(&decoded)
        .enumerate()
        .map(|(i, (m, d))| PairMic {
            drum: m.drum.as_deref().expect("checked above"),
            signal: &d.samples,
            times: &m.times,
            output: &output[i],
            target: &target[i],
            guarded: versions[i] >= GUARD_FROM,
        })
        .collect();
    Ok(tom_pairs(&input, sr, &TomBleedParams::default()))
}

/// Decode the mics and run the core rule: per mic the marks and whether it is played, or why it is off. When rule CD
/// is off for fewer than two played, version 5 still applies the guarded shared kick marks (#362): the error carries
/// them (`kick_marks`, `None` for digital silence and for the other off states).
type Marks = Result<(Vec<Vec<bool>>, Vec<bool>), OffRule>;

/// Why the rule is off, with the marks version 5 applies despite the off state.
struct OffRule {
    reason: String,
    kick_marks: Option<Vec<Vec<bool>>>,
}

impl OffRule {
    fn plain(reason: String) -> Self {
        OffRule {
            reason,
            kick_marks: None,
        }
    }
}

/// [`rule_marks`] with the notes of the shared pairs counted as one drum ("t3 and t4 hear one drum: counted as one")
/// and the names of the kick and snare sources the rule read (version 2).
fn rule(svc: &Service, mics: &[Mic], srcs: &[Mic], names: &[String]) -> (Marks, Vec<String>, Vec<String>) {
    let mut shared = Vec::new();
    let mut used = Vec::new();
    let marks = rule_marks(svc, mics, srcs, names, &mut shared, &mut used);
    (marks, shared, used)
}

fn rule_marks(
    svc: &Service,
    mics: &[Mic],
    srcs: &[Mic],
    names: &[String],
    shared: &mut Vec<String>,
    used: &mut Vec<String>,
) -> Marks {
    let unknown: Vec<&str> = mics
        .iter()
        .filter(|m| m.drum.is_none())
        .map(|m| m.name.as_str())
        .collect();
    if !unknown.is_empty() {
        // two mics of one drum must never explain each other: without the drum the rule cannot tell them
        return Err(OffRule::plain(format!(
            "the drum of {} cannot be told from the track name",
            unknown.join(", ")
        )));
    }
    let mut decoded = Vec::with_capacity(mics.len());
    for m in mics {
        let d = svc
            .inner
            .media
            .decode_cached(&m.path, mix_mode(&m.mix))
            .map_err(|e| OffRule::plain(format!("{} cannot be read: {}", m.name, e.message)))?;
        decoded.push(d);
    }
    let sr = decoded.first().map_or(0, |d| d.sr);
    if decoded.iter().any(|d| d.sr != sr) {
        return Err(OffRule::plain("the tom tracks have different sample rates".into()));
    }
    let input: Vec<TomMic> = mics
        .iter()
        .zip(&decoded)
        .map(|(m, d)| TomMic {
            drum: m.drum.clone().expect("checked above"),
            signal: &d.samples,
            hits: m.times.iter().map(|t| (t * sr as f64).round() as i64).collect(),
        })
        .collect();
    // version 2: a source that cannot be read or has another rate is left out
    let mut src_dec = Vec::with_capacity(srcs.len());
    for m in srcs {
        if let Ok(d) = svc.inner.media.decode_cached(&m.path, mix_mode(&m.mix)) {
            if d.sr == sr {
                src_dec.push((m, d));
            }
        }
    }
    let sources: Vec<SourceMic> = src_dec
        .iter()
        .map(|(m, d)| SourceMic {
            kind: m.kind.expect("a source"),
            signal: &d.samples,
            hits: m.times.iter().map(|t| (t * sr as f64).round() as i64).collect(),
        })
        .collect();
    used.extend(src_dec.iter().map(|(m, _)| m.name.clone()));
    let r =
        tom_bleed_with(&input, &sources, sr, &TomBleedParams::default()).map_err(|e| OffRule::plain(e.to_string()))?;
    shared.extend(
        r.merged
            .iter()
            .map(|&(a, b)| format!("{} and {} hear one drum: counted as one", names[a], names[b])),
    );
    match &r.off {
        Some(off) => Err(OffRule {
            reason: off_reason(off, names),
            kick_marks: matches!(off, Off::FewerThanTwoPlayed { .. }).then(|| r.shared_kick_bleed.clone()),
        }),
        None => Ok((r.bleed, r.mics.iter().map(|m| m.played).collect())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pin: Option<u32>) -> ServiceConfig {
        ServiceConfig {
            refine_rule: None,
            bleed_rule: pin,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        }
    }

    #[test]
    fn versions_of_an_analysis_and_of_a_repick() {
        assert_eq!(parse_bleed_rule(" 1 "), Some(1));
        assert_eq!(parse_bleed_rule("0"), Some(0));
        assert_eq!(parse_bleed_rule("2"), Some(2));
        assert_eq!(parse_bleed_rule("3"), Some(3));
        assert_eq!(parse_bleed_rule("4"), Some(4));
        assert_eq!(parse_bleed_rule("5"), Some(5));
        assert_eq!(parse_bleed_rule("on"), None);
        assert_eq!(analysis_bleed_rule(&cfg(None)), BLEED_RULE);
        assert_eq!(analysis_bleed_rule(&cfg(Some(0))), 0);
        let det = json!({"detector": "rust-dsp", "refine": 3, "bleed_rule": 1});
        assert_eq!(recorded_bleed_rule(&cfg(None), Some(&det)), 1);
        assert_eq!(
            recorded_bleed_rule(&cfg(None), Some(&json!({"bleed_rule": 2}))),
            2,
            "version 2 stays 2"
        );
        assert_eq!(recorded_bleed_rule(&cfg(None), Some(&json!({"bleed_rule": 3}))), 3);
        assert_eq!(
            recorded_bleed_rule(&cfg(Some(2)), Some(&json!({"bleed_rule": 3}))),
            2,
            "the pin wins"
        );
        assert_eq!(BLEED_RULE, 5);
        assert_eq!(GUARD_FROM, 4, "the keep guard runs from version 4");
        assert_eq!(PAIRS_FROM, 3);
        assert_eq!(
            recorded_bleed_rule(&cfg(None), Some(&json!({"refine": 3}))),
            0,
            "before P4-14: no rule"
        );
        assert_eq!(recorded_bleed_rule(&cfg(None), None), 0);
        assert_eq!(
            recorded_bleed_rule(&cfg(None), Some(&json!({"bleed_rule": 9}))),
            BLEED_RULE
        );
        assert_eq!(recorded_bleed_rule(&cfg(Some(0)), Some(&det)), 0, "the pin wins");
    }

    #[test]
    fn the_pair_record_names_the_outcome() {
        assert_eq!(
            pair_record(&Some(Ok(vec![vec![false]])), 4),
            json!({"on": true, "reason": null, "removed": 4})
        );
        assert_eq!(
            pair_record(&Some(Err("why".into())), 4),
            json!({"on": false, "reason": "why", "removed": 0})
        );
        assert_eq!(PAIRS_FROM, 3);
    }

    #[test]
    fn close_mics_marks_and_reasons() {
        assert!(is_tom_close_mic("t1 hh_02", Some("tom")));
        assert!(
            !is_tom_close_mic("t1 sampl_01", Some("tom")),
            "a sample track is no close mic"
        );
        assert!(!is_tom_close_mic("t trigger", Some("tom")));
        assert!(!is_tom_close_mic("t2", Some("snare")));
        assert!(!is_tom_close_mic("t2", None));
        assert_eq!(source_kind("s top", Some("snare")), Some(SourceKind::Snare));
        assert_eq!(source_kind("Kick In 421", Some("kick")), Some(SourceKind::Kick));
        for (name, role) in [
            ("s sampl trig_01", "snare"),
            ("s trigger_03", "snare"),
            ("k samp 1_01", "kick"),
            ("k triggerr_03", "kick"),
            ("KickSmpl", "kick"),
            ("s top", "tom"),
            ("oh", "other"),
        ] {
            assert_eq!(source_kind(name, Some(role)), None, "{name} is no source");
        }
        assert!(is_bleed(&json!({"t": 1, "bleed": true})));
        assert!(!is_bleed(&json!({"t": 1, "bleed": false})) && !is_bleed(&json!({"t": 1})));
        let names = vec!["t1".to_string(), "t2".to_string()];
        assert_eq!(
            off_reason(&Off::FewerThanTwoPlayed { played: vec![1] }, &names),
            "fewer than two played tom close mics (played: t2)"
        );
        assert_eq!(
            off_reason(&Off::FewerThanTwoPlayed { played: vec![] }, &names),
            "fewer than two played tom close mics (played: none)"
        );
        assert!(off_reason(&Off::DigitalSilence { mics: vec![0, 1] }, &names).contains("digital silence (t1, t2)"));
        let mut e = drumspike_core::project::Event::new(1.0, 100);
        assert!(!edited(&e));
        e.orig = Some(0.9);
        assert!(edited(&e));
        e.orig = None;
        e.added = Some(true);
        assert!(edited(&e));
        e.added = None;
        e.edited = Some(true);
        assert!(edited(&e), "a restored hit");
    }
}
