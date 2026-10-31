//! P4-7a and P4-7b: the one-shot layer of a track. The engineer gives a track one one-shot sample
//! (WAV or AIFF, any rate) and hears it at every hit with the drum, in the app, and renders it as a
//! WAV that lines up when it is imported at the session start.
//!
//! | Command | Effect |
//! |---|---|
//! | `track_layer_set {sid, name, path?, level_db?, polarity?, on?, curve?, align?, onset?}` | set, change or (`path: null` or `""`) remove the layer of a track; a new `path` reads the file and measures its onset; `onset` (samples) overrides the measured one, `onset: null` resets to it (P4-40e) |
//! | `track_layer_get {sid, name}` | the layer, the state of its file and its onset |
//! | `track_layer_align {sid, name}` | P4-7b: per hit the alignment of the one-shot to the close mic (lag, match, sign) and the track's polarity |
//! | `export_layer {sid, name}` | write `Exports/<name>_layer.wav` |
//!
//! # The layer object
//!
//! The track's `layer` key (an optional field; the format stays 1): `source` (a source record like a
//! track's: absolute path, project-relative path, size, time, head hash; the file resolves by the
//! same order, so a moved file is found again in the project's `Audio Files/`), `level_db` (-60 to
//! +12), `polarity` (`normal`, `invert`, `auto`: the majority of the alignment), `on` (playback),
//! `curve` (`{kind: "db", range_db}` default 24 dB, `linear`, `fixed`), `align` (P4-7b: the render and
//! the playback use the per-hit alignment), `onset` (the one-shot's own measured onset, kept as
//! measured), `onset_set` (P4-40e: the engineer's onset override, absent when there is none; the
//! render, the playback and the Align place the one-shot by it), `sr`, `channels`, `frames` (of
//! the one-shot file).
//!
//! # Placement
//!
//! A hit at `t` puts the one-shot's onset (`drumspike_core::layer::one_shot_onset`, the refinement
//! rule of the analysis) on the hit's sample `round(t * sr)`, not the file start. Each hit's gain is
//! the level, the polarity and the velocity curve of its `v` (100 when absent). Cut and bleed hits
//! (P4-3, P4-14) have no layer, as they have no click. With `align` (P4-7b), the onset goes to the
//! hit's sample plus the lag of the cross-correlation with the close mic (sub-sample in the render,
//! by a fractional delay; the nearest device frame in the playback).
//!
//! # Render
//!
//! `Exports/<name>_layer.wav`: the track's source rate and length, the one-shot's channels (1 or 2),
//! IEEE float 32 (overlapping voices may pass full scale; the file keeps them), and the source's
//! `bext` time reference, so the file sits at the session position when a DAW imports it at the
//! session start. The one-shot is resampled to the track's rate once for the render (and to the
//! device rate for the playback, `transport.rs`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use drumspike_core::io::{Bext, MappedWav, WavFormat, WavSpec, write_wav};
use drumspike_core::layer::{
    AlignParams, Curve, HitXcorr, OneShot, attack_template, majority_sign, read_one_shot, render_layer, xcorr_hit,
};
use drumspike_core::project::{DOC_EXT, EXPORTS_DIR, Source, SourceStatus, build_source, resolve_source};
use drumspike_core::spike::hit_index;
use serde_json::{Map, Value, json};

use crate::Service;
use crate::error::AppError;
use crate::export::check_file_owner;
use crate::media::mix_mode;
use crate::registry::{OpenProject, lock};

/// The commands of the layer.
pub const LAYER_COMMANDS: [&str; 4] = [
    "track_layer_set",
    "track_layer_get",
    "track_layer_align",
    "export_layer",
];

/// The lowest layer level in dB.
pub const LEVEL_MIN_DB: f64 = -60.0;
/// The highest layer level in dB.
pub const LEVEL_MAX_DB: f64 = 12.0;
/// A hit whose match is under this is uncertain (P4-7b: the later review queue reads it).
pub const UNCERTAIN_MATCH: f64 = 0.5;
/// The `bext` description of a layer file.
pub const BEXT_DESCRIPTION: &str = "Drum Spike layer";
/// One-shots kept decoded (by path, size, time and rate).
const SHOT_CACHE_LEN: usize = 16;

/// The voices of a layer for the transport: per hit the time of the one-shot's first sample and the gain.
pub type Voices = Vec<(f64, f32)>;

/// The polarity of a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    /// The one-shot as it is.
    Normal,
    /// The one-shot inverted.
    Invert,
    /// P4-7b: the majority of the per-hit alignment (as is without an alignment).
    Auto,
}

impl Polarity {
    /// The name in the document.
    pub fn as_str(self) -> &'static str {
        match self {
            Polarity::Normal => "normal",
            Polarity::Invert => "invert",
            Polarity::Auto => "auto",
        }
    }

    /// The polarity of a name; `None` for another text.
    pub fn parse(s: &str) -> Option<Polarity> {
        match s {
            "normal" => Some(Polarity::Normal),
            "invert" => Some(Polarity::Invert),
            "auto" => Some(Polarity::Auto),
            _ => None,
        }
    }
}

/// The layer of a track (the `layer` object, see the module docs).
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// The one-shot file.
    pub source: Source,
    /// Level in dB.
    pub level_db: f64,
    /// Polarity.
    pub polarity: Polarity,
    /// Heard in the playback.
    pub on: bool,
    /// Gain from velocity.
    pub curve: Curve,
    /// P4-7b: the per-hit alignment is used.
    pub align: bool,
    /// The one-shot's measured onset in its own samples (P4-40e: kept as measured).
    pub onset: usize,
    /// P4-40e: the engineer's onset override in the one-shot's own samples; the render, the
    /// playback and the Align place the one-shot by it, `Reset` clears it back to `onset`.
    pub onset_set: Option<usize>,
    /// The one-shot's rate.
    pub sr: u32,
    /// The one-shot's channels (1 or 2).
    pub channels: usize,
    /// The one-shot's frames.
    pub frames: usize,
}

