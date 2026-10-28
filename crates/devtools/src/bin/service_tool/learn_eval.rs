//! P4-6b stage A, DSP-only (#293): `service_tool learn-eval <spec.json>`, the Rust side of
//! `tools/learn_eval.py` (`docs/migration/concept-learn-mode.md` section 4.3). The spec: `{jobs: [...]}`;
//! each job `{id, wav, role, mode, ...}` prints one JSON line.
//!
//! The candidates come from the DSP detector (`detect_onsets_and_cands` with `OnsetParams::for_role`, the
//! app's kept detection), not from model activations (#281 D16: the model is gone). Recipes are the
//! DSP-only `patch-2` (option A) and `mfcc-2` (the ablation).
//!
//! - `mode: "cands"`: the detection candidates `[t, class, similarity, v]` (`histogram::dsp_detection_candidates`).
//! - `mode: "baseline"`, `values: [v, ...]`: the hits at each Sensitivity value: `{hits: [[t, ...], ...]}`.
//! - `mode: "learn"`, `sensitivity`, `this: [t]`, `not: [t]`, `variants: [{name, recipe, weights, centre}]`,
//!   `grid: [threshold, ...]`: per variant the candidate scores, the examples' scores and the automatic
//!   threshold (`learn_score::score_all`), and the hits with the Learn filter (`similarity::filter_detected`
//!   then `analyze_detected`) at the automatic threshold and at each grid value. Each variant carries
//!   P4-6c's suggestion (`similarity::suggest_threshold`).
//! - `mode: "learn"` with `sets: [{name, this, not}]` and `hit_values`: several example sets in one decode
//!   (the Unmatched measurement), plus the hits at the asked Sensitivities.
//! - `mode: "filter"`, `sensitivity`, `this: [t]`, `not: [t]`, optional `threshold` (absent: the suggestion,
//!   `learn.auto`): the app's own filter path (`learn_filter::learn_pick`) against the filter off:
//!   `{off: [t], on: [t], out: [t]}`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use drumspike_app::learn::CANDIDATE_REACH;
use drumspike_app::{histogram, jobs, learn_filter, learn_score, media, sensitivity};
use drumspike_core::analyze::{analyze_detected, detect_events, role_note, AnalyzeParams, Detector};
use drumspike_core::detect::onset::{detect_onsets_and_cands, Candidate, OnsetParams, Role};
use drumspike_core::detect::peaks::FPS;
use drumspike_core::similarity::{self, Recipe, Weights};
use serde_json::{json, Map, Value};

/// A decoded file with its DSP detection (the kept candidates of a track).
struct Track {
    dec: Arc<media::Decoded>,
    cands: Vec<Candidate>,
    cparams: OnsetParams,
    weak_limit: f32,
    rows: Vec<Value>,
}

type Cache = HashMap<(String, String), Arc<Track>>;

/// The decoded file and its DSP candidates (one detection per file and role, kept in `cache`).
fn load(cache: &mut Cache, wav: &str, role: &str) -> Result<Arc<Track>, String> {
    let key = (wav.to_string(), role.to_string());
    if let Some(t) = cache.get(&key) {
        return Ok(t.clone());
    }
    let dec = media::decode(Path::new(wav), media::mix_mode("ffmpeg")).map_err(|e| e.message)?;
    let cparams = OnsetParams::for_role(Role::parse(role).unwrap_or(Role::Other));
    let (_, cands, weak_limit) = detect_onsets_and_cands(&dec.samples, dec.sr, &cparams).map_err(|e| e.to_string())?;
    let rows = histogram::dsp_detection_candidates(&cands, &cparams, weak_limit, role);
    let t = Arc::new(Track {
        dec: Arc::new(dec),
        cands,
        cparams,
        weak_limit,
        rows,
    });
    cache.insert(key, t.clone());
    Ok(t)
}

/// The params of the slider value `v`, as the page sends them.
fn params_at(v: f64) -> Map<String, Value> {
    json!({"thresholds": sensitivity::thresholds_of(v), "sensitivity": v, "min_gap_ms": 20, "refine": true,
           "shift_samples": 0})
    .as_object()
    .expect("an object")
    .clone()
}

