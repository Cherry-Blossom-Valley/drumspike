//! P4-6b stage A: the Learn scores of a track's detection candidates (`docs/migration/concept-learn-mode.md`,
//! sections 3.1, 3.2 and 3.5). The features come from `drumspike_core::similarity`; this module keeps
//! them in a cache and scores them against the track's Learn examples. No filter runs here (stage B).
//!
//! The cache: `Analysis/<track>/learn.bin` with `learn.json` next to it as its key (the key of the kept
//! activations, [`crate::activation::activation_key`], the hash of the candidate frames and the method),
//! written key-last like `activation.bin`, so a half-written pair never looks current. Layout of
//! `learn.bin` (little endian): the magic `DSLRN\0\0\0`, `u32` version 1, `u32` dimension, `u64`
//! candidates, then one `u32` frame per candidate, then `candidates x dimension` `f32` (the flat,
//! unweighted blocks of `similarity::Blocks::flat`). Regenerable, never in the document.
//!
//! | Command | What it does |
//! |---|---|
//! | `track_learn_scores {sid, name}` | the score of every candidate and of every example, and the automatic threshold |
//!
//! The command changes no document; it may write the cache (an `Analysis/` file), so the registry
//! lists it as a read for agents.

use std::fs;
use std::io;
use std::path::Path;
use std::time::Instant;

use drumspike_core::detect::peaks::FPS;
use drumspike_core::similarity::{self, Recipe, Weights};
use serde_json::{Map, Value, json};

use crate::Service;
use crate::error::AppError;
use crate::learn::{CANDIDATE_REACH, METHOD, learn_of};
use crate::media::mix_mode;
use crate::registry::{lock, resolve_shared};

/// The features file inside `Analysis/<track>/`.
pub const LEARN_BIN: &str = "learn.bin";
/// The key file next to it.
pub const LEARN_KEY: &str = "learn.json";
/// The magic bytes at the start of [`LEARN_BIN`].
pub const MAGIC: [u8; 8] = *b"DSLRN\0\0\0";
/// The format version.
pub const VERSION: u32 = 1;
const HEADER: usize = 24;

/// FNV-1a (64 bit) of the candidate frames, as 16 hex digits: the candidate part of the cache key.
#[must_use]
pub fn frames_hash(frames: &[u32]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for f in frames {
        for b in f.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

/// The key of a track's features: the source's key (the candidates' lifetime), the candidate frames' hash
/// and the method. #293: the DSP candidates replace the kept activations, so the key has no model part.
#[must_use]
pub fn learn_key(source: &Value, frames: &[u32], method: &str) -> Value {
    json!({"source": source, "candidates": frames_hash(frames), "method": method})
}

/// Write the features (`frames.len() x dim` values in `feats`) and their key into `dir`. The key goes last.
pub fn write_learn(dir: &Path, frames: &[u32], feats: &[f32], dim: usize, key: &Value) -> io::Result<()> {
    if feats.len() != frames.len() * dim {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "features are not (candidates, dim)",
        ));
    }
    fs::create_dir_all(dir)?;
    let _ = fs::remove_file(dir.join(LEARN_KEY));
    let mut buf = Vec::with_capacity(HEADER + frames.len() * 4 + feats.len() * 4);
    buf.extend_from_slice(&MAGIC);
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&(dim as u32).to_le_bytes());
    buf.extend_from_slice(&(frames.len() as u64).to_le_bytes());
    for f in frames {
        buf.extend_from_slice(&f.to_le_bytes());
    }
    for v in feats {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    // two calls at once (the page and an agent) each write their own temporary files
    let n = TMP_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = |name: &str| dir.join(format!("{name}.{}.{n}.tmp", std::process::id()));
    let t = tmp(LEARN_BIN);
    fs::write(&t, &buf)?;
    if let Err(e) = fs::rename(&t, dir.join(LEARN_BIN)) {
        let _ = fs::remove_file(&t);
        return Err(e);
    }
    let t = tmp(LEARN_KEY);
    fs::write(&t, key.to_string())?;
    fs::rename(&t, dir.join(LEARN_KEY))
}