impl Layer {
    /// The layer of a JSON object; `None` when it is not one (a missing key takes its default; the
    /// source record and the onset facts are required).
    pub fn from_json(v: &Value) -> Option<Layer> {
        let m = v.as_object()?;
        let source: Source = serde_json::from_value(m.get("source")?.clone()).ok()?;
        let num = |k: &str| m.get(k).and_then(Value::as_u64);
        Some(Layer {
            source,
            level_db: m.get("level_db").and_then(Value::as_f64).unwrap_or(0.0),
            polarity: m
                .get("polarity")
                .and_then(Value::as_str)
                .and_then(Polarity::parse)
                .unwrap_or(Polarity::Normal),
            on: m.get("on").and_then(Value::as_bool).unwrap_or(true),
            curve: m.get("curve").and_then(Curve::from_json).unwrap_or_default(),
            align: m.get("align").and_then(Value::as_bool).unwrap_or(false),
            onset: num("onset")? as usize,
            onset_set: m.get("onset_set").and_then(Value::as_u64).map(|x| x as usize),
            sr: u32::try_from(num("sr")?).ok().filter(|r| *r > 0)?,
            channels: num("channels").unwrap_or(1).clamp(1, 2) as usize,
            frames: num("frames").unwrap_or(0) as usize,
        })
    }

    /// The JSON object of the layer.
    pub fn to_json(&self) -> Value {
        json!({
            "source": serde_json::to_value(&self.source).unwrap_or(Value::Null),
            "level_db": self.level_db,
            "polarity": self.polarity.as_str(),
            "on": self.on,
            "curve": self.curve.to_json(),
            "align": self.align,
            "onset": self.onset,
            "onset_set": self.onset_set,
            "sr": self.sr,
            "channels": self.channels,
            "frames": self.frames,
        })
    }

    /// The onset the layer uses: the override, else the measured one (P4-40e).
    pub fn onset_eff(&self) -> usize {
        self.onset_set.unwrap_or(self.onset)
    }

    /// The onset in seconds (of the effective onset, P4-40e).
    pub fn onset_s(&self) -> f64 {
        self.onset_eff() as f64 / self.sr as f64
    }

    /// The onset at `sr` in whole samples (the one-shot resampled to `sr` keeps it there).
    pub fn onset_at(&self, sr: u32) -> usize {
        self.onset_at_of(self.onset_eff(), sr)
    }

    /// `onset` at `sr` in whole samples, for a given onset in the one-shot's own samples.
    fn onset_at_of(&self, onset: usize, sr: u32) -> usize {
        if sr == self.sr {
            onset
        } else {
            (onset as f64 * sr as f64 / self.sr as f64).round() as usize
        }
    }

    /// The onset at `sr` in samples, not rounded: the render places the one-shot resampled to `sr`
    /// with its attack at this fractional sample (P4-7a fix 7).
    pub fn onset_exact_at(&self, sr: u32) -> f64 {
        self.onset_eff() as f64 * sr as f64 / self.sr as f64
    }

    /// The gain of a hit with velocity `v` and polarity `sign` (+1 or -1).
    pub fn gain(&self, v: i64, sign: i8) -> f32 {
        let level = 10f64.powf(self.level_db.clamp(LEVEL_MIN_DB, LEVEL_MAX_DB) / 20.0);
        (sign as f64 * level * self.curve.gain(v)) as f32
    }
}

/// The layer of a track's extra keys, `None` when absent or not a layer.
pub fn layer_of(extra: &Map<String, Value>) -> Option<Layer> {
    extra.get("layer").and_then(Layer::from_json)
}

/// The decoded one-shots and the per-hit alignments of the service.
#[derive(Default)]
pub struct LayerState {
    /// (path, size, time, rate) to the one-shot at that rate.
    shots: Mutex<Vec<(ShotKey, Arc<OneShot>)>>,
    /// (sid, track) to the alignment of its hits (P4-7b).
    align: Mutex<HashMap<(String, String), AlignCache>>,
}

type ShotKey = (PathBuf, u64, Option<std::time::SystemTime>, u32);

/// The alignments of one track's hits for one layer file and one mic: the hit sample to its result.
struct AlignCache {
    key: String,
    hits: HashMap<i64, Option<HitXcorr>>,
}

/// One hit of a layer: time, sample of the track, velocity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerHit {
    /// Seconds.
    pub t: f64,
    /// Velocity.
    pub v: i64,
}

/// What a track's layer needs from the project, taken under the lock.
#[derive(Debug, Clone)]
pub(crate) struct LayerJob {
    pub layer: Layer,
    /// The one-shot file, `None` when it is missing.
    pub file: Option<PathBuf>,
    pub status: SourceStatus,
    /// The close mic (the track's source), `None` when offline.
    pub mic: Option<PathBuf>,
    pub mode: drumspike_core::MixMode,
    /// The track's role: the alignment search range comes from it.
    pub role: String,
    /// The uncut hits of the document.
    pub hits: Vec<LayerHit>,
}

/// The uncut hits of a track with their velocities.
fn doc_hits(g: &OpenProject, name: &str) -> Result<Vec<LayerHit>, AppError> {
    Ok(g.track(name)?
        .events()
        .iter()
        .filter(|e| !crate::histogram::event_cut(e))
        .map(|e| LayerHit {
            t: e.t,
            v: e.v.unwrap_or(100),
        })
        .collect())
}

/// The layer job of a track; `None` when it has no layer.
pub(crate) fn layer_job(g: &OpenProject, name: &str) -> Result<Option<LayerJob>, AppError> {
    let tr = g.track(name)?;
    let Some(layer) = layer_of(&tr.extra) else {
        return Ok(None);
    };
    let (file, status) = resolve_source(Some(&layer.source), &g.project.dir);
    Ok(Some(LayerJob {
        file,
        status,
        mic: g.source(name).ok(),
        mode: mix_mode(tr.mix_mode_or_default()),
        role: tr.role.clone().unwrap_or_else(|| "other".into()),
        hits: doc_hits(g, name)?,
        layer,
    }))
}