/// The DSP detection at the params' Sensitivity: the candidates re-judged at the slider's gate scale.
fn picked(t: &Track, role: &str, params: &AnalyzeParams) -> Result<Vec<(f64, u8)>, String> {
    let scale = sensitivity::scale_of(sensitivity::sensitivity_of(&params.thresholds)) as f32;
    let det = Detector::Candidates {
        cands: &t.cands,
        params: &t.cparams,
        weak_limit: t.weak_limit,
        scale,
    };
    detect_events(&t.dec.samples, t.dec.sr, role, &params.thresholds, det).map_err(|e| e.to_string())
}

fn times(v: &Value) -> Vec<f64> {
    v.as_array().into_iter().flatten().filter_map(Value::as_f64).collect()
}

fn r6(t: f64) -> f64 {
    (t * 1e6).round() / 1e6
}

fn recipe_of(v: &Value) -> Result<Recipe, String> {
    Recipe::parse(v.as_str().unwrap_or("patch-2")).ok_or_else(|| "unknown recipe".to_string())
}

/// The block weights `[attack, body, env]` of a variant (the activation block is gone, #281 D16).
fn weights_of(v: &Value) -> Weights {
    match v.as_array() {
        Some(a) if a.len() >= 3 => {
            let g = |i: usize| a[i].as_f64().unwrap_or(0.0) as f32;
            Weights {
                attack: g(0),
                body: g(1),
                env: g(2),
            }
        }
        _ => Weights::DEFAULT,
    }
}

fn centre_of(v: &Value) -> Result<learn_score::Centre, String> {
    match v {
        Value::Bool(true) => Ok(learn_score::Centre::Track),
        Value::Bool(false) => Ok(learn_score::Centre::None),
        Value::String(n) => learn_score::Centre::parse(n).ok_or_else(|| "unknown centre".to_string()),
        _ => Ok(learn_score::Centre::APP),
    }
}

/// The candidate frames and features of one track for one recipe, and the solid frames at `sens`.
struct Feats {
    frames: Vec<u32>,
    ts: Vec<f64>,
    f: Vec<f32>,
    solid: Vec<bool>,
}

fn feats(t: &Track, recipe: Recipe, sens: f64) -> Feats {
    let frames = learn_score::candidate_frames(&t.rows);
    let ts: Vec<f64> = frames.iter().map(|&f| f64::from(f) / FPS).collect();
    let f: Vec<f32> = similarity::candidate_features(&t.dec.samples, t.dec.sr, &ts, recipe)
        .into_iter()
        .flatten()
        .collect();
    let solid = learn_score::solid_frames(&learn_score::frame_values(&t.rows, &frames), sens);
    Feats { frames, ts, f, solid }
}

/// The Learn scores of one example set (`learn_score::score_all`, the scorer of `track_learn_scores`).
fn score(
    t: &Track,
    fe: &Feats,
    recipe: Recipe,
    w: &Weights,
    centre: learn_score::Centre,
    this: &[f64],
    not: &[f64],
) -> learn_score::Scored {
    let d_opt = Some(t.dec.clone());
    let extra = learn_score::extra_of(&d_opt, recipe);
    learn_score::score_all(&fe.frames, &fe.f, recipe, w, centre, &fe.solid, this, not, &extra)
}

/// The Learn score of each frame, keyed by frame index.
fn by_frame(fe: &Feats, s: &learn_score::Scored) -> Vec<(usize, f32)> {
    fe.frames
        .iter()
        .map(|&f| f as usize)
        .zip(s.scores.iter().copied())
        .collect()
}

/// The hits of the Learn filter at threshold `th`: the detection's candidates the scores keep, analysed.
fn filtered_hits(
    t: &Track,
    by_frame: &[(usize, f32)],
    det: &[(f64, u8)],
    role: &str,
    params: &AnalyzeParams,
    (this, not): (&[f64], &[f64]),
    th: f32,
) -> Result<Vec<f64>, String> {
    let kept = similarity::filter_detected(det, by_frame, th, this, not, role_note(role), CANDIDATE_REACH);
    let a = analyze_detected(&t.dec.samples, t.dec.sr, role, &kept, params).map_err(|e| e.to_string())?;
    Ok(a.events.iter().map(|h| r6(h.t)).collect())
}