static TMP_SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The features in `dir` when their key equals `key`, the dimension is `dim` and the file is whole:
/// `(frames, features)`; else `None`.
#[must_use]
pub fn read_learn(dir: &Path, key: &Value, dim: usize) -> Option<(Vec<u32>, Vec<f32>)> {
    let stored: Value = serde_json::from_str(&fs::read_to_string(dir.join(LEARN_KEY)).ok()?).ok()?;
    if stored != *key {
        return None;
    }
    let b = fs::read(dir.join(LEARN_BIN)).ok()?;
    if b.len() < HEADER || b[..8] != MAGIC {
        return None;
    }
    let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().expect("4 bytes"));
    let n = u64::from_le_bytes(b[16..24].try_into().expect("8 bytes")) as usize;
    if u32_at(8) != VERSION || u32_at(12) as usize != dim || b.len() != HEADER + n * 4 + n * dim * 4 {
        return None;
    }
    let words: Vec<[u8; 4]> = b[HEADER..].as_chunks::<4>().0.to_vec();
    let frames = words[..n].iter().map(|c| u32::from_le_bytes(*c)).collect();
    let feats = words[n..].iter().map(|c| f32::from_le_bytes(*c)).collect();
    Some((frames, feats))
}

/// The unique candidate frames of `[t, class, strength, v]` rows (time order).
#[must_use]
pub fn candidate_frames(cands: &[Value]) -> Vec<u32> {
    let mut f: Vec<u32> = cands
        .iter()
        .filter_map(|c| c.get(0)?.as_f64())
        .map(|t| (t * FPS).round() as u32)
        .collect();
    f.dedup();
    f
}

/// The lowest slider value (`v`, the fourth value of a row) of each frame of [`candidate_frames`]: a
/// frame passes the Sensitivity `s` when its value is `s` or less.
pub fn frame_values(cands: &[Value], frames: &[u32]) -> Vec<u64> {
    let mut out = vec![u64::MAX; frames.len()];
    for c in cands {
        let (Some(t), Some(v)) = (c.get(0).and_then(Value::as_f64), c.get(3).and_then(Value::as_u64)) else {
            continue;
        };
        if let Ok(i) = frames.binary_search(&((t * FPS).round() as u32)) {
            out[i] = out[i].min(v);
        }
    }
    out
}

/// The index of the frame nearest to `t` within [`CANDIDATE_REACH`] in the sorted `frames`.
#[must_use]
pub fn nearest_frame(frames: &[u32], t: f64) -> Option<usize> {
    let f = t * FPS;
    let i = frames.partition_point(|&x| f64::from(x) < f);
    [i.checked_sub(1), (i < frames.len()).then_some(i)]
        .into_iter()
        .flatten()
        .filter(|&j| (f64::from(frames[j]) / FPS - t).abs() <= CANDIDATE_REACH + 1e-9)
        .min_by(|&a, &b| {
            (f64::from(frames[a]) - f)
                .abs()
                .total_cmp(&(f64::from(frames[b]) - f).abs())
        })
}

/// The scores of one track: every candidate frame's score, the examples' own scores (each without
/// itself) and the automatic threshold.
#[derive(Debug, Clone, PartialEq)]
pub struct Scored {
    /// The score of each candidate frame (the order of the frames).
    pub scores: Vec<f32>,
    /// #293 (B8) diagnosis: per candidate, the cosine to the confirmed-hit template (`None` without one: no
    /// "not" example, or no template) and the mean cosine to the nearest "not" examples (0 without any).
    pub shape: Vec<(Option<f32>, f32)>,
    /// The scores of the "this" examples.
    pub this: Vec<f32>,
    /// The scores of the "not" examples.
    pub not: Vec<f32>,
    /// [`similarity::auto_threshold`].
    pub auto: f32,
    /// P4-6c: [`similarity::suggest_threshold`] (the threshold while `learn.auto` is set).
    pub suggest: f32,
}

/// Which mean a vector is taken relative to before the cosine ([`similarity::centred`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Centre {
    /// No centring: the raw blocks.
    None,
    /// The mean of all candidates of the track (stage A's first rule).
    Track,
    /// The mean of the bleed: the candidates that score under the automatic threshold of a first pass
    /// with [`Centre::Track`] and are no example. Fewer than [`BLEED_MIN`] such: no centring.
    Bleed,
    /// [`Centre::Track`], or no centring when the hits dominate: more than half of the candidates that
    /// pass the Sensitivity score at or above the first pass's automatic threshold.
    Auto,
}

/// The fewest bleed candidates [`Centre::Bleed`] centres on.
pub const BLEED_MIN: usize = 8;

impl Centre {
    /// The centring the app uses (chosen on Session A, `phase4-report.md` "P4-6b stage A fix round").
    pub const APP: Centre = Centre::Track;

