//! The commands of the thin slice (P3-3a). Each is a method of [`Service`] that takes plain
//! arguments and returns JSON with the keys of the server's response, or bytes. The Tauri
//! wrappers in `apps/desktop/src-tauri` only convert arguments and results.
//!
//! | Command | Server route (route map row) |
//! |---|---|
//! | `app_info` | none (P3-1b) |
//! | `project_open` | POST `/api/project/open` (5) |
//! | `project_get` | GET `/api/project/{sid}` (6) |
//! | `project_list` | GET `/api/sessions` (19) and GET `/api/project` (1) |
//! | `tracks_get` | GET `/api/tracks/{sid}` (21) |
//! | `events_get`, `events_set` | GET and POST `/api/events/{sid}/{name}` (29, 30) |
//! | `peaks_info`, `peaks_tile` | GET `/api/peaks/{sid}/{name}` (28) |
//! | `samples_window` | GET `/api/samples/{sid}/{name}` (31) |
//! | `analyze_start`, `analyze_status` | POST `/api/analyze/{sid}` (25), GET `/api/job/{sid}` (26) |
//! | `export_midi` | POST `/api/export/{sid}/{name}` |
//! | `export_spikes`, `export_all`, `velocities_recompute` | P3-3c, see [`crate::export`] (routes 33, 34) |
//!
//! # Byte layouts (all little endian)
//!
//! `peaks_tile`: `u32 level`, `u32 start`, `u32 n`, then `min[n]`, `max[n]`, `rms[n]` as f32
//! (12 bytes of header, so a `Float32Array` can view the values). `samples_window`:
//! `u32 start`, `u32 sample rate`, then the samples as f32, not rounded (8 bytes of header).

use std::path::Path;

use drumspike_core::cache::CacheTile;
use drumspike_core::midi::export_midi_map as write_midi;
use drumspike_core::project::{Event, load_project};
use serde_json::{Value, json};

use crate::error::AppError;
use crate::jobs;
use crate::media::PeakRequest;
use crate::registry::{OpenProject, lock};
use crate::{Progress, Service};

/// The detector family the app uses (the Rust detector in `drumspike-core`).
pub const DETECTOR: &str = "rust";
/// The default chain (`drumspike_core::detect::Chain::Mix`).
pub const CHAIN: &str = "mix";
/// The most samples one `samples_window` answer holds (the server's cap).
pub const MAX_WINDOW_SAMPLES: i64 = 262_144;
/// P3-15e: how often `project_open` loads again when a parallel close took out the session it
/// found (each extra round needs another close of the same project in that short window).
const OPEN_ATTEMPTS: usize = 3;

/// Every command of the slice, as the route map names it.
pub const COMMANDS: [&str; 16] = [
    "app_info",
    "project_open",
    "project_get",
    "project_list",
    "tracks_get",
    "events_get",
    "events_set",
    "peaks_info",
    "peaks_tile",
    "samples_window",
    "analyze_start",
    "analyze_status",
    "export_midi",
    "export_spikes",
    "export_all",
    "velocities_recompute",
];

/// The answer of `app_info`: core version, detector and chain (main is model-free, #281 D16:
/// the DSP detector is the only path). The arguments are the legacy signature; both are ignored.
pub fn app_info_value(_dir: &Path, _load: bool) -> Value {
    json!({
        "core_version": drumspike_core::VERSION,
        "detector": DETECTOR,
        "chain": CHAIN,
    })
}

/// The rows of `project_list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListShape {
    /// `/api/sessions` (route 19): `sid, name, tracks, folder, dirty, mtime`, newest first.
    #[default]
    Sessions,
    /// `/api/project` (route 1): `id, name, folder, dirty`, in the order of opening.
    Ids,
}

