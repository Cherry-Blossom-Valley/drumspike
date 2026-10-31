//! P4-6a: Learn marks. The engineer marks good ("this") and bad ("not") hits of a track with a
//! range or a selected hit; the project keeps the marks in the track's `learn` object. The scorer
//! is P4-6b ([`crate::learn_score`]), the filter P4-6c ([`crate::learn_filter`];
//! `docs/migration/concept-learn-mode.md`).
//!
//! P4-6c: the first "not" example of a track whose filter is off turns the filter on, with the
//! threshold following the examples (`auto`; [`first_not_turns_on`]). K alone never turns it on.
//! Every command of this module that changes a `learn` object while the filter runs, or turns it
//! on or off, marks the track's hits again at once (`learn_filter::refilter_merge`: the times,
//! hand deletions, velocities and edited hits stay): the answer carries `filter` (`{events,
//! followers?}`) or `filter_error` (no kept activations, an analysis running). `track_learn_set`
//! with `events` stores that list instead (the page's undo and redo restore their snapshot).
//!
//! | Command | What it does |
//! |---|---|
//! | `track_learn_mark {sid, name, kind, t0, t1 \| hit}` | resolve the examples of a range or a hit and store one mark |
//! | `track_learn_remove {sid, name, id, at?}` | remove a mark, or one example of it |
//! | `track_learn_forget_at {sid, name, t0, t1 \| hit}` | remove the examples at one hit or inside a range (Forget This) |
//! | `track_learn_flip {sid, name, id}` | change a mark from "this" to "not" or back |
//! | `track_learn_forget {sid, name}` | remove every mark of the track |
//! | `track_learn_set {sid, name, learn}` | store the whole `learn` object (the page's undo and redo) |
//! | `track_learn_scores {sid, name}` | P4-6b: the scores of the candidates and the examples ([`crate::learn_score`]) |
//!
//! A follower routes to its source (`routed`); a source that is gone gives 409.
//!
//! The rule of a range (concept section 2.2): the active hits in it (a hit the loudness cut marked
//! is not active); else the strongest detection candidate of the kept activations
//! ([`crate::histogram`]); else the strongest envelope rise of 6 dB or more ([`rise_example`]);
//! else no mark (`marked: false`). A hit is the nearest hit within [`HIT_REACH`], else the nearest
//! candidate within [`CANDIDATE_REACH`]. An example time is stored before the Hitpoint shift. A
//! second mark of the other kind on the same hit (within [`SAME_HIT`]) changes its kind: the last
//! mark wins. A track holds at most [`MAX_EXAMPLES`] examples of each kind.
//!
//! Document: `learn` is a key of the track that the document passes through
//! (`docs/migration/project-format.md`): `{on, threshold, auto, method, marks: [{id, kind, t0, t1, at}], dropped?}`
//! (P4-38e: `dropped`, the times of the Learn hits the engineer deleted, before the Hitpoint shift; a Learn change
//! does not bring them back, [`crate::learn_filter::note_dropped`]).

use drumspike_core::refine::{RefineWindow, refine_sr};
use serde_json::{Map, Value, json};

use crate::Service;
use crate::commands::{assign_events, events_set_out};
use crate::error::AppError;
use crate::export::hit_times;
use crate::media::mix_mode;
use crate::registry::{lock, resolve_shared};

/// The commands of this card, as the route map names them.
pub const LEARN_COMMANDS: [&str; 7] = [
    "track_learn_mark",
    "track_learn_remove",
    "track_learn_forget_at",
    "track_learn_flip",
    "track_learn_forget",
    "track_learn_set",
    "track_learn_scores",
];

/// The most examples of one kind on a track.
pub const MAX_EXAMPLES: usize = 64;
/// Two examples closer than this (seconds) are the same hit.
pub const SAME_HIT: f64 = 0.002;
/// A hit given by its time must be this near (seconds) to a stored hit.
pub const HIT_REACH: f64 = 0.005;
/// ... else this near to a detection candidate (seconds; the candidate frames are 10 ms apart).
pub const CANDIDATE_REACH: f64 = 0.030;
/// The envelope rise a range needs for the last rule (dB).
pub const RISE_MIN_DB: f64 = 6.0;
/// The rise rule looks at the range in pieces of this length (seconds; a long piece lets a loud
/// tail hide a small onset), each starting this far after the one before ...
const RISE_PIECE: f64 = 0.05;
const RISE_STEP: f64 = 0.025;
/// ... and at most this many pieces (the first minute of a longer range).
const RISE_PIECES: usize = 2400;
/// The feature recipe the scorer names: the DSP recipe of #293 (the model's `patch-1` is gone; an
/// old learn object that names it falls back to this one, see `track_learn_scores`).
pub const METHOD: &str = "patch-2";

fn kind_ok(kind: &str) -> Result<(), AppError> {
    match kind {
        "this" | "not" => Ok(()),
        _ => Err(AppError::bad("`kind` must be \"this\" or \"not\"")),
    }
}

