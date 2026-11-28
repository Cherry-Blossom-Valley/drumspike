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

use drumspike_app::roles::{default_note, guess_role, guess_role_matched};
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
    crate::test_switch(name).ok().filter(|v| !v.is_empty())
}

/// True for an id of exactly the form the plugin makes: 16 lowercase hex digits. The id
/// becomes a folder name, and it comes from a file the user may have damaged or edited: any
/// other value (a path, `..`, an empty string) is refused.
#[must_use]
pub fn valid_doc_id(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// [`valid_doc_id`], except under `DS_ARA_ID_FAULT=trust` (the check's fault run: the old
/// behaviour that trusted the archive).
#[must_use]
pub fn accept_doc_id(id: &str) -> bool {
    valid_doc_id(id) || (switch("DS_ARA_ID_FAULT").as_deref() == Some("trust") && !id.is_empty())
}

/// True for a track name that is safe to keep from an archive: not empty, no path
/// separator, no `..`, no control character, at most 200 bytes.
#[must_use]
pub fn valid_track_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.contains(['/', '\\', '\0'])
        && !name.contains("..")
        && !name.chars().any(char::is_control)
}

/// [`valid_track_name`], except under `DS_ARA_ID_FAULT=trust` (the check's fault run).
#[must_use]
pub fn accept_track_name(name: &str) -> bool {
    valid_track_name(name) || (switch("DS_ARA_ID_FAULT").as_deref() == Some("trust") && !name.is_empty())
}

/// The folder of one document below the cache folder. `DS_ARA_FOLDER_FAULT=shared` makes
/// every document use one folder (the check's fault run of the two-documents line). An id
/// that is not [`accept_doc_id`] never reaches the file system: it maps to one fixed name.
#[must_use]
pub fn folder_of(cache: &Path, id: &str) -> PathBuf {
    let name = if switch("DS_ARA_FOLDER_FAULT").as_deref() == Some("shared") {
        "shared"
    } else if accept_doc_id(id) {
        id
    } else {
        "refused-id"
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
    wav_of_gen(cache, id, seq, name, 0)
}

/// The WAV of one generation of the audio of a source: generation 0 is the first read, each
/// content update of the host makes a new one (the old file stays until the new one is committed).
#[must_use]
pub fn wav_of_gen(cache: &Path, id: &str, seq: usize, name: &str, gen: u64) -> PathBuf {
    let tag = if gen == 0 { String::new() } else { format!(".g{gen}") };
    audio_dir_of(cache, id).join(format!("{seq:03}-{}{tag}.wav", stem_of(name)))
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
    /// This process wrote the WAV (a reader committed it). A WAV left from an earlier
    /// session, or the path of a restored track, is not trusted.
    pub ready: bool,
    /// The size the committed WAV must have (header and samples); a file of another size is
    /// not the one the reader wrote.
    pub wav_len: u64,
    /// The generation of the audio (a content update of the host makes a new one).
    pub audio_gen: u64,
    /// The playback regions of the source.
    pub regions: Vec<RegionRec>,
    /// M1b10: the name the DAW shows for the track of the source (the name of the region sequence
    /// of the first region, else the name of that region), when the host gave one. The role guess
    /// reads it before the name of the audio source.
    pub label: Option<String>,
    /// #577: the `orderIndex` of the DAW track of the source (`i32::MAX`: unknown); new tracks of one round get start values in this order.
    pub order: i32,
}

impl SourceSnapshot {
    /// The role the DAW name gives (`guess_role` of the label), when it gives one.
    #[must_use]
    pub fn label_role(&self) -> Option<&'static str> {
        self.label.as_deref().and_then(guess_role)
    }

    /// The same source with the same regions, properties and generation of the audio (the WAV
    /// file is not compared). `self` is the older snapshot: a new name that gives no role is no
    /// change (the role stays), a new name that gives another role is one.
    #[must_use]
    pub fn same_as(&self, other: &SourceSnapshot) -> bool {
        self.same_but_label(other) && (other.label_role().is_none() || self.label_role() == other.label_role())
    }

    /// [`SourceSnapshot::same_as`] without the role that the DAW name gives.
    #[must_use]
    pub fn same_but_label(&self, other: &SourceSnapshot) -> bool {
        self.name == other.name
            && self.persistent_id == other.persistent_id
            && self.sample_rate == other.sample_rate
            && self.sample_count == other.sample_count
            && self.audio_gen == other.audio_gen
            && self.same_regions(other)
    }

    /// The regions the analysis uses: the modification range of each. A move in playback
    /// time alone (`start_play`) changes nothing in the events or the notes.
    /// `DS_ARA_TOUCH_FAULT=playback-time` (the check's fault run) compares it too.
    fn same_regions(&self, other: &SourceSnapshot) -> bool {
        let strict = switch("DS_ARA_TOUCH_FAULT").as_deref() == Some("playback-time");
        self.regions.len() == other.regions.len()
            && self.regions.iter().zip(&other.regions).all(|(a, b)| {
                a.start_mod == b.start_mod && a.dur_mod == b.dur_mod && (!strict || a.start_play == b.start_play)
            })
    }

    /// Why the audio of the source cannot be analysed now, or `None` when it can.
    #[must_use]
    pub fn audio_problem(&self) -> Option<&'static str> {
        if !self.ready {
            return Some("no audio yet (the host has not delivered it in this session)");
        }
        match std::fs::metadata(&self.wav) {
            Ok(m) if m.len() == self.wav_len => None,
            Ok(_) => Some("the cached audio is not the file the reader wrote (size)"),
            Err(_) => Some("the cached audio is missing"),
        }
    }
}

/// F6 (#582): what the repair of one restored track found and did (a row of the report).
#[derive(Debug, Clone)]
pub struct RepairRow {
    /// The track name.
    pub track: String,
    /// The status of the source before (`ok`, `changed`, `missing`).
    pub before: String,
    /// The status after.
    pub after: String,
    /// The record was made again (false: the track was online already).
    pub repointed: bool,
    /// The hits of the track as the service holds them, one JSON line.
    pub events_json: String,
    /// The role of the track.
    pub role: String,
    /// The note of the track (-1: none).
    pub note: i64,
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
    /// The notes of every source of the project: `(sample in the source, volume)` per hit.
    pub notes: HashMap<usize, Vec<[f64; 2]>>,
    /// The track name of every source added by the round.
    pub track_of: HashMap<usize, String>,
    /// The tracks the round added, in order.
    pub added: Vec<String>,
    /// The tracks the round analysed (together: every track whose audio is here).
    pub analysed: Vec<String>,
    /// The tracks of the project after the round.
    pub project_tracks: Vec<String>,
    /// Tracks that were not analysed in this round and whose events differ (the gate: 0).
    pub earlier_changed: usize,
    /// Tracks the service could not analyse (track, message): no events, no notes.
    pub errors: Vec<(String, String)>,
    /// Tracks left out of the round (track, why): the result is then partial.
    pub skipped: Vec<(String, String)>,
    /// Tracks with an error that keep the events of an earlier round (stale, not fresh).
    pub stale: Vec<String>,
    /// A round ran. False: nothing had changed (no new source, no changed source, nothing removed).
    pub ran: bool,
    /// F6: the repair of a restored track's record failed with a permanent error (the round went on).
    pub repair_error: Option<String>,
    /// The tracks of the project after the round with the creation index of their source.
    pub tracks: Vec<(String, usize)>,
    /// The regions and the rate each source of the project has in the snapshot the notes were
    /// made with (R6): the plugin maps the notes of a page edit with exactly these, also for a
    /// changed source without audio whose snapshot was kept.
    pub regions: HashMap<usize, (Vec<RegionRec>, f64)>,
    /// M1b10: the tracks whose role was asked of the audio in this round: (track, bytes read from the WAV, the
    /// role the hint gave).
    pub role_audio: Vec<(String, usize, Option<String>)>,
}

impl JobResult {
    /// The result is partial: a track was left out of the round (no audio here) or could not
    /// be analysed. The other tracks are analysed together; the left-out ones keep what the
    /// project had.
    #[must_use]
    pub fn partial(&self) -> bool {
        !self.skipped.is_empty() || !self.errors.is_empty()
    }
}

impl JobResult {
    /// The answer of a round that did nothing.
    fn noop(project_tracks: Vec<String>) -> JobResult {
        JobResult {
            events: Map::new(),
            decode_counts: HashMap::new(),
            analysis_ms: 0,
            notes: HashMap::new(),
            track_of: HashMap::new(),
            added: Vec::new(),
            analysed: Vec::new(),
            project_tracks,
            earlier_changed: 0,
            errors: Vec::new(),
            skipped: Vec::new(),
            stale: Vec::new(),
            ran: false,
            repair_error: None,
            tracks: Vec::new(),
            regions: HashMap::new(),
            role_audio: Vec::new(),
        }
    }
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
    /// M1b10: the settings of the user that the archive holds for the track (empty: the role is guessed).
    pub settings: Map<String, Value>,
}

/// The keys of a track answer of the service that are not settings of the user: what the service computes
/// (`status`, `offline`, `n_events`, `source_mode`, `layer_status`) and what the plugin makes from the
/// source on every restore (`sr`, `n_samples`, `duration`).
const NOT_SETTINGS: [&str; 8] = [
    "status",
    "offline",
    "n_events",
    "source_mode",
    "layer_status",
    "sr",
    "n_samples",
    "duration",
];

/// The keys of a track record that are settings of the user, in ONE place (the restore lays only these over
/// a track; a key of an archive that is in no list is dropped): the typed fields of the record and the keys
/// the app keeps in `extra` (`detector`, `learn`, `layer`, `sampler`; the shift is in `params`). The unit test
/// `every_key_of_a_track_answer_is_a_setting_or_computed` fails when the answer of the service gets a key that
/// is in neither this list nor [`NOT_SETTINGS`].
pub const SETTINGS: [&str; 14] = [
    "role",
    "role_source",
    "role_match",
    "note",
    "color",
    "params",
    "mix_mode",
    "volume_db",
    "pan",
    "follows",
    "detector",
    "learn",
    "layer",
    "sampler",
];