impl ListShape {
    /// `ids` gives [`ListShape::Ids`]; anything else, or nothing, [`ListShape::Sessions`].
    pub fn parse(s: Option<&str>) -> ListShape {
        if s == Some("ids") {
            ListShape::Ids
        } else {
            ListShape::Sessions
        }
    }
}

/// The f32 values of a slice as little-endian bytes appended to `out`.
fn push_f32(out: &mut Vec<u8>, v: &[f32]) {
    out.reserve(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

/// The bytes of a peak tile (layout in the module docs).
pub fn tile_bytes(t: &CacheTile) -> Vec<u8> {
    let n = t.min.len();
    let mut out = Vec::with_capacity(12 + 12 * n);
    for v in [t.level as u32, t.start as u32, n as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    push_f32(&mut out, &t.min);
    push_f32(&mut out, &t.max);
    push_f32(&mut out, &t.rms);
    out
}

impl Service {
    /// The answer of `app_info` (commands.rs, model-free).
    pub(crate) fn app_info_value(&self) -> Value {
        crate::commands::app_info_value(&std::path::PathBuf::new(), false)
    }

    /// Register a loaded project (P4-5): with the lock of this process's client when one is set (409 when another
    /// process holds the project), or read-only without one.
    pub(crate) fn register_project(
        &self,
        project: drumspike_core::Project,
        read_only: bool,
    ) -> Result<crate::registry::Shared, AppError> {
        let client = self.lock_client();
        self.inner.registry.register_with(project, read_only, client.as_deref())
    }

    /// `app_info`.
    pub fn app_info(&self) -> Value {
        self.app_info_value()
    }

    /// `POST /api/project/open`: open a project folder or document. A folder that is open
    /// already keeps its id.
    ///
    /// P3-15e: since P3-15c, opens and closes run in parallel on the blocking pool. A close of
    /// the same project can take out the entry that `register` returned before this call answers;
    /// the answer then named a closed session and the page's next call failed with "no such
    /// project". Now the call answers only when the entry is still registered after the summary
    /// ([`Registry::is_open`](crate::registry::Registry::is_open)); else the project loads again
    /// as a new session. A close after that check comes after the open, as on one thread.
    ///
    /// # Errors
    ///
    /// See [`Service::project_open_with`].
    pub fn project_open(&self, path: &str) -> Result<Value, AppError> {
        self.project_open_with(path, false)
    }

    /// [`Service::project_open`] with `read_only` (P4-5): no project lock is taken, the document is the saved one,
    /// and a command that changes the project is refused (409) by [`crate::call::call`]. The summary then has
    /// `read_only: true`. A folder that is open in this process already keeps its entry and its mode.
    ///
    /// # Errors
    ///
    /// A 400 [`AppError`] for an empty `path`; the load error when the folder or document cannot be read or
    /// parsed; the registry error when it cannot be registered (for example a project lock held elsewhere);
    /// a 409 when the project was closed again while it opened on every one of `OPEN_ATTEMPTS` attempts.
    pub fn project_open_with(&self, path: &str, read_only: bool) -> Result<Value, AppError> {
        if path.is_empty() {
            return Err(AppError::bad("path required"));
        }
        for _ in 0..OPEN_ATTEMPTS {
            let mut project = load_project(Path::new(path))?;
            // P4-6c: a 0.4.0 learn object turned on by a first K loads with the filter off
            for tr in project.doc.tracks_mut().values_mut() {
                crate::learn::migrate_learn(&mut tr.extra);
            }
            let op = self.register_project(project, read_only)?;
            let (dir, name) = {
                let g = lock(&op);
                (g.project.dir.clone(), g.project.name())
            };
            self.touch_recent(&dir, &name); // a file write: not under the project lock (P3-15c)
            let summary = lock(&op).summary();
            if self.inner.registry.is_open(&op) {
                return Ok(summary);
            }
        }
        Err(AppError::conflict(
            "the project was closed while it opened: open it again",
        ))
    }

    /// `GET /api/project/{sid}`: the summary; a file moved since the open shows up here.
    ///
    /// # Errors
    ///
    /// A 404 [`AppError`] when no project is open under `sid`.
    pub fn project_get(&self, sid: &str) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        crate::registry::resolve_shared(&op);
        let g = lock(&op);
        Ok(g.summary())
    }

    /// `GET /api/sessions` or `GET /api/project`: the open projects.
    pub fn project_list(&self, shape: ListShape) -> Value {
        let mut rows: Vec<(f64, Value)> = Vec::new();
        for (sid, op) in self.inner.registry.all() {
            let g = lock(&op);
            let row = match shape {
                ListShape::Ids => {
                    json!({"id": sid, "name": g.project.name(), "folder": g.project.dir.display().to_string(), "dirty": g.dirty})
                }
                ListShape::Sessions => json!({
                    "sid": sid,
                    "name": g.project.name(),
                    "tracks": g.project.doc.track_order(),
                    "folder": g.project.dir.display().to_string(),
                    "dirty": g.dirty,
                    "mtime": g.mtime(),
                }),
            };
            rows.push((g.mtime(), row));
        }
        if shape == ListShape::Sessions {
            rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        }
        Value::Array(rows.into_iter().map(|r| r.1).collect())
    }

    /// `GET /api/tracks/{sid}`: every track as the page sees it; every source is resolved again.
    ///
    /// # Errors
    ///
    /// A 404 [`AppError`] when no project is open under `sid`; the error of building the public form of a
    /// track that cannot be shown.
    pub fn tracks_get(&self, sid: &str) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        crate::registry::resolve_shared(&op);
        // P4-4: a follower that went stale while its audio was offline is derived once it is back
        crate::follow::refresh_stale(self, &op);
        let g = lock(&op);
        let mut tracks = serde_json::Map::new();
        for (name, _) in g.project.doc.tracks() {
            tracks.insert(name.clone(), g.public_track(name)?);
        }
        Ok(json!({
            "sid": sid,
            "name": g.project.name(),
            "dirty": g.dirty,
            "tracks": Value::Object(tracks),
            "order": g.project.doc.track_order(),
        }))
    }

    /// `GET /api/events/{sid}/{name}`: `{sr, duration, n_samples, events}`.
    ///
    /// # Errors
    ///
    /// A 404 [`AppError`] when no project is open under `sid` or the project has no track `name`.
    pub fn events_get(&self, sid: &str, name: &str) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        let g = lock(&op);
        let m = g.track_object(name)?;
        let pick = |k: &str, d: Value| m.get(k).filter(|v| !v.is_null()).cloned().unwrap_or(d);
        Ok(json!({
            "sr": pick("sr", json!(48000)),
            "duration": pick("duration", json!(0)),
            "n_samples": pick("n_samples", json!(0)),
            "events": pick("events", json!([])),
        }))
    }

    /// `POST /api/events/{sid}/{name}`: replace the events. Every key of every event is kept
    /// as sent (I1); a list the typed events cannot hold (an event without a numeric `t`)
    /// is kept as it is, untyped. R07: the plain write is a writer of the project like an edit
    /// or a re-pick store: it takes the store guard once here, so it cannot commit over a
    /// concurrent write of the same project (the write refuses with a retryable 409 instead).
    ///
    /// # Errors
    ///
    /// A 404 [`AppError`] for an unknown `sid` or track; a 409 when the project is open read-only, when
    /// another write or a running analysis holds the project, or when the track is a follower (its hits
    /// are the source's); a 400 when `events` cannot be stored.
    pub fn events_set(&self, sid: &str, name: &str, events: &Value) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let _guard = self.begin_write(sid)?;
        self.events_set_with(&op, name, events)
    }

    /// [`Service::events_set`] with the project's store guard already held (the caller owns it:
    /// `events_edit_as`, `track_shift_set`, `track_velocity_set`). Never takes the guard itself.
    pub(crate) fn events_set_with(
        &self,
        op: &crate::registry::Shared,
        name: &str,
        events: &Value,
    ) -> Result<Value, AppError> {
        let n = {
            let mut g = lock(op);
            g.track(name)?;
            crate::follow::check_own(&g, name)?; // P3-18: a follower's hits are the source's
            let n = assign_events(&mut g, name, events)?;
            g.mark_dirty();
            n
        };
        // P3-18: the followers of this track take the edited hits (the lock is free: a decode may run)
        let rows = crate::follow::refresh_followers(self, op, name);
        Ok(events_set_out(n, &rows))
    }

    /// The peak cache of a track, built when it is missing or stale. The project lock is
    /// free while the cache builds.
    fn open_peaks(&self, sid: &str, name: &str) -> Result<drumspike_core::PeakCache, AppError> {
        let op = self.inner.registry.get(sid)?;
        let req = {
            let g = lock(&op);
            if g.project.doc.track(name).is_none() {
                return Err(AppError::not_found("no peaks"));
            }
            PeakRequest::of(&g, name)?
        };
        self.inner.media.peak_cache(&req)
    }

    /// `GET /api/peaks/{sid}/{name}`, the header part: `{sr, n_samples, levels}` where
    /// `levels` maps each block size to `{"n": bins}`. The values come as tiles.
    ///
    /// # Errors
    ///
    /// A 404 [`AppError`] for an unknown `sid` or track; the error of building the peak cache when the
    /// track's audio cannot be read or decoded.
    pub fn peaks_info(&self, sid: &str, name: &str) -> Result<Value, AppError> {
        let cache = self.open_peaks(sid, name)?;
        let mut levels = serde_json::Map::new();
        for l in cache.levels() {
            levels.insert(l.block.to_string(), json!({"n": l.bins}));
        }
        Ok(json!({"sr": cache.sr(), "n_samples": cache.n_samples(), "levels": Value::Object(levels)}))
    }

    /// `GET /api/peaks/{sid}/{name}`, one tile: bins `[from, to)` of the level with block
    /// size `level`, clamped to the level (bytes, see the module docs).
    ///
    /// # Errors
    ///
    /// The errors of [`Service::peaks_info`] (unknown project or track, unreadable audio); a 400
    /// [`AppError`] when the cache has no level with block size `level` or the range is invalid.
    pub fn peaks_tile(&self, sid: &str, name: &str, level: usize, from: i64, to: i64) -> Result<Vec<u8>, AppError> {
        let cache = self.open_peaks(sid, name)?;
        match cache.tile(level, from, to)? {
            Some(t) => Ok(tile_bytes(&t)),
            None => Err(AppError::bad(format!("no peak level {level}"))),
        }
    }

    /// `GET /api/samples/{sid}/{name}`: samples `[start, end)`, at most
    /// [`MAX_WINDOW_SAMPLES`], clamped to the track (bytes, see the module docs). P3-15d: only
    /// the window is read from the file (`Media::window_bytes`); the bytes equal the window of
    /// the full decode.
    ///
    /// # Errors
    ///
    /// A 404 [`AppError`] for an unknown `sid` or track; the error of resolving the track's source or
    /// reading its audio file.
    pub fn samples_window(&self, sid: &str, name: &str, start: i64, end: i64) -> Result<Vec<u8>, AppError> {
        let op = self.inner.registry.get(sid)?;
        let (src, mix) = {
            let g = lock(&op);
            let tr = g.track(name)?;
            (g.source(name)?, crate::media::mix_mode(tr.mix_mode_or_default()))
        };
        self.inner.media.window_bytes(&src, mix, start, end, MAX_WINDOW_SAMPLES)
    }

    /// `POST /api/analyze/{sid}`: store `params` per track, then start the job on a worker
    /// thread. `tracks` defaults to every track. `progress` gets `{"sid", "job"}` after every
    /// change of the job (the `analyze_status` value). Returns `{"started": [names]}`.
    ///
    /// # Errors
    ///
    /// See [`Service::analyze_start_keep`].
    pub fn analyze_start(
        &self,
        sid: &str,
        tracks: Option<Vec<String>>,
        params: Option<&serde_json::Map<String, Value>>,
        progress: Option<Progress>,
    ) -> Result<Value, AppError> {
        self.analyze_start_keep(sid, tracks, params, None, progress, false)
    }

    /// [`Service::analyze_start`] with `keep_edited` (P4-5 step 2b): the hits the engineer added or
    /// moved stay in the new events and win within 20 ms over the detected ones. The page sends
    /// `keep_edited` for Re-analyze > Keep (P4-9); the agent surface (`call`) defaults it to true.
    ///
    /// # Errors
    ///
    /// A 404 [`AppError`] for an unknown `sid`; a 409 when the project is open read-only, an analysis is
    /// already queued or running, or a re-pick is being stored; a 400 when a name in `tracks` is not a track.
    pub fn analyze_start_keep(
        &self,
        sid: &str,
        tracks: Option<Vec<String>>,
        params: Option<&serde_json::Map<String, Value>>,
        detector: Option<&str>,
        progress: Option<Progress>,
        keep_edited: bool,
    ) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        // R08: a duplicate start is refused: one analysis per project at a time. Cancel the
        // queued or running job first (`job_cancel`) or wait for it; a replaced generation
        // keeps the late callbacks of an older worker out either way.
        if self.inner.jobs.running(sid) {
            return Err(AppError::conflict(
                "an analysis is already queued or running for this project: cancel it or wait for it",
            ));
        }
        // P4-3 fix round: not while a re-pick stores; held while the job queues, so a re-pick
        // that starts now sees the job
        let _store = self
            .inner
            .jobs
            .begin_store(sid)
            .ok_or_else(|| AppError::conflict("a re-pick is being stored: try again"))?;
        let targets = {
            let mut g = lock(&op);
            let targets: Vec<String> = match tracks {
                Some(t) if !t.is_empty() => t,
                _ => g.project.doc.tracks().map(|(n, _)| n.clone()).collect(),
            };
            for t in &targets {
                if g.project.doc.track(t).is_none() {
                    return Err(AppError::bad(format!("unknown track {t}")));
                }
            }
            for (name, p) in params.into_iter().flatten() {
                if let (Some(tr), Some(map)) = (g.project.doc.track_mut(name), p.as_object()) {
                    let mut map = map.clone();
                    // #388: the flag sets params.detector (a track's own param was not sent here)
                    if let Some(id) = detector {
                        map.insert("detector".to_string(), Value::from(id));
                    }
                    tr.params = Some(map);
                    g.mark_dirty();
                }
            }
            // #388: the flag also reaches tracks the call gave no params for
            if let Some(id) = detector {
                for t in &targets {
                    if let Some(tr) = g.project.doc.track_mut(t) {
                        let map = tr.params.get_or_insert_with(serde_json::Map::new);
                        map.insert("detector".to_string(), Value::from(id));
                    }
                }
                g.mark_dirty();
            }
            targets
        };
        jobs::start(self, sid, targets.clone(), progress, keep_edited);
        Ok(json!({"started": targets}))
    }

    /// `GET /api/job/{sid}`: `{state, tracks: {name: {state, stage?, n_events, detector, ...}}}`.
    pub fn analyze_status(&self, sid: &str) -> Value {
        self.inner.jobs.status(sid)
    }

    /// `POST /api/export/{sid}/{name}`: single-note MIDI into `Exports/<name>_spike.mid`.
    /// The bytes depend on `events` (`t` and `v`; a missing `v` is 100), `note`, `ppq` and
    /// `tempo` only while the project's tempo map is the default; any other map (P4-8) sets the
    /// tempo, the time signatures and the note ticks and `tempo` is not used. Returns `{path, size, n_events, outside, before}`. P4-4: `outside` counts the
    /// hits with no sample in the track's audio (the file keeps them), `before` those of them
    /// before its start (the file moves them to 0 s); 409
    /// when the track's files are another track's (`export::check_file_owner`) or a stale
    /// follower cannot be derived; a follower writes its own derived hits, never `events`.
    ///
    /// # Errors
    ///
    /// See [`Service::export_midi_with`].
    pub fn export_midi(
        &self,
        sid: &str,
        name: &str,
        note: i64,
        events: &[Value],
        ppq: i64,
        tempo: f64,
    ) -> Result<Value, AppError> {
        self.export_midi_to(sid, name, note, events, ppq, tempo, None)
    }

    /// P4-30: [`Service::export_midi`] into the folder `dest` (`export::check_dest`); `None` is `Exports/`.
    ///
    /// # Errors
    ///
    /// See [`Service::export_midi_with`].
    #[allow(clippy::too_many_arguments)]
    pub fn export_midi_to(
        &self,
        sid: &str,
        name: &str,
        note: i64,
        events: &[Value],
        ppq: i64,
        tempo: f64,
        dest: Option<&str>,
    ) -> Result<Value, AppError> {
        self.export_midi_with(sid, name, note, events, ppq, tempo, dest, true)
    }

    /// P4-7c: [`Service::export_midi_to`] with the choice to compensate the track's sampler offset (the notes
    /// at the aligned attack minus the offset; `sampler` and `warnings` in the answer say what moved). A track
    /// without a calibration is written as before.
    ///
    /// # Errors
    ///
    /// A 404 [`AppError`] for an unknown `sid` or track; a 409 when the project is open read-only, the track's
    /// files are another track's, or a stale follower cannot be derived; a 400 when `ppq` does not fit a
    /// `u16`, `dest` is not an allowed folder, or the tempo map or sampler offset is invalid; an error when
    /// the target is a protected file or the `.mid` file cannot be written.
    #[allow(clippy::too_many_arguments)]
    pub fn export_midi_with(
        &self,
        sid: &str,
        name: &str,
        note: i64,
        events: &[Value],
        ppq: i64,
        tempo: f64,
        dest: Option<&str>,
        compensate: bool,
    ) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let (dir, follower, map) = {
            let g = lock(&op);
            g.track(name)?;
            crate::export::check_file_owner(&g, name)?;
            (
                crate::export::export_dir(&g, dest)?,
                g.track(name)?.follows.is_some(),
                crate::tempo::doc_map(&g.project.doc)?,
            )
        };
        let ppq = u16::try_from(ppq).map_err(|_| AppError::bad(format!("ppq out of range: {ppq}")))?;
        // R10: a second export to the same folder serializes behind this one
        let _dest_guard = crate::export::export_dest_acquire(&dir);
        let refreshed = if follower {
            crate::registry::resolve_shared(&op);
            crate::follow::fresh_for_export(self, &op, name)?
        } else {
            false
        };
        let own;
        let events = if follower {
            own = self.track_events(sid, name)?;
            &own[..]
        } else {
            events
        };
        let uncut = crate::export::uncut(events);
        let shift = if compensate {
            self.sampler_shift(sid, name, &uncut)?
        } else {
            None
        };
        let (ev, info) = crate::sampler::midi_events_shifted(&uncut, shift.as_ref())?;
        let out = dir.join(format!("{name}_spike.mid"));
        // R01: the MIDI file must not be a protected file either (a source, one-shot or bounce
        // of any open project)
        crate::export::check_export_plan(std::slice::from_ref(&out), &self.protected_files_everywhere())?;
        write_midi(&ev, note, &out, ppq, &map, tempo)?;
        let size = std::fs::metadata(&out).map_or(0, |m| m.len());
        let times: Vec<f64> = ev.iter().map(|e| e.t).collect();
        let (outside, before) = crate::export::audio_length(&op, name).map_or((Value::Null, Value::Null), |(sr, n)| {
            (
                Value::from(crate::export::outside_count(&times, sr, n)),
                Value::from(crate::export::before_count(&times, sr)),
            )
        });
        let mut r = json!({"path": out.display().to_string(), "size": size, "n_events": ev.len(),
                           "outside": outside, "before": before});
        if refreshed {
            r["followers"] = json!([name]);
        }
        if let Some(i) = info {
            r["warnings"] = json!(i.warning(name).into_iter().collect::<Vec<_>>());
            r["sampler"] = i.to_json();
        }
        Ok(r)
    }
}