fn marks_of(learn: &Map<String, Value>) -> &[Value] {
    learn.get("marks").and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn ats_of(mark: &Value) -> Vec<f64> {
    mark.get("at")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}

/// The number of examples of each kind in a `learn` object: `(this, not)`.
pub fn counts(learn: &Map<String, Value>) -> (usize, usize) {
    let mut c = (0, 0);
    for m in marks_of(learn) {
        let n = ats_of(m).len();
        match m.get("kind").and_then(Value::as_str) {
            Some("this") => c.0 += n,
            Some("not") => c.1 += n,
            _ => {}
        }
    }
    c
}

/// P4-6c: true when the filter of this `learn` object cuts: `on` is true and it holds at least one example.
pub fn filter_on(learn: &Map<String, Value>) -> bool {
    let (this, not) = counts(learn);
    learn.get("on").and_then(Value::as_bool) == Some(true) && this + not > 0
}

/// P4-38d: true when the Learn pick of this `learn` object runs: the filter is on ([`filter_on`]),
/// or it holds a Learn This example (the Add rule runs with the filter off too).
pub fn pick_runs(learn: &Map<String, Value>) -> bool {
    filter_on(learn) || counts(learn).0 > 0
}

/// The `learn` object of a track when its Learn pick runs ([`pick_runs`]), else `None`.
pub(crate) fn learn_of_track(op: &crate::registry::Shared, name: &str) -> Option<Map<String, Value>> {
    let g = lock(op);
    let t = g.track(name).ok()?;
    let learn = learn_of(&t.extra)?;
    pick_runs(&learn).then_some(learn)
}

/// The `learn` object of a track's extra keys, `None` when absent or not an object.
pub fn learn_of(extra: &Map<String, Value>) -> Option<Map<String, Value>> {
    extra.get("learn").and_then(Value::as_object).cloned()
}

/// The result of [`apply_mark`].
#[derive(Debug, Clone, PartialEq)]
pub struct MarkResult {
    /// The id of the new mark, `None` when every example was marked already.
    pub id: Option<String>,
    /// New examples.
    pub added: usize,
    /// Examples that changed kind (the last mark wins).
    pub flipped: usize,
    /// Examples that had this kind already.
    pub skipped: usize,
}

/// The id after the highest `m<n>` of the marks.
fn next_id(marks: &[Value]) -> String {
    let n = marks
        .iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .filter_map(|s| s.strip_prefix('m').and_then(|d| d.parse::<u64>().ok()))
        .max()
        .unwrap_or(0);
    format!("m{}", n + 1)
}

/// Add the mark `{kind, t0, t1, at: ats}` to `learn`. An example that exists with the same kind is
/// skipped; one that exists with the other kind moves to the new mark (the old mark loses it, and
/// goes when it is empty). 400 when a kind would hold more than [`MAX_EXAMPLES`]: then nothing
/// changes. A new object gets `on: false`, `threshold: 0.5`, `auto: true` (concept 3.2: the threshold
/// follows the examples until the engineer drags it) and the method.
pub fn apply_mark(
    learn: &mut Map<String, Value>,
    kind: &str,
    t0: f64,
    t1: f64,
    ats: &[f64],
) -> Result<MarkResult, AppError> {
    kind_ok(kind)?;
    let marks: Vec<Value> = marks_of(learn).to_vec();
    let mut want: Vec<f64> = Vec::new();
    for &a in ats {
        if !want.iter().any(|w| (w - a).abs() < SAME_HIT) {
            want.push(a);
        }
    }
    // where each wanted example is now: (mark index, example index, kind)
    let mut fresh: Vec<f64> = Vec::new();
    let mut flips: Vec<(usize, usize)> = Vec::new();
    let mut skipped = 0;
    for &a in &want {
        let found = marks.iter().enumerate().find_map(|(mi, m)| {
            ats_of(m)
                .iter()
                .position(|x| (x - a).abs() < SAME_HIT)
                .map(|ai| (mi, ai, m.get("kind").and_then(Value::as_str) == Some(kind)))
        });
        match found {
            Some((_, _, true)) => skipped += 1,
            Some((mi, ai, false)) => flips.push((mi, ai)),
            None => fresh.push(a),
        }
    }
    let (n_this, n_not) = counts(learn);
    let have = if kind == "this" { n_this } else { n_not };
    if have + fresh.len() + flips.len() > MAX_EXAMPLES {
        return Err(AppError::bad(format!(
            "a track holds at most {MAX_EXAMPLES} {kind} examples: use Forget or remove old examples"
        )));
    }
    let mut result = MarkResult {
        id: None,
        added: fresh.len(),
        flipped: flips.len(),
        skipped,
    };
    if fresh.is_empty() && flips.is_empty() {
        return Ok(result);
    }
    // the flipped examples leave their old marks (from the back, so the indices stay)
    let mut marks = marks;
    let mut moved: Vec<f64> = Vec::new();
    flips.sort_by(|a, b| b.cmp(a));
    for (mi, ai) in flips {
        let mut ats = ats_of(&marks[mi]);
        moved.push(ats.remove(ai));
        marks[mi]["at"] = json!(ats);
    }
    marks.retain(|m| !ats_of(m).is_empty());
    let id = next_id(&marks);
    let mut all = fresh;
    all.extend(moved);
    all.sort_by(f64::total_cmp);
    marks.push(json!({"id": id, "kind": kind, "t0": t0, "t1": t1, "at": all}));
    learn.insert("marks".into(), Value::Array(marks));
    learn.entry("on").or_insert(json!(false)); // K alone never turns the filter on ([`first_not_turns_on`])
    learn.entry("threshold").or_insert(json!(0.5));
    learn.entry("auto").or_insert(json!(true)); // P4-6c: the threshold follows the examples until the engineer drags it
    learn.entry("method").or_insert(json!(METHOD));
    result.id = Some(id);
    Ok(result)
}

/// Remove the mark `id` (with `at`: only the example nearest to it, within [`SAME_HIT`]). The
/// number of removed examples; 400 for an unknown id or time. The mark goes when it is empty.
pub fn remove_mark(learn: &mut Map<String, Value>, id: &str, at: Option<f64>) -> Result<usize, AppError> {
    let mut marks: Vec<Value> = marks_of(learn).to_vec();
    let mi = marks
        .iter()
        .position(|m| m.get("id").and_then(Value::as_str) == Some(id))
        .ok_or_else(|| AppError::bad(format!("no mark `{id}`")))?;
    let n = match at {
        None => {
            let n = ats_of(&marks[mi]).len();
            marks.remove(mi);
            n
        }
        Some(a) => {
            let mut ats = ats_of(&marks[mi]);
            let ai = ats
                .iter()
                .position(|x| (x - a).abs() < SAME_HIT)
                .ok_or_else(|| AppError::bad(format!("mark `{id}` has no example at {a}")))?;
            ats.remove(ai);
            marks[mi]["at"] = json!(ats);
            marks.retain(|m| !ats_of(m).is_empty());
            1
        }
    };
    learn.insert("marks".into(), Value::Array(marks));
    Ok(n)
}

/// P4-36 Forget This: remove the examples (of both kinds) at the hit `hit`, or inside the range
/// `t0..=t1` (stored times, the shift already taken off). A hit takes the examples within
/// [`HIT_REACH`], the match of Learn This. The number of removed examples; 0 changes nothing. A
/// mark goes when it is empty.
pub fn forget_at(learn: &mut Map<String, Value>, t0: f64, t1: f64, hit: bool) -> usize {
    let near = |a: f64| {
        if hit {
            (a - t0).abs() <= HIT_REACH
        } else {
            a >= t0 && a <= t1
        }
    };
    let mut n = 0;
    let marks: Vec<Value> = marks_of(learn)
        .iter()
        .map(|m| {
            let (keep, gone): (Vec<f64>, Vec<f64>) = ats_of(m).into_iter().partition(|a| !near(*a));
            n += gone.len();
            let mut m = m.clone();
            if !gone.is_empty() {
                m["at"] = json!(keep);
            }
            m
        })
        .filter(|m| !ats_of(m).is_empty())
        .collect();
    if n > 0 {
        learn.insert("marks".into(), Value::Array(marks));
    }
    n
}

/// Change the kind of the mark `id`. 400 for an unknown id, or when the other kind would hold more
/// than [`MAX_EXAMPLES`]. Returns the new kind.
pub fn flip_mark(learn: &mut Map<String, Value>, id: &str) -> Result<&'static str, AppError> {
    let (n_this, n_not) = counts(learn);
    let mut marks: Vec<Value> = marks_of(learn).to_vec();
    let m = marks
        .iter_mut()
        .find(|m| m.get("id").and_then(Value::as_str) == Some(id))
        .ok_or_else(|| AppError::bad(format!("no mark `{id}`")))?;
    let n = ats_of(m).len();
    let to = if m.get("kind").and_then(Value::as_str) == Some("this") {
        "not"
    } else {
        "this"
    };
    if (if to == "this" { n_this } else { n_not }) + n > MAX_EXAMPLES {
        return Err(AppError::bad(format!(
            "a track holds at most {MAX_EXAMPLES} {to} examples: use Forget or remove old examples"
        )));
    }
    m["kind"] = json!(to);
    learn.insert("marks".into(), Value::Array(marks));
    Ok(to)
}