    /// The name in `service_tool learn-eval` (`none`, `track`, `bleed`, `auto`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Centre::None => "none",
            Centre::Track => "track",
            Centre::Bleed => "bleed",
            Centre::Auto => "auto",
        }
    }

    /// The mode of a name, `None` for an unknown one.
    #[must_use]
    pub fn parse(name: &str) -> Option<Centre> {
        [Centre::None, Centre::Track, Centre::Bleed, Centre::Auto]
            .into_iter()
            .find(|c| c.name() == name)
    }
}

/// The frames that pass the Sensitivity `sens`: a frame's lowest slider value ([`frame_values`]) at
/// or under it.
#[must_use]
pub fn solid_frames(values: &[u64], sens: f64) -> Vec<bool> {
    values.iter().map(|&v| v as f64 <= sens).collect()
}

/// #293: the confirmed-hit template of the track: the mean of the prepared candidates that pass the Sensitivity
/// (`solid`; empty: all), at unit length as the candidates are.
fn confirmed_template(cand: &[Vec<f32>], solid: &[bool], recipe: Recipe) -> Vec<f32> {
    let flat: Vec<f32> = cand
        .iter()
        .enumerate()
        .filter(|(i, _)| solid.get(*i).copied().unwrap_or(true))
        .flat_map(|(_, c)| c.iter().copied())
        .collect();
    let mut t = similarity::track_mean(&flat, recipe);
    let n = t.iter().map(|v| v * v).sum::<f32>().sqrt();
    if n > 1e-9 {
        for v in &mut t {
            *v /= n;
        }
    }
    t
}

/// One scoring pass, each vector taken relative to `mean` (none: raw).
#[allow(clippy::too_many_arguments)]
fn score_once(
    frames: &[u32],
    feats: &[f32],
    recipe: Recipe,
    w: &Weights,
    mean: Option<&[f32]>,
    solid: &[bool],
    this: &[f64],
    not: &[f64],
    extra: &dyn Fn(f64) -> Option<Vec<f32>>,
) -> Scored {
    let prep = |f: &[f32]| -> Vec<f32> {
        match mean {
            Some(m) => similarity::weigh(&similarity::centred(f, m, recipe), recipe, w),
            None => similarity::weigh(f, recipe, w),
        }
    };
    let cand: Vec<Vec<f32>> = feats.chunks(recipe.len()).map(prep).collect();
    // an example far from every candidate is prepared exactly as the candidates are
    let example = |t: f64| -> Option<Vec<f32>> {
        match nearest_frame(frames, t) {
            Some(i) => Some(cand[i].clone()),
            None => extra(t).map(|f| prep(&f)),
        }
    };
    let pos: Vec<Vec<f32>> = this.iter().filter_map(|&t| example(t)).collect();
    let neg: Vec<Vec<f32>> = not.iter().filter_map(|&t| example(t)).collect();
    let pr: Vec<&[f32]> = pos.iter().map(Vec::as_slice).collect();
    let nr: Vec<&[f32]> = neg.iter().map(Vec::as_slice).collect();
    // #293: Forget marks and no "this" example: a candidate goes only when it is closer to a mark than to the
    // track's confirmed-hit template; otherwise it scores as a candidate with no example (0.5) and stays
    let template = (pos.is_empty() && !neg.is_empty()).then(|| confirmed_template(&cand, solid, recipe));
    // #293 (B8) diagnosis: per candidate the cosine to the template (none without one) and the mean cosine to the
    // nearest "not" examples; the rule below reads the same two numbers
    let shape: Vec<(Option<f32>, f32)> = cand
        .iter()
        .map(|c| {
            (
                template.as_ref().map(|t| similarity::cosine(c, t)),
                similarity::nearest_mean(c, &nr),
            )
        })
        .collect();
    let scores: Vec<f32> = cand
        .iter()
        .zip(&shape)
        .map(|(c, &(tc, mc))| match tc {
            Some(tc) if tc >= mc => 0.5,
            _ => similarity::score(c, &pr, &nr),
        })
        .collect();
    let (t, n) = similarity::example_scores(&pr, &nr);
    let passing: Vec<f32> = scores
        .iter()
        .enumerate()
        .filter(|(i, _)| solid.get(*i).copied().unwrap_or(true))
        .map(|(_, v)| *v)
        .collect();
    let auto = similarity::auto_threshold(&passing, &t, &n);
    let suggest = similarity::suggest_threshold(&passing, &t, &n);
    Scored {
        scores,
        shape,
        this: t,
        not: n,
        auto,
        suggest,
    }
}