/// The files that a save must never write over (P4-7d): the source audio of every track and the one-shot
/// file of every layer. A file that is offline or missing from the record is not listed.
pub(crate) fn protected_files(g: &OpenProject) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for name in g.project.doc.tracks.iter().flat_map(|t| t.keys()) {
        out.extend(g.source(name).ok());
        if let Ok(Some(job)) = layer_job(g, name) {
            out.extend(job.file);
        }
    }
    out
}

/// The 400 for a render target that is a folder (and was not a recoverable leftover).
fn folder_error(path: &Path) -> AppError {
    AppError::bad(format!(
        "the path is a folder: {}: the render writes a file there; remove the folder or choose another file",
        path.display()
    ))
}

/// A file that a render or a calibration save writes (P4-7d): an absolute path in an existing folder, not a
/// folder, and not one of the `protected` files. 400 for a relative path or a folder, 404 for a missing
/// folder, 409 for a protected file.
pub(crate) fn save_target(path: &str, protected: &[PathBuf]) -> Result<PathBuf, AppError> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(AppError::bad(format!("the file must be an absolute path: {path}")));
    }
    if !p.parent().is_some_and(Path::is_dir) {
        return Err(AppError::not_found(format!("the folder does not exist: {path}")));
    }
    if p.is_dir() {
        return Err(folder_error(p));
    }
    let canon = |f: &Path| crate::export::path_key(f);
    if protected.iter().any(|f| canon(f) == canon(p)) {
        return Err(AppError::conflict(format!(
            "{path} is a track's source audio or a one-shot in use: choose another file"
        )));
    }
    Ok(p.to_path_buf())
}

/// An optional string argument: absent or null is `None`; a number or a list is a 400.
pub(crate) fn opt_path<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, AppError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(AppError::bad(format!("`{key}` must be a string"))),
    }
}

/// The onset to use for a one-shot: the stored one, or re-measured when the layer file changed
/// under the layer record (the stored onset no longer matches the file's attack).
fn job_onset(job: &LayerJob, shot: &OneShot) -> usize {
    if job.status == SourceStatus::Changed {
        drumspike_core::layer::one_shot_onset(&shot.mono(), shot.sr)
    } else {
        job.layer.onset_at(shot.sr)
    }
}

/// [`job_onset`] without the rounding: the render puts the sample at the exact (fractional) onset of
/// the resampled one-shot, so a one-shot at another rate than the track is sub-sample exact.
fn job_onset_exact(job: &LayerJob, shot: &OneShot) -> f64 {
    if job.status == SourceStatus::Changed {
        job_onset(job, shot) as f64
    } else {
        job.layer.onset_exact_at(shot.sr)
    }
}

/// The alignment parameters of a track's role: the kick's attack is longer, so its search window is wider.
fn align_params(role: &str) -> AlignParams {
    AlignParams {
        max_lag_s: if role == "kick" { 0.005 } else { 0.003 },
        ..AlignParams::default()
    }
}

/// The time key of an alignment cache: everything the per-hit results depend on.
fn align_key(file: &Path, layer: &Layer, mic: &Path, p: &AlignParams) -> String {
    let meta = |f: &Path| std::fs::metadata(f).map(|m| (m.len(), m.modified().ok())).ok();
    format!(
        "{}|{:?}|{}|{}|{:?}|{:?}",
        file.display(),
        meta(file),
        layer.onset_eff(), // P4-40e: the override moves the template
        mic.display(),
        meta(mic),
        p
    )
}

/// The lag (samples) the render, the playback and the MIDI export apply for one hit and polarity `sign`: the
/// lag of the signed correlation for that sign (the search maximises the correlation of the one-shot as it
/// plays, not its absolute value), 0 when it sits on the search edge (the true lag may be outside the window). A
/// lag with a low match ([`UNCERTAIN_MATCH`]) is applied and only flagged uncertain.
pub fn applied_lag(x: &HitXcorr, sign: i8) -> f64 {
    let (lag, _, edge) = x.for_sign_edge(sign);
    if edge { 0.0 } else { lag }
}

/// The per-hit alignment of a track (P4-7b).
#[derive(Debug, Clone, PartialEq)]
pub struct Alignment {
    /// Per hit (in the order given) the cross-correlation, `None` for a silent window.
    pub hits: Vec<Option<HitXcorr>>,
    /// The majority sign of the hits (+1 as is, -1 inverted) and the counts of each.
    pub majority: (i8, usize, usize),
    /// The mic's rate (the lags are samples at this rate).
    pub sr: u32,
}

impl Alignment {
    /// The sign the layer uses: its polarity, or the majority for `auto`.
    pub fn sign_for(&self, p: Polarity) -> i8 {
        match p {
            Polarity::Normal => 1,
            Polarity::Invert => -1,
            Polarity::Auto => self.majority.0,
        }
    }
}

impl Service {
    /// A one-shot at `sr` (its own rate when `None`), from the cache when the file is unchanged.
    pub(crate) fn layer_shot(&self, path: &Path, sr: Option<u32>) -> Result<Arc<OneShot>, AppError> {
        let meta = std::fs::metadata(path).map_err(|e| AppError::conflict(format!("{}: {e}", path.display())))?;
        // rate 0 in the key: the file's own rate
        let key: ShotKey = (path.to_path_buf(), meta.len(), meta.modified().ok(), sr.unwrap_or(0));
        let cache = &self.inner.layers.shots;
        if let Some((_, s)) = lock(cache).iter().find(|(k, _)| *k == key) {
            return Ok(Arc::clone(s));
        }
        let shot = read_one_shot(path).map_err(AppError::bad)?;
        let rate = sr.unwrap_or(shot.sr);
        let out = Arc::new(shot.resampled(rate));
        let mut c = lock(cache);
        c.retain(|(k, _)| *k != key);
        c.push((key, Arc::clone(&out)));
        if c.len() > SHOT_CACHE_LEN {
            c.remove(0);
        }
        Ok(out)
    }