/// Check a `learn` object that the page or an agent sends back (`track_learn_set`): `marks` is a
/// list of `{id (string), kind, t0, t1 (numbers), at (list of numbers)}` with unique ids, the
/// optional keys have their type, and no kind holds more than [`MAX_EXAMPLES`]. Unknown keys pass.
pub fn check_learn(learn: &Map<String, Value>) -> Result<(), AppError> {
    let bad = |m: &str| Err(AppError::bad(format!("`learn`: {m}")));
    for (k, ok) in [
        ("on", Value::is_boolean as fn(&Value) -> bool),
        ("auto", Value::is_boolean),
        ("threshold", Value::is_number),
        ("method", Value::is_string),
        ("add", Value::is_number), // P4-38b: the Add line's threshold
    ] {
        if learn.get(k).is_some_and(|v| !ok(v)) {
            return bad(&format!("`{k}` has the wrong type"));
        }
    }
    if let Some(t) = learn.get("threshold").and_then(Value::as_f64)
        && !(0.0..=1.0).contains(&t)
    {
        return bad("`threshold` must be 0 to 1");
    }
    if let Some(t) = learn.get("add").and_then(Value::as_f64)
        && !(0.0..=1.0).contains(&t)
    {
        return bad("`add` must be 0 to 1");
    }
    // P4-38e: the deleted Learn hits
    let num = |x: &Value| x.as_f64().is_some_and(f64::is_finite);
    if learn
        .get("dropped")
        .is_some_and(|d| !d.as_array().is_some_and(|a| a.iter().all(num)))
    {
        return bad("`dropped` must be a list of numbers");
    }
    let Some(marks) = learn.get("marks").and_then(Value::as_array) else {
        return bad("`marks` must be a list");
    };
    let mut ids: Vec<&str> = Vec::new();
    for m in marks {
        let Some(id) = m.get("id").and_then(Value::as_str) else {
            return bad("every mark needs a string `id`");
        };
        if ids.contains(&id) {
            return bad(&format!("the id `{id}` is twice in `marks`"));
        }
        ids.push(id);
        if !matches!(m.get("kind").and_then(Value::as_str), Some("this" | "not")) {
            return bad("every mark needs `kind` \"this\" or \"not\"");
        }
        let finite = |k: &str| m.get(k).and_then(Value::as_f64).is_some_and(f64::is_finite);
        if !finite("t0") || !finite("t1") {
            return bad("every mark needs numeric `t0` and `t1`");
        }
        let ok = m
            .get("at")
            .and_then(Value::as_array)
            .is_some_and(|a| a.iter().all(|x| x.as_f64().is_some_and(f64::is_finite)));
        if !ok {
            return bad("every mark needs `at`, a list of numbers");
        }
    }
    let (a, b) = counts(learn);
    if a > MAX_EXAMPLES || b > MAX_EXAMPLES {
        return bad(&format!("at most {MAX_EXAMPLES} examples of each kind"));
    }
    Ok(())
}

/// The strongest candidate (`[t, class, strength, v]`, see `histogram::detection_candidates`) with
/// `t0 <= t <= t1`: its time.
pub fn strongest_candidate(cands: &[Value], t0: f64, t1: f64) -> Option<f64> {
    cands
        .iter()
        .filter_map(|c| Some((c.get(0)?.as_f64()?, c.get(2)?.as_f64()?)))
        .filter(|(t, _)| *t >= t0 && *t <= t1)
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.total_cmp(&a.0)))
        .map(|(t, _)| t)
}

/// The candidate nearest to `t`, within [`CANDIDATE_REACH`]: its time.
pub fn nearest_candidate(cands: &[Value], t: f64) -> Option<f64> {
    cands
        .iter()
        .filter_map(|c| c.get(0)?.as_f64())
        .filter(|ct| (ct - t).abs() <= CANDIDATE_REACH)
        .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
}

/// The steepest envelope rise of at least [`RISE_MIN_DB`] in `t0..=t1` of `x` (the refine step's
/// anchor rule, [`refine_sr`]): the time of the attack, or `None`. A range longer than
/// `RISE_PIECES` steps of [`RISE_STEP`] seconds counts its first part only.
pub fn rise_example(x: &[f32], sr: u32, t0: f64, t1: f64) -> Option<f64> {
    rise_at_least(x, sr, t0, t1, RISE_MIN_DB)
}

/// #81: the smallest rise of [`weak_rise`] (dB): over the wobble of a steady floor.
pub const WEAK_RISE_DB: f64 = 2.0;

/// #81: the time of a K-add on a range with no hit, no candidate and no [`rise_example`]: the steepest envelope rise
/// of at least [`WEAK_RISE_DB`] in `t0..=t1` (the attack the engineer heard is soft or under a ring), else `None` (the
/// [`range_centre`]).
pub fn weak_rise(x: &[f32], sr: u32, t0: f64, t1: f64) -> Option<f64> {
    weak_rise_clear(x, sr, t0, t1, &[])
}

/// #81: a hit of another close mic this near (seconds) to a rise makes the rise its bleed when that mic is louder.
pub const BLEED_REACH: f64 = 0.012;
/// #81: the level of a mic at a time: its peak over this window after the time (seconds).
const PEAK_WIN: f64 = 0.020;