fn baseline(cache: &mut Cache, job: &Value, role: &str, wav: &str) -> Result<Value, String> {
    let t = load(cache, wav, role)?;
    let mut hits = Vec::new();
    for v in times(&job["values"]) {
        let parsed = jobs::parse_params(Some(&params_at(v)))?;
        let det = picked(&t, role, &parsed.params)?;
        let a = analyze_detected(&t.dec.samples, t.dec.sr, role, &det, &parsed.params).map_err(|e| e.to_string())?;
        hits.push(a.events.iter().map(|h| r6(h.t)).collect::<Vec<_>>());
    }
    Ok(json!({"hits": hits}))
}

fn learn_sets(cache: &mut Cache, job: &Value, role: &str, wav: &str) -> Result<Value, String> {
    let t = load(cache, wav, role)?;
    let sens = job["sensitivity"].as_f64().unwrap_or(100.0);
    let recipe = recipe_of(&job["recipe"])?;
    let fe = feats(&t, recipe, sens);
    let mut sets = Vec::new();
    for set in job["sets"].as_array().into_iter().flatten() {
        let (this, not) = (times(&set["this"]), times(&set["not"]));
        let s = score(
            &t,
            &fe,
            recipe,
            &Weights::DEFAULT,
            learn_score::Centre::APP,
            &this,
            &not,
        );
        sets.push(json!({
            "name": set["name"], "this": this, "auto": s.auto, "suggest": s.suggest, "this_scores": s.this,
            "scores": fe.ts.iter().zip(&s.scores).map(|(t, v)| json!([t, v])).collect::<Vec<_>>(),
        }));
    }
    let mut hits_at = Vec::new();
    for v in times(&job["hit_values"]) {
        let parsed = jobs::parse_params(Some(&params_at(v)))?;
        let det = picked(&t, role, &parsed.params)?;
        let a = analyze_detected(&t.dec.samples, t.dec.sr, role, &det, &parsed.params).map_err(|e| e.to_string())?;
        hits_at.push(json!([v, a.events.iter().map(|h| r6(h.t)).collect::<Vec<_>>()]));
    }
    let duration = t.dec.samples.len() as f64 / f64::from(t.dec.sr);
    Ok(json!({"n_candidates": fe.frames.len(), "duration": duration, "sets": sets, "hits_at": hits_at}))
}

fn learn(cache: &mut Cache, job: &Value, role: &str, wav: &str) -> Result<Value, String> {
    let t = load(cache, wav, role)?;
    let sens = job["sensitivity"].as_f64().unwrap_or(100.0);
    let parsed = jobs::parse_params(Some(&params_at(sens)))?;
    let det = picked(&t, role, &parsed.params)?;
    let (this, not) = (times(&job["this"]), times(&job["not"]));
    let mut out = Vec::new();
    let n_candidates = learn_score::candidate_frames(&t.rows).len();
    for var in job["variants"].as_array().into_iter().flatten() {
        let recipe = recipe_of(&var["recipe"])?;
        let t0 = Instant::now();
        let fe = feats(&t, recipe, sens);
        let ms_features = t0.elapsed().as_secs_f64() * 1000.0;
        let w = weights_of(&var["weights"]);
        let centre = centre_of(&var["centre"])?;
        let t1 = Instant::now();
        let s = score(&t, &fe, recipe, &w, centre, &this, &not);
        let ms_score = t1.elapsed().as_secs_f64() * 1000.0;
        let scored = by_frame(&fe, &s);
        let mut grid = Vec::new();
        for th in times(&job["grid"]) {
            let hits = filtered_hits(&t, &scored, &det, role, &parsed.params, (&this, &not), th as f32)?;
            grid.push(json!([th, hits]));
        }
        let hits_auto = filtered_hits(&t, &scored, &det, role, &parsed.params, (&this, &not), s.auto)?;
        out.push(json!({
            "name": var["name"], "recipe": recipe.name(), "centre": centre.name(), "auto": s.auto,
            "this_scores": s.this, "not_scores": s.not, "hits_auto": hits_auto, "grid": grid,
            "ms_features": ms_features, "ms_score": ms_score, "suggest": s.suggest,
            "scores": fe.ts.iter().zip(&s.scores).map(|(t, v)| json!([t, v])).collect::<Vec<_>>(),
        }));
    }
    let duration = t.dec.samples.len() as f64 / f64::from(t.dec.sr);
    Ok(json!({"n_candidates": n_candidates, "duration": duration, "variants": out}))
}