    /// The alignment of `hits` (P4-7b): the one-shot's attack against the mic around each hit. The
    /// results are kept per hit sample, so after an edit only the new hits are computed. 409 when the
    /// track's audio or the layer file is missing.
    pub(crate) fn layer_alignment(
        &self,
        sid: &str,
        name: &str,
        job: &LayerJob,
        hits: &[LayerHit],
    ) -> Result<Alignment, AppError> {
        let file = job
            .file
            .as_ref()
            .ok_or_else(|| AppError::conflict(format!("the layer file of track {name} is missing")))?;
        let mic = job
            .mic
            .as_ref()
            .ok_or_else(|| AppError::conflict(format!("track {name} is offline: relink it first")))?;
        let p = align_params(&job.role);
        let decoded = self.inner.media.decode_cached(mic, job.mode)?;
        let sr = decoded.sr;
        let key = align_key(file, &job.layer, mic, &p);
        let idx: Vec<i64> = hits
            .iter()
            .map(|h| drumspike_core::pyops::py_round(h.t * sr as f64))
            .collect();
        let mut missing: Vec<i64> = {
            let c = lock(&self.inner.layers.align);
            match c.get(&(sid.to_string(), name.to_string())) {
                Some(a) if a.key == key => idx.iter().copied().filter(|i| !a.hits.contains_key(i)).collect(),
                _ => idx.clone(),
            }
        };
        missing.sort_unstable();
        missing.dedup();
        let mut fresh = HashMap::new();
        if !missing.is_empty() {
            let shot = self.layer_shot(file, Some(sr))?;
            let mono = shot.mono();
            let (a, pre) = attack_template(&mono, job_onset(job, &shot), sr, &p);
            let max_lag = (p.max_lag_s * sr as f64).round() as usize;
            for i in missing {
                fresh.insert(i, xcorr_hit(&decoded.samples, i, &a, pre, max_lag));
            }
        }
        let mut c = lock(&self.inner.layers.align);
        let entry = c
            .entry((sid.to_string(), name.to_string()))
            .or_insert_with(|| AlignCache {
                key: key.clone(),
                hits: HashMap::new(),
            });
        if entry.key != key {
            entry.key = key;
            entry.hits.clear();
        }
        entry.hits.extend(fresh);
        let out: Vec<Option<HitXcorr>> = idx.iter().map(|i| entry.hits.get(i).copied().flatten()).collect();
        let found: Vec<HitXcorr> = out.iter().flatten().copied().collect();
        Ok(Alignment {
            majority: majority_sign(&found),
            hits: out,
            sr,
        })
    }

    /// The layer's voices for the transport: per hit the time of the one-shot's first sample and its
    /// gain, and the one-shot. `None` when the track has no layer that is on and whose file is there.
    /// With `align` the alignment is used (a failure leaves the hits on their samples).
    pub(crate) fn layer_voices(
        &self,
        sid: &str,
        name: &str,
        job: &LayerJob,
        hits: &[LayerHit],
    ) -> Option<(Arc<OneShot>, Voices)> {
        if !job.layer.on {
            return None;
        }
        let file = job.file.as_ref()?;
        let shot = self.layer_shot(file, None).ok()?;
        let al = if job.layer.align || job.layer.polarity == Polarity::Auto {
            self.layer_alignment(sid, name, job, hits).ok()
        } else {
            None
        };
        let sign = al
            .as_ref()
            .map_or(if job.layer.polarity == Polarity::Invert { -1 } else { 1 }, |a| {
                a.sign_for(job.layer.polarity)
            });
        let onset_s = if job.status == SourceStatus::Changed {
            // the file changed under the layer record: re-measure (samples at the shot's rate)
            job_onset(job, &shot) as f64 / shot.sr as f64
        } else {
            job.layer.onset_s()
        };
        let starts = hits
            .iter()
            .enumerate()
            .map(|(k, h)| {
                let lag_s = match (&al, job.layer.align) {
                    // an edge lag is uncertain: do not shift (the true lag may be outside the window)
                    (Some(a), true) => a.hits[k].map_or(0.0, |x| applied_lag(&x, sign) / a.sr as f64),
                    _ => 0.0,
                };
                (h.t + lag_s - onset_s, job.layer.gain(h.v, sign))
            })
            .collect();
        Some((shot, starts))
    }