/// Put `events` on the track `name` under the HELD project lock: typed events go into
/// `tr.events`, a list the typed events cannot hold (an event without a numeric `t`) as it is
/// into `extra.events` (I1: every key is kept as sent). The caller holds the project's store
/// guard and marks the project dirty itself. Returns the number of events.
pub(crate) fn assign_events(g: &mut OpenProject, name: &str, events: &Value) -> Result<usize, AppError> {
    let Some(list) = events.as_array() else {
        return Err(AppError::bad("events must be a list"));
    };
    let n = list.len();
    let typed = serde_json::from_value::<Vec<Event>>(events.clone());
    let tr = g.track_mut(name)?;
    if let Ok(ev) = typed {
        tr.events = Some(ev);
        tr.extra.shift_remove("events");
    } else {
        tr.events = None;
        tr.extra.insert("events".into(), events.clone());
    }
    Ok(n)
}

/// The answer of `events_set` and `events_set_by_hand`: the saved count and the followers that
/// took the hits ([`crate::follow::refreshed_names`]).
pub(crate) fn events_set_out(n: usize, rows: &[(String, Result<usize, AppError>)]) -> Value {
    let mut out = json!({"saved": n});
    if !rows.is_empty() {
        out["followers"] = Value::Array(crate::follow::refreshed_names(rows));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_shape_parses_ids_and_defaults_to_sessions() {
        assert_eq!(ListShape::parse(Some("ids")), ListShape::Ids);
        assert_eq!(ListShape::parse(Some("sessions")), ListShape::Sessions);
        assert_eq!(ListShape::parse(None), ListShape::Sessions);
    }

    #[test]
    fn tile_bytes_has_a_12_byte_header_and_three_columns() {
        let t = CacheTile {
            level: 256,
            start: 7,
            min: vec![-1.0, -0.5],
            max: vec![1.0, 0.5],
            rms: vec![0.25, 0.125],
        };
        let b = tile_bytes(&t);
        assert_eq!(b.len(), 12 + 3 * 2 * 4);
        assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 256);
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()), 2);
        assert_eq!(f32::from_le_bytes(b[12..16].try_into().unwrap()), -1.0);
        assert_eq!(f32::from_le_bytes(b[b.len() - 4..].try_into().unwrap()), 0.125);
    }

    #[test]
    fn push_f32_writes_little_endian() {
        let mut out = vec![9u8];
        push_f32(&mut out, &[1.5, -2.0]);
        assert_eq!(out.len(), 9);
        assert_eq!(&out[1..5], &1.5f32.to_le_bytes());
    }

    #[test]
    fn the_command_list_has_no_duplicates() {
        let mut v = COMMANDS.to_vec();
        v.sort_unstable();
        v.dedup();
        assert_eq!(v.len(), COMMANDS.len());
    }

    #[test]
    fn app_info_of_an_empty_folder_has_an_empty_notice() {
        let dir = std::env::temp_dir().join(format!("ds_appinfo_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let v = app_info_value(&dir, false);
        assert_eq!(v["core_version"], drumspike_core::VERSION);
        assert_eq!(v["detector"], DETECTOR);
        assert_eq!(v["chain"], CHAIN);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