fn filter(cache: &mut Cache, job: &Value, role: &str, wav: &str) -> Result<Value, String> {
    let t = load(cache, wav, role)?;
    let sens = job["sensitivity"].as_f64().unwrap_or(sensitivity::DEFAULT);
    let parsed = jobs::parse_params(Some(&params_at(sens)))?;
    let off = analyze_detected(
        &t.dec.samples,
        t.dec.sr,
        role,
        &picked(&t, role, &parsed.params)?,
        &parsed.params,
    )
    .map_err(|e| e.to_string())?;
    let mark = |i: usize, kind: &str, t: f64| json!({"id": format!("m{i}"), "kind": kind, "t0": t, "t1": t, "at": [t]});
    let mut marks: Vec<Value> = Vec::new();
    for t in times(&job["this"]) {
        marks.push(mark(marks.len() + 1, "this", t));
    }
    for t in times(&job["not"]) {
        marks.push(mark(marks.len() + 1, "not", t));
    }
    let mut learn = json!({"on": true, "auto": true, "threshold": 0.5, "method": "patch-2", "marks": marks});
    if let Some(th) = job["threshold"].as_f64() {
        learn["auto"] = json!(false);
        learn["threshold"] = json!(th);
    }
    let learn = learn.as_object().expect("an object").clone();
    let scale_sens = sensitivity::sensitivity_of(&parsed.params.thresholds).round();
    let pick = learn_filter::learn_pick(
        &t.dec.samples,
        t.dec.sr,
        role,
        (&t.cands, &t.cparams, t.weak_limit),
        &t.rows,
        &learn,
        &[],
        &parsed.params,
        scale_sens,
        sensitivity::scale_of(sensitivity::sensitivity_of(&parsed.params.thresholds)) as f32,
        None,
        None,
    )?;
    let (mut on, mut out) = (Vec::new(), Vec::new());
    for (i, h) in pick.base.events.iter().enumerate() {
        if pick.base.is_learn_out(i) {
            out.push(r6(h.t));
        } else {
            on.push(r6(h.t));
        }
    }
    on.extend(pick.forced.events.iter().map(|h| r6(h.t)));
    on.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let off_hits: Vec<f64> = off.events.iter().map(|h| r6(h.t)).collect();
    Ok(json!({"off": off_hits, "on": on, "out": out}))
}

fn one(cache: &mut Cache, job: &Value) -> Result<Value, String> {
    let id = job["id"].as_str().unwrap_or("");
    let role = job["role"].as_str().unwrap_or("other");
    let wav = job["wav"].as_str().ok_or("job without wav")?;
    let mut body = match job["mode"].as_str().unwrap_or("") {
        "cands" => {
            let t = load(cache, wav, role)?;
            json!({"candidates": t.rows})
        }
        "baseline" => baseline(cache, job, role, wav)?,
        "learn" if job["sets"].is_array() => learn_sets(cache, job, role, wav)?,
        "learn" => learn(cache, job, role, wav)?,
        "filter" => filter(cache, job, role, wav)?,
        other => return Err(format!("unknown mode {other:?}")),
    };
    body["id"] = json!(id);
    Ok(body)
}

/// Run every job of `<spec.json>`; one JSON line per job (`{error}` for a failed one).
pub fn run(spec_file: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(spec_file).map_err(|e| format!("{spec_file}: {e}"))?;
    let spec: Value = serde_json::from_str(&text).map_err(|e| format!("{spec_file}: {e}"))?;
    let mut cache = Cache::new();
    for job in spec["jobs"].as_array().into_iter().flatten() {
        let r = one(&mut cache, job).unwrap_or_else(|e| json!({"id": job["id"], "error": e}));
        println!("{r}");
    }
    Ok(())
}
