//! The analysis job (`server.analyze`, `server.job_status`): one worker thread per
//! `analyze_start`, one analysis at a time in the whole app, the state of every track in a
//! JSON value with the server's shape, and a progress callback that gets the same value.
//!
//! The detector is the one the Python engine picks in the Rust mode (`engine.detector_info`):
//! the DSP detector (`rust-dsp`); main is model-free (#281 D16).
//!
//! P3-15b: the job is a two-stage pipeline ([`crate::overlap`]). The preparation of a track
//! ([`prepare_one`]: the decode and downmix, and for the model detector the resample and the
//! features) runs on the small preparation pool one track ahead of the model step
//! ([`finish_one`]: the model run, the peak picking, the steps after the detection, the
//! store of the events), which runs on the job thread with the one loaded model. At most two
//! prepared tracks are in memory. The job states are unchanged: a track is `running` with
//! the stage `decoding` while it is prepared and `analyzing` in the model step, so two tracks
//! can be `running` at once (the page shows the first one). A cancel ([`Jobs::cancel`], also
//! the close of the project) ends the job after the track in the model step; the tracks that
//! did not run get the error `analysis cancelled`.
//!
//! R08 (#237): a job has an identity. Every start mints the next generation with its own
//! cancel flag ([`JobToken`]); the worker owns the token of its generation. A duplicate
//! start is refused by `analyze_start` (one analysis per project); a start that still
//! replaces a job (a close and a reopen, or a direct `queue`) makes the older generation
//! stale. Every state write, progress callback and commit of the worker checks its
//! generation first, so a stale worker writes no state, emits no progress and commits no
//! events: a closed project ignores the late callbacks of its last worker, a cancel
//! reaches only the active generation, and a late update cannot resurrect a cleared
//! session. The close of the project does not wait for a worker: it invalidates the
//! generation and takes the session out of the registry at once. The on-disk project lock
//! is released when the last holder drops its session - a worker holds its session until
//! it ends (at most until the end of its current model step), so a reopen of the same
//! folder waits for the stale worker to stop, and the stale worker commits nothing.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use drumspike_core::analyze::{Analysis, AnalyzeParams};
use drumspike_core::detect::N_CLASSES;
use drumspike_core::project::{cache_key, CacheKey, Event, ANALYSIS_DIR, BACKUPS_DIR};
use drumspike_core::refine::{RefineRule, RefineWindow};
use drumspike_core::velocity::VelocityMap;
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::error::AppError;
use crate::media::{decode, mix_mode, Decoded};
use crate::overlap;
use crate::registry::{lock, Shared};
use crate::sensitivity;
use crate::{Progress, Service, ServiceConfig};

/// The detector name for the DSP fallback.
pub const DETECTOR_DSP: &str = "rust-dsp";
/// The chain the app runs (`engine.CHAIN`); the chain of the stored spectrogram features.
pub const CHAIN_KIND: drumspike_core::detect::Chain = drumspike_core::detect::Chain::Mix;
/// The error of a track that a cancel left unanalysed (P3-15b).
pub const CANCELLED_ERROR: &str = "analysis cancelled";

/// R08: the identity of a queued or running job: its generation and its own cancel flag.
/// The worker owns a clone from [`Jobs::queue`]; a later [`Jobs::queue`], [`Jobs::cancel`]
/// or [`Jobs::clear`] never touches an older token, and a generation that is no longer the
/// active one counts as cancelled, so a stale worker writes no state and commits nothing.
#[derive(Clone, Debug)]
pub struct JobToken {
    gen: u64,
    flag: Arc<AtomicBool>,
}

impl JobToken {
    /// The generation of the job (unique per process, higher is newer).
    pub fn gen(&self) -> u64 {
        self.gen
    }

    /// Whether the job was asked to stop. The flag belongs to this generation alone; a
    /// stale generation is stopped by [`Jobs::active`] instead (it is never reused).
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// The job states of all projects.
#[derive(Default)]
pub struct Jobs {
    map: Mutex<std::collections::HashMap<String, Value>>,
    /// R08: the active generation of a project with its owned cancel flag. `queue` mints
    /// the next one, `cancel` sets the active flag, `clear` drops the entry; a token whose
    /// generation is gone (closed or replaced) is never active.
    active: Mutex<std::collections::HashMap<String, JobToken>>,
    /// R08: the next generation (unique per process).
    next: AtomicU64,
    run: Mutex<()>,
    /// P4-3 fix round: the projects with a re-pick store running ([`Jobs::begin_store`]).
    stores: Mutex<std::collections::HashSet<String>>,
    /// P4-3 fix round: the tracks re-picked and stored since their last analysis (`sid`, name):
    /// only the first re-pick writes the backup, so it keeps the hits from before the re-picks.
    repicked: Mutex<std::collections::HashSet<(String, String)>>,
}

/// P4-3 fix round: a re-pick store of a project, from [`Jobs::begin_store`] until it is dropped.
pub struct StoreGuard<'a> {
    jobs: &'a Jobs,
    sid: String,
}

impl Drop for StoreGuard<'_> {
    fn drop(&mut self) {
        lock(&self.jobs.stores).remove(&self.sid);
    }
}

impl Jobs {
    /// `server.job_status`: the job of `sid`, or `{"state": "idle", "tracks": {}}`.
    pub fn status(&self, sid: &str) -> Value {
        lock(&self.map)
            .get(sid)
            .cloned()
            .unwrap_or_else(|| json!({"state": "idle", "tracks": {}}))
    }

    /// P4-3 fix round: whether an analysis of `sid` is queued or running.
    pub fn running(&self, sid: &str) -> bool {
        lock(&self.map)
            .get(sid)
            .is_some_and(|j| j["state"] == "queued" || j["state"] == "running")
    }