    /// `track_layer_get {sid, name}`: `{layer, status, path, file, onset_ms}`; `layer` is `null` (and
    /// the rest too) for a track without a layer.
    pub fn track_layer_get(&self, sid: &str, name: &str) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        let g = lock(&op);
        let job = layer_job(&g, name)?;
        Ok(get_json(job.as_ref()))
    }

    /// `track_layer_wave {sid, name, start?, end?}` (P4-40e): the mono samples of the one-shot
    /// `[start, end)` in its own samples — the Layer view's waveform. The bytes of
    /// `samples_window`: `u32 start`, `u32 rate`, then f32. The window is clamped to the file and
    /// capped at [`MAX_WINDOW_SAMPLES`]. 409 without a layer or its file.
    pub fn track_layer_wave(&self, sid: &str, name: &str, start: i64, end: i64) -> Result<Vec<u8>, AppError> {
        let op = self.inner.registry.get(sid)?;
        let file = {
            let g = lock(&op);
            layer_job(&g, name)?
                .ok_or_else(|| AppError::conflict(format!("track {name} has no layer")))?
                .file
        };
        let file = file.ok_or_else(|| AppError::conflict(format!("the layer file of track {name} is missing")))?;
        let shot = self.layer_shot(&file, None)?;
        Ok(crate::media::wave_bytes(&shot, start, end))
    }

    /// `track_layer_set {sid, name, path?, level_db?, polarity?, on?, curve?, align?, onset?}`: a `path` (WAV or
    /// AIFF) sets the one-shot (read, onset measured, a source record kept; the other settings stay
    /// when a layer was there, else they take their defaults), `path: null` or `""` removes the layer; the
    /// other keys change one setting each. `onset` (P4-40e, samples of the one-shot) overrides the
    /// measured onset for the render, the playback and the Align; `onset: null` resets to the measured
    /// one; a new `path` clears an override (an `onset` of the same call sets a fresh one). Marks the
    /// project dirty and updates an open transport of the project. Answers as
    /// [`Service::track_layer_get`]. 400 for a file the app cannot read or a wrong value; 409 for a
    /// setting on a track without a layer.
    pub fn track_layer_set(&self, sid: &str, name: &str, args: &Value) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let dir = {
            let g = lock(&op);
            g.track(name)?;
            g.project.dir.clone()
        };
        // a new file is read without the project lock
        let new_file = match args.get("path") {
            None => None,
            Some(Value::Null) => Some(None),
            // an empty text removes it too (the Tauri wrapper cannot tell `null` from a missing key)
            Some(Value::String(p)) if p.trim().is_empty() => Some(None),
            Some(Value::String(p)) => {
                let path = PathBuf::from(p);
                let shot = read_one_shot(&path).map_err(AppError::bad)?;
                let onset = drumspike_core::layer::one_shot_onset(&shot.mono(), shot.sr);
                let source = build_source(&path, &dir, "referenced").map_err(|e| AppError::bad(e.to_string()))?;
                Some(Some((source, shot.sr, shot.channels(), shot.frames(), onset)))
            }
            Some(_) => return Err(AppError::bad("`path` must be a file path or null")),
        };
        let level = match args.get("level_db") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_f64()
                    .filter(|x| x.is_finite())
                    .ok_or_else(|| AppError::bad("`level_db` must be a number"))?
                    .clamp(LEVEL_MIN_DB, LEVEL_MAX_DB),
            ),
        };
        let polarity = match args.get("polarity") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_str()
                    .and_then(Polarity::parse)
                    .ok_or_else(|| AppError::bad("`polarity` must be normal, invert or auto"))?,
            ),
        };
        let curve = match args.get("curve") {
            None | Some(Value::Null) => None,
            Some(v) => Some(Curve::from_json(v).ok_or_else(|| {
                AppError::bad("`curve` must be {kind: db, range_db 0 to 96}, {kind: linear} or {kind: fixed}")
            })?),
        };
        let flag = |k: &str| -> Result<Option<bool>, AppError> {
            match args.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v
                    .as_bool()
                    .map(Some)
                    .ok_or_else(|| AppError::bad(format!("`{k}` must be true or false"))),
            }
        };
        let (on, align) = (flag("on")?, flag("align")?);
        // P4-40e: `onset` (samples of the one-shot) overrides the measured one; `null` resets to
        // the measured onset (a missing key changes nothing). `frames` bounds it; the frames of a
        // file set in the same call are known only under the lock, so the check happens there.
        let onset_arg = match args.get("onset") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(v) => Some(Some(v.as_u64().ok_or_else(|| {
                AppError::bad("`onset` must be a number of samples (or null to reset)")
            })?)),
        };
        {
            let mut g = lock(&op);
            let tr = g.track_mut(name)?;
            let old = layer_of(&tr.extra);
            let mut layer = match (new_file, old) {
                (Some(None), _) => {
                    tr.extra.shift_remove("layer");
                    g.mark_dirty();
                    g.emit_layer(name);
                    drop(g);
                    let pushed = self.transport_refresh_layers(sid);
                    return self.layer_answer(sid, name, pushed);
                }
                (Some(Some((source, sr, channels, frames, onset))), old) => {
                    let base = old.unwrap_or(Layer {
                        source: source.clone(),
                        level_db: 0.0,
                        polarity: Polarity::Normal,
                        on: true,
                        curve: Curve::default(),
                        align: false,
                        onset,
                        onset_set: None,
                        sr,
                        channels,
                        frames,
                    });
                    // a new file re-measures: the override of the old one-shot does not survive it
                    // (an `onset` of the same call sets a fresh one below)
                    Layer {
                        source,
                        sr,
                        channels,
                        frames,
                        onset,
                        onset_set: None,
                        ..base
                    }
                }
                (None, Some(l)) => l,
                (None, None) => {
                    return Err(AppError::conflict(format!(
                        "track {name} has no layer: give a one-shot file (`path`) first"
                    )));
                }
            };
            if let Some(v) = level {
                layer.level_db = v;
            }
            if let Some(v) = polarity {
                layer.polarity = v;
            }
            if let Some(v) = curve {
                layer.curve = v;
            }
            if let Some(v) = on {
                layer.on = v;
            }
            if let Some(v) = align {
                layer.align = v;
            }
            if let Some(set) = onset_arg {
                match set {
                    // P4-40e: the Reset goes back to the measured onset
                    None => layer.onset_set = None,
                    Some(n) => {
                        if n >= layer.frames as u64 {
                            return Err(AppError::bad(format!(
                                "`onset` {n} is at or after the end of the one-shot ({} frames)",
                                layer.frames
                            )));
                        }
                        layer.onset_set = Some(n as usize);
                    }
                }
            }
            tr.extra.insert("layer".into(), layer.to_json());
            g.mark_dirty();
            g.emit_layer(name);
        }
        let pushed = self.transport_refresh_layers(sid);
        self.layer_answer(sid, name, pushed)
    }

    /// The answer of `track_layer_set`: `track_layer_get`, plus `transport_error` (text) when the
    /// open transport did not take the new layers (the setting is stored; the page shows the text).
    fn layer_answer(&self, sid: &str, name: &str, pushed: Result<(), AppError>) -> Result<Value, AppError> {
        let mut v = self.track_layer_get(sid, name)?;
        if let (Err(e), Some(m)) = (pushed, v.as_object_mut()) {
            m.insert("transport_error".into(), json!(e.to_string()));
        }
        Ok(v)
    }

    /// `track_layer_align {sid, name}` (P4-7b): the alignment of every uncut hit: `{n, sr, polarity,
    /// sign, as_is, inverted, flips, median_match, uncertain, attack_ms, max_lag_ms, hits: [{t, lag,
    /// match, sign, uncertain}]}`. `lag` is in samples of the track (sub-sample) for the sign the layer
    /// uses (`sign`: its polarity, or the majority for `auto`); `match` is 0 to 1; a hit's own `sign`
    /// is the better of the two; `flips` are the hits whose own sign is not the majority's. 409 without
    /// a layer, with a missing layer file or an offline track.
    pub fn track_layer_align(&self, sid: &str, name: &str) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        let job = {
            let g = lock(&op);
            layer_job(&g, name)?.ok_or_else(|| AppError::conflict(format!("track {name} has no layer")))?
        };
        let al = self.layer_alignment(sid, name, &job, &job.hits)?;
        let sign = al.sign_for(job.layer.polarity);
        let maj = al.majority.0;
        let mut matches = Vec::new();
        let mut flips = Vec::new();
        let rows: Vec<Value> = job
            .hits
            .iter()
            .zip(&al.hits)
            .map(|(h, x)| match x {
                Some(x) => {
                    let (lag, m, edge) = x.for_sign_edge(sign);
                    matches.push(m);
                    if x.sign() != maj {
                        flips.push(json!(h.t));
                    }
                    // an edge lag is not applied by the render: report 0 (`applied` false); a low match is applied
                    // and only marks the hit uncertain
                    let applied = !edge;
                    let lag = if applied { lag } else { 0.0 };
                    json!({"t": h.t, "lag": (lag * 100.0).round() / 100.0, "match": (m * 1000.0).round() / 1000.0,
                        "sign": x.sign(), "edge": edge, "applied": applied, "uncertain": edge || m < UNCERTAIN_MATCH})
                }
                None => json!({"t": h.t, "lag": 0.0, "match": Value::Null, "sign": Value::Null, "edge": false, "uncertain": true}),
            })
            .collect();
        let mut sorted = matches.clone();
        sorted.sort_by(f64::total_cmp);
        let median = if sorted.is_empty() {
            Value::Null
        } else {
            json!((sorted[sorted.len() / 2] * 1000.0).round() / 1000.0)
        };
        let p = align_params(&job.role);
        Ok(json!({
            "n": rows.len(),
            "sr": al.sr,
            "polarity": if maj < 0 { "inverted" } else { "normal" },
            "sign": sign,
            "as_is": al.majority.1,
            "inverted": al.majority.2,
            "flips": flips,
            "median_match": median,
            "uncertain": rows.iter().filter(|r| r["uncertain"] == true).count(),
            "attack_ms": p.attack_s * 1e3,
            "max_lag_ms": p.max_lag_s * 1e3,
            "hits": rows,
        }))
    }

    /// `export_layer {sid, name, path?}`: write `Exports/<name>_layer.wav` (see the module docs), or the
    /// file `path` when the page gives one (P4-7d: the save dialog). Answers
    /// `{path, size, n_events, sr, n_samples, channels, format, time_reference, onset, aligned, sign,
    /// outside}` (`onset`: the one-shot's onset in samples of the track's rate; `outside`: hits with no
    /// sample in the file). 409 without a layer, for a missing layer file, an offline track, a track
    /// whose files are another track's, a stale follower that cannot be derived, and a `path` that is
    /// a source or a one-shot in use.
    pub fn export_layer(&self, sid: &str, name: &str) -> Result<Value, AppError> {
        self.export_layer_to(sid, name, None)
    }

    /// The leftover of a failed layer render (campaign 0.9.6 S3, docs/review/test-campaign-0.9.6/117):
    /// a folder named like the target file plus a `<target>.drumspike` sidecar blocked every later
    /// render with 400 "the path is a folder" until the folder was removed by hand. Only the proven
    /// leftover shape is cleared: an empty folder at the target with the `<target>.drumspike`
    /// sidecar file beside it (`true`; the sidecar goes with it). Every other folder at the target
    /// belongs to the engineer: it stays exactly as it was and the render answers 400 with a
    /// message that says so (`false`).
    fn clear_render_leftover(path: &Path) -> bool {
        if !path.is_dir() {
            return false;
        }
        let name = path
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let sidecar = path.with_file_name(format!("{name}{DOC_EXT}"));
        if !sidecar.is_file() {
            return false; // without the sidecar the folder is not provably a render leftover
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            return false;
        };
        if entries.count() > 0 {
            return false; // any content: foreign, left alone
        }
        if std::fs::remove_dir(path).is_err() {
            return false;
        }
        let _ = std::fs::remove_file(&sidecar);
        true
    }

    /// [`Service::export_layer`] into the file `path` (an absolute path, see [`save_target`]) when given.
    pub fn export_layer_to(&self, sid: &str, name: &str, path: Option<&str>) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        {
            let mut g = lock(&op);
            check_file_owner(&g, name)?;
            g.resolve();
        }
        crate::follow::fresh_for_export(self, &op, name)?;
        let (job, dir, protected) = {
            let g = lock(&op);
            let job = layer_job(&g, name)?.ok_or_else(|| AppError::conflict(format!("track {name} has no layer")))?;
            (job, g.project.subdir(EXPORTS_DIR), protected_files(&g))
        };
        let target = match path {
            Some(p) => {
                // a folder at the target from a failed earlier render must not block this one
                Self::clear_render_leftover(Path::new(p));
                Some(save_target(p, &protected)?)
            }
            None => None,
        };
        let file = job
            .file
            .clone()
            .ok_or_else(|| AppError::conflict(format!("the layer file of track {name} is missing")))?;
        let mic = job
            .mic
            .clone()
            .ok_or_else(|| AppError::conflict(format!("track {name} is offline: relink it first")))?;
        let wav = MappedWav::open(&mic).map_err(|e| AppError::bad(format!("{}: {e}", mic.display())))?;
        let (sr, n) = (wav.info.sample_rate, wav.info.n_frames);
        let bext = wav.info.bext.clone();
        drop(wav);
        let shot = self.layer_shot(&file, Some(sr))?;
        let al = if job.layer.align || job.layer.polarity == Polarity::Auto {
            Some(self.layer_alignment(sid, name, &job, &job.hits)?)
        } else {
            None
        };
        let sign = al
            .as_ref()
            .map_or(if job.layer.polarity == Polarity::Invert { -1 } else { 1 }, |a| {
                a.sign_for(job.layer.polarity)
            });
        let mut outside = 0usize;
        let mut hits = Vec::with_capacity(job.hits.len());
        for (k, h) in job.hits.iter().enumerate() {
            let Some(i) = hit_index(h.t, sr, n) else {
                outside += 1;
                continue;
            };
            let lag = match (&al, job.layer.align) {
                // an edge lag is uncertain: do not shift
                (Some(a), true) => a.hits[k].map_or(0.0, |x| applied_lag(&x, sign)),
                _ => 0.0,
            };
            hits.push((i as f64 + lag, job.layer.gain(h.v, sign)));
        }
        let onset = job_onset(&job, &shot);
        let out = render_layer(n, &shot, job_onset_exact(&job, &shot), &hits);
        let ch = out.len();
        let inter: Vec<f32> = (0..n).flat_map(|i| out.iter().map(move |c| c[i])).collect();
        let path = match target {
            Some(p) => p,
            None => {
                std::fs::create_dir_all(&dir).map_err(|e| AppError::internal(format!("{}: {e}", dir.display())))?;
                dir.join(format!("{name}_layer.wav"))
            }
        };
        if path.is_dir() && !Self::clear_render_leftover(&path) {
            return Err(folder_error(&path));
        }
        // R01: the render must not write a protected file (a source, one-shot or bounce of any
        // open project) in any spelling of the path
        crate::export::check_export_plan(std::slice::from_ref(&path), &self.protected_files_everywhere())?;
        let spec = WavSpec {
            sample_rate: sr,
            channels: ch as u16,
            format: WavFormat::Float32,
            extensible: false,
            channel_mask: 0,
        };
        let b = bext.as_ref().map(|s| layer_bext(s, sr, ch));
        write_wav(&path, &spec, &inter, b.as_ref())
            .map_err(|e| AppError::internal(format!("{}: {e}", path.display())))?;
        Ok(json!({
            "path": path.display().to_string(),
            "size": std::fs::metadata(&path).map_or(0, |m| m.len()),
            "n_events": job.hits.len(),
            "sr": sr,
            "n_samples": n,
            "channels": ch,
            "format": "float32",
            "time_reference": bext.as_ref().map(|b| b.time_reference),
            "onset": onset,
            "aligned": al.is_some() && job.layer.align,
            "sign": sign,
            "outside": outside,
        }))
    }
}