/// Score the candidates (`feats`, flat, `frames.len() x recipe.len()`) against the examples at the
/// times `this` and `not`: an example within [`CANDIDATE_REACH`] of a candidate takes its features,
/// another one the features `extra(t)` gives (`None`: the example is left out). `centre` picks the
/// mean every vector is taken relative to ([`Centre`]; the app uses [`Centre::APP`]). `solid` marks
/// the frames that pass the Sensitivity (empty: all); only those set the automatic threshold.
#[allow(clippy::too_many_arguments)]
pub fn score_all(
    frames: &[u32],
    feats: &[f32],
    recipe: Recipe,
    w: &Weights,
    centre: Centre,
    solid: &[bool],
    this: &[f64],
    not: &[f64],
    extra: &dyn Fn(f64) -> Option<Vec<f32>>,
) -> Scored {
    let once = |mean: Option<&[f32]>| score_once(frames, feats, recipe, w, mean, solid, this, not, extra);
    let track = || similarity::track_mean(feats, recipe);
    match centre {
        Centre::None => once(None),
        Centre::Track => once(Some(&track())),
        Centre::Auto => {
            let first = once(Some(&track()));
            let (mut n, mut above) = (0usize, 0usize);
            for (i, v) in first.scores.iter().enumerate() {
                if solid.get(i).copied().unwrap_or(true) {
                    n += 1;
                    above += usize::from(*v >= first.auto);
                }
            }
            if 2 * above > n { once(None) } else { first }
        }
        Centre::Bleed => {
            let first = once(Some(&track()));
            let examples: Vec<usize> = this
                .iter()
                .chain(not)
                .filter_map(|&t| nearest_frame(frames, t))
                .collect();
            let dim = recipe.len();
            let bleed: Vec<f32> = feats
                .chunks(dim)
                .enumerate()
                .filter(|(i, _)| first.scores[*i] < first.auto && !examples.contains(i))
                .flat_map(|(_, f)| f.iter().copied())
                .collect();
            if bleed.len() / dim < BLEED_MIN {
                once(None)
            } else {
                once(Some(&similarity::track_mean(&bleed, recipe)))
            }
        }
    }
}

/// The example times of a `learn` object: `(this, not)`.
pub fn example_times(learn: &serde_json::Map<String, Value>) -> (Vec<f64>, Vec<f64>) {
    let (mut this, mut not) = (Vec::new(), Vec::new());
    for m in learn.get("marks").and_then(Value::as_array).into_iter().flatten() {
        let ats = m
            .get("at")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_f64);
        match m.get("kind").and_then(Value::as_str) {
            Some("this") => this.extend(ats),
            Some("not") => not.extend(ats),
            _ => {}
        }
    }
    (this, not)
}