/// The peak magnitude of `x` in `t..t + PEAK_WIN`.
pub fn peak_after(x: &[f32], sr: u32, t: f64) -> f32 {
    let a = ((t.max(0.0) * f64::from(sr)) as usize).min(x.len());
    let b = (((t + PEAK_WIN).max(0.0) * f64::from(sr)) as usize).min(x.len());
    x[a..b].iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// #81: [`weak_rise`] without the bleed rises: the steepest rise of at least [`WEAK_RISE_DB`] in `t0..=t1` that has
/// no hit of another close mic within [`BLEED_REACH`] whose level ([`peak_after`]) is over the rise's level in `x`.
/// `others`: the hits of the other close mics, `(time, level)`. `None`: no clear rise (the [`range_centre`]).
pub fn weak_rise_clear(x: &[f32], sr: u32, t0: f64, t1: f64, others: &[(f64, f32)]) -> Option<f64> {
    let mut rises = rises_of(x, sr, t0, t1, WEAK_RISE_DB);
    rises.sort_by(|a, b| b.1.total_cmp(&a.1));
    rises
        .into_iter()
        .map(|(t, _)| t)
        .find(|&t| bleed_of(x, sr, t, others).is_none())
}

/// The louder hit of another close mic within [`BLEED_REACH`] of a rise at `t` (`(time, level)`), if any.
fn bleed_of(x: &[f32], sr: u32, t: f64, others: &[(f64, f32)]) -> Option<(f64, f32)> {
    let own = peak_after(x, sr, t);
    others
        .iter()
        .copied()
        .find(|&(h, lv)| (h - t).abs() <= BLEED_REACH && lv > own)
}

/// #81: the K-add of a range with no clear rise: the range centre. The engineer draws the range around the hit they
/// heard, so the centre is off by half the range at most; the range start was off by up to a whole range, and the
/// loudest point lies after the attack (the peak follows it) or on the bleed of another drum.
pub fn range_centre(t0: f64, t1: f64) -> f64 {
    (t0 + t1) / 2.0
}

/// #81: why [`weak_rise_clear`] took or left each piece of `t0..=t1`: one `{piece, t, db, why}` per piece, `why` one of
/// `taken`, `no attack` (refine gave the piece start back), `under 2 dB`, `outside the range`, `bleed` (with
/// `bleed_t` and `bleed_level` of the louder hit, and the rise's own `level`), `steeper rise taken`, `same time`.
pub fn rise_log(x: &[f32], sr: u32, t0: f64, t1: f64, others: &[(f64, f32)]) -> Vec<Value> {
    let taken = weak_rise_clear(x, sr, t0, t1, others);
    let mut seen: Vec<f64> = Vec::new();
    pieces_of(x, sr, t0, t1)
        .into_iter()
        .map(|(a, t, rise)| {
            let mut row = json!({"piece": a, "t": t, "db": rise});
            let why = match rise {
                _ if t == a => "no attack",
                None => "no attack",
                Some(r) if r < WEAK_RISE_DB => "under 2 dB",
                _ if t < t0 || t > t1 => "outside the range",
                _ if seen.iter().any(|s| (s - t).abs() < 1e-9) => "same time",
                _ => {
                    seen.push(t);
                    if let Some((h, lv)) = bleed_of(x, sr, t, others) {
                        row["bleed_t"] = json!(h);
                        row["bleed_level"] = json!(lv);
                        row["level"] = json!(peak_after(x, sr, t));
                        "bleed"
                    } else if taken.is_some_and(|k| (k - t).abs() < 1e-9) {
                        "taken"
                    } else {
                        "steeper rise taken"
                    }
                }
            };
            row["why"] = json!(why);
            row
        })
        .collect()
}

fn rise_at_least(x: &[f32], sr: u32, t0: f64, t1: f64, min_db: f64) -> Option<f64> {
    rises_of(x, sr, t0, t1, min_db)
        .into_iter()
        .fold(None::<(f64, f64)>, |best, (t, r)| match best {
            Some((_, b)) if r <= b => best,
            _ => Some((t, r)),
        })
        .map(|(t, _)| t)
}

/// Every rise of at least `min_db` the pieces of `t0..=t1` find: `(time, dB)`, in piece order (a time once). A piece
/// whose refine answer is the piece start found no attack (#81: refine gives its input back for silence, a window
/// with no rise and a crossing at its start), whatever its dB.
fn rises_of(x: &[f32], sr: u32, t0: f64, t1: f64, min_db: f64) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, t, rise) in pieces_of(x, sr, t0, t1) {
        if let Some(r) = rise.filter(|r| *r >= min_db && t != a && t >= t0 && t <= t1) {
            match out.iter_mut().find(|(o, _)| (*o - t).abs() < 1e-9) {
                Some(o) => o.1 = o.1.max(r),
                None => out.push((t, r)),
            }
        }
    }
    out
}

/// The refine answer of each piece of `t0..=t1`: `(piece start, time, dB)`, in piece order.
fn pieces_of(x: &[f32], sr: u32, t0: f64, t1: f64) -> Vec<(f64, f64, Option<f64>)> {
    let mut out = Vec::new();
    let mut a = t0;
    for _ in 0..RISE_PIECES {
        if a > t1 {
            break;
        }
        let len = RISE_PIECE.min(t1 - a);
        let win = RefineWindow {
            lo_ms: 0.0,
            hi_ms: (len * 1000.0).max(1.0),
        };
        let (t, rise) = refine_sr(x, a, sr, &[], win);
        out.push((a, t, rise));
        a += RISE_STEP;
    }
    out
}

/// What the rule found: where the examples come from and their times (before the shift).
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// `hits`, `candidate`, `rise` or `hit`.
    pub from: &'static str,
    pub at: Vec<f64>,
}

/// The active hits of a range: `t0 <= t <= t1`, no loudness cut and no tom bleed mark; their times minus `shift_s`.
pub fn active_hits(events: &[Value], t0: f64, t1: f64, shift_s: f64) -> Result<Vec<f64>, AppError> {
    let times = hit_times(events)?;
    Ok(events
        .iter()
        .zip(times)
        .filter(|(e, t)| *t >= t0 && *t <= t1 && !crate::export::is_cut(e))
        .map(|(_, t)| t - shift_s)
        .collect())
}

/// The nearest stored hit to `t` within [`HIT_REACH`] (cut hits too): its time minus `shift_s`.
pub fn hit_at(events: &[Value], t: f64, shift_s: f64) -> Result<Option<f64>, AppError> {
    let times = hit_times(events)?;
    Ok(times
        .into_iter()
        .filter(|x| (x - t).abs() <= HIT_REACH)
        .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
        .map(|x| x - shift_s))
}

/// P4-6c (controller decision 2026-10-13): the first "not" example of a track whose filter is off
/// turns the filter on, with the threshold following the examples (`auto: true`; the suggestion
/// of `similarity::suggest_threshold`). `not_before` is the number of "not" examples before the
/// mark. Returns true when it turned the filter on. K alone never does; neither does a later
/// Shift+K after the engineer turned the filter off.
pub fn first_not_turns_on(learn: &mut Map<String, Value>, kind: &str, not_before: usize) -> bool {
    let (_, not_now) = counts(learn);
    if kind != "not" || not_before > 0 || not_now == 0 || learn.get("on") == Some(&Value::Bool(true)) {
        return false;
    }
    learn.insert("on".into(), json!(true));
    learn.insert("auto".into(), json!(true));
    true
}

/// P4-6c: a project of 0.4.0 holds `on: true` after a first K (stage A switched the filter on
/// with the first "this" mark). Stage B turns the filter on only with a "not" example or the
/// engineer's tick, so a learn object with no "not" example loads with the filter off. Returns
/// true when it changed the track's `extra`.
pub fn migrate_learn(extra: &mut Map<String, Value>) -> bool {
    let Some(Value::Object(learn)) = extra.get_mut("learn") else {
        return false;
    };
    if learn.get("on") != Some(&Value::Bool(true)) || counts(learn).1 > 0 {
        return false;
    }
    learn.insert("on".into(), json!(false));
    true
}