/// The `bext` chunk of a layer file: the source's time reference, date, time and originator, the
/// layer's description and coding history.
pub fn layer_bext(source: &Bext, sr: u32, channels: usize) -> Bext {
    Bext {
        description: BEXT_DESCRIPTION.to_string(),
        originator: source.originator.clone(),
        originator_reference: String::new(),
        origination_date: source.origination_date.clone(),
        origination_time: source.origination_time.clone(),
        time_reference: source.time_reference,
        version: 1,
        coding_history: format!(
            "A=PCM,F={sr},W=32,M={},T=Drum Spike layer",
            if channels == 2 { "stereo" } else { "mono" }
        ),
    }
}

/// The answer of `track_layer_get` for a job.
fn get_json(job: Option<&LayerJob>) -> Value {
    match job {
        None => {
            json!({"layer": Value::Null, "status": Value::Null, "path": Value::Null, "file": Value::Null, "onset_ms": Value::Null})
        }
        Some(j) => {
            let shown = j
                .file
                .as_ref()
                .map_or_else(|| PathBuf::from(j.layer.source.path_or_relative()), |p| p.to_path_buf());
            json!({
                "layer": j.layer.to_json(),
                "status": j.status.as_str(),
                "path": j.file.as_ref().map(|p| p.display().to_string()),
                "file": shown.file_name().map(|f| f.to_string_lossy().into_owned()),
                "onset_ms": (j.layer.onset_s() * 1e6).round() / 1e3,
            })
        }
    }
}

