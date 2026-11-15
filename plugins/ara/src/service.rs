//! The Drum Spike service of one ARA document: one `drumspike_app::Service` and one project
//! folder for the life of the document controller (M1b1). The project keeps every source
//! that was analysed as a track (one track per audio source, the host's playback regions
//! as the segments of P1); a later request adds its sources to the open project and the
//! earlier tracks are not touched; `destroyAudioSource` removes the track; a restore from
//! the ARA archive opens a project with the stored events and decodes nothing.
//!
//! Where things live (all below the one cache folder the README names):
//!
//! ```text
//! <cache>/documents/<id>/audio/<seq>-<stem>.wav   the source audio of this document
//! <cache>/documents/<id>/project/                 the project folder (the document, the Analysis cache)
//! <cache>/documents/<id>/service/                 the home folder of the service (its recent list)
//! ```
//!
//! `<id>` is made by the plugin once per document and stored in the ARA archive, so a
//! restore in a new process finds the same folder. Two documents never share a folder, a
//! source numbering or a file name.
//!
//! The track's timeline is the audio source's own timeline (ARA content is per source,
//! in the source's time): a playback region plays the slice
//! `[startInModificationTime, startInModificationTime + durationInModificationTime)` of
//! the source, so its segment places that slice at its own modification position. A
//! source with no playback regions is a plain track (the whole file from its first
//! sample).
//!
//! Every method of [`DocService`] runs on the plugin's worker thread, never on the audio
//! thread and never on the request caller's thread (the one exception is the deliberate
//! `DS_ARA_FAULT=in-process` run of the check).
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use drumspike_app::roles::{default_note, guess_role};
use drumspike_app::tracks::safe_name;
use drumspike_app::{Service, ServiceConfig};
use drumspike_core::project::{build_source, document_path, write_document, Document, Segment, Source, Track, FORMAT};
use serde_json::{json, Map, Value};

/// The services made and dropped in this process (the report names both; the gate: one
/// service per document controller, dropped with it).
pub static SERVICES_MADE: AtomicUsize = AtomicUsize::new(0);
/// See [`SERVICES_MADE`].
pub static SERVICES_DROPPED: AtomicUsize = AtomicUsize::new(0);