fn round4(v: f32) -> f64 {
    (f64::from(v) * 10_000.0).round() / 10_000.0
}
impl Service {
    /// `track_learn_scores {sid, name}`: the Learn scores of a track (a follower: of its source,
    /// `routed`). `{state: "ok", track, routed, method, candidates: [[t, class, score, v, gate, attack]], shape: [[template cos, mark cos]],
    /// examples: {this: [[t, score]], not: [[t, score]]}, auto, suggest, sens, threshold, auto_on, attack, cache, ms}`:
    /// `shape` (#293 B8 diagnosis) one pair per candidate, see [`Scored::shape`];
    /// one row per detection candidate (`track_histograms`' rows with the score in place of the
    /// strength; `gate`: the Forget-only attack gate keeps it out of the grey, #293 B8; `attack`: its
    /// attack time in ms, null unless the gate runs; the top-level `attack` is `{marks, window}`: the
    /// "not" examples' attack times and the ms span an attack must lie in, null unless the gate runs),
    /// each example with its own score (without itself), stage A's automatic threshold
    /// (`auto`), P4-6c's suggestion (`suggest`, [`similarity::suggest_threshold`]), the track's
    /// Sensitivity, and the threshold the filter uses (`threshold`: the suggestion while
    /// `learn.auto` is set, `auto_on`, else the stored one), `cache` `hit` or `built`, the time in
    /// ms. Without kept candidates `{state: "missing"}` (#293: there is no model state). 409 for an offline track or a
    /// gone source.
    pub fn track_learn_scores(&self, sid: &str, name: &str) -> Result<Value, AppError> {
        let c = match self.learn_ctx(sid, name)? {
            Ok(c) => c,
            Err((state, target, routed)) => return Ok(json!({"state": state, "track": target, "routed": routed})),
        };
        let score_of: std::collections::HashMap<u32, f32> =
            c.frames.iter().copied().zip(c.scored.scores.iter().copied()).collect();
        let rows: Vec<Value> = c
            .cands
            .iter()
            .zip(&c.gate)
            .enumerate()
            .map(|(i, (r, &kept))| {
                let t = r[0].as_f64().unwrap_or(0.0);
                let sc = score_of.get(&((t * FPS).round() as u32)).copied().unwrap_or(0.0);
                // #293 (B8) diagnosis: the attack time (ms, one decimal) of the candidate, null unless the gate runs
                let attack = c.attack.as_ref().map(|p| (p.attacks[i] * 10.0).round() / 10.0);
                json!([t, r[1], round4(sc), r[3], kept, attack])
            })
            .collect();
        let attack = c.attack.as_ref().map(|p| {
            json!({"marks": p.marks_ms.iter().map(|m| (m * 10.0).round() / 10.0).collect::<Vec<f64>>(),
                   "window": [p.window.0, p.window.1]})
        });
        let pair = |ts: &[f64], sc: &[f32]| -> Vec<Value> {
            // `example_scores` skips the examples it could not place, so the times follow the placed ones
            let placed: Vec<f64> = ts
                .iter()
                .copied()
                .filter(|&t| nearest_frame(&c.frames, t).is_some() || c.decoded.is_some())
                .collect();
            placed.iter().zip(sc).map(|(t, v)| json!([t, round4(*v)])).collect()
        };
        Ok(json!({
            "state": "ok",
            "track": c.target,
            "routed": c.routed,
            "method": c.method,
            "candidates": rows,
            "examples": {"this": pair(&c.this, &c.scored.this), "not": pair(&c.not, &c.scored.not)},
            "auto": round4(c.scored.auto),
            "suggest": round4(c.scored.suggest),
            "sens": c.sens,
            "threshold": round4(effective_threshold(&c.learn, &c.scored)),
            "auto_on": auto_on(&c.learn),
            "attack": attack,
            "shape": c.scored.shape.iter().map(|&(tc, mc)| json!([tc.map(round4), round4(mc)])).collect::<Vec<Value>>(),
            "cache": c.cache,
            "ms": {"features": (c.ms_features * 10.0).round() / 10.0,
                   "total": (c.start.elapsed().as_secs_f64() * 10_000.0).round() / 10.0},
        }))
    }
}

/// P4-6c: true while the track's threshold follows the examples (`learn.auto`; concept 3.2: until
/// the engineer drags it).
pub fn auto_on(learn: &Map<String, Value>) -> bool {
    learn.get("auto").and_then(Value::as_bool).unwrap_or(false)
}

/// P4-6c: the threshold the Learn filter uses: the suggestion of the scores while `learn.auto` is
/// set ([`auto_on`]), else the stored `threshold` (default [`similarity::DEFAULT_THRESHOLD`]).
#[must_use]
pub fn effective_threshold(learn: &Map<String, Value>, scored: &Scored) -> f32 {
    if auto_on(learn) {
        scored.suggest
    } else {
        crate::learn_filter::threshold_of(learn)
    }
}

/// The shared front half of the endpoint: the target track, its candidates, frames, features
/// (cached), examples and scores.
struct LearnCtx {
    target: String,
    routed: bool,
    learn: Map<String, Value>,
    method: String,
    cands: Vec<Value>,
    frames: Vec<u32>,
    cache: &'static str,
    this: Vec<f64>,
    not: Vec<f64>,
    /// The decoded source when an example has no candidate frame (its features from the audio).
    decoded: Option<std::sync::Arc<crate::media::Decoded>>,
    sens: f64,
    scored: Scored,
    /// #293 (B8): per candidate, the Forget-only attack gate keeps it out of the grey ([`attack_gate_kept`]).
    gate: Vec<bool>,
    /// #293 (B8) diagnosis: the attack times behind `gate` (`None` unless the gate runs).
    attack: Option<crate::learn_filter::AttackProfile>,
    ms_features: f64,
    start: Instant,
}