/// Run a command of the layer for the bridge, the command line and the MCP server; `None` when `cmd`
/// is not one of them.
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    if !LAYER_COMMANDS.contains(&cmd) {
        return None;
    }
    let run = || -> Result<Value, AppError> {
        let (sid, name) = (
            crate::command_policy::arg_str(args, "sid")?,
            crate::command_policy::arg_str(args, "name")?,
        );
        match cmd {
            "track_layer_set" => svc.track_layer_set(sid, name, args),
            "track_layer_get" => svc.track_layer_get(sid, name),
            "track_layer_align" => svc.track_layer_align(sid, name),
            "export_layer" => svc.export_layer_to(sid, name, opt_path(args, "path")?),
            _ => Err(AppError::not_found(format!("unknown command {cmd}"))),
        }
    };
    Some(run())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src() -> Source {
        Source {
            mode: Some("referenced".into()),
            path: Some("/x/kick.wav".into()),
            relative: Some("../x/kick.wav".into()),
            size: Some(10),
            mtime: Some(1),
            head_sha1: Some("ab".into()),
            bext_origination: None,
            extra: Map::new(),
        }
    }

    fn layer() -> Layer {
        Layer {
            source: src(),
            level_db: -6.0,
            polarity: Polarity::Invert,
            on: false,
            curve: Curve::Linear,
            align: true,
            onset: 96,
            onset_set: None,
            sr: 96_000,
            channels: 2,
            frames: 9_600,
        }
    }

    #[test]
    fn job_onset_re_measures_when_the_file_changed() {
        let l = layer();
        let job = |status: SourceStatus| LayerJob {
            layer: l.clone(),
            file: None,
            status,
            mic: None,
            mode: drumspike_core::MixMode::Average,
            role: "kick".into(),
            hits: vec![],
        };
        // the stored onset (96 at 96 kHz) is used while the file is as recorded
        let shot = OneShot {
            sr: 96_000,
            chans: vec![vec![0.0; 96_000]],
        };
        assert_eq!(job_onset(&job(SourceStatus::Ok), &shot), 96);
        // the file changed: the onset is re-measured from the actual samples
        let mut chans = vec![0.0f32; 96_000];
        chans[1000..].fill(1.0); // full scale from 1000 on
        let shot2 = OneShot {
            sr: 96_000,
            chans: vec![chans],
        };
        // the foot of the version 4 rule (#101): one sample before the first full-scale sample
        assert_eq!(job_onset(&job(SourceStatus::Changed), &shot2), 999);
        // fix 7: the render's onset is not rounded; a re-measured one stays whole
        let shot44 = OneShot {
            sr: 44_100,
            chans: vec![vec![0.0; 44_100]],
        };
        assert!((job_onset_exact(&job(SourceStatus::Ok), &shot44) - 44.1).abs() < 1e-9);
        assert_eq!(job_onset_exact(&job(SourceStatus::Changed), &shot2), 999.0);
    }

    #[test]
    fn the_onset_override_moves_every_placement_and_survives_the_document() {
        let mut l = layer();
        assert_eq!(l.onset_eff(), 96);
        // the override moves every placement (the render, the playback, the Align)
        l.onset_set = Some(196);
        assert_eq!(l.onset_eff(), 196);
        assert_eq!(l.onset_s(), 196.0 / 96_000.0);
        assert_eq!(l.onset_at(96_000), 196);
        assert_eq!(l.onset_at(48_000), 98);
        assert!((l.onset_exact_at(48_000) - 98.0).abs() < 1e-9);
        // the measured onset is kept, the override round-trips the document
        assert_eq!(l.onset, 96);
        let back = Layer::from_json(&l.to_json()).unwrap();
        assert_eq!((back.onset, back.onset_set), (96, Some(196)));
        // without an override the JSON has none and the measured one is used
        l.onset_set = None;
        let back = Layer::from_json(&l.to_json()).unwrap();
        assert_eq!(back.onset_set, None);
        assert_eq!(back.onset_eff(), 96);
        // the Reset is the same as no override
        assert_eq!(l.onset_eff(), back.onset_eff());
    }

    #[test]
    fn the_layer_object_round_trips_and_takes_defaults() {
        let l = layer();
        assert_eq!(Layer::from_json(&l.to_json()), Some(l.clone()));
        let mut m = Map::new();
        m.insert("layer".into(), l.to_json());
        assert_eq!(layer_of(&m), Some(l));
        let min = json!({"source": serde_json::to_value(src()).unwrap(), "onset": 5, "sr": 48000});
        let d = Layer::from_json(&min).unwrap();
        assert_eq!(
            (d.level_db, d.polarity, d.on, d.curve, d.align, d.channels),
            (0.0, Polarity::Normal, true, Curve::default(), false, 1)
        );
        assert_eq!(Layer::from_json(&json!({"onset": 5, "sr": 48000})), None, "no source");
        assert_eq!(
            Layer::from_json(&json!({"source": {}, "onset": 5, "sr": 0})),
            None,
            "a rate of 0"
        );
        assert_eq!(Layer::from_json(&json!(3)), None);
        assert_eq!(layer_of(&Map::new()), None);
    }

    #[test]
    fn align_params_follow_the_role() {
        // fix: the align answer reports the window the render uses (kick 5 ms, others 3 ms)
        assert_eq!(align_params("kick").max_lag_s, 0.005);
        assert_eq!(align_params("snare").max_lag_s, 0.003);
    }

    #[test]
    fn onset_gain_and_polarity_names() {
        let l = layer();
        assert_eq!(l.onset_s(), 0.001);
        assert_eq!(l.onset_at(96_000), 96);
        assert_eq!(l.onset_at(48_000), 48);
        assert_eq!(l.onset_at(44_100), 44);
        // fix 7: the exact onset keeps the fraction (96 samples at 96 kHz is 44.1 at 44.1 kHz)
        assert!((l.onset_exact_at(44_100) - 44.1).abs() < 1e-9);
        assert_eq!(l.onset_exact_at(48_000), 48.0);
        let g = l.gain(127, 1);
        assert!((g as f64 - 10f64.powf(-6.0 / 20.0)).abs() < 1e-6);
        assert_eq!(l.gain(127, -1), -g);
        assert!((l.gain(0, 1)).abs() < 1e-9, "linear curve at velocity 0");
        let loud = Layer {
            level_db: 40.0,
            ..layer()
        };
        assert!(
            (loud.gain(127, 1) as f64 - 10f64.powf(12.0 / 20.0)).abs() < 1e-5,
            "clamped"
        );
        for p in [Polarity::Normal, Polarity::Invert, Polarity::Auto] {
            assert_eq!(Polarity::parse(p.as_str()), Some(p));
        }
        assert_eq!(Polarity::parse("up"), None);
        let a = Alignment {
            hits: vec![],
            majority: (-1, 1, 2),
            sr: 48_000,
        };
        assert_eq!(
            (
                a.sign_for(Polarity::Normal),
                a.sign_for(Polarity::Invert),
                a.sign_for(Polarity::Auto)
            ),
            (1, -1, -1)
        );
    }

    #[test]
    fn layer_bext_keeps_the_time_reference() {
        let s = Bext {
            description: "take".into(),
            originator: "Pro Tools".into(),
            originator_reference: "x".into(),
            origination_date: "2026-10-14".into(),
            origination_time: "10:00:00".into(),
            time_reference: 123_456,
            version: 2,
            coding_history: String::new(),
        };
        let b = layer_bext(&s, 48_000, 2);
        assert_eq!(b.time_reference, 123_456);
        assert_eq!(b.description, BEXT_DESCRIPTION);
        assert!(b.coding_history.contains("M=stereo"));
        assert!(layer_bext(&s, 48_000, 1).coding_history.contains("M=mono"));
    }

    #[test]
    fn get_json_without_a_layer_is_all_null() {
        let v = get_json(None);
        assert_eq!(v["layer"], Value::Null);
        assert_eq!(v["status"], Value::Null);
    }

    #[test]
    fn dispatch_knows_its_commands_only() {
        let s = Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        assert!(dispatch(&s, "nope", &json!({})).is_none());
        assert_eq!(
            dispatch(&s, "track_layer_get", &json!({})).unwrap().unwrap_err().code,
            400
        );
        assert_eq!(
            dispatch(&s, "export_layer", &json!({"sid": "x", "name": "k"}))
                .unwrap()
                .unwrap_err()
                .code,
            404
        );
    }
}