/// The value of a test switch of the check (a real host sets none of them).
fn switch(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The folder of one document below the cache folder. `DS_ARA_FOLDER_FAULT=shared` makes
/// every document use one folder (the check's fault run of the two-documents line).
#[must_use]
pub fn folder_of(cache: &Path, id: &str) -> PathBuf {
    let name = if switch("DS_ARA_FOLDER_FAULT").as_deref() == Some("shared") {
        "shared"
    } else {
        id
    };
    cache.join("documents").join(name)
}

/// Where the audio of the sources of a document is cached.
#[must_use]
pub fn audio_dir_of(cache: &Path, id: &str) -> PathBuf {
    folder_of(cache, id).join("audio")
}

/// The cached WAV of one source: `<seq>-<stem>.wav` in the audio folder of the document.
#[must_use]
pub fn wav_of(cache: &Path, id: &str, seq: usize, name: &str) -> PathBuf {
    audio_dir_of(cache, id).join(format!("{seq:03}-{}.wav", stem_of(name)))
}

/// A new document id: 64 random bits as hex (the plugin makes it once per document; the
/// archive stores it).
#[must_use]
pub fn new_doc_id() -> String {
    use std::io::Read;
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let mut bytes = [0u8; 8];
    let got = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .is_ok();
    if !got {
        // no random device: the clock, the process and a counter still differ per document
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let mix = nanos ^ (u64::from(std::process::id()) << 32) ^ (COUNTER.fetch_add(1, Ordering::SeqCst) as u64);
        bytes = mix.wrapping_mul(0x9E37_79B9_7F4A_7C15).to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One playback region of a source, as the document controller recorded it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionRec {
    /// `startInModificationTime`.
    pub start_mod: f64,
    /// `durationInModificationTime`.
    pub dur_mod: f64,
    /// `startInPlaybackTime` (carried for the renderer; the analysis does not use it).
    pub start_play: f64,
}

/// What the job needs from one source (a snapshot, taken under the plugin's lock).
#[derive(Debug, Clone)]
pub struct SourceSnapshot {
    /// The creation index of the source (stable key for the results, unique per document).
    pub seq: usize,
    /// The ARA name of the source (its file path in the test host).
    pub name: String,
    /// The persistent ID of the source.
    pub persistent_id: String,
    /// Sample rate.
    pub sample_rate: f64,
    /// Sample count.
    pub sample_count: i64,
    /// The cached WAV file of the source (written by the reader thread).
    pub wav: PathBuf,
    /// The playback regions of the source.
    pub regions: Vec<RegionRec>,
}

/// What one analysis round produced.
#[derive(Debug, Clone)]
pub struct JobResult {
    /// The events of every track of the project, `{track: [events]}`, as the service gave
    /// them (the bytes the check compares).
    pub events: Map<String, Value>,
    /// The decode counter of the service (per file).
    pub decode_counts: HashMap<PathBuf, usize>,
    /// The analysis wall time of the round.
    pub analysis_ms: u64,
    /// The notes per source of the round: `(sample in the source, volume)` per event.
    pub notes: HashMap<usize, Vec<[f64; 2]>>,
    /// The track name of every source of the round.
    pub track_of: HashMap<usize, String>,
    /// The tracks the round added, in order.
    pub added: Vec<String>,
    /// The tracks of the project after the round.
    pub project_tracks: Vec<String>,
    /// Tracks of earlier rounds whose events differ after this one (the gate: 0).
    pub earlier_changed: usize,
}

/// One source the archive restore brings back: its snapshot, its track name and the stored
/// events of that track.
#[derive(Debug, Clone)]
pub struct RestoredTrack {
    /// The source (its WAV is where the reader would write it; nothing is decoded).
    pub snapshot: SourceSnapshot,
    /// The track name.
    pub track: String,
    /// The stored events of the track.
    pub events: Value,
}

/// What a restore produced.
#[derive(Debug, Clone)]
pub struct RestoreResult {
    /// The events of every track as the open project holds them.
    pub events: Map<String, Value>,
    /// The tracks of the project.
    pub project_tracks: Vec<String>,
    /// The decode counter of the service after the restore (the gate: empty).
    pub decode_counts: HashMap<PathBuf, usize>,
}

/// The service and the project of one document.
pub struct DocService {
    cache: PathBuf,
    id: String,
    svc: Option<Service>,
    sid: Option<String>,
    /// The tracks of the project in order: the track name and the creation index of its source.
    tracks: Vec<(String, usize)>,
    /// The events of every track as of the last round (to prove that a later round leaves
    /// them alone).
    events: Map<String, Value>,
    canceller: Canceller,
}

/// The home folder of the service of a document: inside the document's folder, so inside
/// the cache folder. `DS_ARA_HOME_FAULT=1` puts it below the user's home instead (the
/// check's fault run of the home line).
fn home_of(cache: &Path, id: &str) -> PathBuf {
    if switch("DS_ARA_HOME_FAULT").as_deref() == Some("1") {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
            .join("Library")
            .join("Application Support")
            .join("DrumSpike")
    } else {
        folder_of(cache, id).join("service")
    }
}

/// A new service with its home inside the document's folder.
fn make_service(cache: &Path, id: &str) -> Service {
    let svc = Service::new(ServiceConfig::from_env(Path::new("/")));
    SERVICES_MADE.fetch_add(1, Ordering::SeqCst);
    point_service(&svc, cache, id);
    svc
}

/// Point the service at the folder of the document: its home (the recent list) and the
/// default folder of new projects (never used: the plugin writes the document itself).
fn point_service(svc: &Service, cache: &Path, id: &str) {
    svc.set_home(home_of(cache, id));
    svc.set_projects_dir(folder_of(cache, id).join("projects"));
}

/// How the thread that destroys a controller reaches the running job of its project: the
/// worker holds the document while it analyses.
#[derive(Clone, Default)]
pub struct Canceller(std::sync::Arc<std::sync::Mutex<Option<(Service, String)>>>);

impl Canceller {
    /// Ask the running analysis of the project to stop (a no-op without a project).
    pub fn cancel(&self) {
        let live = self.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some((svc, sid)) = live {
            let _ = svc.job_cancel(&sid);
        }
    }

    fn set(&self, live: Option<(Service, String)>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = live;
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl DocService {
    /// The service of a new document (made with the document controller).
    #[must_use]
    pub fn new(cache: PathBuf, id: String) -> DocService {
        let svc = make_service(&cache, &id);
        DocService {
            cache,
            id,
            svc: Some(svc),
            sid: None,
            tracks: Vec::new(),
            events: Map::new(),
            canceller: Canceller::default(),
        }
    }

    /// The handle that cancels the running job from another thread.
    #[must_use]
    pub fn canceller(&self) -> Canceller {
        self.canceller.clone()
    }

    /// The document id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The folder of the document.
    #[must_use]
    pub fn folder(&self) -> PathBuf {
        folder_of(&self.cache, &self.id)
    }

    /// The project folder of the document.
    #[must_use]
    pub fn project_dir(&self) -> PathBuf {
        self.folder().join("project")
    }

    /// The home folder of the service.
    #[must_use]
    pub fn home(&self) -> PathBuf {
        self.svc
            .as_ref()
            .and_then(Service::home)
            .unwrap_or_else(|| self.folder().join("service"))
    }

    /// The tracks of the project, in order.
    #[must_use]
    pub fn track_names(&self) -> Vec<String> {
        self.tracks.iter().map(|(n, _)| n.clone()).collect()
    }

    /// The track of a source (by its creation index).
    #[must_use]
    pub fn track_of(&self, seq: usize) -> Option<String> {
        self.tracks.iter().find(|(_, s)| *s == seq).map(|(n, _)| n.clone())
    }

    /// Take the id the archive stored (a restore): the document keeps that folder from now
    /// on. The project of the old id, when it was open, is closed first.
    pub fn set_id(&mut self, id: &str) {
        if id == self.id {
            return;
        }
        self.close_project();
        self.id = id.to_string();
        // the service stays the one made with the controller; its home follows the id
        if let Some(svc) = &self.svc {
            point_service(svc, &self.cache, id);
        }
    }

    fn service(&self) -> Result<Service, String> {
        self.svc.clone().ok_or_else(|| "the document is closed".to_string())
    }

    /// Open the project of the document (an empty one when the folder has none yet).
    fn ensure_open(&mut self) -> Result<String, String> {
        if let Some(sid) = &self.sid {
            return Ok(sid.clone());
        }
        let svc = self.service()?;
        let dir = self.project_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let doc_path = document_path(&dir);
        if !doc_path.exists() {
            write_document(&empty_document(&dir), &doc_path).map_err(|e| format!("{}: {e}", doc_path.display()))?;
        }
        let opened = svc.project_open(&dir.display().to_string()).map_err(err)?;
        let sid = opened["sid"].as_str().unwrap_or_default().to_string();
        self.sid = Some(sid.clone());
        self.canceller.set(Some((svc, sid.clone())));
        Ok(sid)
    }

    /// The names the tracks of a round get: the file stem, made unique within the project.
    fn names_for(&self, sources: &[SourceSnapshot]) -> Vec<String> {
        let mut taken: Vec<String> = self.tracks.iter().map(|(n, _)| n.to_lowercase()).collect();
        let mut out = Vec::with_capacity(sources.len());
        for (i, s) in sources.iter().enumerate() {
            let base = safe_name(&stem_of(&s.name), self.tracks.len() + i);
            let mut name = base.clone();
            let mut k = 1;
            while taken.contains(&name.to_lowercase()) {
                k += 1;
                name = format!("{base}_{k}");
            }
            taken.push(name.to_lowercase());
            out.push(name);
        }
        out
    }

    /// One analysis round: add the sources as tracks of the open project, analyse those
    /// tracks only, read the events of every track, save the project. A track of an
    /// earlier round is not analysed again and its events stay as they are.
    ///
    /// # Errors
    ///
    /// A message when the project cannot be opened, a track cannot be added, the service
    /// refuses the analysis or the analysis is cancelled.
    pub fn analyse(&mut self, sources: &[SourceSnapshot]) -> Result<JobResult, String> {
        let started = std::time::Instant::now();
        if switch("DS_ARA_SERVICE_FAULT").as_deref() == Some("per-job") {
            // the fault run: a new service for every job (the M1a way)
            self.close_service();
            self.svc = Some(make_service(&self.cache, &self.id));
        }
        let sid = self.ensure_open()?;
        let svc = self.service()?;
        if switch("DS_ARA_TRACKS_FAULT").as_deref() == Some("forget") && !self.tracks.is_empty() {
            // the fault run: the earlier tracks leave the project (the M1a way)
            let names = self.track_names();
            svc.tracks_remove(&sid, &names, false).map_err(err)?;
            self.tracks.clear();
            self.events.clear();
        }
        let dir = self.project_dir();
        let names = self.names_for(sources);
        let rows: Vec<Value> = sources
            .iter()
            .zip(&names)
            .map(|(s, name)| json!({"name": name, "track": serde_json::to_value(make_track(s, &dir)).unwrap_or(Value::Null)}))
            .collect();
        svc.tracks_restore(&sid, &Value::Array(rows)).map_err(err)?;
        let mut track_of = HashMap::new();
        for (s, name) in sources.iter().zip(&names) {
            self.tracks.push((name.clone(), s.seq));
            track_of.insert(s.seq, name.clone());
        }
        svc.analyze_start(&sid, Some(names.clone()), None, None).map_err(err)?;
        loop {
            let st = svc.analyze_status(&sid);
            match st["state"].as_str() {
                Some("done") => break,
                Some("cancelled" | "cancelling") => return Err("the analysis was cancelled".into()),
                _ => std::thread::sleep(std::time::Duration::from_millis(5)),
            }
        }
        let mut all = Map::new();
        for (name, _) in &self.tracks {
            let ev = svc.events_get(&sid, name).map_err(err)?;
            all.insert(name.clone(), ev["events"].clone());
        }
        let earlier_changed = self
            .events
            .iter()
            .filter(|(n, v)| all.get(n.as_str()) != Some(*v))
            .count();
        self.events = all.clone();
        svc.project_save(&sid).map_err(err)?;
        let analysis_ms = started.elapsed().as_millis() as u64;

        // the notes: the events of the track per source, mapped back into the source
        let mut notes: HashMap<usize, Vec<[f64; 2]>> = HashMap::new();
        for s in sources {
            let list = all
                .get(&track_of[&s.seq])
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut per_source: Vec<[f64; 2]> = Vec::with_capacity(list.len());
            for ev in &list {
                let t = ev.get("t").and_then(Value::as_f64).unwrap_or(f64::NAN);
                let v = ev.get("v").and_then(Value::as_i64).unwrap_or(100);
                if let Some(sample) = source_sample(t, &s.regions, s.sample_rate) {
                    per_source.push([sample as f64, (v.clamp(0, 127) as f64) / 127.0]);
                }
            }
            notes.insert(s.seq, per_source);
        }
        Ok(JobResult {
            events: all,
            decode_counts: svc.decode_counts(),
            analysis_ms,
            notes,
            track_of,
            added: names,
            project_tracks: self.track_names(),
            earlier_changed,
        })
    }

    /// The track of a destroyed source leaves the project (and the project is saved).
    /// Answers the tracks that are left.
    ///
    /// # Errors
    ///
    /// A message when the service refuses the removal or the save.
    pub fn remove(&mut self, seq: usize) -> Result<Vec<String>, String> {
        let Some(name) = self.track_of(seq) else {
            return Ok(self.track_names());
        };
        if let (Some(svc), Some(sid)) = (&self.svc, &self.sid) {
            svc.tracks_remove(sid, std::slice::from_ref(&name), false)
                .map_err(err)?;
            svc.project_save(sid).map_err(err)?;
        }
        self.tracks.retain(|(n, _)| *n != name);
        self.events.shift_remove(&name);
        Ok(self.track_names())
    }

    /// Restore: write the project document from the sources and the stored events, open it.
    /// No audio file is decoded (the decode counter of the service is part of the answer).
    ///
    /// # Errors
    ///
    /// A message when the document cannot be written or opened.
    pub fn restore(&mut self, tracks: &[RestoredTrack]) -> Result<RestoreResult, String> {
        self.close_project();
        let dir = self.project_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut doc = empty_document(&dir);
        for t in tracks {
            let mut track = serde_json::to_value(make_track(&t.snapshot, &dir)).map_err(err)?;
            track["events"] = t.events.clone();
            // what the analysis would have recorded about the file (no decode needed)
            track["sr"] = json!(t.snapshot.sample_rate as i64);
            track["n_samples"] = json!(t.snapshot.sample_count);
            track["duration"] = json!(t.snapshot.sample_count as f64 / t.snapshot.sample_rate.max(1.0));
            let track: Track = serde_json::from_value(track).map_err(err)?;
            doc.order.get_or_insert_with(Vec::new).push(t.track.clone());
            doc.tracks_mut().insert(t.track.clone(), track);
        }
        let doc_path = document_path(&dir);
        write_document(&doc, &doc_path).map_err(|e| format!("{}: {e}", doc_path.display()))?;
        let sid = self.ensure_open()?;
        let svc = self.service()?;
        self.tracks = tracks.iter().map(|t| (t.track.clone(), t.snapshot.seq)).collect();
        let mut events = Map::new();
        for (name, _) in &self.tracks {
            let ev = svc.events_get(&sid, name).map_err(err)?;
            events.insert(name.clone(), ev["events"].clone());
        }
        self.events = events.clone();
        Ok(RestoreResult {
            events,
            project_tracks: self.track_names(),
            decode_counts: svc.decode_counts(),
        })
    }

    /// Save the project when it has unsaved changes, then close it.
    fn close_project(&mut self) {
        self.canceller.set(None);
        if let (Some(svc), Some(sid)) = (&self.svc, self.sid.take()) {
            if svc.project_get(&sid).is_ok_and(|p| p["dirty"] == true) {
                let _ = svc.project_save(&sid);
            }
            let _ = svc.project_close(&sid, true);
        }
    }

    fn close_service(&mut self) {
        self.close_project();
        if self.svc.take().is_some() {
            SERVICES_DROPPED.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// The document controller ends: save and close the project, drop the service.
    pub fn shutdown(&mut self) {
        self.close_service();
    }
}

/// The document of a project with no track yet.
fn empty_document(dir: &Path) -> Document {
    Document {
        format: FORMAT,
        name: Some(
            dir.file_name()
                .map_or_else(|| "ara".into(), |s| s.to_string_lossy().into_owned()),
        ),
        created: None,
        modified: None,
        settings: None,
        order: Some(Vec::new()),
        tracks: Some(indexmap::IndexMap::new()),
        view: None,
        extra: Default::default(),
    }
}

/// The track of one source: the role from the name, the whole file or the playback regions
/// as segments.
fn make_track(s: &SourceSnapshot, dir: &Path) -> Track {
    let stem = stem_of(&s.name);
    let role = guess_role(&stem).unwrap_or("other");
    let segments: Option<Vec<Segment>> = if s.regions.is_empty() {
        None
    } else {
        Some(
            s.regions
                .iter()
                .map(|r| Segment {
                    start: r.start_mod,
                    offset: r.start_mod,
                    length: r.dur_mod,
                    source: Some(Some(wav_source(&s.wav, dir))),
                    extra: Default::default(),
                })
                .collect(),
        )
    };
    Track {
        role: Some(role.to_string()),
        note: Some(default_note(role)),
        mix_mode: Some("average".into()),
        source: segments.is_none().then(|| Some(wav_source(&s.wav, dir))),
        events: Some(Vec::new()),
        segments,
        ..Track::default()
    }
}

/// The file stem of a source name (the ARA name of the test host is the file path).
#[must_use]
pub fn stem_of(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map_or_else(|| name.to_string(), |s| s.to_string_lossy().into_owned())
}

/// The source sample of a timeline event `t`. The track's timeline is the source's own
/// timeline (each region places the slice `[startInModificationTime, ..)` at its own
/// modification position), so the event's sample is `t * rate`; a region must hold `t`
/// (a hit in a gap or a trimmed-away part does not exist).
#[must_use]
pub fn source_sample(t: f64, regions: &[RegionRec], rate: f64) -> Option<i64> {
    let in_a_region = regions.iter().any(|r| t >= r.start_mod && t < r.start_mod + r.dur_mod);
    if !regions.is_empty() && !in_a_region {
        return None;
    }
    Some((t * rate).round() as i64)
}

/// The source record of a cached WAV (size and head hash included, so the service
/// resolves it).
fn wav_source(wav: &Path, project_dir: &Path) -> Source {
    build_source(wav, project_dir, "referenced").unwrap_or_else(|_| Source {
        mode: Some("referenced".into()),
        path: Some(wav.display().to_string()),
        ..Source::default()
    })
}

/// The events object as one JSON line value (the bytes the check compares).
#[must_use]
pub fn events_line(events: &serde_json::Map<String, Value>) -> String {
    serde_json::to_string(&json!(events)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_sample_maps_the_timeline_back_into_the_file() {
        let regions = vec![
            RegionRec {
                start_mod: 0.0,
                dur_mod: 1.0,
                start_play: 0.0,
            },
            RegionRec {
                start_mod: 2.0,
                dur_mod: 0.5,
                start_play: 5.0,
            },
        ];
        let rate = 48_000.0;
        // the timeline is the source's own: an event at t sits at sample t * rate
        assert_eq!(source_sample(0.25, &regions, rate), Some(12_000));
        assert_eq!(source_sample(2.1, &regions, rate), Some(100_800));
        // a gap between the regions has no audio
        assert_eq!(source_sample(1.5, &regions, rate), None);
        // without regions the whole file exists
        assert_eq!(source_sample(0.25, &[], rate), Some(12_000));
    }

    /// A mono 48 kHz WAV with decaying 60 Hz bursts at `hits` (seconds), `secs` long.
    fn write_drum_wav(path: &Path, hits: &[f64], secs: f64) {
        let rate = 48_000u32;
        let n = (f64::from(rate) * secs) as usize;
        let mut w = crate::wav::WavWriter::create(path, rate, 1, n).unwrap();
        let mut lcg = 12345u32;
        let samples: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(rate);
                let mut v = 0.0f64;
                for h in hits {
                    let dt = t - h;
                    if (0.0..0.25).contains(&dt) {
                        v += (2.0 * std::f64::consts::PI * 60.0 * dt).sin() * (-dt * 30.0).exp() * 0.9;
                    }
                }
                lcg = lcg.wrapping_mul(1_103_515_245).wrapping_add(12_345) & 0x7fff_ffff;
                v += (f64::from(lcg) / f64::from(0x7fff_ffffu32) - 0.5) * 0.003;
                v as f32
            })
            .collect();
        w.write_interleaved_f32(&samples).unwrap();
        w.commit().unwrap();
    }

    fn snapshot_of(cache: &Path, id: &str, seq: usize, name: &str) -> SourceSnapshot {
        let wav = wav_of(cache, id, seq, name);
        write_drum_wav(&wav, &[0.2, 0.9, 1.5], 2.5);
        SourceSnapshot {
            seq,
            name: name.into(),
            persistent_id: String::new(),
            sample_rate: 48_000.0,
            sample_count: 120_000,
            wav,
            regions: Vec::new(),
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("drumspike-ara-test-{tag}-{}", new_doc_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn document_ids_and_folders_are_made_apart() {
        let cache = Path::new("/c");
        let (a, b) = (new_doc_id(), new_doc_id());
        assert_ne!(a, b);
        assert_eq!(a.len(), 16);
        assert_ne!(folder_of(cache, &a), folder_of(cache, &b));
        assert_eq!(folder_of(cache, &a), cache.join("documents").join(&a));
        assert_ne!(wav_of(cache, &a, 0, "kick.wav"), wav_of(cache, &b, 0, "kick.wav"));
    }

    #[test]
    fn a_later_round_adds_tracks_and_leaves_the_events_of_the_earlier_ones() {
        let cache = scratch("rounds");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let first = doc.analyse(&[snapshot_of(&cache, &id, 0, "kick.wav")]).unwrap();
        assert_eq!(first.project_tracks, ["kick"]);
        let before = first.events["kick"].to_string();
        assert!(
            first.events["kick"].as_array().is_some_and(|e| !e.is_empty()),
            "the detector found hits"
        );
        let second = doc.analyse(&[snapshot_of(&cache, &id, 1, "snare.wav")]).unwrap();
        assert_eq!(second.project_tracks, ["kick", "snare"]);
        assert_eq!(second.earlier_changed, 0);
        assert_eq!(second.events["kick"].to_string(), before, "byte for byte");
        // the document on disk holds both tracks
        let text = std::fs::read_to_string(document_path(&doc.project_dir())).unwrap();
        let on_disk: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(on_disk["tracks"].as_object().unwrap().len(), 2);
        // a destroyed source leaves the project and the document
        assert_eq!(doc.remove(0).unwrap(), ["snare"]);
        let text = std::fs::read_to_string(document_path(&doc.project_dir())).unwrap();
        let on_disk: Value = serde_json::from_str(&text).unwrap();
        assert!(on_disk["tracks"].get("kick").is_none() && on_disk["tracks"].get("snare").is_some());
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn two_documents_keep_their_own_folders_and_numbering() {
        let cache = scratch("two");
        let (ida, idb) = (new_doc_id(), new_doc_id());
        let mut a = DocService::new(cache.clone(), ida.clone());
        let mut b = DocService::new(cache.clone(), idb.clone());
        let ra = a.analyse(&[snapshot_of(&cache, &ida, 0, "kick.wav")]).unwrap();
        let rb = b.analyse(&[snapshot_of(&cache, &idb, 0, "kick.wav")]).unwrap();
        assert_ne!(a.folder(), b.folder());
        assert!(wav_of(&cache, &ida, 0, "kick.wav").is_file() && wav_of(&cache, &idb, 0, "kick.wav").is_file());
        // the same audio in two documents gives the same events: nothing is shared
        assert_eq!(ra.events["kick"], rb.events["kick"]);
        assert_eq!(a.home(), a.folder().join("service"));
        a.shutdown();
        b.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_restore_opens_the_project_with_the_stored_events_and_decodes_nothing() {
        let cache = scratch("restore");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let snap = snapshot_of(&cache, &id, 0, "kick.wav");
        let stored = doc.analyse(std::slice::from_ref(&snap)).unwrap();
        doc.shutdown();
        // a new service on the same id (the new process): the WAV is gone, the events stay
        std::fs::remove_file(&snap.wav).unwrap();
        let mut again = DocService::new(cache.clone(), id.clone());
        let restored = again
            .restore(&[RestoredTrack {
                snapshot: snap,
                track: "kick".into(),
                events: stored.events["kick"].clone(),
            }])
            .unwrap();
        assert_eq!(restored.events["kick"].to_string(), stored.events["kick"].to_string());
        assert!(restored.decode_counts.is_empty(), "{:?}", restored.decode_counts);
        assert_eq!(restored.project_tracks, ["kick"]);
        again.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_name_that_exists_gets_a_suffix() {
        let cache = scratch("names");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let a = snapshot_of(&cache, &id, 0, "/x/kick.wav");
        let b = snapshot_of(&cache, &id, 1, "/y/kick.wav");
        // two files with one stem: the second track gets its own name
        assert_eq!(doc.analyse(&[a]).unwrap().project_tracks, ["kick"]);
        assert_eq!(doc.analyse(&[b]).unwrap().project_tracks, ["kick", "kick_2"]);
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn stem_of_takes_the_file_stem_of_a_path() {
        assert_eq!(stem_of("/a/b/kick_48k.wav"), "kick_48k");
        assert_eq!(stem_of("kick.wav"), "kick");
    }
}