/// The Hitpoint shift of a track in seconds (`params.shift_samples` over the sample rate; 0 without either).
pub(crate) fn shift_s_of(tr: &drumspike_core::project::Track) -> f64 {
    let shift = tr
        .params
        .as_ref()
        .and_then(|p| p.get("shift_samples"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    match tr.sr {
        Some(sr) if sr > 0 => shift / sr as f64,
        _ => 0.0,
    }
}

fn learn_json(extra: &Map<String, Value>) -> Value {
    extra.get("learn").cloned().unwrap_or(Value::Null)
}

impl Service {
    /// P4-38e: `events_set` for a hand edit (the page's write and `events_edit`): the hits of `name` become `events`,
    /// and the Learn hits that the edit deleted go in the track's `learn.dropped`, so the next Learn change does not
    /// bring them back ([`crate::learn_filter::note_dropped`]; a hit put back takes its time out again).
    /// R07: the write is one writer of the project: it takes the store guard once here.
    pub fn events_set_by_hand(&self, sid: &str, name: &str, events: &Value) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let _guard = self.begin_write(sid)?;
        self.events_set_by_hand_with(&op, name, events)
    }

    /// [`Service::events_set_by_hand`] with the project's store guard already held (the caller
    /// owns it: `events_edit_as`). The old list, the events and the Learn bookkeeping are one
    /// transaction under one project-lock take, so the `learn.dropped` state always matches the
    /// events that are stored. Never takes the guard itself.
    pub(crate) fn events_set_by_hand_with(
        &self,
        op: &crate::registry::Shared,
        name: &str,
        events: &Value,
    ) -> Result<Value, AppError> {
        let n = {
            let mut g = lock(op);
            g.track(name)?;
            crate::follow::check_own(&g, name)?; // P3-18: a follower's hits are the source's
            let old = g
                .track_object(name)?
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let n = assign_events(&mut g, name, events)?;
            let new = events.as_array().map(Vec::as_slice).unwrap_or_default();
            let tr = g.track_mut(name)?;
            if let Some(mut learn) = learn_of(&tr.extra)
                && crate::learn_filter::note_dropped(&mut learn, &old, new, shift_s_of(tr))
            {
                tr.extra.insert("learn".into(), Value::Object(learn));
            }
            g.mark_dirty();
            n
        };
        // P3-18: the followers of this track take the edited hits (the lock is free: a decode may run)
        let rows = crate::follow::refresh_followers(self, op, name);
        Ok(events_set_out(n, &rows))
    }

    /// The track whose `learn` a command changes: the track itself, or the source of a follower
    /// (`routed`). 409 when the source is gone.
    pub(crate) fn learn_target(&self, sid: &str, name: &str) -> Result<(String, bool), AppError> {
        let op = self.inner.registry.get(sid)?;
        let g = lock(&op);
        let tr = g.track(name)?;
        match &tr.follows {
            Some(src) => {
                if g.project.doc.track(src).is_none() {
                    return Err(AppError::conflict(format!("the source track {src} of {name} is gone")));
                }
                Ok((src.clone(), true))
            }
            None => Ok((name.to_string(), false)),
        }
    }

    /// Change the `learn` of `target` with `f` under the project lock; the object goes back (or
    /// away when it holds no mark). Returns what `f` returned and the new `learn` (`null` when none).
    fn learn_edit<T>(
        &self,
        sid: &str,
        target: &str,
        f: impl FnOnce(&mut Map<String, Value>) -> Result<T, AppError>,
    ) -> Result<(T, Value), AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let mut g = lock(&op);
        let tr = g.track_mut(target)?;
        let mut learn = learn_of(&tr.extra).unwrap_or_default();
        let out = f(&mut learn)?;
        if learn.is_empty() {
            tr.extra.shift_remove("learn");
        } else {
            tr.extra.insert("learn".into(), Value::Object(learn));
        }
        let now = learn_json(&tr.extra);
        g.mark_dirty();
        Ok((out, now))
    }

    /// `track_learn_mark {sid, name, kind, t0, t1 | hit}`: resolve the examples ([module rules](self))
    /// and store one mark. A range keeps `t0 <= t1` (swapped when reversed, negative `t0` is 0); a
    /// hit is a time in seconds and stores `t0 = t1 = hit`. Answers `{marked, mark?, learn, track,
    /// routed, from?, added, flipped, skipped, counts, filter_on, filter? | filter_error?, reason?, k_rise?}`
    /// (`filter_on`: this mark turned the filter on, [`first_not_turns_on`]); `marked: false` with a `reason`
    /// when the range holds nothing to learn (no error). 400 for wrong arguments, a hit that is not
    /// there and a kind over [`MAX_EXAMPLES`]; 409 for an offline track whose audio the last rule
    /// needs, and a follower whose source is gone.
    pub fn track_learn_mark(
        &self,
        sid: &str,
        name: &str,
        kind: &str,
        range: Option<(f64, f64)>,
        hit: Option<f64>,
    ) -> Result<Value, AppError> {
        kind_ok(kind)?;
        let (target, routed) = self.learn_target(sid, name)?;
        let (t0, t1, hit) = match (range, hit) {
            (Some(_), Some(_)) => return Err(AppError::bad("give `t0` and `t1`, or `hit`, not both")),
            (None, None) => return Err(AppError::bad("give `t0` and `t1`, or `hit`")),
            (Some((a, b)), None) => {
                if !a.is_finite() || !b.is_finite() {
                    return Err(AppError::bad("`t0` and `t1` must be numbers"));
                }
                (a.min(b).max(0.0), a.max(b).max(0.0), None)
            }
            (None, Some(h)) => {
                if !h.is_finite() || h < 0.0 {
                    return Err(AppError::bad("`hit` must be a time in seconds, 0 or more"));
                }
                (h, h, Some(h))
            }
        };
        let op = self.inner.registry.get_writable(sid)?;
        resolve_shared(&op);
        let events = self.track_events(sid, &target)?;
        let shift_s = shift_s_of(lock(&op).track(&target)?);
        let none = |reason: &str| -> Result<Value, AppError> {
            let g = lock(&op);
            let learn = learn_json(&g.track(&target)?.extra);
            Ok(json!({"marked": false, "reason": reason, "learn": learn, "track": target, "routed": routed}))
        };
        // #81: how the K-add of a range with no hit, candidate or 6 dB rise found its time ([`rise_log`])
        let mut k_rise: Option<Value> = None;
        let resolved = if let Some(h) = hit {
            let found = match hit_at(&events, h, shift_s)? {
                Some(a) => Some(Resolved {
                    from: "hit",
                    at: vec![a],
                }),
                None => self
                    .track_candidates(sid, &target)?
                    .and_then(|c| nearest_candidate(&c, h))
                    .map(|a| Resolved {
                        from: "candidate",
                        at: vec![a],
                    }),
            };
            match found {
                Some(r) => r,
                // P4-38b: K adds at the playhead time (the hand-add path) when neither a hit nor a
                // candidate is there; Shift+K keeps today's error
                None if kind == "this" => Resolved {
                    from: "hand",
                    at: vec![h],
                },
                None => return Err(AppError::bad(format!("no hit or candidate at {h} s"))),
            }
        } else {
            let active = active_hits(&events, t0, t1, shift_s)?;
            if !active.is_empty() {
                Resolved {
                    from: "hits",
                    at: active,
                }
            } else if let Some(a) = self
                .track_candidates(sid, &target)?
                .and_then(|c| strongest_candidate(&c, t0, t1))
            {
                Resolved {
                    from: "candidate",
                    at: vec![a],
                }
            } else if let Some(a) = self.rise_in(sid, &target, t0, t1)? {
                Resolved {
                    from: "rise",
                    at: vec![a],
                }
            } else if kind == "this" {
                // P4-38b: K adds a hit (the hand-add path) - no "no hit in the selection"; #81: at the steepest soft
                // rise of the range, so that a soft hit is not added up to a range length early; a rise with a louder
                // hit on another close mic is that drum's bleed (kick bleed on a tom); the range centre when no rise
                // is clear
                let (at, log) = self.clear_rise_in(sid, &target, t0, t1)?;
                k_rise = Some(json!({"at": if at.is_some() { "rise" } else { "centre" }, "pieces": log}));
                Resolved {
                    from: "hand",
                    at: vec![at.unwrap_or_else(|| range_centre(t0, t1))],
                }
            } else {
                return none("no hit in the selection");
            }
        };
        let was_on = self.learn_pick_runs(sid, &target);
        // P4-38b: K on a range or the playhead where the detection has no hit adds one at the
        // resolved time (the strongest candidate, the candidate near the playhead, or the refined
        // onset), flagged learned; Shift+K keeps today's answers. The re-pick's Add rule keeps it
        // on the next picks (its example scores it at or over the Add threshold).
        let mut added_hit: Option<f64> = None;
        let mut added_followers: Option<Value> = None;
        if kind == "this" && !matches!(resolved.from, "hits" | "hit") {
            let t = resolved.at.first().copied().unwrap_or(t0);
            let t = (t * 1e6).round() / 1e6;
            let er = self.events_edit(sid, &target, &[json!({"op": "add", "t": t, "learned": true})])?;
            added_followers = er.get("followers").cloned();
            added_hit = Some(t);
        }
        let ((r, turned_on), learn) = self.learn_edit(sid, &target, |l| {
            let not_before = counts(l).1;
            let r = apply_mark(l, kind, t0, t1, &resolved.at)?;
            if kind == "this" {
                // P4-38e: K on the spot of a deleted Learn hit brings it back (the refilter forces it again)
                crate::learn_filter::undrop_at(l, &resolved.at);
            }
            if let (Some(id), true) = (r.id.as_deref(), added_hit.is_some()) {
                // the mark a K-add creates (the sweep of Forget This takes its hit with it)
                if let Some(m) = l
                    .get_mut("marks")
                    .and_then(Value::as_array_mut)
                    .and_then(|ms| ms.iter_mut().find(|m| m["id"] == id))
                {
                    m["added"] = json!(true);
                }
            }
            // P4-38d: a K-add never turns the filter on (the Add rule runs with it off); Shift+K does
            let on = r.id.is_some() && first_not_turns_on(l, kind, not_before);
            Ok((r, on))
        })?;
        let learn_map = learn.as_object().cloned().unwrap_or_default();
        let (n_this, n_not) = counts(&learn_map);
        let mark =
            r.id.as_deref()
                .and_then(|id| marks_of(&learn_map).iter().find(|m| m["id"] == id).cloned());
        let mut out = json!({
            "marked": r.id.is_some(),
            "reason": if r.id.is_none() { json!("already marked") } else { Value::Null },
            "mark": mark,
            "learn": learn,
            "track": target,
            "routed": routed,
            "from": resolved.from,
            "added": r.added,
            "flipped": r.flipped,
            "skipped": r.skipped,
            "counts": {"this": n_this, "not": n_not},
            "filter_on": turned_on,
            "added_hit": added_hit,
        });
        if let Some(k) = k_rise {
            out["k_rise"] = k;
        }
        if r.id.is_some() {
            self.learn_refilter(sid, &target, was_on, &mut out);
            // P4-38b: the page needs the new hit even when no re-pick ran (it adopts `filter.events`)
            if added_hit.is_some() && out.get("filter").is_none() {
                out["filter"] = json!({"events": self.events_of(sid, &target)?});
                if let Some(f) = added_followers {
                    out["filter"]["followers"] = f;
                }
            }
        }
        Ok(out)
    }

    /// P4-6c: true when the Learn pick of `target` runs now (`on` and at least one example; P4-38d:
    /// or a Learn This example with the filter off, [`crate::jobs::pick_runs`]).
    fn learn_pick_runs(&self, sid: &str, target: &str) -> bool {
        self.inner
            .registry
            .get(sid)
            .is_ok_and(|op| learn_of_track(&op, target).is_some())
    }

    /// P4-6c: after a change of the `learn` of `target`: when the Learn pick ran before (`was_on`) or
    /// runs now, the track's hits marked again (`Service::learn_refilter_track`: no new pick is
    /// stored, hand deletions and velocity edits stay). The answer gets `filter` (`{events,
    /// followers?}`) or `filter_error` (no kept activations, an analysis running).
    fn learn_refilter(&self, sid: &str, target: &str, was_on: bool, out: &mut Value) {
        // P4-35: the hits the filter takes out before and after the step (the answer's guard counts)
        let before = self.events_of(sid, target).map_or((0, 0), |e| out_counts(&e));
        if !was_on && !self.learn_pick_runs(sid, target) {
            add_out_counts(out, before, before);
            return;
        }
        match self.learn_refilter_track(sid, target) {
            Ok(r) => {
                let after = r["events"].as_array().map_or(before, |e| out_counts(e));
                add_out_counts(out, before, after);
                out["filter"] = r;
            }
            Err(e) => {
                add_out_counts(out, before, before);
                out["filter_error"] = json!(e.message);
            }
        }
    }

    /// The strongest envelope rise of the range in the audio of `name` ([`rise_example`]).
    fn rise_in(&self, sid: &str, name: &str, t0: f64, t1: f64) -> Result<Option<f64>, AppError> {
        let op = self.inner.registry.get(sid)?;
        let (source, mix) = {
            let g = lock(&op);
            (g.source(name), g.track(name)?.mix_mode_or_default().to_string())
        };
        let d = self.inner.media.decode_cached(&source?, mix_mode(&mix))?;
        Ok(rise_example(&d.samples, d.sr, t0, t1))
    }

    /// #81: the K-add rise of a range in the audio of `name` ([`weak_rise_clear`]). On a close mic (tom, kick or snare;
    /// [`crate::tom_bleed`]) the active hits of the other close mics within [`BLEED_REACH`] of the range rule out the
    /// rises under them; another track judges the rises alone ([`weak_rise`]). With the [`rise_log`] of the range.
    fn clear_rise_in(&self, sid: &str, name: &str, t0: f64, t1: f64) -> Result<(Option<f64>, Vec<Value>), AppError> {
        use crate::tom_bleed::{is_tom_close_mic, source_kind};
        let close = |n: &str, role: Option<&str>| is_tom_close_mic(n, role) || source_kind(n, role).is_some();
        let op = self.inner.registry.get(sid)?;
        // per other close mic: its source, mix and the audio times of its active hits near the range
        let (source, mix, others) = {
            let g = lock(&op);
            let tr = g.track(name)?;
            let mine = close(name, tr.role.as_deref());
            let mut others = Vec::new();
            for n in g.project.doc.track_order() {
                if !mine || n == name || g.offline(&n) {
                    continue;
                }
                let Ok(o) = g.track(&n) else { continue };
                if !close(&n, o.role.as_deref()) || o.follows.is_some() || o.events.is_none() {
                    continue;
                }
                let shift = shift_s_of(o);
                let flag = |e: &drumspike_core::project::Event, k: &str| e.extra.get(k) == Some(&Value::Bool(true));
                let near: Vec<f64> = o
                    .events()
                    .iter()
                    .filter(|e| !flag(e, "cut") && !flag(e, "bleed"))
                    .map(|e| e.t - shift)
                    .filter(|&h| h >= t0 - BLEED_REACH && h <= t1 + BLEED_REACH)
                    .collect();
                if let (false, Ok(src)) = (near.is_empty(), g.source(&n)) {
                    others.push((src, o.mix_mode_or_default().to_string(), near));
                }
            }
            (g.source(name), tr.mix_mode_or_default().to_string(), others)
        };
        let d = self.inner.media.decode_cached(&source?, mix_mode(&mix))?;
        let mut hits: Vec<(f64, f32)> = Vec::new();
        for (src, m, near) in others {
            let Ok(o) = self.inner.media.decode_cached(&src, mix_mode(&m)) else {
                continue;
            };
            hits.extend(near.into_iter().map(|h| (h, peak_after(&o.samples, o.sr, h))));
        }
        let at = weak_rise_clear(&d.samples, d.sr, t0, t1, &hits);
        Ok((at, rise_log(&d.samples, d.sr, t0, t1, &hits)))
    }

    /// `track_learn_remove {sid, name, id, at?}`: remove a mark, or with `at` (seconds, an example
    /// time of `learn`) one example of it. Answers `{removed, learn, track, routed}`.
    pub fn track_learn_remove(&self, sid: &str, name: &str, id: &str, at: Option<f64>) -> Result<Value, AppError> {
        let (target, routed) = self.learn_target(sid, name)?;
        let was_on = self.learn_pick_runs(sid, &target);
        let (n, learn) = self.learn_edit(sid, &target, |l| remove_mark(l, id, at))?;
        let mut out = json!({"removed": n, "learn": learn, "track": target, "routed": routed});
        self.learn_refilter(sid, &target, was_on, &mut out);
        Ok(out)
    }

    /// `track_learn_forget_at {sid, name, t0, t1 | hit}` (P4-36, Forget This): remove the examples at
    /// one hit ([`HIT_REACH`]) or inside a range. Answers `{removed, learn, track, routed}`;
    /// `removed: 0` (no example there) changes nothing, not even the dirty flag. 400 for wrong arguments.
    pub fn track_learn_forget_at(
        &self,
        sid: &str,
        name: &str,
        range: Option<(f64, f64)>,
        hit: Option<f64>,
    ) -> Result<Value, AppError> {
        let (t0, t1, is_hit) = match (range, hit) {
            (Some(_), Some(_)) => return Err(AppError::bad("give `t0` and `t1`, or `hit`, not both")),
            (None, None) => return Err(AppError::bad("give `t0` and `t1`, or `hit`")),
            (Some((a, b)), None) if a.is_finite() && b.is_finite() => (a.min(b), a.max(b), false),
            (None, Some(h)) if h.is_finite() => (h, h, true),
            _ => return Err(AppError::bad("`t0`, `t1` and `hit` must be numbers")),
        };
        let (target, routed) = self.learn_target(sid, name)?;
        let op = self.inner.registry.get_writable(sid)?;
        let shift_s = shift_s_of(lock(&op).track(&target)?);
        let (t0, t1) = (t0 - shift_s, t1 - shift_s);
        {
            let g = lock(&op);
            let mut learn = learn_of(&g.track(&target)?.extra).unwrap_or_default();
            if forget_at(&mut learn, t0, t1, is_hit) == 0 {
                let now = learn_json(&g.track(&target)?.extra);
                return Ok(json!({"removed": 0, "learn": now, "track": target, "routed": routed}));
            }
        }
        let was_on = self.learn_pick_runs(sid, &target);
        let (n, learn) = self.learn_edit(sid, &target, |l| Ok(forget_at(l, t0, t1, is_hit)))?;
        let mut out = json!({"removed": n, "learn": learn, "track": target, "routed": routed});
        self.learn_refilter(sid, &target, was_on, &mut out);
        // P4-38b: the learned hits the removed examples supported go with them
        let swept = self.sweep_unsupported_learned(sid, &target)?;
        out["swept"] = json!(swept);
        Ok(out)
    }

    /// P4-38b: delete the document hits that K added (`learned` + `added`) and that no remaining
    /// Learn This example supports any more (within `CANDIDATE_REACH`). Returns how many went.
    fn sweep_unsupported_learned(&self, sid: &str, name: &str) -> Result<usize, AppError> {
        let learn = {
            let op = self.inner.registry.get(sid)?;
            let g = lock(&op);
            learn_of(&g.track(name)?.extra).unwrap_or_default()
        };
        let this: Vec<f64> = marks_of(&learn)
            .iter()
            .filter(|m| m.get("kind").and_then(Value::as_str) == Some("this"))
            .filter_map(|m| m.get("at").and_then(Value::as_array).cloned())
            .flatten()
            .filter_map(|v| v.as_f64())
            .collect();
        let events = self.track_events(sid, name)?;
        let mut ops = Vec::new();
        for (i, e) in events.iter().enumerate().rev() {
            if e.get("learned") != Some(&Value::Bool(true)) || e.get("added") != Some(&Value::Bool(true)) {
                continue;
            }
            let Some(t) = e.get("t").and_then(Value::as_f64) else {
                continue;
            };
            if !this.iter().any(|&x| (x - t).abs() <= crate::learn::CANDIDATE_REACH) {
                ops.push(json!({"op": "delete", "index": i}));
            }
        }
        let n = ops.len();
        if n > 0 {
            // P4-38e: not a hand delete, so the swept hits do not go in `learn.dropped`
            self.events_edit_as(sid, name, &ops, false)?;
        }
        Ok(n)
    }

    /// `track_learn_flip {sid, name, id}`: change a mark from "this" to "not" or back. Answers
    /// `{kind, learn, track, routed}`.
    pub fn track_learn_flip(&self, sid: &str, name: &str, id: &str) -> Result<Value, AppError> {
        let (target, routed) = self.learn_target(sid, name)?;
        let was_on = self.learn_pick_runs(sid, &target);
        let (kind, learn) = self.learn_edit(sid, &target, |l| flip_mark(l, id))?;
        let mut out = json!({"kind": kind, "flipped": true, "learn": learn, "track": target, "routed": routed});
        self.learn_refilter(sid, &target, was_on, &mut out);
        Ok(out)
    }

    /// `track_learn_forget {sid, name}`: remove every mark of the track (none is fine). Answers
    /// `{forgotten, examples, learn, track, routed}`; the examples go, `on`, `threshold`, `auto` and `method` stay.
    pub fn track_learn_forget(&self, sid: &str, name: &str) -> Result<Value, AppError> {
        let (target, routed) = self.learn_target(sid, name)?;
        let was_on = self.learn_pick_runs(sid, &target);
        let ((marks, examples), learn) = self.learn_edit(sid, &target, |l| {
            if l.is_empty() {
                return Ok((0, 0));
            }
            let (a, b) = counts(l);
            let n = marks_of(l).len();
            l.insert("marks".into(), json!([])); // the examples only: `on`, `threshold` and `auto` stay for P4-6b
            Ok((n, a + b))
        })?;
        let mut out =
            json!({"forgotten": marks, "examples": examples, "learn": learn, "track": target, "routed": routed});
        self.learn_refilter(sid, &target, was_on, &mut out);
        if marks > 0 {
            // P4-38b: Forget All takes the learned hits the examples supported with them
            let swept = self.sweep_unsupported_learned(sid, &target)?;
            out["swept"] = json!(swept);
        }
        Ok(out)
    }

    /// `track_learn_set {sid, name, learn}`: store the whole `learn` object (`null` removes the key): the undo and redo of a step. Checked by [`check_learn`]. Answers
    /// `{learn, track, routed}`.
    pub fn track_learn_set(&self, sid: &str, name: &str, learn: &Value) -> Result<Value, AppError> {
        self.track_learn_set_events(sid, name, learn, None)
    }

    /// P4-6c: [`Service::track_learn_set`] with `events`: the learn object and that hit list of the
    /// target track are stored as they are (`events_set`), no marking runs; the answer's `filter`
    /// holds the stored hits. The page's undo and redo of a Learn step restore their snapshot so.
    pub fn track_learn_set_events(
        &self,
        sid: &str,
        name: &str,
        learn: &Value,
        events: Option<&Value>,
    ) -> Result<Value, AppError> {
        if events.is_some_and(|e| !e.is_array()) {
            return Err(AppError::bad("`events` must be a list of hits"));
        }
        let new = match learn {
            Value::Null => Map::new(),
            Value::Object(m) => {
                check_learn(m)?;
                m.clone()
            }
            _ => return Err(AppError::bad("`learn` must be an object or null")),
        };
        let (target, routed) = self.learn_target(sid, name)?;
        let was_on = self.learn_pick_runs(sid, &target);
        let before = self.events_of(sid, &target).map_or((0, 0), |e| out_counts(&e));
        let ((), learn) = self.learn_edit(sid, &target, |l| {
            *l = new;
            Ok(())
        })?;
        let mut out = json!({"learn": learn, "track": target, "routed": routed});
        match events {
            Some(ev) => {
                let r = self.events_set(sid, &target, ev)?;
                add_out_counts(&mut out, before, out_counts(&self.events_of(sid, &target)?));
                out["filter"] = json!({"events": self.events_of(sid, &target)?});
                if let Some(f) = r.get("followers") {
                    out["filter"]["followers"] = f.clone();
                }
            }
            None => self.learn_refilter(sid, &target, was_on, &mut out),
        }
        Ok(out)
    }
}