/// The `extra` feature closure of [`score_all`] from the decoded source and the recipe (#293: the DSP
/// features of an example time with no candidate; a free function so [`LearnCtx`] can hold the data).
pub fn extra_of<'a>(
    d: &'a Option<std::sync::Arc<crate::media::Decoded>>,
    recipe: Recipe,
) -> impl Fn(f64) -> Option<Vec<f32>> + 'a {
    move |t: f64| -> Option<Vec<f32>> {
        let d = d.as_ref()?;
        similarity::candidate_features(&d.samples, d.sr, &[t], recipe)
            .into_iter()
            .next()
    }
}

impl Service {
    /// The front half of [`Service::track_learn_scores`]: everything up to and including
    /// [`score_all`]. `Err((state, target, routed))` is the endpoint's short answer (`missing`,
    /// `missing`).
    fn learn_ctx(&self, sid: &str, name: &str) -> Result<Result<LearnCtx, (&'static str, String, bool)>, AppError> {
        let start = Instant::now();
        let (target, routed) = self.learn_target(sid, name)?;
        let op = self.inner.registry.get(sid)?;
        resolve_shared(&op);
        let st = self.track_state(&op, &target)?;
        let learn = {
            let g = lock(&op);
            learn_of(&g.track(&target)?.extra).unwrap_or_default()
        };
        let method = learn
            .get("method")
            .and_then(Value::as_str)
            .filter(|m| Recipe::parse(m).is_some())
            .unwrap_or(METHOD)
            .to_string();
        let recipe = Recipe::parse(&method).expect("checked above");
        let Some(cands) = self.track_candidates(sid, &target)? else {
            return Ok(Err(("missing", target, routed)));
        };
        let frames = candidate_frames(&cands);
        let key = learn_key(&json!({"source": st.key, "mix": st.mix}), &frames, &method);
        let source = st.source.clone()?;
        let decoded = std::cell::OnceCell::new();
        let decode = || -> Result<std::sync::Arc<crate::media::Decoded>, AppError> {
            if let Some(d) = decoded.get() {
                return Ok(std::sync::Arc::clone(d));
            }
            let d = self.inner.media.decode_cached(&source, mix_mode(&st.mix))?;
            let _ = decoded.set(std::sync::Arc::clone(&d));
            Ok(d)
        };
        let (feats, cache) = match read_learn(&st.dir, &key, recipe.len()) {
            Some((f, feats)) if f == frames => (feats, "hit"),
            _ => {
                let d = decode()?;
                let times: Vec<f64> = frames.iter().map(|&f| f64::from(f) / FPS).collect();
                let flat: Vec<f32> = similarity::candidate_features(&d.samples, d.sr, &times, recipe)
                    .into_iter()
                    .flatten()
                    .collect();
                // the cache is regenerable: a folder that cannot be written only costs the next call the time
                let _ = write_learn(&st.dir, &frames, &flat, recipe.len(), &key);
                (flat, "built")
            }
        };
        let ms_features = start.elapsed().as_secs_f64() * 1000.0;
        let (this, not) = example_times(&learn);
        let far: Vec<f64> = this
            .iter()
            .chain(&not)
            .copied()
            .filter(|&t| nearest_frame(&frames, t).is_none())
            .collect();
        let d = if far.is_empty() { None } else { Some(decode()?) };
        let sens = crate::jobs::parse_params(st.params.as_ref()).map_or(crate::sensitivity::DEFAULT, |p| {
            crate::sensitivity::sensitivity_of(&p.params.thresholds).round()
        });
        let solid = solid_frames(&frame_values(&cands, &frames), sens);
        let scored = score_all(
            &frames,
            &feats,
            recipe,
            &Weights::DEFAULT,
            Centre::APP,
            &solid,
            &this,
            &not,
            &extra_of(&d, recipe),
        );
        // #293 (B8): the gate of a Forget-only track reads the attacks of the audio the pick reads (the decoded source)
        let times: Vec<f64> = cands.iter().map(|r| r[0].as_f64().unwrap_or(0.0)).collect();
        let (gate, attack) = if this.is_empty() && !not.is_empty() {
            let dd = decode()?;
            let gate = crate::learn_filter::attack_gate_kept(&dd.samples, dd.sr, &times, &this, &not);
            (
                gate,
                Some(crate::learn_filter::attack_profile(&dd.samples, dd.sr, &times, &not)),
            )
        } else {
            (vec![false; times.len()], None)
        };
        Ok(Ok(LearnCtx {
            target,
            routed,
            learn,
            method,
            cands,
            frames,
            cache,
            this,
            not,
            decoded: d,
            sens,
            scored,
            gate,
            attack,
            ms_features,
            start,
        }))
    }
}

#[cfg(test)]
mod tests;