/// The settings of the tracks `names` as the service holds them now (N10), by track name: the answer of
/// `tracks_get` without the computed keys. The events, segments and source are not in it. `None` when the
/// service cannot answer (the caller keeps what it had).
pub fn settings_of(svc: &Service, sid: &str, names: &[String]) -> Option<Map<String, Value>> {
    let all = svc.tracks_get(sid).ok()?;
    let mut out = Map::new();
    for n in names {
        if let Some(obj) = all["tracks"][n.as_str()].as_object() {
            let kept: Map<String, Value> = obj
                .iter()
                .filter(|(k, _)| !NOT_SETTINGS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            out.insert(n.clone(), Value::Object(kept));
        }
    }
    Some(out)
}

/// The settings to keep after a read (R1): a failed read (`fresh` is None) keeps the old map; a track that is
/// missing from the answer keeps its old entry; an entry leaves only with its track (`names`: the tracks of the
/// document now).
#[must_use]
pub fn merge_settings(
    old: &Map<String, Value>,
    fresh: Option<Map<String, Value>>,
    names: &[String],
) -> Map<String, Value> {
    let Some(fresh) = fresh else {
        return old.clone();
    };
    let mut out = Map::new();
    for n in names {
        if let Some(v) = fresh.get(n).or_else(|| old.get(n)) {
            out.insert(n.clone(), v.clone());
        }
    }
    out
}

/// The type a setting must have (the record keeps a key of another type in `extra`, where it would shadow
/// the setting): a text, an integer, a number or an object. A key of another type is not laid over.
fn setting_type_ok(key: &str, v: &Value) -> bool {
    match key {
        "role" | "role_source" | "role_match" | "color" | "mix_mode" | "follows" => v.is_string(),
        "note" => v.is_i64(),
        "volume_db" | "pan" => v.is_number(),
        _ => v.is_object(),
    }
}

/// The settings of an archive laid over `base` (the record a track gets from its layout, or from a guess)
/// (R2): only the keys of [`SETTINGS`], and the role keys only when the archive holds a role. When the
/// result is not a valid track record (a wrong type in a hostile archive) there is no overlay (`None`).
fn overlay_settings(base: &Value, settings: &Map<String, Value>) -> Option<Value> {
    let has_role = settings.get("role").is_some_and(Value::is_string);
    let mut v = base.clone();
    for (k, val) in settings {
        if !SETTINGS.contains(&k.as_str()) || !setting_type_ok(k, val) {
            continue;
        }
        if !has_role && ["role", "note", "role_source", "role_match"].contains(&k.as_str()) {
            continue;
        }
        v[k.as_str()] = val.clone();
    }
    serde_json::from_value::<Track>(v.clone()).is_ok().then_some(v)
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
    /// The events of every track as of the last round.
    events: Map<String, Value>,
    /// The source of every track by creation index (its regions and rate map the events
    /// back into the source).
    snaps: HashMap<usize, SourceSnapshot>,
    /// A source left the project: the tracks that are left need a round.
    dirty: bool,
    /// F6: a repointed record is in the open project but the project was not saved yet (the save failed or is due).
    save_owed: bool,
    /// #586: a record was pointed at its new file and the call that did it ended with an error (the save was busy): the
    /// next call that ends well tells the page to read the project again, as the call that failed could not.
    reload_owed: bool,
    /// The test round that fails once (`DS_ARA_SCENARIO=fail-first-round`) has failed.
    failed_once: bool,
    /// How many rounds `DS_ARA_SCENARIO=fail-rounds:<n>` has failed.
    failed_rounds: usize,
    /// M1d4 (#578): the creation indexes of the sources whose tracks the next round analyses (a Retry of a failed track); `None`: every track.
    only: Option<Vec<usize>>,
    canceller: Canceller,
    /// The editor's way to the service (see [`PageHandle`]).
    page: std::sync::Arc<PageHandle>,
    /// A test aid: the next calls of `tracks_restore` in a round fail (R2).
    pub fail_restores: usize,
    /// #577 (Q4): the settings that the archive holds for a source that has no result yet (F5). The first round makes the new
    /// record of that track and lays them over it, so the values of the session win over any start value.
    kept: HashMap<usize, Map<String, Value>>,
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

/// The way the editor window reaches the service of a document (M1b2): a clone of the
/// service (it is one `Arc`, shared with the worker) and the log of its events. The worker
/// holds the `DocService` lock for a whole round, so the editor must never wait for that
/// lock: it asks this handle, which has a lock of its own that nobody holds for long.
#[derive(Default)]
pub struct PageHandle {
    svc: std::sync::Mutex<Option<Service>>,
    /// The sid of the document's open project (the editor and the worker ask whether a job of
    /// the page runs without taking the `DocService` lock).
    sid: std::sync::Mutex<Option<String>>,
    /// The events of the service (`project-dirty`, `track-layer`) as a numbered log; the page
    /// reads them with its own number, so an editor that opens later misses none it needs.
    pub log: std::sync::Arc<drumspike_app::bridge::EventLog>,
}

impl PageHandle {
    /// The service of the document, or `None` after the document closed.
    #[must_use]
    pub fn service(&self) -> Option<Service> {
        self.svc.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn set_sid(&self, sid: Option<String>) {
        *self.sid.lock().unwrap_or_else(|e| e.into_inner()) = sid;
    }

    /// True while an analysis job of the project is queued or running (the app allows one job
    /// at a time, and a job blocks the removal of tracks). Takes no `DocService` lock.
    #[must_use]
    pub fn job_running(&self) -> bool {
        let (Some(svc), Some(sid)) = (
            self.service(),
            self.sid.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        ) else {
            return false;
        };
        matches!(svc.analyze_status(&sid)["state"].as_str(), Some("queued" | "running"))
    }

    /// The sid of the document's open project.
    #[must_use]
    pub fn sid(&self) -> Option<String> {
        self.sid.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn set(&self, svc: Option<Service>) {
        *self.svc.lock().unwrap_or_else(|e| e.into_inner()) = svc;
    }
}

/// The page handle of an editor whose plugin instance is bound to no ARA document (a host
/// without ARA): a service of its own with no project, its home below the cache folder.
#[must_use]
pub fn unbound_page(cache: &Path) -> std::sync::Arc<PageHandle> {
    let page = std::sync::Arc::new(PageHandle::default());
    let svc = Service::new(ServiceConfig::from_env(Path::new("/")));
    let home = cache.join("editor-unbound");
    svc.set_home(home.join("service"));
    svc.set_projects_dir(home.join("projects"));
    let log = std::sync::Arc::clone(&page.log);
    svc.set_emitter(std::sync::Arc::new(move |name: &str, payload: Value| {
        log.push(name, payload)
    }));
    page.set(Some(svc));
    page
}

/// A new service with its home inside the document's folder; its events go to the log of
/// the page handle, and the handle holds a clone of it.
fn make_service(cache: &Path, id: &str, page: &PageHandle) -> Service {
    let svc = Service::new(ServiceConfig::from_env(Path::new("/")));
    SERVICES_MADE.fetch_add(1, Ordering::SeqCst);
    point_service(&svc, cache, id);
    let log = std::sync::Arc::clone(&page.log);
    svc.set_emitter(std::sync::Arc::new(move |name: &str, payload: Value| {
        log.push(name, payload)
    }));
    page.set(Some(svc.clone()));
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
pub struct Canceller(
    std::sync::Arc<std::sync::Mutex<Option<(Service, String)>>>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
);

impl Canceller {
    /// Ask the running analysis of the project to stop. The flag stays set: a round that has
    /// not started its analysis yet sees it and ends at once.
    pub fn cancel(&self) {
        self.1.store(true, Ordering::SeqCst);
        let live = self.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some((svc, sid)) = live {
            let _ = svc.job_cancel(&sid);
        }
    }

    fn cancelled(&self) -> bool {
        self.1.load(Ordering::SeqCst)
    }

    fn set(&self, live: Option<(Service, String)>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = live;
    }
}

/// A job of the page may start between the worker's check and its call (S1): the app answers
/// 409. The call is tried again for at most 1 s (R4: the worker holds the document lock meanwhile);
/// a 409 after that goes back to the worker, which puts the round or the removal back in the
/// queue like a job of the page.
fn retry_busy<T>(f: impl FnMut() -> Result<T, drumspike_app::AppError>) -> Result<T, drumspike_app::AppError> {
    call_busy(true, f)
}

/// [`retry_busy`] with the choice (#586): `retry` false makes ONE try and gives a 409 back at once. The repair of a
/// restored track uses that: it runs on the worker with the service lock held, so it must never sleep there; the
/// worker plans the next try itself, 100 ms later.
fn call_busy<T>(
    retry: bool,
    mut f: impl FnMut() -> Result<T, drumspike_app::AppError>,
) -> Result<T, drumspike_app::AppError> {
    let mut tries = 0;
    loop {
        #[cfg(test)]
        let answer = busy_test::call(&mut f);
        #[cfg(not(test))]
        let answer = f();
        match answer {
            Err(e)
                if retry
                    && e.code == 409
                    && tries < 10
                    && switch("DS_ARA_ROUND_FAULT").as_deref() != Some("no-page-wait") =>
            {
                tries += 1;
                #[cfg(test)]
                busy_test::SLEEPS.with(|c| c.set(c.get() + 1));
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            other => return other,
        }
    }
}

/// The test seam of the busy path: a service call that answers 409 as often as a test says (this thread only).
#[cfg(test)]
mod busy_test {
    use std::cell::Cell;
    thread_local! {
        /// Calls that still answer 409 (`u32::MAX`: for ever).
        pub static FORCE_BUSY: Cell<u32> = const { Cell::new(0) };
        /// Calls that pass before the forced answers begin.
        pub static SKIP: Cell<u32> = const { Cell::new(0) };
        /// Calls made (also the forced ones).
        pub static CALLS: Cell<u32> = const { Cell::new(0) };
        /// Sleeps of the retry loop.
        pub static SLEEPS: Cell<u32> = const { Cell::new(0) };
    }

    pub fn call<T>(f: &mut impl FnMut() -> Result<T, drumspike_app::AppError>) -> Result<T, drumspike_app::AppError> {
        CALLS.with(|c| c.set(c.get() + 1));
        if SKIP.with(|k| {
            let n = k.get();
            k.set(n.saturating_sub(1));
            n > 0
        }) {
            return f();
        }
        let busy = FORCE_BUSY.with(|b| {
            let n = b.get();
            if n > 0 && n != u32::MAX {
                b.set(n - 1);
            }
            n > 0
        });
        if busy {
            return Err(drumspike_app::AppError::conflict("a job of the page is running"));
        }
        f()
    }

    pub fn reset(force: u32) {
        reset_after(0, force);
    }

    /// The first `skip` calls pass, then `force` calls answer 409.
    pub fn reset_after(skip: u32, force: u32) {
        SKIP.with(|k| k.set(skip));
        FORCE_BUSY.with(|b| b.set(force));
        CALLS.with(|c| c.set(0));
        SLEEPS.with(|c| c.set(0));
    }
}

/// The app answers a track it does not have with a 400 "unknown track" (a 404 for a project it
/// does not have).
fn is_missing_track(e: &drumspike_app::AppError) -> bool {
    e.code == 404 || (e.code == 400 && e.message.contains("unknown track"))
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl DocService {
    /// The service of a new document (made with the document controller).
    #[must_use]
    pub fn new(cache: PathBuf, id: String) -> DocService {
        let page = std::sync::Arc::new(PageHandle::default());
        let svc = make_service(&cache, &id, &page);
        DocService {
            cache,
            id,
            svc: Some(svc),
            sid: None,
            tracks: Vec::new(),
            events: Map::new(),
            snaps: HashMap::new(),
            dirty: false,
            save_owed: false,
            reload_owed: false,
            failed_once: false,
            failed_rounds: 0,
            only: None,
            canceller: Canceller::default(),
            page,
            fail_restores: 0,
            kept: HashMap::new(),
        }
    }

    /// #577 (Q4, S7, S8): the settings that the document holds for a source (by its creation index) that has no track in the project yet: a source
    /// the archive holds with no result, or a source whose track record was lost in a round (`forget_track`). The caller replaces the whole map
    /// before each round; `analyse` lays an entry over the new record of exactly that source and drops the entries of sources that are gone.
    /// A source that is not in the map (a new clip, a source that the DAW removed and made again) gets the user defaults.
    pub fn only_sources(&mut self, only: Option<Vec<usize>>) {
        self.only = only;
    }

    pub fn keep_settings(&mut self, kept: HashMap<usize, Map<String, Value>>) {
        self.kept = kept;
    }

    /// The handle the editor window uses to reach the service without the document lock.
    #[must_use]
    pub fn page_handle(&self) -> std::sync::Arc<PageHandle> {
        std::sync::Arc::clone(&self.page)
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
        self.page.set_sid(Some(sid.clone()));
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

    /// M1b10: a source whose only change is the role that its DAW name gives is no change when the track
    /// record would keep its role: the user set it (`role_source` is `user`) or it is the role already. The
    /// new snapshot is kept at once, so such a name starts no analysis (the touch ends as a round that does
    /// nothing). The record is read with `tracks_get`; when it cannot be read the source stays changed.
    fn without_role_noops<'a>(&mut self, changed: Vec<&'a SourceSnapshot>) -> Vec<&'a SourceSnapshot> {
        let role_only =
            |me: &DocService, s: &SourceSnapshot| me.snaps.get(&s.seq).is_some_and(|old| old.same_but_label(s));
        if !changed.iter().any(|s| role_only(self, s)) {
            return changed;
        }
        let (Some(sid), Some(svc)) = (self.sid.clone(), self.svc.clone()) else {
            return changed;
        };
        let Ok(all) = svc.tracks_get(&sid) else {
            return changed;
        };
        let mut keep = Vec::with_capacity(changed.len());
        for s in changed {
            if role_only(self, s) {
                let track = self.tracks.iter().find(|(_, q)| *q == s.seq).map(|(n, _)| n.clone());
                let rec = track.as_ref().map(|n| &all["tracks"][n.as_str()]);
                let blocked = rec.is_some_and(|r| {
                    (r["role_source"].as_str() == Some("user")
                        && switch("DS_ARA_NAMES_FAULT").as_deref() != Some("role-hand"))
                        || s.label_role().is_none_or(|new| r["role"].as_str() == Some(new))
                });
                if blocked {
                    self.snaps.insert(s.seq, s.clone());
                    continue;
                }
            }
            keep.push(s);
        }
        keep
    }

    /// One analysis round over the sources of the document in `snaps` (every source that took
    /// part in an analysis, with a fresh snapshot): a source without a track gets one (a
    /// source is one track for its life, keyed by its creation index, so a retry never makes
    /// a second track), a source whose regions or properties changed gets its track record
    /// made again, then ALL tracks whose audio is here are analysed together, the events of
    /// every track are read and the project is saved.
    ///
    /// The rule: whenever the set of analysed tracks of a document changes, the result of all
    /// tracks is the result of one analysis of all those tracks together (the service
    /// reference). The session steps of the service (cross bleed, ghost gate, tom bleed,
    /// flam) run over the tracks of one job only, so a job over the new tracks alone would
    /// never let a track see the other microphones. The decode is cached by the service.
    ///
    /// A track whose audio is not here (a track restored from the archive whose source the
    /// host has not delivered again, or a WAV that is not the one the reader wrote) is left
    /// out and named in `skipped`: the result is partial and says so. A track the service
    /// cannot analyse is named in `errors` and has no events and no notes.
    ///
    /// Without `force` the round does nothing when nothing changed (`ran` is false).
    ///
    /// # Errors
    ///
    /// A message when the project cannot be opened, a track cannot be added, the service
    /// refuses the analysis or the analysis is cancelled. The tracks stay; the next round
    /// takes them again.
    pub fn analyse(&mut self, snaps: &[SourceSnapshot], force: bool) -> Result<JobResult, String> {
        let started = std::time::Instant::now();
        // M1d4: a Retry names the sources it is for; the other tracks are not analysed again (the fault run `retry-all-tracks` ignores it)
        let only = self
            .only
            .take()
            .filter(|_| switch("DS_ARA_FIRSTOPEN_FAULT").as_deref() != Some("retry-all-tracks"));
        if self.canceller.cancelled() {
            return Err("the analysis was cancelled before it started".into());
        }
        if switch("DS_ARA_SERVICE_FAULT").as_deref() == Some("per-job") {
            // the fault run: a new service for every job (the M1a way)
            self.close_service();
            self.svc = Some(make_service(&self.cache, &self.id, &self.page));
        }
        // F6 (#582): a restored track whose audio is read now gets the record of the new cache file first
        // (the round may find nothing else to do, and an analysis of an offline track fails)
        let ready: Vec<(usize, PathBuf)> = snaps
            .iter()
            .filter(|s| s.ready && s.audio_problem().is_none())
            .map(|s| (s.seq, s.wav.clone()))
            .collect();
        let mut repair_error = None;
        if !ready.is_empty() && self.snaps.values().any(|o| !o.ready) {
            match self.repoint(&ready) {
                Ok(_) => {}
                // a job of the page is running: the round is deferred like any other 409
                Err(e) if e.starts_with("409") => return Err(e),
                // anything else: the round goes on (the track may be analysed by another record); the report says so
                Err(e) => repair_error = Some(e),
            }
        }
        let duplicate = switch("DS_ARA_RETRY_FAULT").as_deref() == Some("duplicate");
        let known: Vec<usize> = self.tracks.iter().map(|(_, s)| *s).collect();
        let new: Vec<&SourceSnapshot> = snaps.iter().filter(|s| duplicate || !known.contains(&s.seq)).collect();
        let changed: Vec<&SourceSnapshot> = snaps
            .iter()
            .filter(|s| !duplicate && self.snaps.get(&s.seq).is_some_and(|old| !old.same_as(s)))
            .collect();
        let changed = self.without_role_noops(changed);
        if !force && new.is_empty() && changed.is_empty() && !self.dirty {
            let mut idle = JobResult::noop(self.track_names());
            idle.repair_error = repair_error;
            return Ok(idle);
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
        // a changed source: its track record is made again (the new segments), in its place
        for s in &changed {
            let Some(at) = self.tracks.iter().position(|(_, seq)| *seq == s.seq) else {
                continue;
            };
            if s.audio_problem().is_some() {
                // a track without audio here (restored, not delivered yet) keeps its stored
                // events: its record is not made again (that would drop them); the notes
                // follow the new regions through the snapshot
                continue;
            }
            let name = self.tracks[at].0.clone();
            // M1: the track record is made again with the new segments, and EVERYTHING the user
            // set on the track goes with it: the removal answers the whole old record (hits, role,
            // note, mix, params, Learn data, layer, follow, every other key), and only its
            // `segments` and `source` are replaced by the new ones. Hits that now lie outside the
            // new regions stay in the event list (as the app keeps them); the notes leave them out.
            let removed = match retry_busy(|| svc.tracks_remove(&sid, std::slice::from_ref(&name), true)) {
                Ok(v) => v,
                Err(e) if is_missing_track(&e) => {
                    // R2: the project has no such track (it went behind the plugin's back): the
                    // source is a new track at the next round
                    self.forget_track(&name, s.seq);
                    return Err(format!(
                        "the track {name} is not in the project any more; it is made again at the next round"
                    ));
                }
                Err(e) => return Err(err(e)),
            };
            // the snapshot row of the app as a whole (name, index, order_index, track, followers):
            // only the track changes, so a new field of the app's snapshot is carried too (R1)
            let old_row = removed["snapshot"][0].clone();
            let mut track = old_row["track"].clone();
            let fresh = serde_json::to_value(track_layout(s, &dir)).unwrap_or(Value::Null);
            let drop_data = switch("DS_ARA_EDITOR_FAULT").as_deref() == Some("drop-track-data");
            if drop_data {
                track = fresh; // the fault run: the old behaviour, the hand data are gone
            } else if let Some(obj) = track.as_object_mut() {
                // what the analysis of this round writes again (the audio may have changed)
                for key in ["sr", "n_samples", "duration"] {
                    obj.remove(key);
                }
                for key in ["segments", "source"] {
                    match fresh.get(key) {
                        Some(v) => {
                            obj.insert(key.to_string(), v.clone());
                        }
                        None => {
                            obj.remove(key);
                        }
                    }
                }
                // M1b10: the DAW name gives another role than the track has, and the user did not
                // set the role (`role_source` is `user` only for a role set by hand in the page):
                // the role is guessed again, with its default note when the note is still the
                // default note of the old role
                apply_label_role(obj, s);
            }
            let mut row = if drop_data {
                json!({"name": name, "index": at, "order_index": at})
            } else {
                old_row.clone()
            };
            row["track"] = track;
            let restored = if self.fail_restores > 0 {
                self.fail_restores -= 1;
                Err(drumspike_app::AppError::internal("test: the restore fails"))
            } else {
                svc.tracks_restore(&sid, &Value::Array(vec![row]))
            };
            if let Err(e) = restored {
                // R2: the track is out of the project: the OLD record goes back; when that fails
                // too, the track leaves the plugin's list, so that later rounds work
                let back = if self.fail_restores > 0 {
                    self.fail_restores -= 1;
                    Err(drumspike_app::AppError::internal("test: the restore fails"))
                } else {
                    svc.tracks_restore(&sid, &Value::Array(vec![old_row]))
                };
                return Err(match back {
                    Ok(_) => format!("the new record of {name} was refused ({e}); the old record is back"),
                    Err(e2) => {
                        self.forget_track(&name, s.seq);
                        format!("the record of {name} could not be made again ({e}) nor put back ({e2}); the track left the plugin's list")
                    }
                });
            }
            if drop_data {
                self.events.shift_remove(&name);
            }
        }
        let new_sources: Vec<SourceSnapshot> = new.iter().map(|s| (*s).clone()).collect();
        let names = self.names_for(&new_sources);
        // a new track takes its place by the creation index of its source, so the order of the
        // project (and of the events) is the order of the sources however the requests came
        let mut placed: Vec<(String, usize)> = self.tracks.clone();
        let mut rows: Vec<Value> = Vec::with_capacity(new_sources.len());
        let mut track_of = HashMap::new();
        let mut role_audio: Vec<(String, usize, Option<String>)> = Vec::new();
        // #577: ONE snapshot of the user defaults for the round, and the notes the tracks of each role use now
        if switch("DS_ARA_DEFAULTS_FAULT").as_deref() != Some("keep-gone") {
            self.kept.retain(|seq, _| snaps.iter().any(|s| s.seq == *seq)); // S7: a removed source leaves no values behind
        }
        let defaults = crate::userdefaults::current();
        let mut used: HashMap<String, Vec<i64>> = HashMap::new();
        if let Ok(all) = svc.tracks_get(&sid) {
            for (_, tr) in all["tracks"].as_object().into_iter().flatten() {
                if let (Some(r), Some(n)) = (tr["role"].as_str(), tr["note"].as_i64()) {
                    used.entry(r.to_string()).or_default().push(n);
                }
            }
        }
        let mut fresh: Vec<bool> = Vec::new();
        for (s, name) in new_sources.iter().zip(&names) {
            let at = placed.iter().position(|(_, q)| *q > s.seq).unwrap_or(placed.len());
            placed.insert(at, (name.clone(), s.seq));
            let (track, bytes) = make_track_audited(s, &dir);
            if let Some(b) = bytes {
                role_audio.push((name.clone(), b, track.role_source.as_ref().and(track.role.clone())));
            }
            let mut record = serde_json::to_value(track).unwrap_or(Value::Null);
            let is_kept = self.kept.contains_key(&s.seq);
            fresh.push(!is_kept);
            // #577 (Q4): a source that the archive held with no result: the settings of the session win over the new record
            // (`DS_ARA_DEFAULTS_FAULT=lose-kept` is the fault run: they are dropped, as main did)
            if let Some(kept) = self.kept.remove(&s.seq) {
                if switch("DS_ARA_DEFAULTS_FAULT").as_deref() != Some("lose-kept") {
                    if let Some(v) = overlay_settings(&record, &kept) {
                        record = v;
                    }
                }
            }
            rows.push(json!({"name": name, "index": at, "order_index": at, "track": record}));
            track_of.insert(s.seq, name.clone());
        }
        // #577: the user defaults reach NEW tracks only (a source that the archive held keeps its session values), one after the other in
        // the order of the DAW tracks, so several new toms take the notes of the list in turn
        let mut order: Vec<usize> = (0..new_sources.len()).collect();
        order.sort_by_key(|&i| (new_sources[i].order, new_sources[i].seq));
        for i in order {
            if !fresh[i] {
                continue;
            }
            let role = rows[i]["track"]["role"].as_str().unwrap_or("other").to_string();
            let mut record = rows[i]["track"].clone();
            apply_defaults(&mut record, &role, &defaults, &mut used);
            rows[i]["track"] = record;
        }
        if !rows.is_empty() {
            svc.tracks_restore(&sid, &Value::Array(rows)).map_err(err)?;
        }
        self.tracks = placed;
        for s in snaps {
            // a changed source without audio here keeps its old snapshot: its track record is
            // not made again now, so the change must still be seen when the audio comes
            let keep_old = s.audio_problem().is_some() && self.snaps.get(&s.seq).is_some_and(|old| !old.same_as(s));
            if !keep_old {
                self.snaps.insert(s.seq, s.clone());
            }
        }
        self.dirty = true; // until a round has run to its end
        if switch("DS_ARA_SCENARIO").as_deref() == Some("slow-setup") {
            // the check's scenario: a slow setup, so a cancel arrives before the analysis starts
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
        if switch("DS_ARA_SCENARIO").as_deref() == Some("fail-first-round") && !self.failed_once {
            self.failed_once = true;
            return Err("test: the round fails once".into());
        }
        // `fail-rounds:<n>`: the first n rounds fail (the automatic retry of the plugin)
        if let Some(n) =
            switch("DS_ARA_SCENARIO").and_then(|v| v.strip_prefix("fail-rounds:").and_then(|n| n.parse::<usize>().ok()))
        {
            if self.failed_rounds < n {
                self.failed_rounds += 1;
                return Err(format!("test: round {} of {n} fails", self.failed_rounds));
            }
        }
        // the tracks of the round: every track whose audio is here (the fault run brings
        // back the M1a way: the new tracks only)
        let only_new = switch("DS_ARA_ROUND_FAULT").as_deref() == Some("new-only");
        let mut analysed: Vec<String> = Vec::new();
        let mut skipped: Vec<(String, String)> = Vec::new();
        for (name, seq) in &self.tracks {
            if only_new && !names.contains(name) {
                continue;
            }
            if only.as_ref().is_some_and(|o| !o.contains(seq)) {
                continue;
            }
            match self.snaps.get(seq).map(SourceSnapshot::audio_problem) {
                Some(None) => analysed.push(name.clone()),
                Some(Some(why)) => skipped.push((name.clone(), why.to_string())),
                None => skipped.push((name.clone(), "no source".to_string())),
            }
        }
        let mut errors: Vec<(String, String)> = Vec::new();
        if !analysed.is_empty() {
            if self.canceller.cancelled() {
                return Err("the analysis was cancelled before it started".into());
            }
            // F1: a round keeps the hits the user placed in the page (the app's own
            // "keep my edits": they stay in the new events and win within 20 ms over the
            // detected ones). `DS_ARA_EDITOR_FAULT=drop-edits` brings back the plain analysis.
            let keep = switch("DS_ARA_EDITOR_FAULT").as_deref() != Some("drop-edits");
            retry_busy(|| svc.analyze_start_keep(&sid, Some(analysed.clone()), None, None, None, keep, false))
                .map_err(err)?;
            let mut idle_since: Option<std::time::Instant> = None;
            let st = loop {
                let st = svc.analyze_status(&sid);
                if self.canceller.cancelled() {
                    let _ = svc.job_cancel(&sid);
                }
                match st["state"].as_str() {
                    Some("done") => break st,
                    // the service has no such state today (a cancelled job ends as `done`
                    // with per-track states); the arm stays so a later service that has one
                    // ends the round instead of polling for ever
                    Some("cancelled" | "cancelling") => return Err("the analysis was cancelled".into()),
                    Some("idle") => {
                        // a job that never shows up: not for ever
                        let since = *idle_since.get_or_insert_with(std::time::Instant::now);
                        if since.elapsed() > std::time::Duration::from_secs(10) {
                            return Err("the service reports no job for this round".into());
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    _ => {
                        idle_since = None;
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
            };
            if self.canceller.cancelled() {
                return Err("the analysis was cancelled".into());
            }
            // the per-track result: a track the service could not analyse is not complete
            if switch("DS_ARA_ERRORS_FAULT").as_deref() != Some("swallow") {
                for name in &analysed {
                    let t = &st["tracks"][name.as_str()];
                    if t["state"] == "error" {
                        errors.push((name.clone(), t["error"].as_str().unwrap_or("error").to_string()));
                    }
                }
            }
        }
        let mut all = Map::new();
        for (name, _) in &self.tracks {
            let ev = svc.events_get(&sid, name).map_err(err)?;
            all.insert(name.clone(), ev["events"].clone());
        }
        // tracks that were not analysed again (the fault run, a track without audio) must
        // not change; an analysed track may change when the set of tracks changed
        let earlier_changed = self
            .events
            .iter()
            .filter(|(n, v)| !analysed.contains(n) && all.get(n.as_str()) != Some(*v))
            .count();
        self.events = all.clone();
        svc.project_save(&sid).map_err(err)?;
        self.dirty = false;
        let analysis_ms = started.elapsed().as_millis() as u64;
        // A track with an error that had events before keeps them (the service leaves them in
        // the document) and so do its notes: they are STALE, from an earlier round, and the
        // result says so (`stale`); they are never presented as the result of this round.
        let stale: Vec<String> = errors
            .iter()
            .filter(|(n, _)| {
                all.get(n.as_str())
                    .and_then(Value::as_array)
                    .is_some_and(|a| !a.is_empty())
            })
            .map(|(n, _)| n.clone())
            .collect();
        let notes = self.notes_of(&all);
        Ok(JobResult {
            events: all,
            decode_counts: svc.decode_counts(),
            analysis_ms,
            notes,
            track_of,
            added: names,
            analysed,
            project_tracks: self.track_names(),
            earlier_changed,
            errors,
            skipped,
            stale,
            ran: true,
            repair_error,
            tracks: self.tracks.clone(),
            regions: self
                .snaps
                .iter()
                .map(|(q, x)| (*q, (x.regions.clone(), x.sample_rate)))
                .collect(),
            role_audio,
        })
    }

    /// The page changed the project (an edit, a job of the page): read the events of every
    /// track again and save the project. `Some((events, notes))` when the events differ from
    /// the last known ones; the notes follow the events.
    ///
    /// # Errors
    ///
    /// A message when the project cannot be saved.
    #[allow(clippy::type_complexity)]
    pub fn sync_page_edits(&mut self) -> Result<SyncOutcome, String> {
        let (Some(svc), Some(sid)) = (self.svc.clone(), self.sid.clone()) else {
            return Ok((None, false));
        };
        let mut all = Map::new();
        for (name, _) in &self.tracks {
            // an error is not swallowed: a track the project no longer has is reported (the host
            // owns the track set, so this means the project changed behind the plugin's back)
            let ev = svc
                .events_get(&sid, name)
                .map_err(|e| format!("events_get {name}: {e}"))?;
            all.insert(name.clone(), ev["events"].clone());
        }
        let mut saved = false;
        if svc.project_get(&sid).is_ok_and(|p| p["dirty"] == true) {
            svc.project_save(&sid).map_err(err)?;
            saved = true;
        }
        if all == self.events {
            return Ok((None, saved));
        }
        self.events = all.clone();
        let notes = self.notes_of(&all);
        Ok((Some((all, notes)), saved))
    }

    /// The notes of every source of the project from the events: the hits that are not out
    /// of the output (the app's own `policy::is_out`: cut, bleed, filtered or deactivated,
    /// unless kept), mapped back into the source.
    fn notes_of(&self, events: &Map<String, Value>) -> HashMap<usize, Vec<[f64; 2]>> {
        let mut notes: HashMap<usize, Vec<[f64; 2]>> = HashMap::new();
        for (name, seq) in &self.tracks {
            let Some(s) = self.snaps.get(seq) else { continue };
            let list = events.get(name).and_then(Value::as_array).cloned().unwrap_or_default();
            notes.insert(*seq, notes_for(&list, &s.regions, s.sample_rate, s.sample_count));
        }
        notes
    }

    /// The track is not in the project (any more): the plugin's list forgets it.
    fn forget_track(&mut self, name: &str, seq: usize) {
        self.tracks.retain(|(n, _)| n != name);
        self.snaps.remove(&seq);
        self.events.shift_remove(name);
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
            match retry_busy(|| svc.tracks_remove(sid, std::slice::from_ref(&name), false)) {
                Ok(_) => {
                    svc.project_save(sid).map_err(err)?;
                }
                // R2: no such track: it is gone already (no retries for a ghost)
                Err(e) if is_missing_track(&e) => {}
                Err(e) => return Err(err(e)),
            }
        }
        self.tracks.retain(|(n, _)| *n != name);
        self.snaps.remove(&seq);
        self.dirty = true;
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
            // the archive holds the settings of the user for this track: they come back as they were and
            // no role is guessed when the archive holds one (`DS_ARA_NAMES_FAULT=restore-guess` guesses again,
            // as before M1b10); without a role in it the role is guessed and the other settings come over it
            let use_settings =
                !t.settings.is_empty() && switch("DS_ARA_NAMES_FAULT").as_deref() != Some("restore-guess");
            let has_role = t.settings.get("role").is_some_and(Value::is_string);
            let layout = || serde_json::to_value(track_layout(&t.snapshot, &dir)).map_err(err);
            let guessed = || serde_json::to_value(make_track(&t.snapshot, &dir)).map_err(err);
            let overlaid = if !use_settings {
                None
            } else if has_role {
                overlay_settings(&layout()?, &t.settings)
            } else {
                overlay_settings(&guessed()?, &t.settings)
            };
            let mut track = match overlaid {
                Some(v) => v,
                None => guessed()?,
            };
            // #577 fault runs: a restore reads the user defaults (always: for every track; session-loses: over the stored settings)
            let f = switch("DS_ARA_DEFAULTS_FAULT").unwrap_or_default();
            if f == "always" || f == "session-loses" {
                let role = track["role"].as_str().unwrap_or("other").to_string();
                let mut used = HashMap::new();
                apply_defaults(&mut track, &role, &crate::userdefaults::current(), &mut used);
            }
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
        self.snaps = tracks.iter().map(|t| (t.snapshot.seq, t.snapshot.clone())).collect();
        let mut events = Map::new();
        for (name, _) in &self.tracks {
            let ev = svc.events_get(&sid, name).map_err(err)?;
            events.insert(name.clone(), ev["events"].clone());
        }
        self.events = events.clone();
        self.dirty = false;
        Ok(RestoreResult {
            events,
            project_tracks: self.track_names(),
            decode_counts: svc.decode_counts(),
        })
    }

    /// M1d3 (#577): the export settings of an archive go back into the project. Each key passes the checks of `export_last_set` (the
    /// same call the page makes), so a hostile archive gets the refusals of a hostile page. Returns the names of the refused keys.
    pub fn restore_export(&mut self, export: &Map<String, Value>) -> Vec<String> {
        let (Ok(sid), Ok(svc)) = (self.ensure_open(), self.service()) else {
            return vec!["export".into()];
        };
        let mut refused = Vec::new();
        let mut args = json!({"sid": sid});
        for k in ["formats", "midi_mode", "compensate"] {
            if let Some(v) = export.get(k) {
                args[k] = v.clone();
            }
        }
        // `formats` is required by the call: without a valid list nothing is set, and the other keys are named
        if args.get("formats").is_none() || svc.export_last_set(&sid, &args).is_err() {
            // try the formats alone to name the refused key
            let only = json!({"sid": sid, "formats": export.get("formats").cloned().unwrap_or(Value::Null)});
            if svc.export_last_set(&sid, &only).is_err() {
                refused.push("formats".to_string());
            } else {
                for k in ["midi_mode", "compensate"] {
                    let mut one = only.clone();
                    if let Some(v) = export.get(k) {
                        one[k] = v.clone();
                        if svc.export_last_set(&sid, &one).is_err() {
                            refused.push(k.to_string());
                        }
                    }
                }
                // the last accepted combination stands: set the good keys again
                let mut good = only.clone();
                for k in ["midi_mode", "compensate"] {
                    if let Some(v) = export.get(k) {
                        let mut t = only.clone();
                        t[k] = v.clone();
                        if svc.export_last_set(&sid, &t).is_ok() {
                            good[k] = v.clone();
                        }
                    }
                }
                let _ = svc.export_last_set(&sid, &good);
            }
        }
        if let Some(b) = export.get("compensate").and_then(Value::as_bool) {
            let _ = svc.export_compensate_set(&sid, b);
        }
        refused
    }

    /// F6 (#582): point the project record of restored tracks at the cache file that the reader wrote.
    ///
    /// A restore writes the project record of a track when the cache file does not exist (a new document
    /// of the DAW, a new file number): the record has a path and no size or head hash, so the service
    /// calls the track offline (changed) as soon as the file is there, and nothing repairs it. `wavs` has
    /// `(source index, the cache file)` of sources whose audio is read now. For each track of the project
    /// whose status is not `ok`, the record gets the new file
    /// (`segments` and `source` only: the hits and every setting of the user stay) and the project is
    /// saved. A track that is online already is left as it is. Runs inside a round (before it looks for
    /// changes) and alone after the read of a restored source with the same content.
    ///
    /// # Errors
    ///
    /// A message when the service refuses the new record (the old one is put back) or the save.
    pub fn repoint(&mut self, wavs: &[(usize, PathBuf)]) -> Result<Vec<RepairRow>, String> {
        self.repoint_with(wavs, true)
    }

    /// [`Self::repoint`] for the repair after a read (#586): every call of the service is tried ONCE. A busy service
    /// (409) comes back as the error text that starts with `409`, at once; nothing sleeps with the service lock held.
    ///
    /// # Errors
    ///
    /// As [`Self::repoint`].
    pub fn repoint_once(&mut self, wavs: &[(usize, PathBuf)]) -> Result<Vec<RepairRow>, String> {
        self.repoint_with(wavs, false)
    }

    fn repoint_with(&mut self, wavs: &[(usize, PathBuf)], retry: bool) -> Result<Vec<RepairRow>, String> {
        let mut rows = Vec::new();
        if switch("DS_ARA_RESTORE_FAULT").as_deref() == Some("no-repair") {
            return Ok(rows);
        }
        let (Some(svc), Some(sid)) = (self.svc.clone(), self.sid.clone()) else {
            return Ok(rows);
        };
        let dir = self.project_dir();
        for (seq, wav) in wavs {
            let Some(old) = self.snaps.get(seq) else { continue };
            if !wav.is_file() {
                continue;
            }
            let Some(name) = self.track_of(*seq) else { continue };
            let mut s = old.clone();
            s.wav = wav.clone();
            s.ready = true;
            let all = svc.tracks_get(&sid).map_err(err)?;
            let record = &all["tracks"][name.as_str()];
            let before = record["status"].as_str().unwrap_or("missing").to_string();
            let mut repointed = false;
            if before != "ok" {
                let removed =
                    call_busy(retry, || svc.tracks_remove(&sid, std::slice::from_ref(&name), true)).map_err(err)?;
                let old_row = removed["snapshot"][0].clone();
                let mut track = old_row["track"].clone();
                let fresh = serde_json::to_value(track_layout(&s, &dir)).unwrap_or(Value::Null);
                if let Some(obj) = track.as_object_mut() {
                    for key in ["segments", "source"] {
                        match fresh.get(key) {
                            Some(v) => {
                                obj.insert(key.to_string(), v.clone());
                            }
                            None => {
                                obj.remove(key);
                            }
                        }
                    }
                }
                let mut row = old_row.clone();
                row["track"] = track;
                if let Err(e) = svc.tracks_restore(&sid, &Value::Array(vec![row])) {
                    return Err(match svc.tracks_restore(&sid, &Value::Array(vec![old_row])) {
                        Ok(_) => format!("the new record of {name} was refused ({e}); the old record is back"),
                        Err(e2) => {
                            self.forget_track(&name, *seq);
                            format!("the record of {name} could not be made again ({e}) nor put back ({e2}); the track left the plugin's list")
                        }
                    });
                }
                repointed = true;
                self.save_owed = true;
                self.reload_owed = true;
            }
            let all = svc.tracks_get(&sid).map_err(err)?;
            let record = &all["tracks"][name.as_str()];
            let events = svc.events_get(&sid, &name).map_err(err)?;
            rows.push(RepairRow {
                track: name.clone(),
                before,
                after: record["status"].as_str().unwrap_or("missing").to_string(),
                repointed: repointed || self.reload_owed,
                events_json: serde_json::to_string(&events["events"]).unwrap_or_default(),
                role: record["role"].as_str().unwrap_or_default().to_string(),
                note: record["note"].as_i64().unwrap_or(-1),
            });
        }
        // a save that failed stays owed: the next call finds the track `ok` in memory and saves again
        if self.save_owed {
            call_busy(retry, || svc.project_save(&sid)).map_err(err)?;
            self.save_owed = false;
        }
        self.reload_owed = false;
        Ok(rows)
    }

    /// Save the project when it has unsaved changes, then close it.
    fn close_project(&mut self) {
        self.canceller.set(None);
        self.page.set_sid(None);
        if let (Some(svc), Some(sid)) = (&self.svc, self.sid.take()) {
            if svc.project_get(&sid).is_ok_and(|p| p["dirty"] == true) {
                let _ = svc.project_save(&sid);
            }
            let _ = svc.project_close(&sid, true);
        }
    }

    fn close_service(&mut self) {
        self.close_project();
        self.page.set(None);
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

/// The role of a new track and where it comes from (M1b10): the DAW name first, then the name of
/// the audio source, then (as the scan of the app does, only for a name that gives no role) the
/// model-free suggestion from the audio, at last `other`. The second item is the `role_source` of
/// the track (`guess` from a name, `audio` from the audio; `None` for `other`), the third the words
/// of the name that matched.
/// The fourth item is the number of bytes read from the WAV when the audio was asked (None: it was not).
#[allow(clippy::type_complexity)]
fn pick_role(s: &SourceSnapshot) -> (&'static str, Option<&'static str>, Option<String>, Option<usize>) {
    if let Some((role, words)) = s.label.as_deref().and_then(guess_role_matched) {
        return (role, Some("guess"), Some(words), None);
    }
    let stem = stem_of(&s.name);
    if let Some((role, words)) = guess_role_matched(&stem) {
        return (role, Some("guess"), Some(words), None);
    }
    if s.ready && audio_hint_wanted(&stem) {
        let (hint, bytes) = audio_role(&s.wav);
        if let Some(role) = hint {
            return (role, Some("audio"), None, Some(bytes));
        }
        return ("other", None, None, Some(bytes));
    }
    ("other", None, None, None)
}

thread_local! {
    /// How often this thread asked the audio for a role (the unit test proves a name that gives a role never does).
    static AUDIO_ASKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A Drum Spike export gets no role from its audio (the app's rule of the scan).
fn audio_hint_wanted(stem: &str) -> bool {
    !drumspike_app::roles::is_export_name(stem)
}

/// The suggestion of `role_from_audio` for the first 30 s of the cached WAV: the app's hint mode
/// (only a role whose confidence bar exists comes back). Only the header and those 30 s are read from
/// the file (`wav::read_head_mono`), never the whole file; the second item is the number of bytes read.
/// A file that cannot be read gives no hint.
fn audio_role(wav: &Path) -> (Option<&'static str>, usize) {
    const SUGGEST_S: usize = 30;
    AUDIO_ASKS.with(|n| n.set(n.get() + 1));
    let Some((mono, sr, bytes)) = crate::wav::read_head_mono(wav, SUGGEST_S) else {
        return (None, 0);
    };
    (
        drumspike_core::role::role_from_audio(&mono, sr).map(drumspike_core::role::Role::as_str),
        bytes,
    )
}

/// #577: the start values of a NEW track of `role` from the user defaults: the first note of the role's list that no other track of the
/// role uses (`used` grows), and the detection settings. A role with no entry keeps the built-in note. A record that is not an object
/// is left alone.
fn apply_defaults(
    record: &mut Value,
    role: &str,
    defaults: &crate::userdefaults::Loaded,
    used: &mut HashMap<String, Vec<i64>>,
) {
    let Some(obj) = record.as_object_mut() else { return };
    let list = used.entry(role.to_string()).or_default();
    if let Some(entry) = defaults.checked.roles.get(role) {
        if let Some(n) = crate::userdefaults::pick_note(&entry.notes, list) {
            obj.insert("note".into(), Value::from(i64::from(n)));
        }
        if !entry.params.is_empty() {
            let mut p = obj
                .get("params")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for (k, v) in &entry.params {
                p.insert(k.clone(), v.clone());
            }
            obj.insert("params".into(), Value::Object(p));
        }
    }
    if let Some(n) = obj.get("note").and_then(Value::as_i64) {
        list.push(n);
    }
}

/// The notes that a new track of `role` would start with now: the user default list, else the built-in note.
fn start_notes(role: &str) -> Vec<i64> {
    let d = crate::userdefaults::current();
    match d.checked.roles.get(role) {
        Some(e) if !e.notes.is_empty() => e.notes.iter().map(|n| i64::from(*n)).collect(),
        _ => vec![default_note(role)],
    }
}

/// The record of a track (`obj`, JSON) gets the role that the DAW name of `s` gives, unless the
/// user set the role. Returns whether the role changed.
fn apply_label_role(obj: &mut Map<String, Value>, s: &SourceSnapshot) -> bool {
    if switch("DS_ARA_NAMES_FAULT").as_deref() == Some("role-hand") {
        // the fault run: the guess overwrites a role the user set
    } else if obj.get("role_source").and_then(Value::as_str) == Some("user") {
        return false;
    }
    if switch("DS_ARA_NAMES_FAULT").as_deref() == Some("role-frozen") {
        return false; // the fault run: the role is guessed once and never again
    }
    let Some((new_role, words)) = s.label.as_deref().and_then(guess_role_matched) else {
        return false;
    };
    let current = obj.get("role").and_then(Value::as_str).unwrap_or("other").to_string();
    if current == new_role {
        return false;
    }
    // #577: "the default note of the old role" is the note that a NEW track of that role would get now (user list, else built-in); a note
    // that is none of them was set by the user and stays
    let note_is_default = obj
        .get("note")
        .and_then(Value::as_i64)
        .is_some_and(|n| n == default_note(&current) || start_notes(&current).contains(&n));
    obj.insert("role".into(), Value::from(new_role));
    obj.insert("role_source".into(), Value::from("guess"));
    obj.insert("role_match".into(), Value::from(words));
    if note_is_default {
        obj.insert("note".into(), Value::from(start_notes(new_role)[0]));
    }
    true
}

/// The track of one NEW source: the layout (below) and the role from the DAW name, the name of the
/// source or the audio. A remake of a track record (a region change) takes only the layout: the role is
/// not asked again there, so the audio is not read for nothing.
fn make_track(s: &SourceSnapshot, dir: &Path) -> Track {
    make_track_audited(s, dir).0
}

/// [`make_track`] and the bytes read from the WAV for the role (None: the audio was not asked).
fn make_track_audited(s: &SourceSnapshot, dir: &Path) -> (Track, Option<usize>) {
    let (role, role_source, role_match, bytes) = pick_role(s);
    let mut t = track_layout(s, dir);
    t.role = Some(role.to_string());
    t.note = Some(default_note(role));
    t.role_source = role_source.map(str::to_string);
    t.role_match = role_match;
    (t, bytes)
}

/// The track of one source without a role: the whole file or the playback regions as segments.
fn track_layout(s: &SourceSnapshot, dir: &Path) -> Track {
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
        mix_mode: Some("average".into()),
        source: segments.is_none().then(|| Some(wav_source(&s.wav, dir))),
        events: Some(Vec::new()),
        segments,
        ..Track::default()
    }
}

/// The file stem of a source name (the ARA name of the test host is the file path), as ONE
/// safe path component for every input. The name comes from the host: it may be empty, hold
/// `..` as its last component (then `file_stem` is `None`), or be an absolute path. The
/// result has no separator and is never empty, `.` or `..`; any other character than a
/// letter, a digit, `-`, `_`, `.` or a space becomes `_`; at most 100 characters. It names the
/// cached WAV and gives the role guess, never a folder. `DS_ARA_ID_FAULT=trust` (the check's
/// fault run) brings back the old stem.
#[must_use]
pub fn stem_of(name: &str) -> String {
    let raw = Path::new(name)
        .file_stem()
        .map_or_else(|| name.to_string(), |s| s.to_string_lossy().into_owned());
    if switch("DS_ARA_ID_FAULT").as_deref() == Some("trust") {
        return raw;
    }
    let safe: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    if safe.is_empty() || safe.chars().all(|c| c == '.') {
        "source".to_string()
    } else {
        safe
    }
}

/// What a sync of the page's edits found: the new events and notes when they differ from the last
/// known ones, and whether the project was saved (R3).
pub type SyncOutcome = (Option<(Map<String, Value>, HashMap<usize, Vec<[f64; 2]>>)>, bool);

/// The notes of one source from the events of its track: the hits that are not out of the
/// output (the app's own `policy::is_out`), mapped back into the source; a hit at or after
/// `sample_count` is left out.
#[must_use]
pub fn notes_for(list: &[Value], regions: &[RegionRec], rate: f64, sample_count: i64) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(list.len());
    for ev in list.iter().filter(|e| !drumspike_app::policy::is_out(e)) {
        let t = ev.get("t").and_then(Value::as_f64).unwrap_or(f64::NAN);
        let v = ev.get("v").and_then(Value::as_i64).unwrap_or(100);
        // a hand-placed hit beyond the end of the source (new, shorter audio) stays in the project
        // but is no note: the host would get a note past the end of the source
        if let Some(sample) = source_sample(t, regions, rate).filter(|s| sample_count <= 0 || *s < sample_count) {
            out.push([sample as f64, (v.clamp(0, 127) as f64) / 127.0]);
        }
    }
    out
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

    fn labelled(label: Option<&str>) -> SourceSnapshot {
        SourceSnapshot {
            seq: 0,
            name: "0123456789abcdef0123456789abcdef".into(),
            persistent_id: String::new(),
            sample_rate: 48_000.0,
            sample_count: 0,
            wav: PathBuf::from("/nonexistent.wav"),
            ready: false,
            wav_len: 0,
            audio_gen: 0,
            regions: Vec::new(),
            label: label.map(str::to_string),
            order: i32::MAX,
        }
    }

    #[test]
    fn the_daw_name_gives_the_role_when_the_source_name_is_hex() {
        for (label, role, note) in [
            ("Kick In", "kick", 36),
            ("Snare Top", "snare", 38),
            ("Hi-Hat", "hat", 42),
        ] {
            let t = make_track(&labelled(Some(label)), Path::new("/p"));
            assert_eq!((t.role.as_deref(), t.note), (Some(role), Some(note)), "{label}");
            assert_eq!(t.role_source.as_deref(), Some("guess"));
        }
        // the names of the stage `editor-names` give no role
        assert_eq!(guess_role("Drum A"), None);
        assert_eq!(guess_role("Drum Big"), None);
        let none = make_track(&labelled(Some("Mystery")), Path::new("/p"));
        assert_eq!(
            none.role.as_deref(),
            Some("other"),
            "a name with no role and no audio: other"
        );
        assert_eq!(
            make_track(&labelled(None), Path::new("/p")).role.as_deref(),
            Some("other")
        );
    }

    #[test]
    fn the_audio_is_asked_for_a_role_only_when_neither_name_gives_one() {
        let asks = || AUDIO_ASKS.with(std::cell::Cell::get);
        let mut s = labelled(Some("Kick In"));
        s.ready = true;
        let before = asks();
        assert_eq!(pick_role(&s).0, "kick");
        assert_eq!(asks(), before, "a DAW name that gives a role never asks the audio");
        s.label = Some("Mystery".into());
        s.name = "snare_top.wav".into();
        assert_eq!(pick_role(&s).0, "snare");
        assert_eq!(asks(), before, "a source name that gives a role never asks the audio");
        s.name = "0123456789abcdef0123456789abcdef".into();
        assert_eq!(pick_role(&s).0, "other");
        assert_eq!(
            asks(),
            before + 1,
            "no name gives a role: the audio is asked once (here the file is missing: no hint)"
        );
        s.ready = false;
        let _ = pick_role(&s);
        assert_eq!(asks(), before + 1, "a source whose audio is not here is not asked");
    }

    #[test]
    fn a_new_daw_name_changes_an_automatic_role_and_never_a_role_of_the_user() {
        let snare = labelled(Some("Snare Bottom"));
        let mut auto = json!({"role": "other", "note": 36, "role_source": "guess"});
        assert!(apply_label_role(auto.as_object_mut().unwrap(), &snare));
        assert_eq!(
            (auto["role"].as_str(), auto["note"].as_i64()),
            (Some("snare"), Some(38))
        );
        let mut by_hand = json!({"role": "cymbal", "note": 49, "role_source": "user"});
        assert!(!apply_label_role(by_hand.as_object_mut().unwrap(), &snare));
        assert_eq!(by_hand["role"], "cymbal");
        // a note the user changed stays
        let mut note = json!({"role": "other", "note": 40, "role_source": "guess"});
        assert!(apply_label_role(note.as_object_mut().unwrap(), &snare));
        assert_eq!(
            (note["role"].as_str(), note["note"].as_i64()),
            (Some("snare"), Some(40))
        );
        // a name with no role leaves the role as it is
        let mut keep = json!({"role": "kick", "note": 36});
        assert!(!apply_label_role(
            keep.as_object_mut().unwrap(),
            &labelled(Some("Mystery"))
        ));
        assert!(!labelled(Some("Kick In")).same_as(&labelled(Some("Snare Bottom"))));
        assert!(
            !labelled(Some("Mystery")).same_as(&labelled(Some("Snare Bottom"))),
            "no role before, a role now"
        );
        assert!(
            labelled(Some("Kick In")).same_as(&labelled(Some("Mystery"))),
            "a name with no role leaves the role"
        );
        assert!(
            labelled(Some("Kick In")).same_as(&labelled(Some("Kick Inside"))),
            "a rename that keeps the role is no change"
        );
    }

    #[test]
    fn a_remake_of_a_track_record_never_asks_the_audio_for_a_role() {
        let cache = scratch("role-remake");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let hex = "0123456789abcdef0123456789abcdef";
        let mut src = snapshot_of(&cache, &id, 0, hex);
        let asks = || AUDIO_ASKS.with(std::cell::Cell::get);
        let before = asks();
        let first = doc.analyse(std::slice::from_ref(&src), false).unwrap();
        assert_eq!(
            asks(),
            before + 1,
            "a new track with no role in any name asks the audio once"
        );
        assert_eq!(first.role_audio.len(), 1, "the round says it");
        assert!(
            first.role_audio[0].1 > 44 && first.role_audio[0].1 <= 44 + 30 * 48_000 * 4,
            "bytes read: {:?}",
            first.role_audio
        );
        // a region change makes the track record again: the role is not asked again
        src.regions = vec![RegionRec {
            start_mod: 0.0,
            dur_mod: 1.0,
            start_play: 0.0,
        }];
        let second = doc.analyse(std::slice::from_ref(&src), false).unwrap();
        assert!(second.ran);
        assert_eq!(asks(), before + 1, "a remake computes no role and reads no audio");
        assert!(second.role_audio.is_empty());
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_new_daw_name_starts_no_round_when_the_role_is_the_users_or_the_same() {
        let cache = scratch("role-noop");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let hex = "0123456789abcdef0123456789abcdef";
        let mut src = snapshot_of(&cache, &id, 0, hex);
        src.label = Some("Kick In".into());
        let _ = doc.analyse(std::slice::from_ref(&src), false).unwrap();
        // Kick -> Mystery -> Kick: no role change, no round
        src.label = Some("Mystery".into());
        assert!(!doc.analyse(std::slice::from_ref(&src), false).unwrap().ran);
        src.label = Some("Kick Inside".into());
        assert!(
            !doc.analyse(std::slice::from_ref(&src), false).unwrap().ran,
            "the same role again"
        );
        // the user sets the role by hand: a name that gives another role changes nothing and starts no round
        let (svc, sid) = (doc.svc.clone().unwrap(), doc.sid.clone().unwrap());
        let name = doc.tracks[0].0.clone();
        svc.track_set_role(&sid, &name, "cymbal", None, None, None, None)
            .unwrap();
        src.label = Some("Snare Bottom".into());
        assert!(
            !doc.analyse(std::slice::from_ref(&src), false).unwrap().ran,
            "a role of the user: no round"
        );
        assert_eq!(svc.tracks_get(&sid).unwrap()["tracks"][name.as_str()]["role"], "cymbal");
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_daw_name_that_gives_the_role_the_track_already_has_starts_no_round() {
        let cache = scratch("role-same");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        // the role snare comes from the name of the source
        let mut src = snapshot_of(&cache, &id, 0, "snare_top.wav");
        let _ = doc.analyse(std::slice::from_ref(&src), false).unwrap();
        let (svc, sid) = (doc.svc.clone().unwrap(), doc.sid.clone().unwrap());
        let name = doc.tracks[0].0.clone();
        assert_eq!(svc.tracks_get(&sid).unwrap()["tracks"][name.as_str()]["role"], "snare");
        // the DAW name gives snare: the source counts as changed, the record keeps its role: the change is dropped
        src.label = Some("Snare".into());
        assert!(
            !doc.snaps[&0].same_as(&src),
            "a name that gives a role to a source with none is a change"
        );
        assert!(
            !doc.analyse(std::slice::from_ref(&src), false).unwrap().ran,
            "the role is already snare: no round"
        );
        assert_eq!(
            doc.snaps[&0].label.as_deref(),
            Some("Snare"),
            "the new snapshot is kept"
        );
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn kick_then_mystery_then_snare_changes_an_automatic_role_once() {
        let cache = scratch("role-chain");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let mut src = snapshot_of(&cache, &id, 0, "0123456789abcdef0123456789abcdef");
        src.label = Some("Kick In".into());
        let _ = doc.analyse(std::slice::from_ref(&src), false).unwrap();
        src.label = Some("Mystery".into());
        assert!(!doc.analyse(std::slice::from_ref(&src), false).unwrap().ran);
        src.label = Some("Snare".into());
        assert!(doc.analyse(std::slice::from_ref(&src), false).unwrap().ran);
        let (svc, sid) = (doc.svc.clone().unwrap(), doc.sid.clone().unwrap());
        let name = doc.tracks[0].0.clone();
        let t = svc.tracks_get(&sid).unwrap();
        assert_eq!(
            (
                t["tracks"][name.as_str()]["role"].as_str(),
                t["tracks"][name.as_str()]["note"].as_i64()
            ),
            (Some("snare"), Some(38))
        );
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_record_from_before_this_change_has_no_role_source_and_counts_as_automatic() {
        // the record of an archive of M1b9: a role from the hex name (other), a note, hits the user edited
        let mut rec = json!({"role": "other", "note": 36, "events": [{"t": 0.5, "v": 100, "edited": true}]});
        let snare = labelled(Some("Snare Top"));
        assert!(
            apply_label_role(rec.as_object_mut().unwrap(), &snare),
            "the role is guessed once"
        );
        assert_eq!(
            (rec["role"].as_str(), rec["note"].as_i64(), rec["role_source"].as_str()),
            (Some("snare"), Some(38), Some("guess"))
        );
        assert_eq!(rec["events"][0]["edited"], true, "every hit stays, the edited ones too");
        assert!(!apply_label_role(rec.as_object_mut().unwrap(), &snare), "and only once");
        // a note that was not the default note stays
        let mut own = json!({"role": "other", "note": 40});
        assert!(apply_label_role(own.as_object_mut().unwrap(), &snare));
        assert_eq!(own["note"].as_i64(), Some(40));
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
            wav_len: std::fs::metadata(&wav).map_or(0, |m| m.len()),
            wav,
            ready: true,
            audio_gen: 0,
            regions: Vec::new(),
            label: None,
            order: i32::MAX,
        }
    }

    /// #577 (Q4): a source that the archive holds with no result keeps the settings of the session in its first round.
    #[test]
    fn a_source_held_without_a_result_keeps_its_session_settings_in_its_first_round() {
        let cache = scratch("kept");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let mut kept = HashMap::new();
        let mut m = Map::new();
        m.insert("role".into(), json!("tom"));
        m.insert("role_source".into(), json!("user"));
        m.insert("note".into(), json!(41));
        m.insert("params".into(), json!({"min_gap_ms": 77}));
        kept.insert(0, m);
        doc.keep_settings(kept);
        // the file name says snare, the session says tom with note 41: the session wins
        let src = snapshot_of(&cache, &id, 0, "snare.wav");
        doc.analyse(std::slice::from_ref(&src), false).unwrap();
        let svc = doc.service().unwrap();
        let sid = doc.sid.clone().unwrap();
        let got = settings_of(&svc, &sid, &["snare".to_string()]).unwrap();
        assert_eq!(got["snare"]["role"], "tom");
        assert_eq!(got["snare"]["note"], 41);
        assert_eq!(got["snare"]["params"]["min_gap_ms"], 77);
    }

    /// S7: a source that is gone before its first round leaves no values behind; a new source of the same name is a new track.
    #[test]
    fn the_kept_values_of_a_removed_source_never_reach_a_new_source_of_the_same_name() {
        let cache = scratch("kept-gone");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let mut m = Map::new();
        m.insert("role".into(), json!("tom"));
        m.insert("role_source".into(), json!("user"));
        m.insert("note".into(), json!(41));
        doc.keep_settings(HashMap::from([(0, m)]));
        // source 0 is removed; source 1 has the same file name and arrives
        let again = snapshot_of(&cache, &id, 1, "snare.wav");
        doc.analyse(std::slice::from_ref(&again), false).unwrap();
        let svc = doc.service().unwrap();
        let sid = doc.sid.clone().unwrap();
        let got = settings_of(&svc, &sid, &["snare".to_string()]).unwrap();
        assert_eq!(
            got["snare"]["role"], "snare",
            "a new track: the role of its name, not the old session"
        );
        assert_eq!(got["snare"]["note"], 38);
        assert!(doc.kept.is_empty(), "the entry of the gone source was dropped");
    }

    /// S8: a source whose record was lost in a round and comes back as new gets the values the document holds (the caller passes them by its
    /// seq); S9: a restore that comes after a first round replaces the project, and the values it brings win over the start values of that round.
    #[test]
    fn a_restore_after_a_first_round_wins_and_the_values_of_the_document_win_for_the_same_source() {
        let cache = scratch("kept-late");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let src = snapshot_of(&cache, &id, 0, "snare.wav");
        doc.analyse(std::slice::from_ref(&src), false).unwrap(); // a round that saw the source before the restore: built-in values
        let mut m = Map::new();
        m.insert("role".into(), json!("tom"));
        m.insert("role_source".into(), json!("user"));
        m.insert("note".into(), json!(41));
        // the restore (no track with a result) resets the project and the tracks of the service; the values wait for the source
        doc.restore(&[]).unwrap();
        doc.keep_settings(HashMap::from([(0, m)]));
        doc.analyse(std::slice::from_ref(&src), false).unwrap();
        let svc = doc.service().unwrap();
        let sid = doc.sid.clone().unwrap();
        let got = settings_of(&svc, &sid, &["snare".to_string()]).unwrap();
        assert_eq!(
            (got["snare"]["role"].as_str(), got["snare"]["note"].as_i64()),
            (Some("tom"), Some(41))
        );
    }

    fn defaults_with(text: &str) -> crate::userdefaults::Loaded {
        let d = scratch("ud");
        std::fs::write(d.join("defaults.json"), text).unwrap();
        crate::userdefaults::load_at(&d)
    }

    #[test]
    fn new_toms_take_the_notes_of_the_list_in_turn_and_the_last_when_the_list_is_used() {
        let d = defaults_with(
            r#"{"v":1,"roles":{"tom":{"notes":[50,48,47],"params":{"min_gap_ms":33}},"snare":{"note":40}}}"#,
        );
        let mut used: HashMap<String, Vec<i64>> = HashMap::new();
        used.entry("tom".into()).or_default().push(50); // a tom of the document uses 50
        let mut got = Vec::new();
        for _ in 0..5 {
            let mut rec = json!({"role": "tom", "note": 48, "params": {"refine": true}});
            apply_defaults(&mut rec, "tom", &d, &mut used);
            assert_eq!(rec["params"]["min_gap_ms"], 33);
            assert_eq!(rec["params"]["refine"], true, "a key the default does not name stays");
            got.push(rec["note"].as_i64().unwrap());
        }
        assert_eq!(got, [48, 47, 47, 47, 47]);
        let mut snare = json!({"role": "snare", "note": 38});
        apply_defaults(&mut snare, "snare", &d, &mut used);
        assert_eq!(snare["note"], 40);
        // a role with no entry keeps the built-in note and its params
        let mut hat = json!({"role": "hat", "note": 42});
        apply_defaults(&mut hat, "hat", &d, &mut used);
        assert_eq!(hat, json!({"role": "hat", "note": 42}));
    }

    #[test]
    fn a_role_that_the_daw_name_changes_keeps_a_note_the_user_set() {
        // the note equals the built-in note of the old role: it follows the new role
        let snare = SourceSnapshot {
            label: Some("Snare".into()),
            ..labelled(Some("Snare"))
        };
        let mut rec = json!({"role": "other", "note": 36});
        assert!(apply_label_role(rec.as_object_mut().unwrap(), &snare));
        assert_eq!(rec["note"], 38);
        let mut own = json!({"role": "other", "note": 41});
        assert!(apply_label_role(own.as_object_mut().unwrap(), &snare));
        assert_eq!(own["note"], 41, "a note that was set by hand stays");
    }

    #[test]
    fn the_export_settings_of_an_archive_are_range_checked_like_a_hostile_page() {
        let cache = scratch("exp");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let mut good = Map::new();
        good.insert("formats".into(), json!(["midi", "spikes"]));
        good.insert("midi_mode".into(), json!("single"));
        good.insert("compensate".into(), json!(false));
        assert!(doc.restore_export(&good).is_empty());
        let svc = doc.service().unwrap();
        let sid = doc.sid.clone().unwrap();
        let got = svc.export_last_get(&sid).unwrap();
        assert_eq!(got["export_last"]["midi_mode"], "single");
        assert_eq!(got["export_last"]["compensate"], false);
        assert_eq!(got["export_compensate"], false);
        // hostile values: each is refused and named, the good key stays
        let mut doc2 = DocService::new(scratch("exp2"), new_doc_id());
        let mut bad = Map::new();
        bad.insert("formats".into(), json!(["midi", "../../x"]));
        bad.insert("midi_mode".into(), json!("evil"));
        bad.insert("compensate".into(), json!("yes"));
        bad.insert("dest".into(), json!("/etc"));
        let refused = doc2.restore_export(&bad);
        assert!(refused.contains(&"formats".to_string()), "{refused:?}");
        let sid2 = doc2.sid.clone().unwrap();
        assert!(
            doc2.service().unwrap().export_last_get(&sid2).unwrap()["export_last"].is_null(),
            "nothing was set"
        );
        let mut half = Map::new();
        half.insert("formats".into(), json!(["audio"]));
        half.insert("midi_mode".into(), json!("evil"));
        half.insert("dest".into(), json!("/etc"));
        let refused = doc2.restore_export(&half);
        assert_eq!(refused, ["midi_mode"]);
        let last = doc2.service().unwrap().export_last_get(&sid2).unwrap();
        assert_eq!(last["export_last"]["formats"], json!(["audio"]));
        assert!(
            last["export_last"]["dest"].is_null(),
            "a dest of an archive is never taken"
        );
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
    fn a_later_round_analyses_all_tracks_together_and_a_removal_analyses_the_rest_again() {
        let cache = scratch("rounds");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let kick = snapshot_of(&cache, &id, 0, "kick.wav");
        let first = doc.analyse(std::slice::from_ref(&kick), false).unwrap();
        assert_eq!(first.project_tracks, ["kick"]);
        assert_eq!(first.analysed, ["kick"]);
        assert!(
            first.events["kick"].as_array().is_some_and(|e| !e.is_empty()),
            "the detector found hits"
        );
        let snare = snapshot_of(&cache, &id, 1, "snare.wav");
        let second = doc.analyse(&[kick.clone(), snare.clone()], false).unwrap();
        assert_eq!(second.project_tracks, ["kick", "snare"]);
        assert_eq!(
            second.analysed,
            ["kick", "snare"],
            "the earlier track is analysed with the new one"
        );
        assert_eq!(second.earlier_changed, 0);
        assert!(second.notes.contains_key(&0) && second.notes.contains_key(&1));
        // the result of the two rounds is the result of one analysis of both together
        let cache2 = scratch("rounds-ref");
        let id2 = new_doc_id();
        let mut one = DocService::new(cache2.clone(), id2.clone());
        let together = one
            .analyse(
                &[
                    snapshot_of(&cache2, &id2, 0, "kick.wav"),
                    snapshot_of(&cache2, &id2, 1, "snare.wav"),
                ],
                false,
            )
            .unwrap();
        assert_eq!(
            serde_json::to_string(&second.events).unwrap(),
            serde_json::to_string(&together.events).unwrap(),
            "byte for byte"
        );
        // the document on disk holds both tracks
        let text = std::fs::read_to_string(document_path(&doc.project_dir())).unwrap();
        let on_disk: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(on_disk["tracks"].as_object().unwrap().len(), 2);
        // a destroyed source leaves the project and the document; the rest is analysed again
        assert_eq!(doc.remove(0).unwrap(), ["snare"]);
        let rest = doc.analyse(std::slice::from_ref(&snare), false).unwrap();
        assert!(rest.ran, "a removal makes the next round run");
        assert_eq!(rest.analysed, ["snare"]);
        assert!(rest.added.is_empty());
        // nothing changed: no round
        assert!(!doc.analyse(&[snare], false).unwrap().ran);
        let text = std::fs::read_to_string(document_path(&doc.project_dir())).unwrap();
        let on_disk: Value = serde_json::from_str(&text).unwrap();
        assert!(on_disk["tracks"].get("kick").is_none() && on_disk["tracks"].get("snare").is_some());
        doc.shutdown();
        one.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
        let _ = std::fs::remove_dir_all(&cache2);
    }

    /// R2: the removal of a changed source's track succeeded and the restore of the new record is
    /// refused: the OLD record is put back, the round reports it, and the next round works.
    #[test]
    fn a_refused_restore_puts_the_old_record_back_and_the_next_round_works() {
        let cache = scratch("restore-once");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let mut kick = snapshot_of(&cache, &id, 0, "kick.wav");
        let first = doc.analyse(std::slice::from_ref(&kick), false).unwrap();
        let hits = first.events["kick"].as_array().unwrap().len();
        kick.regions = vec![RegionRec {
            start_mod: 0.0,
            dur_mod: 2.0,
            start_play: 0.0,
        }];
        doc.fail_restores = 1;
        let e = doc.analyse(std::slice::from_ref(&kick), false).unwrap_err();
        assert!(e.contains("the old record is back"), "{e}");
        assert_eq!(doc.track_names(), vec!["kick".to_string()]);
        let again = doc.analyse(std::slice::from_ref(&kick), false).unwrap();
        assert!(
            again.ran,
            "the change is still seen: the next round makes the new record"
        );
        assert!(again.events["kick"].as_array().unwrap().len() >= hits - 1);
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    /// R2: when the old record cannot be put back either, the track leaves the plugin's list, so
    /// that later rounds do not fail with a 404 for ever; the source is a new track then.
    #[test]
    fn a_track_that_cannot_be_put_back_leaves_the_list_and_later_rounds_work() {
        let cache = scratch("restore-twice");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let mut kick = snapshot_of(&cache, &id, 0, "kick.wav");
        doc.analyse(std::slice::from_ref(&kick), false).unwrap();
        kick.regions = vec![RegionRec {
            start_mod: 0.0,
            dur_mod: 2.0,
            start_play: 0.0,
        }];
        doc.fail_restores = 2;
        let e = doc.analyse(std::slice::from_ref(&kick), false).unwrap_err();
        assert!(
            e.contains("left the plugin's list") || e.contains("old record is back"),
            "{e}"
        );
        let later = doc.analyse(std::slice::from_ref(&kick), false).unwrap();
        assert!(later.ran);
        assert_eq!(
            later.project_tracks.len(),
            1,
            "one track for the source, no ghost: {:?}",
            later.project_tracks
        );
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    /// R2: a removal of a track that the project does not have counts as removed (no retries).
    #[test]
    fn the_removal_of_a_track_the_project_lacks_counts_as_removed() {
        let cache = scratch("remove-404");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let kick = snapshot_of(&cache, &id, 0, "kick.wav");
        doc.analyse(std::slice::from_ref(&kick), false).unwrap();
        // the track goes behind the plugin's back
        let (svc, sid) = (doc.service().unwrap(), doc.sid.clone().unwrap());
        svc.tracks_remove(&sid, &["kick".to_string()], false).unwrap();
        let left = doc.remove(0).unwrap();
        assert!(left.is_empty());
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_round_without_tracks_does_nothing() {
        let cache = scratch("empty");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id);
        let r = doc.analyse(&[], false).unwrap();
        assert!(r.analysed.is_empty() && r.events.is_empty());
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn two_documents_keep_their_own_folders_and_numbering() {
        let cache = scratch("two");
        let (ida, idb) = (new_doc_id(), new_doc_id());
        let mut a = DocService::new(cache.clone(), ida.clone());
        let mut b = DocService::new(cache.clone(), idb.clone());
        let ra = a.analyse(&[snapshot_of(&cache, &ida, 0, "kick.wav")], false).unwrap();
        let rb = b.analyse(&[snapshot_of(&cache, &idb, 0, "kick.wav")], false).unwrap();
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
        let stored = doc.analyse(std::slice::from_ref(&snap), false).unwrap();
        doc.shutdown();
        // a new service on the same id (the new process): the WAV is gone, the events stay
        std::fs::remove_file(&snap.wav).unwrap();
        let mut again = DocService::new(cache.clone(), id.clone());
        let restored = again
            .restore(&[RestoredTrack {
                snapshot: snap,
                track: "kick".into(),
                events: stored.events["kick"].clone(),
                settings: Map::new(),
            }])
            .unwrap();
        assert_eq!(restored.events["kick"].to_string(), stored.events["kick"].to_string());
        assert!(restored.decode_counts.is_empty(), "{:?}", restored.decode_counts);
        assert_eq!(restored.project_tracks, ["kick"]);
        again.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_restore_puts_back_what_the_user_set_and_guesses_nothing_for_a_track_the_archive_holds() {
        let cache = scratch("restore-settings");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let hex = "0123456789abcdef0123456789abcdef";
        let mut snap = snapshot_of(&cache, &id, 0, hex);
        snap.label = Some("Kick In".into());
        let stored = doc.analyse(std::slice::from_ref(&snap), false).unwrap();
        let name = doc.tracks[0].0.clone();
        let (svc, sid) = (doc.svc.clone().unwrap(), doc.sid.clone().unwrap());
        // the user sets a role, a note and a volume by hand
        svc.track_set_role(&sid, &name, "cymbal", Some(60), None, None, None)
            .unwrap();
        svc.track_set_mix(&sid, &name, Some(-3.0), None).unwrap();
        let settings = settings_of(&svc, &sid, std::slice::from_ref(&name)).unwrap();
        let held = settings[name.as_str()].as_object().unwrap();
        assert_eq!(held["role_source"], "user");
        assert!(!held.contains_key("events") && !held.contains_key("segments") && !held.contains_key("status"));
        doc.shutdown();
        // a new service (the new process) and a DAW name that would give another role
        snap.label = Some("Snare Top".into());
        let mut again = DocService::new(cache.clone(), id.clone());
        let t = |settings: Map<String, Value>| RestoredTrack {
            snapshot: snap.clone(),
            track: name.clone(),
            events: stored.events[name.as_str()].clone(),
            settings,
        };
        again.restore(&[t(held.clone())]).unwrap();
        let (svc2, sid2) = (again.svc.clone().unwrap(), again.sid.clone().unwrap());
        let rec = svc2.tracks_get(&sid2).unwrap();
        let rec = &rec["tracks"][name.as_str()];
        assert_eq!(
            (rec["role"].as_str(), rec["note"].as_i64(), rec["role_source"].as_str()),
            (Some("cymbal"), Some(60), Some("user"))
        );
        assert_eq!(rec["volume_db"], -3.0);
        assert_eq!(
            svc2.events_get(&sid2, &name).unwrap()["events"].to_string(),
            stored.events[name.as_str()].to_string(),
            "the hits are the stored ones"
        );
        again.shutdown();
        // an archive of an earlier plugin holds no settings: the role is guessed, as it always was
        let mut old = DocService::new(cache.clone(), id.clone());
        old.restore(&[t(Map::new())]).unwrap();
        let (svc3, sid3) = (old.svc.clone().unwrap(), old.sid.clone().unwrap());
        assert_eq!(
            svc3.tracks_get(&sid3).unwrap()["tracks"][name.as_str()]["role"],
            "snare"
        );
        old.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_failed_read_or_a_missing_track_keeps_the_old_settings() {
        let old = map(json!({"a": {"role": "snare"}, "b": {"role": "kick"}, "gone": {"role": "hat"}}));
        let names = ["a".to_string(), "b".to_string()];
        // the read failed: the old map stays whole
        assert_eq!(merge_settings(&old, None, &names), old);
        // `b` is not in the answer: its old entry stays; `a` is replaced; `gone` leaves with its track
        let fresh = map(json!({"a": {"role": "tom"}}));
        let merged = merge_settings(&old, Some(fresh), &names);
        assert_eq!(
            Value::Object(merged),
            json!({"a": {"role": "tom"}, "b": {"role": "kick"}})
        );
    }

    #[test]
    fn every_key_of_a_track_answer_is_a_setting_or_computed() {
        let cache = scratch("keys");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let mut snap = snapshot_of(&cache, &id, 0, "kick.wav");
        snap.label = Some("Kick In".into());
        let _ = doc.analyse(std::slice::from_ref(&snap), false).unwrap();
        let (svc, sid) = (doc.svc.clone().unwrap(), doc.sid.clone().unwrap());
        svc.track_set_mix(&sid, "kick", Some(-3.0), Some(0.5)).unwrap();
        let all = svc.tracks_get(&sid).unwrap();
        for key in all["tracks"]["kick"].as_object().unwrap().keys() {
            assert!(
                SETTINGS.contains(&key.as_str()) || NOT_SETTINGS.contains(&key.as_str()),
                "the answer of the service has a key `{key}` that is neither a setting nor computed: decide it in service.rs"
            );
        }
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn the_overlay_takes_only_settings_and_all_of_them_and_survives_a_hostile_archive() {
        let base = json!({"role": "other", "note": 36, "mix_mode": "average", "source": {"kind": "wav"}, "segments": [{"start": 0.0}]});
        let hostile = map(json!({
            "role": "cymbal", "note": 60, "role_source": "user", "role_match": null, "color": "#112233",
            "params": {"min_gap_ms": 30, "shift_samples": 12}, "mix_mode": "ffmpeg", "volume_db": -3.0, "pan": 0.5,
            "follows": "kick", "detector": {"detector": "rust-dsp"}, "learn": {"marks": [1]}, "layer": {"file": "x.wav"},
            "sampler": {"o": 1}, "status": "offline", "offline": true, "source": null, "segments": "x", "events": 5,
            "sr": 1, "n_samples": 2, "duration": 3.0, "zz_unknown": 1
        }));
        let v = overlay_settings(&base, &hostile).expect("a valid record");
        for (k, want) in [
            ("role", json!("cymbal")),
            ("note", json!(60)),
            ("role_source", json!("user")),
            ("color", json!("#112233")),
            ("params", json!({"min_gap_ms": 30, "shift_samples": 12})),
            ("mix_mode", json!("ffmpeg")),
            ("volume_db", json!(-3.0)),
            ("pan", json!(0.5)),
            ("follows", json!("kick")),
            ("detector", json!({"detector": "rust-dsp"})),
            ("learn", json!({"marks": [1]})),
            ("layer", json!({"file": "x.wav"})),
            ("sampler", json!({"o": 1})),
        ] {
            assert_eq!(v[k], want, "{k}");
        }
        assert_eq!(v["source"], base["source"], "the source is never laid over");
        assert_eq!(v["segments"], base["segments"], "the segments neither");
        for k in [
            "status",
            "offline",
            "events",
            "sr",
            "n_samples",
            "duration",
            "zz_unknown",
        ] {
            assert!(v.get(k).is_none(), "{k} is not a setting");
        }
        // a wrong type (a note that is a string, a pan that is a list) is not laid over; the rest is
        let v = overlay_settings(
            &base,
            &map(json!({"role": "snare", "note": "high", "pan": [1], "volume_db": -6.0})),
        )
        .unwrap();
        assert_eq!(
            (v["role"].as_str(), v["note"].as_i64(), v["volume_db"].as_f64()),
            (Some("snare"), Some(36), Some(-6.0))
        );
        assert!(v.get("pan").is_none());
    }

    #[test]
    fn a_settings_map_with_no_role_guesses_the_role_and_takes_the_rest() {
        let cache = scratch("restore-norole");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let mut snap = snapshot_of(&cache, &id, 0, "0123456789abcdef0123456789abcdef");
        snap.label = Some("Snare Top".into());
        let stored = doc.analyse(std::slice::from_ref(&snap), false).unwrap();
        let name = doc.tracks[0].0.clone();
        doc.shutdown();
        let mut again = DocService::new(cache.clone(), id.clone());
        again
            .restore(&[RestoredTrack {
                snapshot: snap,
                track: name.clone(),
                events: stored.events[name.as_str()].clone(),
                settings: map(json!({"volume_db": -3.0, "note": 77, "status": "offline"})),
            }])
            .unwrap();
        let (svc, sid) = (again.svc.clone().unwrap(), again.sid.clone().unwrap());
        let t = svc.tracks_get(&sid).unwrap();
        let t = &t["tracks"][name.as_str()];
        assert_eq!(
            (t["role"].as_str(), t["note"].as_i64()),
            (Some("snare"), Some(38)),
            "a guessed role with its note, not no role"
        );
        assert_eq!(t["volume_db"], -3.0, "the other setting comes over it");
        again.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn an_automatic_role_from_an_archive_follows_a_new_daw_name_after_the_restore() {
        let cache = scratch("restore-follow");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let mut snap = snapshot_of(&cache, &id, 0, "0123456789abcdef0123456789abcdef");
        snap.label = Some("Kick In".into());
        let stored = doc.analyse(std::slice::from_ref(&snap), false).unwrap();
        let name = doc.tracks[0].0.clone();
        let (svc, sid) = (doc.svc.clone().unwrap(), doc.sid.clone().unwrap());
        let settings = settings_of(&svc, &sid, std::slice::from_ref(&name)).unwrap();
        let held = settings[name.as_str()].as_object().unwrap().clone();
        assert_eq!(held["role"], "kick");
        doc.shutdown();
        let mut again = DocService::new(cache.clone(), id.clone());
        again
            .restore(&[RestoredTrack {
                snapshot: snap.clone(),
                track: name.clone(),
                events: stored.events[name.as_str()].clone(),
                settings: held,
            }])
            .unwrap();
        // the DAW name changes after the restore: the automatic role follows, with one round
        snap.label = Some("Snare Bottom".into());
        assert!(again.analyse(std::slice::from_ref(&snap), false).unwrap().ran);
        let (svc2, sid2) = (again.svc.clone().unwrap(), again.sid.clone().unwrap());
        assert_eq!(
            svc2.tracks_get(&sid2).unwrap()["tracks"][name.as_str()]["role"],
            "snare"
        );
        again.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_restored_track_without_audio_answers_the_offline_error_not_a_panic() {
        let cache = scratch("offline");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let snap = snapshot_of(&cache, &id, 0, "kick.wav");
        let stored = doc.analyse(std::slice::from_ref(&snap), false).unwrap();
        doc.shutdown();
        std::fs::remove_file(&snap.wav).unwrap(); // the new process has no audio yet
        let mut again = DocService::new(cache.clone(), id);
        again
            .restore(&[RestoredTrack {
                snapshot: snap.clone(),
                track: "kick".into(),
                events: stored.events["kick"].clone(),
                settings: Map::new(),
            }])
            .unwrap();
        // the project opens and answers the events without audio
        let svc = again.svc.clone().unwrap();
        let sid = again.sid.clone().unwrap();
        assert_eq!(
            svc.events_get(&sid, "kick").unwrap()["events"].to_string(),
            stored.events["kick"].to_string()
        );
        // a call that needs the audio answers the app's error, it does not panic
        let started = svc.analyze_start(&sid, Some(vec!["kick".into()]), None, None);
        let answer = match started {
            Err(e) => e.message,
            Ok(_) => loop {
                let st = svc.analyze_status(&sid);
                if st["state"] == "done" {
                    break st["tracks"]["kick"]["error"].as_str().unwrap_or("no error").to_string();
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            },
        };
        assert!(
            answer.to_lowercase().contains("offline")
                || answer.to_lowercase().contains("not found")
                || answer.to_lowercase().contains("no such"),
            "the app's own error: {answer}"
        );
        // the round of the plugin skips a track without audio and keeps its stored events
        let mut restored_snap = snap;
        restored_snap.ready = false;
        let round = again.analyse(&[restored_snap], true).unwrap();
        assert!(round.analysed.is_empty());
        assert_eq!(
            round.skipped.len(),
            1,
            "the restored track without audio is named: {:?}",
            round.skipped
        );
        assert_eq!(round.events["kick"].to_string(), stored.events["kick"].to_string());
        again.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_restored_track_is_pointed_at_the_new_cache_file_and_keeps_its_hits() {
        // F6 (#582): the record of a restored track is made when the cache file does not exist; when the reader has
        // written the file, the service called the track offline (changed) until the record is pointed at it
        let cache = scratch("repoint");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let snap = snapshot_of(&cache, &id, 0, "kick.wav");
        let stored = doc.analyse(std::slice::from_ref(&snap), false).unwrap();
        doc.shutdown();
        let bytes = std::fs::read(&snap.wav).unwrap();
        std::fs::remove_file(&snap.wav).unwrap(); // the new document has no file of this source yet
        let mut again = DocService::new(cache.clone(), id);
        let mut restored_snap = snap.clone();
        restored_snap.ready = false;
        again
            .restore(&[RestoredTrack {
                snapshot: restored_snap,
                track: "kick".into(),
                events: stored.events["kick"].clone(),
                settings: Map::new(),
            }])
            .unwrap();
        std::fs::write(&snap.wav, bytes).unwrap(); // the reader wrote the file
        let status = |d: &DocService| {
            let svc = d.svc.clone().unwrap();
            svc.tracks_get(d.sid.as_ref().unwrap()).unwrap()["tracks"]["kick"]["status"]
                .as_str()
                .unwrap_or("")
                .to_string()
        };
        assert_eq!(
            status(&again),
            "changed",
            "the defect: the file is there, the record knows no identity"
        );
        let rows = again.repoint(&[(0, snap.wav.clone())]).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].before.as_str(), rows[0].after.as_str(), rows[0].repointed),
            ("changed", "ok", true)
        );
        assert_eq!(status(&again), "ok");
        assert_eq!(rows[0].events_json, stored.events["kick"].to_string(), "the hits stay");
        // a track that is online is left as it is
        let rows = again.repoint(&[(0, snap.wav.clone())]).unwrap();
        assert!(!rows[0].repointed && rows[0].after == "ok");
        assert!(!again.save_owed, "the save of the repointed record passed");
        // S4: a save that failed stays owed; the next call finds the track `ok` in memory and still saves
        again.save_owed = true;
        let rows = again.repoint(&[(0, snap.wav.clone())]).unwrap();
        assert!(!rows[0].repointed && !again.save_owed, "the owed save was made");
        // a source with no track gives no row (the caller keeps its mark and asks again)
        assert!(again.repoint(&[(5, snap.wav.clone())]).unwrap().is_empty());
        // S5: the text of a busy service starts with the code, the way the round deferral reads it
        assert!(err(drumspike_app::AppError::conflict("a job of the page is running")).starts_with("409"));
        again.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    /// A restored track whose cache file is back but whose record has no identity (status `changed`), with its service.
    fn changed_restored_track(name: &str) -> (std::path::PathBuf, DocService, SourceSnapshot) {
        let cache = scratch(name);
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let snap = snapshot_of(&cache, &id, 0, "kick.wav");
        let stored = doc.analyse(std::slice::from_ref(&snap), false).unwrap();
        doc.shutdown();
        let bytes = std::fs::read(&snap.wav).unwrap();
        std::fs::remove_file(&snap.wav).unwrap();
        let mut again = DocService::new(cache.clone(), id);
        let mut restored_snap = snap.clone();
        restored_snap.ready = false;
        again
            .restore(&[RestoredTrack {
                snapshot: restored_snap,
                track: "kick".into(),
                events: stored.events["kick"].clone(),
                settings: Map::new(),
            }])
            .unwrap();
        std::fs::write(&snap.wav, bytes).unwrap();
        (cache, again, snap)
    }

    #[test]
    fn the_repair_makes_one_try_on_a_busy_service_and_never_sleeps() {
        // #586: the repair runs on the worker with the service lock held; a 409 must come back at once, and the
        // retry helper (up to 10 sleeps of 100 ms) stays for the round's own calls
        let (cache, mut doc, snap) = changed_restored_track("repair-busy");
        busy_test::reset(u32::MAX);
        let e = doc.repoint_once(&[(0, snap.wav.clone())]).unwrap_err();
        assert!(e.starts_with("409"), "the worker reads the code in front: {e}");
        assert_eq!(busy_test::SLEEPS.with(|c| c.get()), 0, "the retry helper never slept");
        assert_eq!(busy_test::CALLS.with(|c| c.get()), 1, "one try");
        // the same call through the retry helper (the round): bounded at 10 sleeps, 11 tries
        busy_test::reset(u32::MAX);
        let e = doc.repoint(&[(0, snap.wav.clone())]).unwrap_err();
        assert!(e.starts_with("409"));
        assert_eq!(
            busy_test::SLEEPS.with(|c| c.get()),
            10,
            "the round keeps its bounded retry"
        );
        assert_eq!(busy_test::CALLS.with(|c| c.get()), 11);
        // nothing was changed by the busy tries: the track is still the restored one, and a free service repairs it
        busy_test::reset(0);
        let rows = doc.repoint_once(&[(0, snap.wav.clone())]).unwrap();
        assert_eq!(
            (rows[0].before.as_str(), rows[0].after.as_str(), rows[0].repointed),
            ("changed", "ok", true)
        );
        assert_eq!(busy_test::SLEEPS.with(|c| c.get()), 0);
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_save_that_is_busy_after_the_repair_is_saved_and_announced_by_the_next_try() {
        // #586: the record is pointed, the save answers 409 (one try): the call fails with the code in front; the next
        // call finds the track ok, saves, and still reports the repoint so the page reads the project again
        let (cache, mut doc, snap) = changed_restored_track("repair-busy-save");
        let wavs = [(0, snap.wav.clone())];
        // the first busy-aware call (the remove of the old record) passes, the second (the save) answers 409
        busy_test::reset_after(1, u32::MAX);
        let e = doc.repoint_once(&wavs).unwrap_err();
        assert!(e.starts_with("409"), "{e}");
        assert_eq!(busy_test::SLEEPS.with(|c| c.get()), 0, "one try, no sleep");
        assert!(
            doc.save_owed && doc.reload_owed,
            "the pointed record waits for its save and its reload"
        );
        busy_test::reset(0);
        let rows = doc.repoint_once(&wavs).unwrap();
        assert!(
            rows[0].after == "ok" && rows[0].repointed,
            "the next try saves and still tells the page (it could not be told before)"
        );
        assert!(!doc.save_owed && !doc.reload_owed);
        // a track that is online and has nothing owed is not announced again
        let rows = doc.repoint_once(&wavs).unwrap();
        assert!(!rows[0].repointed);
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn only_an_id_of_the_generated_form_is_accepted() {
        assert!(valid_doc_id(&new_doc_id()));
        assert!(valid_doc_id("0123456789abcdef"));
        for bad in [
            "",
            "../x",
            "/etc",
            "0123456789ABCDEF",
            "0123456789abcde",
            "0123456789abcdef0",
            "0123456789abcdeg",
            "..",
            "a/b",
            "0123456789abcde\\",
        ] {
            assert!(!valid_doc_id(bad), "{bad:?}");
        }
        // an id that is not accepted never reaches the file system
        let cache = Path::new("/c");
        assert_eq!(folder_of(cache, "../x"), cache.join("documents").join("refused-id"));
        assert_eq!(folder_of(cache, "/abs"), cache.join("documents").join("refused-id"));
        assert_eq!(
            folder_of(cache, "0123456789abcdef"),
            cache.join("documents").join("0123456789abcdef")
        );
    }

    #[test]
    fn a_track_name_from_an_archive_has_no_path_in_it() {
        assert!(valid_track_name("kick"));
        assert!(valid_track_name("m1a_kick_48k_mono"));
        for bad in ["", "../x", "a/b", "a\\b", "..", "a..b", "x\0y", "x\ny"] {
            assert!(!valid_track_name(bad), "{bad:?}");
        }
        assert!(!valid_track_name(&"k".repeat(201)));
    }

    #[test]
    fn audio_that_is_not_the_one_the_reader_wrote_is_not_analysed() {
        let cache = scratch("audio");
        let id = new_doc_id();
        let mut snap = snapshot_of(&cache, &id, 0, "kick.wav");
        assert_eq!(snap.audio_problem(), None);
        snap.wav_len += 1; // a file of another size: a leftover of an earlier session
        assert!(snap.audio_problem().is_some_and(|w| w.contains("size")));
        snap.wav_len -= 1;
        snap.ready = false; // not written in this session
        assert!(snap.audio_problem().is_some_and(|w| w.contains("no audio yet")));
        snap.ready = true;
        std::fs::remove_file(&snap.wav).unwrap();
        assert!(snap.audio_problem().is_some_and(|w| w.contains("missing")));
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_failed_round_leaves_one_track_per_source_and_the_retry_makes_no_second_track() {
        let cache = scratch("retry");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let kick = snapshot_of(&cache, &id, 0, "kick.wav");
        // a round that is cancelled before the analysis starts
        let canceller = doc.canceller();
        canceller.cancel();
        assert!(doc.analyse(std::slice::from_ref(&kick), false).is_err());
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    fn together_reference(names: &[&str]) -> String {
        let cache = scratch("ref");
        let id = new_doc_id();
        let mut one = DocService::new(cache.clone(), id.clone());
        let snaps: Vec<SourceSnapshot> = names
            .iter()
            .enumerate()
            .map(|(i, n)| snapshot_of(&cache, &id, i, n))
            .collect();
        let r = one.analyse(&snaps, false).unwrap();
        one.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
        serde_json::to_string(&r.events).unwrap()
    }

    #[test]
    fn a_track_that_cannot_be_read_is_reported_and_the_others_equal_the_reference_of_the_two() {
        let cache = scratch("bad");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let kick = snapshot_of(&cache, &id, 0, "kick.wav");
        let bad = snapshot_of(&cache, &id, 1, "snare.wav");
        let tom = snapshot_of(&cache, &id, 2, "tom.wav");
        // garbage over the header; the size stays, so only the decoder can tell
        {
            use std::io::{Seek, Write};
            let mut f = std::fs::OpenOptions::new().write(true).open(&bad.wav).unwrap();
            f.seek(std::io::SeekFrom::Start(0)).unwrap();
            f.write_all(b"NOTAWAVE-NOTAWAVE-NOTAWAVE").unwrap();
        }
        let r = doc.analyse(&[kick, bad, tom], false).unwrap();
        assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
        assert_eq!(r.errors[0].0, "snare");
        assert!(r.partial());
        // the bad track has no notes and no events; it is not complete with an empty list
        assert!(r.notes[&1].is_empty());
        assert!(r.events["snare"].as_array().is_none_or(Vec::is_empty));
        // the two good tracks equal the reference of the two together
        let mut good = Map::new();
        for n in ["kick", "tom"] {
            good.insert(n.to_string(), r.events[n].clone());
        }
        assert_eq!(
            serde_json::to_string(&good).unwrap(),
            together_reference(&["kick.wav", "tom.wav"])
        );
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_change_of_the_regions_of_an_earlier_source_makes_a_new_round_with_the_new_regions() {
        let cache = scratch("regions");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let mut kick = snapshot_of(&cache, &id, 0, "kick.wav");
        kick.regions = vec![RegionRec {
            start_mod: 0.0,
            dur_mod: 1.0,
            start_play: 0.0,
        }];
        let first = doc.analyse(std::slice::from_ref(&kick), false).unwrap();
        let before = first.events["kick"].as_array().unwrap().len();
        // the same source with a longer region: a new round, the segment is the new one
        kick.regions[0].dur_mod = 2.0;
        let second = doc.analyse(std::slice::from_ref(&kick), false).unwrap();
        assert!(second.ran, "a changed source starts a round");
        let after = second.events["kick"].as_array().unwrap().len();
        assert!(
            after > before,
            "the longer region holds more hits: {before} then {after}"
        );
        let text = std::fs::read_to_string(document_path(&doc.project_dir())).unwrap();
        let on_disk: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(on_disk["tracks"]["kick"]["segments"][0]["length"], 2.0);
        // the same snapshot again: nothing changed, no round
        assert!(!doc.analyse(&[kick], false).unwrap().ran);
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_hand_edit_survives_a_content_update_and_the_notes_stop_at_the_new_end() {
        let cache = scratch("update");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let kick = snapshot_of(&cache, &id, 0, "kick.wav");
        let first = doc.analyse(std::slice::from_ref(&kick), false).unwrap();
        let n = first.events["kick"].as_array().unwrap().len();
        assert!(n >= 3, "the first audio has three hits");
        let (svc, sid) = (doc.svc.clone().unwrap(), doc.sid.clone().unwrap());
        // the user moves the last hit to 2.0 s
        svc.events_edit(&sid, "kick", &[json!({"op": "move", "index": n - 1, "t": 2.0})])
            .unwrap();
        // the host gives new audio, shorter (1.25 s) with other hits: generation 1
        let mut new = kick.clone();
        new.audio_gen = 1;
        new.wav = wav_of_gen(&cache, &id, 0, "kick.wav", 1);
        write_drum_wav(&new.wav, &[0.2, 0.6], 1.25);
        new.sample_count = 60_000;
        new.wav_len = std::fs::metadata(&new.wav).map_or(0, |m| m.len());
        let second = doc.analyse(&[new], false).unwrap();
        assert!(second.ran, "new audio starts a round");
        let times: Vec<f64> = second.events["kick"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["t"].as_f64())
            .collect();
        assert!(
            times.iter().any(|t| (t - 2.0).abs() < 1e-9),
            "the hand-placed hit stays: {times:?}"
        );
        assert!(
            !times.iter().any(|t| (t - 0.9).abs() < 0.05),
            "a hit of the old audio is gone: {times:?}"
        );
        let notes = &second.notes[&0];
        assert!(!notes.is_empty());
        assert!(
            notes.iter().all(|[sample, _]| *sample < 60_000.0),
            "no note past the end of the new audio: {notes:?}"
        );
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_round_names_the_tracks_that_took_part_and_marks_a_result_without_audio_partial() {
        let cache = scratch("partial");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let kick = snapshot_of(&cache, &id, 0, "kick.wav");
        let mut snare = snapshot_of(&cache, &id, 1, "snare.wav");
        snare.ready = false; // restored: the host has not delivered the audio again
        let r = doc.analyse(&[kick, snare], false).unwrap();
        assert_eq!(r.analysed, ["kick"]);
        assert_eq!(r.skipped.len(), 1);
        assert_eq!(r.skipped[0].0, "snare");
        assert!(r.skipped[0].1.contains("no audio"));
        assert!(r.partial());
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_track_takes_its_place_by_the_creation_index_of_its_source() {
        let cache = scratch("order");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let kick = snapshot_of(&cache, &id, 0, "kick.wav");
        let snare = snapshot_of(&cache, &id, 1, "snare.wav");
        // the snare is analysed first (the kick failed to read), the kick comes in a retry
        doc.analyse(std::slice::from_ref(&snare), false).unwrap();
        let r = doc.analyse(&[kick, snare], false).unwrap();
        assert_eq!(r.project_tracks, ["kick", "snare"]);
        assert_eq!(r.events.keys().collect::<Vec<_>>(), ["kick", "snare"]);
        doc.shutdown();
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
        assert_eq!(
            doc.analyse(std::slice::from_ref(&a), false).unwrap().project_tracks,
            ["kick"]
        );
        assert_eq!(doc.analyse(&[a, b], false).unwrap().project_tracks, ["kick", "kick_2"]);
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_host_source_name_never_makes_a_path_out_of_the_audio_folder() {
        let cache = Path::new("/c");
        let id = "0123456789abcdef";
        for name in [
            "../../x/..",
            "a/..",
            "..",
            ".",
            "/abs/path/..",
            "x/../../../../../escape/..",
            "",
            "/",
            "a\\..\\b",
            "k\0ick",
        ] {
            let stem = stem_of(name);
            assert!(!stem.is_empty() && stem != "." && stem != "..", "{name:?} -> {stem:?}");
            assert!(!stem.contains(['/', '\\', '\0']), "{name:?} -> {stem:?}");
            let wav = wav_of(cache, id, 3, name);
            assert_eq!(
                wav.parent(),
                Some(audio_dir_of(cache, id).as_path()),
                "{name:?} -> {wav:?}"
            );
            assert!(
                wav.components().all(|c| !matches!(c, std::path::Component::ParentDir)),
                "{name:?} -> {wav:?}"
            );
        }
        // a clean name is kept
        assert_eq!(stem_of("/a/b/kick_48k.wav"), "kick_48k");
        assert_eq!(stem_of("m1a_kick.wav"), "m1a_kick");
    }

    #[test]
    fn a_move_in_playback_time_alone_is_no_change() {
        let cache = scratch("move");
        let id = new_doc_id();
        let mut a = snapshot_of(&cache, &id, 0, "kick.wav");
        a.regions = vec![RegionRec {
            start_mod: 0.0,
            dur_mod: 1.0,
            start_play: 0.0,
        }];
        let mut b = a.clone();
        b.regions[0].start_play = 5.0;
        assert!(a.same_as(&b), "only the playback time moved");
        b.regions[0].dur_mod = 2.0;
        assert!(!a.same_as(&b), "the modification range changed");
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_change_of_a_source_without_audio_is_not_lost() {
        let cache = scratch("lost");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let mut kick = snapshot_of(&cache, &id, 0, "kick.wav");
        kick.regions = vec![RegionRec {
            start_mod: 0.0,
            dur_mod: 1.0,
            start_play: 0.0,
        }];
        doc.analyse(std::slice::from_ref(&kick), false).unwrap();
        // the regions change while the audio is not here (ready false): the round skips the track
        let mut changed = kick.clone();
        changed.regions[0].dur_mod = 2.0;
        let mut offline = changed.clone();
        offline.ready = false;
        doc.analyse(std::slice::from_ref(&offline), false).unwrap();
        // the audio comes back: the change is still seen, the segment is the new one
        let r = doc.analyse(std::slice::from_ref(&changed), false).unwrap();
        assert!(r.ran, "the change was kept for the round with audio");
        let text = std::fs::read_to_string(document_path(&doc.project_dir())).unwrap();
        let on_disk: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(on_disk["tracks"]["kick"]["segments"][0]["length"], 2.0);
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn a_track_with_an_error_keeps_its_old_events_and_says_they_are_stale() {
        let cache = scratch("stale");
        let id = new_doc_id();
        let mut doc = DocService::new(cache.clone(), id.clone());
        let kick = snapshot_of(&cache, &id, 0, "kick.wav");
        let first = doc.analyse(std::slice::from_ref(&kick), false).unwrap();
        let old = first.events["kick"].clone();
        assert!(old.as_array().is_some_and(|e| !e.is_empty()));
        {
            use std::io::{Seek, Write};
            let mut f = std::fs::OpenOptions::new().write(true).open(&kick.wav).unwrap();
            f.seek(std::io::SeekFrom::Start(0)).unwrap();
            f.write_all(b"NOTAWAVE-NOTAWAVE-NOTAWAVE").unwrap();
        }
        let second = doc.analyse(std::slice::from_ref(&kick), true).unwrap();
        assert_eq!(second.errors.len(), 1, "{:?}", second.errors);
        assert_eq!(second.stale, ["kick"]);
        assert_eq!(second.events["kick"], old, "the old events stay, marked stale");
        assert!(second.partial());
        doc.shutdown();
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn stem_of_takes_the_file_stem_of_a_path() {
        assert_eq!(stem_of("/a/b/kick_48k.wav"), "kick_48k");
        assert_eq!(stem_of("kick.wav"), "kick");
    }
}