/// P4-35: `(hits, out)` of a hit list: the hits the loudness cut and the bleed rules left in play
/// (`hits`) and, of those, the ones the Learn filter took out (`"filtered": true`).
fn out_counts(events: &[Value]) -> (usize, usize) {
    let on = |e: &Value, k: &str| e.get(k) == Some(&Value::Bool(true));
    let live: Vec<&Value> = events.iter().filter(|e| !on(e, "cut") && !on(e, "bleed")).collect();
    (
        live.len(),
        live.iter().filter(|e| on(e, crate::learn_filter::FILTERED)).count(),
    )
}

/// P4-35: the guard counts of a Learn change in its answer: `learn_out_before`, `learn_out_after`
/// (the hits Learn takes out) and `hits` (the hits Learn can take out, after the step), plus
/// `hits_before`. The page asks for a decision when the step takes out many more hits.
fn add_out_counts(out: &mut Value, before: (usize, usize), after: (usize, usize)) {
    out["learn_out_before"] = json!(before.1);
    out["learn_out_after"] = json!(after.1);
    out["hits"] = json!(after.0);
    out["hits_before"] = json!(before.0);
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{key}` (string)")))
}

fn opt_f64(args: &Value, key: &str) -> Result<Option<f64>, AppError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .map(Some)
            .ok_or_else(|| AppError::bad(format!("wrong argument `{key}` (number)"))),
    }
}

/// Run a command of this card for the bridge and `call`; `None` when `cmd` is not one of them.
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    let run = || -> Result<Value, AppError> {
        let (sid, name) = (arg_str(args, "sid")?, arg_str(args, "name")?);
        match cmd {
            "track_learn_mark" => {
                let (t0, t1) = (opt_f64(args, "t0")?, opt_f64(args, "t1")?);
                let range = match (t0, t1) {
                    (Some(a), Some(b)) => Some((a, b)),
                    (None, None) => None,
                    _ => return Err(AppError::bad("give both `t0` and `t1`")),
                };
                svc.track_learn_mark(sid, name, arg_str(args, "kind")?, range, opt_f64(args, "hit")?)
            }
            "track_learn_remove" => svc.track_learn_remove(sid, name, arg_str(args, "id")?, opt_f64(args, "at")?),
            "track_learn_forget_at" => {
                let (t0, t1) = (opt_f64(args, "t0")?, opt_f64(args, "t1")?);
                let range = match (t0, t1) {
                    (Some(a), Some(b)) => Some((a, b)),
                    (None, None) => None,
                    _ => return Err(AppError::bad("give both `t0` and `t1`")),
                };
                svc.track_learn_forget_at(sid, name, range, opt_f64(args, "hit")?)
            }
            "track_learn_flip" => svc.track_learn_flip(sid, name, arg_str(args, "id")?),
            "track_learn_forget" => svc.track_learn_forget(sid, name),
            "track_learn_scores" => svc.track_learn_scores(sid, name),
            "track_learn_set" => svc.track_learn_set_events(
                sid,
                name,
                args.get("learn")
                    .ok_or_else(|| AppError::bad("missing argument `learn` (object or null)"))?,
                args.get("events").filter(|v| !v.is_null()),
            ),
            _ => Err(AppError::not_found(format!("unknown command {cmd}"))),
        }
    };
    LEARN_COMMANDS.contains(&cmd).then(run)
}

#[cfg(test)]
mod tests;