    /// P4-3 fix round: whether a re-pick store of `sid` runs.
    pub fn storing(&self, sid: &str) -> bool {
        lock(&self.stores).contains(sid)
    }

    /// P4-3 fix round: mark a re-pick store of `sid` (also taken by `analyze_start` while it
    /// queues, so the two never overlap); `None` when one runs already.
    pub fn begin_store(&self, sid: &str) -> Option<StoreGuard<'_>> {
        lock(&self.stores).insert(sid.to_string()).then(|| StoreGuard {
            jobs: self,
            sid: sid.to_string(),
        })
    }

    /// P4-3 fix round: true when no re-pick of the track was stored since its last analysis
    /// (the next re-pick store writes the backup), false after one was ([`Jobs::repick_stored`]).
    pub fn needs_backup(&self, sid: &str, name: &str) -> bool {
        !lock(&self.repicked).contains(&(sid.to_string(), name.to_string()))
    }

    /// P4-3 fix round 2: a re-pick of the track was stored (after its backup was written), so the
    /// later re-picks keep that backup. Called only after a successful store.
    pub fn repick_stored(&self, sid: &str, name: &str) {
        lock(&self.repicked).insert((sid.to_string(), name.to_string()));
    }

    /// P4-3 fix round: the track was analysed; its next re-pick writes the backup again.
    pub fn analysed(&self, sid: &str, name: &str) {
        lock(&self.repicked).remove(&(sid.to_string(), name.to_string()));
    }

    /// Forget the job of `sid` (the project was closed). A running worker stops at its next
    /// generation check and writes no new state for the closed project (P3-15b, R08).
    pub fn clear(&self, sid: &str) {
        lock(&self.map).remove(sid);
        lock(&self.active).remove(sid);
    }

    /// Set `queued` for every target, synchronously, so a poll never sees an older `done`,
    /// and return the token of the new job: the next generation with its own clear flag. A
    /// start that replaces an older job (only after a close and a reopen, or a direct
    /// `queue`; the command refuses a duplicate start) makes that generation stale: its
    /// worker stops at the next check, and its own flag is not reused.
    pub fn queue(&self, sid: &str, targets: &[String]) -> JobToken {
        let mut tracks = Map::new();
        for t in targets {
            tracks.insert(t.clone(), json!({"state": "queued"}));
        }
        let token = JobToken {
            gen: self.next.fetch_add(1, Ordering::Relaxed),
            flag: Arc::new(AtomicBool::new(false)),
        };
        lock(&self.active).insert(sid.to_string(), token.clone());
        lock(&self.map).insert(sid.to_string(), json!({"state": "queued", "tracks": tracks}));
        token
    }

    /// Ask the queued or running job of `sid` to stop (P3-15b). The flag belongs to the
    /// active generation (R08: no cancel reaches the wrong job); the worker ends after the
    /// track in its model step, the tracks that did not run get the error
    /// [`CANCELLED_ERROR`] and the job is `done`. Returns whether a job was asked.
    pub fn cancel(&self, sid: &str) -> bool {
        let map = lock(&self.map);
        let running = map
            .get(sid)
            .is_some_and(|j| j["state"] == "queued" || j["state"] == "running");
        if !running {
            return false;
        }
        match lock(&self.active).get(sid) {
            Some(t) => {
                t.flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Whether the active job of `sid` was asked to stop, replaced or cleared (no active
    /// generation counts as cancelled: a closed project never keeps running).
    pub fn cancelled(&self, sid: &str) -> bool {
        match lock(&self.active).get(sid) {
            Some(t) => t.is_cancelled(),
            None => true,
        }
    }

    /// R08: whether `token` is still the active generation of `sid` (the project is open
    /// and no newer job replaced it).
    fn is_active(&self, sid: &str, token: &JobToken) -> bool {
        lock(&self.active).get(sid).is_some_and(|t| t.gen == token.gen)
    }

    /// R08: whether the job of `token` may keep running: still the active generation and
    /// not asked to stop. The worker checks this between the tracks (the stop of
    /// `overlap::run`); a stale worker (closed project or replaced generation) stops. The
    /// track already in its model step is refused by the staleness check `is_active` in
    /// `finish_one` alone: a cancel ends the job after that track (P3-15b).
    pub fn active(&self, sid: &str, token: &JobToken) -> bool {
        self.is_active(sid, token) && !token.is_cancelled()
    }

    /// End a cancelled job of the active generation: `remaining` get the error
    /// [`CANCELLED_ERROR`] and the job is `done`. `None` when the generation is stale (the
    /// project was closed or another job started): nothing is written back.
    fn finish_cancelled(&self, sid: &str, token: &JobToken, remaining: &[String]) -> Option<Value> {
        if !self.is_active(sid, token) {
            return None;
        }
        let mut map = lock(&self.map);
        let job = map.get_mut(sid)?;
        for t in remaining {
            job["tracks"][t.as_str()] = json!({"state": "error", "error": CANCELLED_ERROR});
        }
        job["state"] = Value::from("done");
        Some(job.clone())
    }

    /// After a crash of the worker: every track that is neither `done` nor `error` becomes an
    /// error with `message`, and the job is `done`. Returns the new job value, `None` when
    /// the generation is stale (nothing is written for a closed project).
    pub fn fail_unfinished(&self, sid: &str, token: &JobToken, message: &str) -> Option<Value> {
        self.update(sid, token, |job| {
            if let Some(tracks) = job["tracks"].as_object_mut() {
                for t in tracks.values_mut() {
                    if t["state"] != "done" && t["state"] != "error" {
                        *t = json!({"state": "error", "error": message});
                    }
                }
            }
            job["state"] = Value::from("done");
        })
    }

    /// Change the job of `sid` for the generation `token` and return the new value; `None`
    /// when the generation is stale: the worker writes nothing and its caller emits no
    /// progress. This is the fix of R08's "late update resurrects cleared session": a
    /// stale worker no longer re-enters the map after a `clear`.
    fn update(&self, sid: &str, token: &JobToken, f: impl FnOnce(&mut Value)) -> Option<Value> {
        if !self.is_active(sid, token) {
            return None;
        }
        let mut map = lock(&self.map);
        let job = map.get_mut(sid)?;
        f(job);
        Some(job.clone())
    }
}

/// The parameters of a track as `engine._rust_analyze_track` reads them, plus the problem
/// with `thresholds` when it has one (only the model path uses them).
pub struct ParsedParams {
    /// The core parameters.
    pub params: AnalyzeParams,
    /// Why `thresholds` is not five numbers, when it is not.
    pub thresholds_error: Option<String>,
    /// The detector the track names (`params.detector`: a plugin id, `builtin` or empty for
    /// the built-in DSP detector; #387).
    pub detector: Option<String>,
    /// The track's params as they were (the plugin's request params and `timeout_s` come
    /// from here; #387).
    pub raw: serde_json::Map<String, Value>,
}

/// Python's truth value of a JSON value (`bool(x)`).
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn as_float(v: &Value, key: &str) -> Result<f64, String> {
    match v {
        Value::Number(n) => n.as_f64().ok_or_else(|| format!("bad params: {key}")),
        Value::Bool(b) => Ok(f64::from(u8::from(*b))),
        Value::String(s) => s.trim().parse().map_err(|_| format!("bad params: {key}: '{s}'")),
        _ => Err(format!("bad params: {key}")),
    }
}

fn float_or(m: &Map<String, Value>, key: &str, default: f64) -> Result<f64, String> {
    m.get(key).map_or(Ok(default), |v| as_float(v, key))
}

/// `params` of a track document to [`AnalyzeParams`]: `min_gap_ms` (20), `refine` (true),
/// `shift_samples` (0), `vel_lo_db` (-40), `vel_hi_db` (-6), `refine_lo_ms` and
/// `refine_hi_ms` (only with `refine`), `thresholds` (0.22, 0.24, 0.32, 0.22, 0.30), or
/// `sensitivity` (P4-5: the slider value 0 to 150; `thresholds` wins when both are there).
pub fn parse_params(params: Option<&Map<String, Value>>) -> Result<ParsedParams, String> {
    let empty = Map::new();
    let m = params.unwrap_or(&empty);
    let refine = m.get("refine").is_none_or(truthy);
    let window = if refine {
        RefineWindow {
            lo_ms: float_or(m, "refine_lo_ms", RefineWindow::DEFAULT.lo_ms)?,
            hi_ms: float_or(m, "refine_hi_ms", RefineWindow::DEFAULT.hi_ms)?,
        }
    } else {
        RefineWindow::DEFAULT
    };
    let mut p = AnalyzeParams {
        min_gap_ms: float_or(m, "min_gap_ms", 20.0)?,
        refine,
        window,
        shift_samples: match m.get("shift_samples") {
            Some(v) if truthy(v) => as_float(v, "shift_samples")? as i64,
            _ => 0,
        },
        velocity: VelocityMap {
            lo_db: float_or(m, "vel_lo_db", -40.0)?,
            hi_db: float_or(m, "vel_hi_db", -6.0)?,
            ..VelocityMap::default()
        },
        ..AnalyzeParams::default()
    };
    let mut thresholds_error = None;
    if let Some(v) = m.get("thresholds") {
        let list: Result<Vec<f64>, String> = match v.as_array() {
            Some(a) => a.iter().map(|x| as_float(x, "thresholds")).collect(),
            None => Err("bad params: thresholds".to_string()),
        };
        match list.map(|l| <[f64; N_CLASSES]>::try_from(l.as_slice()).map_err(|_| l.len())) {
            Ok(Ok(t)) => p.thresholds = t,
            Ok(Err(n)) => thresholds_error = Some(format!("thresholds: {n} values, expected {N_CLASSES}")),
            Err(e) => thresholds_error = Some(e),
        }
    }
    // P4-5: `sensitivity` (0 to 150, the Inspector slider) without `thresholds` gives the thresholds
    if m.get("thresholds").is_none() {
        if let Some(v) = m.get("sensitivity").and_then(Value::as_f64) {
            p.thresholds = crate::sensitivity::thresholds_of(v);
        }
    }
    // #387: the Detector select of the Inspector. `builtin`, empty and absent select the
    // built-in DSP detector; anything else is a plugin id.
    let detector = match m.get("detector") {
        Some(Value::String(s)) if !s.is_empty() && s != "builtin" => Some(s.clone()),
        _ => None,
    };
    Ok(ParsedParams {
        params: p,
        thresholds_error,
        detector,
        raw: m.clone(),
    })
}

/// What ran: the `detector` record of the document and the job entry. P4-12: `refine` is the
/// version of the refinement rule ([`RefineRule::version`]); a record without it is version 1.
pub(crate) fn detector_record(name: &str, chain: Option<&str>, model: Value, rule: RefineRule) -> Value {
    json!({"detector": name, "chain": chain, "model": model, "refine": rule.version()})
}

/// The `detector` record of an analysis with a detector plugin (#387, the design section 6):
/// the plugin id as the detector name, no chain and no model, the refinement rule and the
/// `plugin` field with the id, the manifest version and the SHA-256 of the accepted licence.
pub(crate) fn detector_plugin_record(id: &str, version: &str, licence_sha256: &str, rule: RefineRule) -> Value {
    json!({
        "detector": id, "chain": Value::Null, "model": Value::Null, "refine": rule.version(),
        "plugin": {"id": id, "version": version, "licence_sha256": licence_sha256},
    })
}

/// P4-12: `DRUMSPIKE_REFINE_RULE` as a rule: `1` (or `reference`), `2` (or `reach`) and `3` (or
/// `first-motion`); `None` for anything else.
pub fn parse_rule(v: &str) -> Option<RefineRule> {
    match v.trim().to_ascii_lowercase().as_str() {
        "reference" => Some(RefineRule::Reference),
        "reach" => Some(RefineRule::Reach),
        "first-motion" => Some(RefineRule::FirstMotion),
        s => s.parse::<u64>().ok().and_then(RefineRule::from_version),
    }
}

/// P4-12: the rule of a new analysis (`analyze_start`): the pin of the configuration, else
/// [`RefineRule::CURRENT`].
pub fn analysis_rule(cfg: &ServiceConfig) -> RefineRule {
    cfg.refine_rule.unwrap_or(RefineRule::CURRENT)
}

/// P4-12: the rule of a re-pick of a stored analysis (`track_repick`): the pin of the
/// configuration, else the version the track's `detector` record carries. A record without a
/// version is an analysis from before P4-12 (version 1), so its hits keep their times until
/// the engineer analyses the track again; a version this build does not know runs the current rule.
pub fn recorded_rule(cfg: &ServiceConfig, detector: Option<&Value>) -> RefineRule {
    if let Some(r) = cfg.refine_rule {
        return r;
    }
    match detector.and_then(|d| d.get("refine")) {
        None | Some(Value::Null) => RefineRule::Reference,
        Some(v) => v
            .as_u64()
            .and_then(RefineRule::from_version)
            .unwrap_or(RefineRule::CURRENT),
    }
}

/// Which detector the slot runs (always the DSP detector since #281 D16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The DSP detector: the preparation is the decode only.
    Dsp,
}

/// A4: the candidate list of a DSP detection, kept for the Sensitivity slider and
/// `track_repick` (the model path keeps activations instead). `params` are the detection's
/// role parameters and `weak_limit` its weak-flux limit; a re-pick re-judges the gates
/// from these.
pub struct DspKept {
    /// Every peak-picked candidate with its gate verdict at the detection's parameters.
    pub cands: Vec<drumspike_core::detect::onset::Candidate>,
    /// The role parameters of the detection.
    pub params: drumspike_core::detect::onset::OnsetParams,
    /// The detection's weak-flux limit (`weak_ratio * template median`).
    pub weak_limit: f32,
}

/// The return of [`DetectorSlot::analyze_kept`]: the analysis, the detector record, the kept
/// candidates of the DSP detection (the model's activations slot stays `None`; the model path
/// lives on the model branch).
pub type KeptAnalysis = Result<(Analysis, Value, Option<Vec<f32>>, Option<DspKept>), AppError>;

/// The DSP detector of the slot (#281 D16: main is model-free; the model path lives on the
/// the model branch).
#[derive(Default)]
pub struct DetectorSlot;

impl DetectorSlot {
    /// The detector the slot runs (P3-15b). The job asks once at its start.
    pub fn kind(&self, _cfg: &ServiceConfig) -> Result<Kind, AppError> {
        Ok(Kind::Dsp)
    }

    /// The DSP detection with its candidate list (A4): the third return carries the kept
    /// candidates for the Sensitivity slider and `track_repick`.
    pub fn analyze_prepared_cands(
        &self,
        _cfg: &ServiceConfig,
        x: &[f32],
        sr: u32,
        role: &str,
        parsed: &ParsedParams,
        _features: Option<&[f32]>,
    ) -> Result<(Analysis, Value, Option<DspKept>), AppError> {
        if let Some(e) = &parsed.thresholds_error {
            return Err(AppError::bad(e.clone()));
        }
        let mut p = parsed.params.clone();
        p.thresholds = AnalyzeParams::default().thresholds;
        p.kick_walk = 0; // the record of a DSP detection carries no kick_walk (version 0)
        if let Some(id) = &parsed.detector {
            return Self::analyze_plugin(x, sr, role, parsed, &mut p, id);
        }

        // A4: one detection pass for the events and the candidate list.
        let (a, cands, params, weak_limit) = drumspike_core::analyze::analyze_track_dsp_cands(x, sr, role, &p)
            .map_err(|e| AppError::bad(e.to_string()))?;
        let kept = DspKept {
            cands,
            params,
            weak_limit,
        };
        Ok((
            a,
            detector_record(DETECTOR_DSP, None, Value::Null, parsed.params.rule),
            Some(kept),
        ))
    }

    /// The detection with a detector plugin (#387, `docs/design/detector-plugins.md`): the
    /// pipeline after step 1 is the same (`analyze_track` with
    /// [`drumspike_core::analyze::Detector::Backend`]); a missing or unsuitable plugin is a
    /// readable track error. No candidate list (the plugin hits enter the pipeline directly).
    fn analyze_plugin(
        x: &[f32],
        sr: u32,
        role: &str,
        parsed: &ParsedParams,
        p: &mut AnalyzeParams,
        id: &str,
    ) -> Result<(Analysis, Value, Option<DspKept>), AppError> {
        let installed = crate::plugins::find(id).ok_or_else(|| {
            AppError::bad(format!(
                "plugin {id} is not installed — install it again or analyze with the built-in detector"
            ))
        })?;
        if !installed.supports_role(role) {
            return Err(AppError::bad(format!(
                "plugin {} does not support the role {role} (it lists {})",
                installed.manifest.id,
                installed.manifest.roles.join(", ")
            )));
        }
        let pass_sr = installed.pass_rate(sr).ok_or_else(|| {
            AppError::bad(format!(
                "plugin {} does not accept {sr} Hz (it lists {})",
                installed.manifest.id,
                installed
                    .manifest
                    .sample_rates
                    .iter()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        let (x, sr) = if pass_sr == sr {
            (x.to_vec(), sr)
        } else {
            let y = drumspike_core::detect::resample::resample_to_44100(x, sr)
                .map_err(|e| AppError::bad(format!("plugin {}: resample failed: {e}", installed.manifest.id)))?;
            (y, pass_sr)
        };
        let backend = drumspike_core::backend::ExternalDetector {
            id: id.to_string(),
            executable: installed.executable.clone(),
            timeout_s: crate::plugins::Installed::timeout_of(&parsed.raw),
            params: crate::plugins::Installed::request_params(&parsed.raw),
        };
        let a = drumspike_core::analyze::analyze_track(
            &x,
            sr,
            role,
            p,
            drumspike_core::analyze::Detector::Backend(&backend),
        )
        .map_err(|e| AppError::bad(e.to_string()))?;
        Ok((
            a,
            detector_plugin_record(
                id,
                &installed.manifest.version,
                &installed.licence_sha256(),
                parsed.params.rule,
            ),
            None,
        ))
    }

    /// [`Self::analyze_prepared_cands`]: the kept candidates are internal to the job path.
    pub fn analyze_prepared(
        &self,
        cfg: &ServiceConfig,
        x: &[f32],
        sr: u32,
        role: &str,
        parsed: &ParsedParams,
        features: Option<&[f32]>,
    ) -> Result<(Analysis, Value), AppError> {
        let (a, record, _) = self.analyze_prepared_cands(cfg, x, sr, role, parsed, features)?;
        Ok((a, record))
    }

    /// The DSP detection with its kept candidates; the activation slot is always `None`
    /// (#281 D16: main is model-free).
    pub fn analyze_kept(
        &self,
        cfg: &ServiceConfig,
        x: &[f32],
        sr: u32,
        role: &str,
        parsed: &ParsedParams,
        features: Option<&[f32]>,
    ) -> KeptAnalysis {
        let (a, record, dsp) = self.analyze_prepared_cands(cfg, x, sr, role, parsed, features)?;
        Ok((a, record, None, dsp))
    }

    /// Always `None` on the model-free main: no model meta (the old documents keep their
    /// recorded `detector.model`).
    pub fn model_meta(&self, _cfg: &ServiceConfig) -> Result<Option<Value>, AppError> {
        Ok(None)
    }
}

/// P3-18 (moved here in P4-3): an analysis without hits runs the retry steps with `run` when the
/// detector is the model; returns the kept analysis and the `weak_signal` note.
fn weak_retry(
    analysis: Analysis,
    thresholds: &[f64; N_CLASSES],
    model: bool,
    run: &mut dyn FnMut(&[f64; N_CLASSES]) -> Result<Analysis, String>,
) -> Result<(Analysis, Option<Value>), String> {
    if !analysis.events.is_empty() {
        return Ok((analysis, None));
    }
    let from = sensitivity::sensitivity_of(thresholds).round();
    let found = if model {
        retry_without_hits(run, thresholds)?
    } else {
        None
    };
    Ok(match found {
        Some((v, th, a)) => (a, Some(json!({"from": from, "to": v, "thresholds": th}))),
        None => (analysis, Some(json!({"from": from, "to": Value::Null}))),
    })
}

/// `json.dump(value, f, indent=1)`.
pub fn to_json_indent1(v: &impl Serialize) -> String {
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    v.serialize(&mut ser).expect("a JSON value serialises");
    String::from_utf8(buf).expect("serde_json writes UTF-8")
}

/// `server.backup_events` (B3): keep the previous events as `Backups/<track>.events.prev.json`.
pub(crate) fn backup_events(op: &crate::registry::OpenProject, name: &str) -> Result<(), AppError> {
    if op.n_events(name) == 0 {
        return Ok(());
    }
    let m = op.track_object(name)?;
    let pick = |k: &str| m.get(k).cloned().unwrap_or(Value::Null);
    let body = json!({"sr": pick("sr"), "duration": pick("duration"), "n_samples": pick("n_samples"), "events": pick("events")});
    let dir = op.project.subdir(BACKUPS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| AppError::internal(format!("{}: {e}", dir.display())))?;
    let path = dir.join(format!("{name}.events.prev.json"));
    std::fs::write(&path, to_json_indent1(&body))
        .map_err(|e| AppError::internal(format!("{}: {e}", path.display())))?;
    Ok(())
}

/// Start the worker for `targets` (already checked); the job is queued before this returns.
pub(crate) fn start(svc: &Service, sid: &str, targets: Vec<String>, progress: Option<Progress>, keep: bool) {
    let token = svc.inner.jobs.queue(sid, &targets);
    let svc = svc.clone();
    let sid = sid.to_string();
    std::thread::spawn(move || {
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_job(&svc, &sid, &targets, &token, progress.as_ref(), keep);
        }));
        if let Err(payload) = run {
            // A panic in the detector must not leave the page polling a job that never ends.
            // The panic text rides in the track error, so a `done` job with a lost track names
            // its cause (R270: the setup then can say why the kept detection is missing).
            let detail = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic without a message".to_string());
            if let Some(job) = svc.inner.jobs.fail_unfinished(
                &sid,
                &token,
                &format!("analysis stopped by an internal error: {detail}"),
            ) {
                if let Some(p) = progress.as_ref() {
                    p(json!({"sid": sid, "job": job}));
                }
            }
        }
    });
}

fn run_job(svc: &Service, sid: &str, targets: &[String], token: &JobToken, progress: Option<&Progress>, keep: bool) {
    let jobs = &svc.inner.jobs;
    // R08: a state write or a progress callback only for the active generation: a stale
    // worker (the project was closed or another job started) emits nothing.
    let emit = |job: Option<Value>| {
        if let (Some(job), Some(p)) = (job, progress) {
            p(json!({"sid": sid, "job": job}));
        }
    };
    let _one_at_a_time = lock(&jobs.run);
    emit(jobs.update(sid, token, |j| j["state"] = Value::from("running")));
    let op = svc.inner.registry.get(sid);
    // The detector is loaded once here (the first track loaded it before); a load error is
    // reported per track by `analyze_prepared`, as before, and the preparation decodes only.
    let kind = svc.inner.detector.kind(&svc.inner.config).ok();
    let prepare = |name: &String| -> Result<Prepared, String> {
        emit(jobs.update(sid, token, |j| {
            j["tracks"][name.as_str()] = json!({"state": "running", "stage": "decoding"})
        }));
        let op = op.as_ref().map_err(|e| e.message.clone())?;
        prepare_one(op, name, kind)
    };
    let mut analysed: Vec<String> = Vec::new(); // P4-14: the tracks the bleed rule may mark
    let mut consume = |name: &String, prepared: Result<Prepared, String>| {
        emit(jobs.update(sid, token, |j| {
            j["tracks"][name.as_str()] = json!({"state": "running", "stage": "analyzing"})
        }));
        let result = prepared.and_then(|p| {
            finish_one(
                svc,
                op.as_ref().expect("prepared only with a project"),
                name,
                p,
                keep,
                sid,
                token,
            )
        });
        jobs.analysed(sid, name); // P4-3 fix round: the next re-pick of this track writes the backup again
        if result.is_ok() {
            analysed.push(name.clone());
        }
        let entry = match result {
            Ok(entry) => entry,
            Err(msg) => {
                let n = msg.chars().count();
                json!({"state": "error", "error": msg.chars().skip(n.saturating_sub(500)).collect::<String>()})
            }
        };
        emit(jobs.update(sid, token, |j| j["tracks"][name.as_str()] = entry));
    };
    let done = overlap::run(targets, prepare, &mut consume, || !jobs.active(sid, token));
    if done < targets.len() {
        emit(jobs.finish_cancelled(sid, token, &targets[done..]));
        return;
    }
    // P4-14: the tom bleed rule over the session's tom close mics, after every track of the job is stored
    if let Ok(op) = op.as_ref() {
        // #281 A6: the cross-track bleed rule first (it re-detects the tom targets; the tom bleed
        // rule judges the hits it leaves)
        let v = crate::cross_bleed::analysis_cross_bleed(&svc.inner.config);
        if v > 0 {
            if let Some(cb) = crate::cross_bleed::run_step(svc, op, &analysed.clone()) {
                emit(jobs.update(sid, token, |j| {
                    if let Some(tracks) = cb["tracks"].as_object() {
                        for (name, out) in tracks {
                            j["tracks"][name.as_str()]["cross_bleed"] = out.clone();
                        }
                    }
                    j["cross_bleed"] = cb.clone();
                }));
            }
        }
        // #281 A1: the snare ghost-note gate's busy clearance next (it re-detects the snare
        // targets with the soft rescue denied to the busy candidates; the flam rule and the
        // snare rescue judge the hits it leaves)
        let v = crate::ghost_gate::analysis_ghost_gate(&svc.inner.config);
        if v > 0 {
            if let Some(gg) = crate::ghost_gate::run_step(svc, op, &analysed.clone()) {
                emit(jobs.update(sid, token, |j| {
                    if let Some(tracks) = gg["tracks"].as_object() {
                        for (name, out) in tracks {
                            j["tracks"][name.as_str()]["ghost_gate"] = out.clone();
                        }
                    }
                    j["ghost_gate"] = gg.clone();
                }));
            }
        }
        let v = crate::tom_bleed::analysis_bleed_rule(&svc.inner.config);
        let targets: Vec<(String, u32)> = analysed.iter().map(|n| (n.clone(), v)).collect();
        if let Some(bleed) = crate::tom_bleed::run_step(svc, op, &targets) {
            emit(jobs.update(sid, token, |j| {
                if let Some(tracks) = bleed["tracks"].as_object() {
                    for (name, removed) in tracks {
                        j["tracks"][name.as_str()]["bleed"] = json!({"rule": bleed["rule"], "on": bleed["on"],
                            "reason": bleed["reason"], "removed": removed});
                    }
                }
                j["bleed"] = bleed.clone();
            }));
        }
        // P4-13 stage B: the flam rule on the analysed snare close mics, after the bleed rules
        let v = crate::flam::analysis_flam_rule(&svc.inner.config);
        let targets: Vec<(String, u32)> = analysed.iter().map(|n| (n.clone(), v)).collect();
        if let Some(flam) = crate::flam::run_step(svc, op, &targets) {
            emit(jobs.update(sid, token, |j| {
                if let Some(tracks) = flam["tracks"].as_object() {
                    for (name, added) in tracks {
                        j["tracks"][name.as_str()]["flam"] = json!({"rule": flam["rule"], "added": added});
                    }
                }
                j["flam"] = flam.clone();
            }));
        }
    }
    emit(jobs.update(sid, token, |j| j["state"] = Value::from("done")));
}

/// A prepared track (P3-15b): the decode and the parameters (the model-free main, #281 D16).
struct Prepared {
    dec: Decoded,
    role: String,
    parsed: ParsedParams,
    /// P3-18: the source this track follows; the model step derives instead of detecting.
    follows: Option<String>,
    /// P4-3: `Analysis/<track>/`, where the model step keeps the activations.
    dir: PathBuf,
    /// P4-3: the source's cache key and the downmix name, for the key of the activations.
    source_key: CacheKey,
    mix: String,
}

/// The retry goes on to the next step while the hit count grows by more than this share
/// (controller decision 2026-10-12: the kick sub of "Session C" gives 42 hits at 95,
/// 336 at 113, 386 at 134 and 386 at 150; the retry stops at 134).
pub const RETRY_GROWTH: f64 = 0.10;

/// P3-18: a track without a hit at its sensitivity. `run` is the model step with other
/// thresholds; the steps of [`sensitivity::retry_steps`] run in order. The first step with a
/// hit is kept; the next step replaces it while its count is more than [`RETRY_GROWTH`]
/// above the kept one; the first step that does not grow that much ends the retry. Returns
/// the kept step (sensitivity, thresholds, analysis), or `None` when every step stays at 0.
pub fn retry_without_hits(
    run: &mut dyn FnMut(&[f64; N_CLASSES]) -> Result<Analysis, String>,
    current: &[f64; N_CLASSES],
) -> Result<Option<(f64, [f64; N_CLASSES], Analysis)>, String> {
    let mut best: Option<(f64, [f64; N_CLASSES], Analysis)> = None;
    for (v, th) in sensitivity::retry_steps(current) {
        let a = run(&th)?;
        match &best {
            None => {
                if !a.events.is_empty() {
                    best = Some((v, th, a));
                }
            }
            Some((_, _, b)) => {
                if a.events.len() as f64 > b.events.len() as f64 * (1.0 + RETRY_GROWTH) {
                    best = Some((v, th, a));
                } else {
                    break;
                }
            }
        }
    }
    Ok(best)
}

/// The preparation of one track (P3-15b): the project's record, the parameters, the decode
/// and downmix, and for the model detector the resample and the features. Runs on the
/// preparation pool. The error is the text the page shows.
fn prepare_one(op: &Shared, name: &str, _kind: Option<Kind>) -> Result<Prepared, String> {
    let (path, role, mix, params, follows, dir, source_key, mix_name) = {
        let g = lock(op);
        if g.offline(name) {
            return Err(format!("track is offline ({}): relink it first", g.status_str(name)));
        }
        let tr = g.track(name).map_err(|e| e.message)?;
        let path = g.source(name).map_err(|e| e.message)?;
        (
            path,
            tr.role.clone().unwrap_or_else(|| "other".into()),
            mix_mode(tr.mix_mode_or_default()),
            tr.params.clone(),
            tr.follows.clone(),
            g.project.subdir(ANALYSIS_DIR).join(name),
            cache_key(tr),
            tr.mix_mode_or_default().to_string(),
        )
    };
    let parsed = parse_params(params.as_ref())?;
    let span = crate::trace::stage("analyze:decode");
    let dec = decode(&path, mix).map_err(|e| e.message)?;
    drop(span);
    Ok(Prepared {
        dec,
        role,
        parsed,
        follows,
        dir,
        source_key,
        mix: mix_name,
    })
}

/// The model step of one track (P3-15b) and the store of its events: the detector on the
/// prepared track, the steps after the detection, the backup of the previous events and the
/// record. Runs on the job thread. Returns the job entry; the error is the text the page shows.
/// R08: a generation that is no longer active (the project was closed or another job started)
/// commits nothing — neither a store nor a follower derive — and answers [`CANCELLED_ERROR`].
/// The cancel flag is not part of this check: it stops the pipeline before the next track,
/// while the track in its model step finishes and commits (P3-15b, `overlap::run`).
fn finish_one(
    svc: &Service,
    op: &Shared,
    name: &str,
    p: Prepared,
    keep: bool,
    sid: &str,
    token: &JobToken,
) -> Result<Value, String> {
    if !svc.inner.jobs.is_active(sid, token) {
        return Err(CANCELLED_ERROR.to_string());
    }
    let Prepared {
        dec,
        role,
        mut parsed,
        follows,
        dir,
        source_key,
        mix,
    } = p;
    // P4-12: a new analysis runs the current refinement rule (or the configuration's pin)
    parsed.params.rule = analysis_rule(&svc.inner.config);
    if let Some(src) = follows {
        // P3-18: the hits of the source at this track's level; no detector runs
        let n = match crate::follow::derive_one(svc, op, name) {
            Ok(n) => n,
            Err(e) => return Err(e.message),
        };
        let still = lock(op).track(name).ok().and_then(|t| t.follows.clone());
        let gone = still.is_none();
        return Ok(
            json!({"state": "done", "n_events": n, "detector": Value::Null, "chain": Value::Null,
            "follows": still.map_or(Value::Null, Value::from), "source_gone": gone, "was": src}),
        );
    }
    let _span = crate::trace::stage("analyze:detect");
    let (analysis, det, _act, dsp) = svc
        .inner
        .detector
        .analyze_kept(&svc.inner.config, &dec.samples, dec.sr, &role, &parsed, None)
        .map_err(|e| e.message)?;
    // #387: a first use of a plugin records its licence acceptance in the project (the
    // design section 5) — the page asks before the select takes effect; a run without the
    // acceptance (the agent surface) records it here, with the same fields.
    if det
        .get("plugin")
        .and_then(|p| p.get("id"))
        .and_then(Value::as_str)
        .is_some()
    {
        let id = det["plugin"]["id"].as_str().unwrap_or_default();
        let recorded = lock(op)
            .project
            .doc
            .extra
            .get("plugin_licences")
            .and_then(Value::as_object)
            .is_some_and(|m| m.contains_key(id));
        if !recorded {
            if let Some(inst) = crate::plugins::find(id) {
                crate::plugins::record_acceptance(&mut lock(op), &inst);
            }
        }
    }
    // P3-18: no hit at this sensitivity: the picking again with lower thresholds; the DSP
    // detection ignores the thresholds (the model path lived on the kept activations, #281 D16)
    let (analysis, weak) = weak_retry(
        analysis,
        &parsed.params.thresholds,
        false,
        &mut |_: &[f64; N_CLASSES]| unreachable!(),
    )?;
    if let Some(dsp) = &dsp {
        // A4: kept for the detection histogram and the re-pick; a failed write costs only those.
        let key = crate::candidates::candidates_key(&source_key, &mix, &det["model"]);
        if let Err(e) = crate::candidates::write_candidates(&dir, &dsp.cands, &dsp.params, dsp.weak_limit, &key) {
            eprintln!("candidates not kept for {name}: {e}");
        }
    }
    // #293: a track with a Learn keeps its Learn marks through the analysis (the job owns the track)
    let learned = match &dsp {
        Some(dsp) => {
            let edited = if keep {
                crate::learn_filter::edited_times(&track_events(op, name))
            } else {
                Vec::new()
            };
            let key = json!({"source": source_key, "mix": mix});
            crate::learn_filter::stored_learn_events(
                op,
                name,
                &dec.samples,
                dec.sr,
                &role,
                (&dsp.cands, &dsp.params, dsp.weak_limit),
                &parsed.params,
                &edited,
                &dir,
                &key,
            )?
        }
        None => None,
    };
    store_analysis(svc, op, name, (analysis, weak), &det, &dec, true, keep, learned)
}

/// The store of an analysis of `name` (the job's model step and P4-3's `track_repick`): the
/// backup of the previous events (`backup`: the job always, a re-pick only the first since the
/// analysis), the events, the rate and length, the detector record, the params the retry changed
/// and the `weak_signal` note (`picked`: the analysis and the note of [`pick_kept`]), then the
/// followers. With `keep` (P4-5 step 2b) the edited hits of the track's current events stay and win
/// within 20 ms over the picked ones ([`crate::edit::merge_edited`]). `learned` (#293): the hits of a track
/// with a Learn ([`crate::learn_filter::stored_learn_events`]), flags included, in place of the analysis's
/// hits. Returns the job entry.
#[allow(clippy::too_many_arguments)]
pub(crate) fn store_analysis(
    svc: &Service,
    op: &Shared,
    name: &str,
    picked: (Analysis, Option<Value>),
    det: &Value,
    dec: &Decoded,
    backup: bool,
    keep: bool,
    learned: Option<Vec<Value>>,
) -> Result<Value, String> {
    let (analysis, weak) = picked;
    let mut events: Vec<Event> = match learned {
        Some(vals) => vals
            .into_iter()
            .filter_map(|v| serde_json::from_value::<Event>(v).ok())
            .collect(),
        None => analysis
            .events
            .iter()
            .map(|h| Event {
                rise: h.rise,
                ..Event::new(h.t, h.v)
            })
            .collect(),
    };
    if keep {
        let current = track_events(op, name);
        let picked: Vec<Value> = events.iter().filter_map(|e| serde_json::to_value(e).ok()).collect();
        let merged = crate::edit::merge_edited(picked, &current);
        events = merged
            .into_iter()
            .filter_map(|v| serde_json::from_value::<Event>(v).ok())
            .collect();
    }
    // P4-3: the loudness cut of the track (`vel_cut_db`, off unless the engineer set it) marks its quiet hits
    let cut = lock(op)
        .track(name)
        .ok()
        .and_then(|t| crate::histogram::cut_of(t.params.as_ref()));
    crate::histogram::mark_cut_events(&mut events, &dec.samples, dec.sr, cut);
    let n_events = events.len();
    {
        let mut g = lock(op);
        if backup {
            backup_events(&g, name).map_err(|e| e.message)?;
        }
        let tr = g.track_mut(name).map_err(|e| e.message)?;
        tr.events = Some(events);
        tr.extra.remove("events");
        tr.sr = Some(i64::from(dec.sr));
        tr.duration = Some(dec.samples.len() as f64 / f64::from(dec.sr));
        tr.n_samples = Some(dec.samples.len() as i64);
        tr.extra.insert("detector".into(), det.clone());
        // P3-18: the sensitivity the retry found goes into the params (the next analysis
        // starts there); a stale weak-signal note goes when the track has hits again
        match &weak {
            Some(w) if !w["to"].is_null() => {
                let params = tr.params.get_or_insert_with(Map::new);
                params.insert("thresholds".into(), w["thresholds"].clone());
                params.insert("sensitivity".into(), w["to"].clone());
                params.insert("weak_signal".into(), w.clone());
            }
            Some(w) => {
                tr.params
                    .get_or_insert_with(Map::new)
                    .insert("weak_signal".into(), w.clone());
            }
            None => {
                // a track without params keeps none (the answer shapes of tracks_get stay)
                if let Some(params) = tr.params.as_mut() {
                    params.shift_remove("weak_signal");
                }
            }
        }
        g.mark_dirty();
    }
    let mut entry = json!({"state": "done", "n_events": n_events, "detector": det["detector"], "chain": det["chain"]});
    if let Some(n) = det["model"].get("note").filter(|n| !n.is_null()) {
        entry["note"] = n.clone();
    }
    if let Some(w) = weak {
        entry["weak_signal"] = w;
    }
    // P3-18: the followers of this track take the new hits
    let rows = crate::follow::refresh_followers(svc, op, name);
    if !rows.is_empty() {
        entry["followers"] = Value::Array(crate::follow::refreshed_names(&rows));
    }
    Ok(entry)
}

/// The events the track holds now (the edited hits a store keeps; none for a track without events).
pub(crate) fn track_events(op: &Shared, name: &str) -> Vec<Value> {
    lock(op)
        .track_object(name)
        .ok()
        .and_then(|m| m.get("events").and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
