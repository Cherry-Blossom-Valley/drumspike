//! Drum Spike ARA plugin (M1a, #525): a CLAP plugin in Rust that speaks ARA 2 through the
//! plain C interface and analyses the host's tracks with the Drum Spike service
//! (`drumspike-app`), wrapped as a self-contained VST3 by `clap-wrapper`.
//!
//! One chain: host -> CLAP -> ARA document controller in Rust -> the samples of every
//! audio source written into the cache folder -> the app service on the project built
//! from the sources and the playback regions -> the hits back to the host as ARA notes.
//!
//! Every document controller owns one `drumspike_app::Service` and one project folder
//! (`<cache>/documents/<id>/`), made with the controller and dropped with it (M1b1, see
//! `service.rs`); the id is made once and stored in the ARA archive. Every analysed source
//! stays a track of the project; a destroyed source leaves it.
//!
//! Environment (the test switches of the check; a real host sets none of them):
//!
//! - `DS_ARA_CACHE`: the cache folder (default `~/Library/Caches/DrumSpike/ARA`).
//! - `DS_ARA_OUT`: where the plugin writes its report (one JSON object per line).
//! - `DS_ARA_FAULT=in-process`: the fault switch — the analysis runs inside `process`
//!   (the audio thread) on purpose, so the overrun check of the host can fail.
//! - `DS_ARA_SERVICE_FAULT=per-job`, `DS_ARA_TRACKS_FAULT=forget`, `DS_ARA_FOLDER_FAULT=shared`,
//!   `DS_ARA_DESTROY_FAULT=keep`, `DS_ARA_HOME_FAULT=1`, `DS_ARA_RESTORE_FAULT=decode|newid|empty`:
//!   the other fault switches of the check; each breaks exactly one line of the M1b1 gate
//!   (the old M1a behaviour), so the check of that line can fail.
//!
//! No analysis runs on the audio thread and none on the host's main thread: the audio
//! readers are created and destroyed on the main thread (the ARA rule), the reads and
//! the service job run on their own threads, and `process` only passes audio through —
//! except under the fault switch, which is the deliberate exception that proves the
//! overrun check of the host can fail. Every CLAP entry point and every ARA callback of
//! the plugin runs inside a panic guard.
#![allow(
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    unused_variables,
    dead_code
)]

pub mod ara {
    #![allow(non_upper_case_globals, non_camel_case_types, non_snake_case, dead_code)]
    include!(concat!(env!("OUT_DIR"), "/ara.rs"));
}
pub mod editor;
pub mod service;
pub mod state;
pub mod transport;
pub mod wav;

use ara::*;
use clap_sys::ext::audio_ports::*;
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};
use clap_sys::version::CLAP_VERSION;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

pub(crate) struct SyncPtr<T>(pub T);
unsafe impl<T> Sync for SyncPtr<T> {}
unsafe impl<T> Send for SyncPtr<T> {}

/// The value of a fault or test switch of the check (`switches.json`, kinds `fault` and
/// `scenario`). The rule of the build: only a build with the cargo feature `test-switches` (the
/// gate builds it so) reads them; a delivery build has no such feature and always answers "not
/// set", so a variable in the environment of a host cannot change the plugin. The report
/// variables `DS_ARA_OUT` and `DS_ARA_CACHE` are not switches: they stay readable in both builds.
#[cfg(feature = "test-switches")]
pub(crate) fn test_switch(name: &str) -> Result<String, std::env::VarError> {
    std::env::var(name)
}

/// See the build with the feature: here the answer is always "not set".
#[cfg(not(feature = "test-switches"))]
pub(crate) fn test_switch(_name: &str) -> Result<String, std::env::VarError> {
    Err(std::env::VarError::NotPresent)
}

/// The version of the plugin crate as a C string: the ARA factory, the CLAP descriptor and the
/// bundle files (`check.sh`) all name this one value.
macro_rules! version_cs {
    () => {
        concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
    };
}

macro_rules! cs {
    ($s:literal) => {
        concat!($s, "\0").as_ptr() as *const c_char
    };
}

/// The boundary guard: every entry point runs its body inside `catch_unwind`. A panic
/// that reached the C ABI would abort the host; caught here, the entry point answers its
/// default. The default is explicit (`Result` and the CLAP status carry no `Default`).
/// `AssertUnwindSafe`: the bodies work on raw ARA handles and plugin state the unwind
/// cannot corrupt beyond what the default answer already covers.
fn guard<T>(where_: &'static str, default: T, body: impl FnOnce() -> T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("drumspike-ara: caught panic in {where_}");
            default
        }
    }
}

unsafe fn text(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// The properties an ARA update may change (the main thread writes, the worker reads).
#[derive(Debug, Clone, Default)]
struct SourceProps {
    name: String,
    persistent_id: String,
    sample_count: i64,
    sample_rate: f64,
    channels: i32,
}

/// The plugin state of one audio source of one document controller.
struct SourceState {
    /// Creation index in its document (stable key in the results; never reused).
    seq: usize,
    /// The document of the source (its folder, its service, its project).
    doc: Arc<DocShared>,
    /// The track of the source in the project, once it has one.
    track: Mutex<Option<String>>,
    /// The host destroyed the source.
    removed: AtomicBool,
    /// Name, persistent ID, sample count, rate and channels (the main thread updates).
    props: Mutex<SourceProps>,
    /// The host instance the reader reads through.
    host: SyncPtr<*const ARADocumentControllerHostInstance>,
    /// The host ref of the source.
    host_ref: SyncPtr<ARAAudioSourceHostRef>,
    /// The audio reader, created and destroyed on the main thread (the ARA rule).
    reader: Mutex<SyncPtr<ARAAudioReaderHostRef>>,
    /// The reader thread.
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Set between `enableAudioSourceSamplesAccess(false)` and the join of the reader.
    cancel: AtomicBool,
    /// The cached WAV, set by the reader thread when the write committed.
    wav: Mutex<Option<PathBuf>>,
    /// The read failed.
    failed: AtomicBool,
    /// The playback regions of the source, each with its own id (the ARA update and
    /// destroy calls name the region, not a position in a list).
    regions: Mutex<Vec<RegionSlot>>,
    /// The notes of the source: `(sample, volume)` per event.
    notes: Mutex<Vec<[f64; 2]>>,
    /// A job covered this source (its content is available).
    has_events: AtomicBool,
    /// The source takes part in the rounds once its audio is here: the host asked for its
    /// analysis, or the plugin took it as wanted (see `asked`).
    requested: AtomicBool,
    /// The host called `requestAudioSourceContentAnalysis` for this source. A source that the
    /// host never asked for is analysed all the same (LUNA does not ask), and the host is told
    /// when its notes are here, since it does not wait for them.
    asked: AtomicBool,
    /// The analysis of this source ended without a result (its read failed, its round failed
    /// or the service could not analyse its track). The host sees the analysis as complete
    /// (it must not wait for ever), and a new request tries again.
    analysis_failed: AtomicBool,
    /// The size of the committed WAV.
    wav_len: AtomicU64,
    /// The generation of the audio: the host changed the samples (`updateAudioSourceContent`)
    /// that many times. Each generation has its own WAV file, so the old file stays until the
    /// new one is committed (M1b4).
    audio_gen: AtomicU64,
    /// The samples changed and the new analysis is not done: the source is incomplete for the
    /// host and needs a round although it has a result (M1b4).
    reanalyse: AtomicBool,
    /// The samples access of the host is on (the plugin reads the new samples at once when the
    /// host updates the content).
    access: AtomicBool,
    /// WAV files of earlier generations, deleted when the new one is committed.
    old_wavs: Mutex<Vec<PathBuf>>,
    /// The read of the new audio of a content update failed: when to try it again (the reader is
    /// made on the host's main thread, so `notifyModelUpdates` does it), and how many tries were made.
    read_retry_at: Mutex<Option<std::time::Instant>>,
    read_tries: std::sync::atomic::AtomicU32,
    /// The events of the track are from an earlier round (its last round had an error); the
    /// archive stores the mark and a restore keeps the track "needs analysis".
    stale: AtomicBool,
    /// When the worker took the request that still waits for the audio of this source.
    requested_at: Mutex<Option<std::time::Instant>>,
}

impl SourceState {
    fn props(&self) -> SourceProps {
        self.props.lock().map(|p| p.clone()).unwrap_or_default()
    }

    /// The snapshot the service job works on.
    fn snapshot(&self) -> service::SourceSnapshot {
        let p = self.props();
        // R2: the path, the ready mark and the generation of the audio are read under ONE lock;
        // the content update takes the WAV and raises the generation under the same lock, so a
        // snapshot never holds the old path with the new generation
        let (wav, ready, audio_gen) = {
            let w = self.wav.lock().unwrap_or_else(|e| e.into_inner());
            let gen = self.audio_gen.load(Ordering::SeqCst);
            (w.clone(), w.is_some(), gen)
        };
        service::SourceSnapshot {
            seq: self.seq,
            name: p.name.clone(),
            persistent_id: p.persistent_id,
            sample_rate: p.sample_rate,
            sample_count: p.sample_count,
            // the WAV of this process; without one (a restored source) the path where the
            // reader will write it, marked not ready
            wav: wav.unwrap_or_else(|| self.doc.wav_path(self.seq, &p.name, audio_gen)),
            ready,
            wav_len: self.wav_len.load(Ordering::SeqCst),
            audio_gen,
            regions: self
                .regions
                .lock()
                .map(|r| r.iter().map(|slot| slot.rec).collect())
                .unwrap_or_default(),
        }
    }
}

/// One audio modification: the creation index of its source (the ARA graph links regions
/// to sources through their modifications).
struct Modification {
    seq: usize,
}

/// One playback region record of a source: its id (the handle the update and destroy
/// calls carry) and the times the service builds the segments from.
#[derive(Debug, Clone, Copy)]
struct RegionSlot {
    id: u64,
    rec: service::RegionRec,
}

/// The next region id (unique per process, so a destroyed region's record cannot be
/// confused with a later one).
static NEXT_REGION_ID: AtomicU64 = AtomicU64::new(1);

/// One playback region the host holds: its source's creation index and its record id.
struct PlaybackRegion {
    seq: usize,
    id: u64,
}

/// One content reader handed to the host.
struct Reader {
    notes: Vec<ARAContentNote>,
}

/// Everything the plugin keeps for one document controller: the service and the project
/// (in `svc`, held by the worker while it works), the document id, the events of the last
/// round (the archive stores them) and the source numbering. Made with the controller,
/// dropped with it.
struct DocShared {
    /// The cache folder (the same for every document; the document's own folder is below).
    cache: PathBuf,
    /// The document id (the name of the folder; a restore replaces it by the stored one).
    id: Mutex<String>,
    /// The Drum Spike service and the project of the document.
    svc: Mutex<service::DocService>,
    /// Cancels the running job of the project from another thread.
    canceller: service::Canceller,
    /// The events of every track as of the last round. A restore sets them on the calling
    /// thread (the worker may be busy for minutes with another document), so an archive
    /// stored right after a restore carries them.
    events: Mutex<serde_json::Map<String, Value>>,
    /// The next source creation index (per document; a destroyed source's index is not reused).
    next_seq: AtomicUsize,
    /// The analysis rounds run for the document.
    rounds: AtomicUsize,
    /// The controller is being destroyed: no new job starts.
    closing: AtomicBool,
    /// The sources of the document (the worker maps the events of a round back to them).
    members: Mutex<Vec<std::sync::Weak<SourceState>>>,
    /// Sources whose notes changed in a later round: the host is told in `notifyModelUpdates`.
    pending: Mutex<Vec<Arc<SourceState>>>,
    /// The editor's way to the service (it never takes the `svc` lock the worker holds).
    page: Arc<service::PageHandle>,
    /// S5: the regions and the rate each source had in the last round (or restore). The notes of
    /// the sources are mapped with THESE, by the worker (`notes_of`) and by the request thread
    /// (`record_page_edit`) alike: the events belong to that round, not to a region change of
    /// the host that its next round has not seen yet.
    round_regions: Mutex<HashMap<usize, (Vec<service::RegionRec>, f64)>>,
    /// The sample count of the snapshot of the last round, by source (the notes are made with it, T5).
    round_samples: Mutex<HashMap<usize, i64>>,
    /// A cheap change counter of the regions the page maps the host position with (M1b5): it grows
    /// when a region of a source changes, a round stores new regions, or a source leaves.
    region_gen: AtomicU64,
}

/// The regions and the rate to map the notes of a source with: those of the last round when
/// there is one, else the live ones.
fn regions_for_notes(
    round: Option<&(Vec<service::RegionRec>, f64)>,
    live: (Vec<service::RegionRec>, f64),
) -> (Vec<service::RegionRec>, f64) {
    round.cloned().unwrap_or(live)
}

/// The document ids in use in this process. Two live documents never share an id, so never
/// a folder: a session file that was copied, or opened twice, restores the same id.
static IDS_IN_USE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The live documents of this process (the unload of the library cancels their rounds).
static LIVE_DOCS: Mutex<Vec<std::sync::Weak<DocShared>>> = Mutex::new(Vec::new());

/// The worker thread runs (it ends on `Msg::Quit`, at the unload of the library).
static WORKER_RUNNING: AtomicBool = AtomicBool::new(false);

/// The number of the newest worker thread. A worker that is left behind at an unload (it was
/// inside a long track) must not clear [`WORKER_RUNNING`] of the worker a later init made.
static WORKER_GEN: AtomicUsize = AtomicUsize::new(0);

/// Make an id that no live document of this process has, and take it.
fn fresh_id() -> String {
    loop {
        let id = service::new_doc_id();
        let mut used = IDS_IN_USE.lock().unwrap_or_else(|e| e.into_inner());
        if !used.contains(&id) {
            used.push(id.clone());
            return id;
        }
    }
}

/// Take the id `want` (a stored id) for a document that has `old`: true when nobody else
/// has it. `DS_ARA_REGISTRY_FAULT=off` (the check's fault run) takes it without looking.
fn claim_id(old: &str, want: &str) -> bool {
    let mut used = IDS_IN_USE.lock().unwrap_or_else(|e| e.into_inner());
    let off = crate::test_switch("DS_ARA_REGISTRY_FAULT").as_deref() == Ok("off");
    if !off && used.iter().any(|u| u == want) && old != want {
        return false;
    }
    used.retain(|u| u != old);
    if !used.iter().any(|u| u == want) {
        used.push(want.to_string());
    }
    true
}

fn release_id(id: &str) {
    IDS_IN_USE.lock().unwrap_or_else(|e| e.into_inner()).retain(|u| u != id);
}

/// The unix time in nanoseconds (report rows that the check orders against the host's clock).
fn now_ns() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

/// `DS_ARA_PANIC_TEST` (the check's scenario switch): `reader` or `worker`.
fn panic_test() -> Option<String> {
    crate::test_switch("DS_ARA_PANIC_TEST").ok().filter(|v| !v.is_empty())
}

static PANIC_READER_DONE: AtomicBool = AtomicBool::new(false);
static PANIC_WORKER_DONE: AtomicBool = AtomicBool::new(false);

/// The scenario of the check: panic once in the thread `kind` (`reader` or `worker`).
fn test_panic(kind: &str) {
    if panic_test().as_deref() != Some(kind) {
        return;
    }
    let done = if kind == "reader" {
        &PANIC_READER_DONE
    } else {
        &PANIC_WORKER_DONE
    };
    if !done.swap(true, Ordering::SeqCst) {
        panic!("test panic in the {kind} thread");
    }
}

impl DocShared {
    fn new() -> Arc<DocShared> {
        let cache = session().cache.clone();
        let id = fresh_id();
        let svc = service::DocService::new(cache.clone(), id.clone());
        let doc = Arc::new(DocShared {
            cache,
            page: svc.page_handle(),
            round_regions: Mutex::new(HashMap::new()),
            round_samples: Mutex::new(HashMap::new()),
            region_gen: AtomicU64::new(0),
            canceller: svc.canceller(),
            id: Mutex::new(id),
            svc: Mutex::new(svc),
            events: Mutex::new(serde_json::Map::new()),
            next_seq: AtomicUsize::new(0),
            rounds: AtomicUsize::new(0),
            closing: AtomicBool::new(false),
            members: Mutex::new(Vec::new()),
            pending: Mutex::new(Vec::new()),
        });
        LIVE_DOCS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::downgrade(&doc));
        doc
    }

    fn id(&self) -> String {
        self.id.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Where the reader writes the WAV of a source of this document.
    fn wav_path(&self, seq: usize, name: &str, gen: u64) -> PathBuf {
        service::wav_of_gen(&self.cache, &self.id(), seq, name, gen)
    }

    /// The page changed the project (F2): before the request thread answers, the events of every
    /// track and the notes of the sources are in the document, so a host store that comes at once
    /// writes them. Runs on the request thread: it takes no `DocService` lock (the worker may hold
    /// it for a whole round), only the locks of the document's own state. The project is saved
    /// later by the worker. `DS_ARA_EDITOR_FAULT=no-record` takes this away (the old way: only
    /// the worker syncs, behind its current work).
    fn record_page_edit(&self, svc: &drumspike_app::Service, sid: &str) -> Result<bool, String> {
        let members: Vec<Arc<SourceState>> = self
            .members
            .lock()
            .map(|m| m.iter().filter_map(std::sync::Weak::upgrade).collect())
            .unwrap_or_default();
        let mut all = serde_json::Map::new();
        for s in members.iter().filter(|s| !s.removed.load(Ordering::SeqCst)) {
            let Some(track) = s.track.lock().ok().and_then(|t| t.clone()) else {
                continue;
            };
            let ev = svc
                .events_get(sid, &track)
                .map_err(|e| format!("events_get {track}: {e}"))?;
            all.insert(track, ev["events"].clone());
        }
        let changed = {
            let mut cur = self.events.lock().unwrap_or_else(|e| e.into_inner());
            if *cur == all {
                false
            } else {
                *cur = all.clone();
                true
            }
        };
        if !changed {
            return Ok(false);
        }
        if crate::test_switch("DS_ARA_EDITOR_FAULT").as_deref() != Ok("no-notes") {
            for s in &members {
                let Some(track) = s.track.lock().ok().and_then(|t| t.clone()) else {
                    continue;
                };
                let list = all.get(&track).and_then(Value::as_array).cloned().unwrap_or_default();
                let live: Vec<service::RegionRec> = s
                    .regions
                    .lock()
                    .map(|r| r.iter().map(|slot| slot.rec).collect())
                    .unwrap_or_default();
                let round = self
                    .round_regions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&s.seq)
                    .cloned();
                let (regions, rate) = regions_for_notes(round.as_ref(), (live, s.props().sample_rate));
                // the sample count of the snapshot the regions and the rate come from (T5)
                let count = match round {
                    Some(_) => self
                        .round_samples
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(&s.seq)
                        .copied(),
                    None => None,
                }
                .unwrap_or_else(|| s.props().sample_count);
                let notes = service::notes_for(&list, &regions, rate, count);
                let differs = s.notes.lock().map(|old| *old != notes).unwrap_or(false);
                if differs {
                    if let Ok(mut old) = s.notes.lock() {
                        *old = notes;
                    }
                    if s.has_events.load(Ordering::SeqCst) && !s.removed.load(Ordering::SeqCst) {
                        if let Ok(mut p) = self.pending.lock() {
                            if !p.iter().any(|x| Arc::ptr_eq(x, s)) {
                                p.push(s.clone());
                            }
                        }
                    }
                }
            }
        }
        Ok(true)
    }

    /// The regions of every source, `(source index, region)`, as the notes use them (M1b5): those
    /// of the last round when the source has one, else the live ones. The page maps the position
    /// of the host with them. Takes no `DocService` lock.
    fn round_region_list(&self) -> Vec<(usize, service::RegionRec)> {
        let members: Vec<Arc<SourceState>> = self
            .members
            .lock()
            .map(|m| m.iter().filter_map(std::sync::Weak::upgrade).collect())
            .unwrap_or_default();
        let round = self.round_regions.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut out = Vec::new();
        for s in members.iter().filter(|s| !s.removed.load(Ordering::SeqCst)) {
            let live: Vec<service::RegionRec> = s
                .regions
                .lock()
                .map(|r| r.iter().map(|slot| slot.rec).collect())
                .unwrap_or_default();
            let (regions, _) = regions_for_notes(round.get(&s.seq), (live, 0.0));
            out.extend(regions.into_iter().map(|r| (s.seq, r)));
        }
        out
    }

    /// Tell the page that the plugin changed its data (a round ended, a restore): the shell
    /// script of the page reloads the project (F1, F8).
    fn announce_reload(&self) {
        if crate::test_switch("DS_ARA_EDITOR_FAULT").as_deref() == Ok("no-reload") {
            return; // the fault run: the page is not told
        }
        self.page.log.push("plugin-reload", json!({"doc": self.id()}));
    }

    fn lock_svc(&self) -> std::sync::MutexGuard<'_, service::DocService> {
        self.svc.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// One message to the analysis worker.
enum Msg {
    /// A source's analysis was requested.
    Request(Arc<SourceState>),
    /// The host enabled the samples access of a source and its read starts: the source will
    /// have audio soon, so a round waits for it (the audio deadline), also without a request.
    Reading(Arc<SourceState>),
    /// A source's WAV is written.
    Ready(Arc<SourceState>),
    /// A source's read failed.
    /// The read of the audio of this generation failed (or was cancelled). A read of an older
    /// generation, cancelled by a content update, is not a failure (T3).
    Failed(Arc<SourceState>, u64),
    /// The host destroyed a source: its track leaves the project.
    Remove {
        src: Arc<SourceState>,
        sources_left: usize,
        tries: u32,
    },
    /// A round waits for a job of the page (S2): it comes back to the queue after a short time.
    Defer {
        doc: Arc<DocShared>,
        targets: Vec<Arc<SourceState>>,
        reason: &'static str,
        since: std::time::Instant,
    },
    /// The regions or the properties of a source changed: the document needs a round.
    Touch(Arc<DocShared>),
    /// The page changed the project: read the events again.
    PageSync(Arc<DocShared>),
    /// The plugin library is being unloaded: the worker ends.
    Quit,
    /// An archive was restored: the project opens with the stored events.
    Restore {
        doc: Arc<DocShared>,
        id: Option<String>,
        tracks: Vec<service::RestoredTrack>,
    },
    /// The document controller ends: save and close the project, drop the service.
    Close { doc: Arc<DocShared>, ack: Sender<()> },
}

/// The session: the state of the process that the document controllers share (the cache
/// folder, the report, the analysis worker). The service and the project of a document are
/// not here: each document controller owns its own ([`DocShared`]).
struct Session {
    /// The cache folder (everything the plugin writes lives below it).
    cache: PathBuf,
    /// Where the report goes (`DS_ARA_OUT`).
    out: Option<PathBuf>,
    /// One writer of the report file at a time (the worker, and the host's main thread for the
    /// rows of the editor view).
    out_lock: Mutex<()>,
    /// The rows of the report (read, notes and job rows, in order).
    rows: Mutex<Vec<String>>,
    /// The last service row (rewritten by every job and by a restore; the last line).
    service_row: Mutex<Option<String>>,
    /// To the worker.
    tx: Mutex<Option<Sender<Msg>>>,
    /// The worker thread (the unload of the library waits for it, a short time).
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The fault job ran (the fault switch runs the analysis inside `process` once).
    fault_ran: AtomicBool,
    /// The fault switch (`DS_ARA_FAULT=in-process`).
    fault: bool,
}

static SESSION: OnceLock<Session> = OnceLock::new();

/// The cache folder the README names, and its override.
fn cache_default() -> PathBuf {
    std::env::var("HOME")
        .map(|h| {
            PathBuf::from(h)
                .join("Library")
                .join("Caches")
                .join("DrumSpike")
                .join("ARA")
        })
        .unwrap_or_else(|_| PathBuf::from("/tmp/drumspike-ara"))
}

fn session() -> &'static Session {
    SESSION.get_or_init(|| Session {
        cache: std::env::var("DS_ARA_CACHE")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(cache_default),
        out: std::env::var("DS_ARA_OUT")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from),
        out_lock: Mutex::new(()),
        rows: Mutex::new(Vec::new()),
        service_row: Mutex::new(None),
        tx: Mutex::new(None),
        worker: Mutex::new(None),
        fault_ran: AtomicBool::new(false),
        fault: crate::test_switch("DS_ARA_FAULT").as_deref() == Ok("in-process"),
    })
}

/// The rows kept when no report file is set.
const RING_ROWS: usize = 512;

/// A row is waiting for the worker to write the report file.
static ROWS_DIRTY: AtomicBool = AtomicBool::new(false);

/// The analysis rounds run in this process (the report names the number: requests that
/// arrive close together are one round).
static ROUNDS: AtomicUsize = AtomicUsize::new(0);

/// How long the worker waits for another request before it starts a round (a DAW sets the
/// plugin on one track after the other), and the longest it keeps collecting.
const SETTLE_QUIET: std::time::Duration = std::time::Duration::from_millis(250);
const SETTLE_MAX: std::time::Duration = std::time::Duration::from_secs(2);

/// A round or a removal that waits for a job of the page is looked at again after this time, at
/// most this many times (about 10 minutes).
const DEFER_EVERY: std::time::Duration = std::time::Duration::from_millis(100);
const DEFER_MAX_TRIES: u32 = 6000;

/// Send `msg` to the worker after [`DEFER_EVERY`] (a short-lived thread: the worker itself never
/// sleeps for a job of the page).
fn defer_msg(msg: Msg) {
    let _ = std::thread::Builder::new()
        .name("drumspike-ara-defer".into())
        .spawn(move || {
            std::thread::sleep(DEFER_EVERY);
            let _ = send(session(), msg);
        });
}

/// The longest the unload of the library waits for the worker.
const UNLOAD_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

/// The report row of the services made and dropped in this process (written before the
/// last service row, so that row stays the last line).
fn services_row() -> String {
    format!(
        "{{\"event\":\"services\",\"made\":{},\"dropped\":{},\"rounds\":{}}}",
        service::SERVICES_MADE.load(Ordering::SeqCst),
        service::SERVICES_DROPPED.load(Ordering::SeqCst),
        ROUNDS.load(Ordering::SeqCst)
    )
}

/// What the worker holds between two messages.
#[derive(Default)]
struct WorkerState {
    /// Since when the round of a document waits for a job of the page (by document id).
    deferred_since: std::collections::HashMap<String, std::time::Instant>,
    /// Sources that asked for the analysis and still need it.
    requested: Vec<Arc<SourceState>>,
    /// Documents that need a round without a new request, with the reason (`removal`: a
    /// source was destroyed; `change`: the regions or the properties of a source changed).
    stale: Vec<(Arc<DocShared>, &'static str)>,
    /// Documents with a failed round that the worker tries again by itself (M2).
    retry: Vec<RetryEntry>,
    /// The sources of the round that runs now (a panic in the worker ends them as failed,
    /// so the host never waits for ever).
    current: Vec<Arc<SourceState>>,
}

/// The automatic retry of a failed round (M1b4): a host that never requests would leave a track
/// without hits until the DAW restarts. The worker tries again `RETRY_MAX` times with a growing wait
/// (`DS_ARA_RETRY_MS`, default 2000, then double), then writes the row `retry_gave_up` and stops.
/// Anything that changes for the document (a new source, a region change, an end of a job of the
/// page, a request) starts the count again.
/// `DS_ARA_ROUND_FAULT=no-retry` brings back the old way: no retry; `retry-forever` never gives up.
const RETRY_MAX: u32 = 3;

struct RetryEntry {
    doc: Arc<DocShared>,
    /// How many retries were planned.
    tries: u32,
    /// When the next retry is due; `None` while it runs or after the plugin gave up.
    due: Option<std::time::Instant>,
    gave_up: bool,
}

fn retry_base() -> std::time::Duration {
    let ms = crate::test_switch("DS_ARA_RETRY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(2000)
        // clamped: no value of the variable may make the worker wait for hours or overflow
        .clamp(10, 60_000);
    std::time::Duration::from_millis(ms)
}

/// The time until the next retry is due.
fn retry_left(st: &WorkerState) -> Option<std::time::Duration> {
    st.retry
        .iter()
        .filter_map(|e| e.due)
        .map(|d| d.saturating_duration_since(std::time::Instant::now()))
        .filter(|d| !d.is_zero())
        .min()
}

/// A source whose analysis failed, has its audio here and no read going on: a retry can help.
fn retryable(s: &Arc<SourceState>) -> bool {
    !s.removed.load(Ordering::SeqCst)
        && s.analysis_failed.load(Ordering::SeqCst)
        && !s.failed.load(Ordering::SeqCst)
        && has_wav(s)
}

/// A source whose analysis is still to do: no result yet, or the last round ended without
/// one. A request for any other source changes nothing and starts no round.
fn needs_analysis(s: &Arc<SourceState>) -> bool {
    !s.removed.load(Ordering::SeqCst)
        && (!s.has_events.load(Ordering::SeqCst)
            || s.analysis_failed.load(Ordering::SeqCst)
            || s.reanalyse.load(Ordering::SeqCst))
}

/// Take the sources of `doc` that need no analysis (they have their result) out of the queue.
fn drop_done(st: &mut WorkerState, doc: &Arc<DocShared>) {
    st.requested.retain(|s| !Arc::ptr_eq(&s.doc, doc) || needs_analysis(s));
}

/// A source with a result that waits for new audio (a content update) and has a read running or
/// a retry planned: states 1 and 2 of the table in the design note. A round for another reason
/// leaves it out and does not mark it failed (U2).
fn waits_for_new_audio(s: &Arc<SourceState>) -> bool {
    s.reanalyse.load(Ordering::SeqCst)
        && s.has_events.load(Ordering::SeqCst)
        && (reader_running(s) || s.read_retry_at.lock().map(|t| t.is_some()).unwrap_or(false))
}

/// The host is told that the analysis state of a source changed (it is complete now, without a new
/// result): `notifyAudioSourceContentChanged` goes out inside `notifyModelUpdates`.
fn tell_host_of_failure(s: &Arc<SourceState>) {
    if let Ok(mut p) = s.doc.pending.lock() {
        if !p.iter().any(|x| Arc::ptr_eq(x, s)) {
            p.push(s.clone());
        }
    }
}

/// The host turned the samples access off. A source that waits for new audio and is not failed yet
/// (state 1 or 2: no reader reports back any more, no retry will run) goes to state 4: failed for
/// good, complete for the host, the old hits stay. A later enable starts again at state 1 (U1).
fn leave_for_access_off(s: &Arc<SourceState>) -> bool {
    if s.reanalyse.load(Ordering::SeqCst)
        && !has_wav(s)
        && !s.analysis_failed.load(Ordering::SeqCst)
        && !s.removed.load(Ordering::SeqCst)
    {
        s.analysis_failed.store(true, Ordering::SeqCst);
        tell_host_of_failure(s);
        session().log_deferred(
            json!({"event": "source_read_failed", "name": s.props().name, "tries": s.read_tries.load(Ordering::SeqCst), "retry": false, "why": "access off"})
                .to_string(),
        );
        return true;
    }
    false
}

/// A round runs although the service sees no change only when a target has new audio to
/// analyse. A source whose read was cancelled or failed has none: a round for it would
/// analyse the complete tracks again for nothing (M1b4).
/// `DS_ARA_ROUND_FAULT=force-failed` brings back the old rule: any target forces a round.
fn round_forced(targets: &[Arc<SourceState>]) -> bool {
    if crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() == Ok("force-failed") {
        return !targets.is_empty();
    }
    targets.iter().any(has_wav)
}

/// How long a request waits for the audio of its source before the round goes on without it
/// (the late source is reported and the result is partial). 30 s; the scenario
/// `DS_ARA_SCENARIO=audio-deadline-ms:<n>` changes it for a stage, and
/// `DS_ARA_ROUND_FAULT=no-deadline` (the check's fault run) waits for ever, as the first
/// version did.
fn audio_deadline() -> Option<std::time::Duration> {
    if crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() == Ok("no-deadline") {
        return None;
    }
    let ms = crate::test_switch("DS_ARA_SCENARIO")
        .ok()
        .and_then(|v| v.strip_prefix("audio-deadline-ms:").and_then(|n| n.parse::<u64>().ok()))
        .unwrap_or(30_000);
    Some(std::time::Duration::from_millis(ms))
}

/// The time left until this request is late, or `None` when it does not wait for audio.
fn deadline_left(s: &Arc<SourceState>) -> Option<std::time::Duration> {
    if has_wav(s) || s.failed.load(Ordering::SeqCst) {
        return None;
    }
    let limit = audio_deadline()?;
    let at = (*s.requested_at.lock().unwrap_or_else(|e| e.into_inner()))?;
    Some(limit.saturating_sub(at.elapsed()))
}

/// How long the worker may wait for a message: the shortest time until a request that still
/// waits becomes late. A request that is late already (left is zero) is not waited for: its
/// document is ready as soon as the other requests of it have their audio, and a wait of zero
/// would make the worker spin until then.
fn wait_left(requested: &[Arc<SourceState>]) -> Option<std::time::Duration> {
    requested
        .iter()
        .filter_map(deadline_left)
        .filter(|d| !d.is_zero())
        .min()
}

fn has_wav(s: &Arc<SourceState>) -> bool {
    s.wav.lock().map(|w| w.is_some()).unwrap_or(false)
}

/// A source takes part in the rounds of its document once it was requested (its audio is
/// here) or was restored with a result. Never one that was only created.
fn takes_part(s: &Arc<SourceState>) -> bool {
    !s.removed.load(Ordering::SeqCst)
        && (s.has_events.load(Ordering::SeqCst) || (s.requested.load(Ordering::SeqCst) && has_wav(s)))
}

/// The sources of a document that the worker still has to analyse, grouped per document in
/// the order of their first request.
fn groups(requested: &[Arc<SourceState>]) -> Vec<(Arc<DocShared>, Vec<Arc<SourceState>>)> {
    let mut out: Vec<(Arc<DocShared>, Vec<Arc<SourceState>>)> = Vec::new();
    for s in requested {
        match out.iter_mut().find(|(d, _)| Arc::ptr_eq(d, &s.doc)) {
            Some((_, list)) => list.push(s.clone()),
            None => out.push((s.doc.clone(), vec![s.clone()])),
        }
    }
    out
}

/// The documents the worker can run a round for: every document whose requested sources
/// all have their WAV (or failed), and every document that needs a round and has no
/// request waiting for its audio.
fn ready_documents(st: &WorkerState) -> Vec<(Arc<DocShared>, Vec<Arc<SourceState>>)> {
    let mut ready: Vec<(Arc<DocShared>, Vec<Arc<SourceState>>)> = groups(&st.requested)
        .into_iter()
        .filter(|(_, list)| {
            list.iter().all(|s| {
                has_wav(s) || s.failed.load(Ordering::SeqCst) || deadline_left(s) == Some(std::time::Duration::ZERO)
            })
        })
        .collect();
    for (doc, _) in &st.stale {
        let waiting = st.requested.iter().any(|s| Arc::ptr_eq(&s.doc, doc));
        if !waiting {
            ready.push((doc.clone(), Vec::new()));
        }
    }
    ready
}

impl Session {
    fn log(&self, line: String) {
        if let Ok(mut rows) = self.rows.lock() {
            // without a report file nobody reads the rows: keep a small ring, not all of them
            // for the life of the process (the unit tests read it)
            if self.out.is_none() && rows.len() >= RING_ROWS {
                rows.remove(0);
            }
            rows.push(line);
        }
    }

    /// A row from a thread that must not write the file (the host's main thread, in the editor
    /// view callbacks): the row is kept and the worker writes the file at its next wake.
    fn log_deferred(&self, line: String) {
        self.log(line);
        ROWS_DIRTY.store(true, Ordering::SeqCst);
    }

    /// Rewrite the report: the rows, the services row, then the last service row. A reader
    /// never sees half a file (write to a temporary, rename).
    fn write_out(&self) {
        let Some(out) = &self.out else { return };
        let _one = self.out_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut text = self.rows.lock().map(|r| r.join("\n")).unwrap_or_default();
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&services_row());
        if let Some(row) = self.service_row.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            text.push('\n');
            text.push_str(row);
        }
        text.push('\n');
        let mut tmp = out.clone().into_os_string();
        tmp.push(".tmp");
        if std::fs::write(&tmp, text.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, out);
        }
    }

    /// The thread of the worker: [`Session::worker_loop`] inside a panic guard. A panic
    /// ends the round it happened in (its sources end as failed, so the host does not wait
    /// for them, and a later request tries again) and the loop starts again: the worker is
    /// shared by every document of the process and must not end. `DS_ARA_GUARD_FAULT=off`
    /// (the check's fault run) takes the guard away.
    fn worker_main(&'static self, rx: Receiver<Msg>, generation: usize) {
        let mut st = WorkerState::default();
        let unguarded = crate::test_switch("DS_ARA_GUARD_FAULT").as_deref() == Ok("off");
        loop {
            let run = std::panic::AssertUnwindSafe(|| self.worker_loop(&rx, &mut st));
            if unguarded {
                self.worker_loop(&rx, &mut st);
                break;
            }
            match std::panic::catch_unwind(run) {
                Ok(()) => break,
                Err(_) => {
                    eprintln!("drumspike-ara: caught panic in the worker thread, the worker starts again");
                    let lost: Vec<Arc<SourceState>> = st.requested.drain(..).chain(st.current.drain(..)).collect();
                    for s in &lost {
                        s.analysis_failed.store(true, Ordering::SeqCst);
                    }
                    st.stale.clear();
                    self.log(format!(
                        "{{\"event\":\"worker_restarted\",\"sources_failed\":{}}}",
                        lost.len()
                    ));
                    self.write_out();
                }
            }
        }
        if WORKER_GEN.load(Ordering::SeqCst) == generation {
            WORKER_RUNNING.store(false, Ordering::SeqCst);
        }
    }

    /// The worker loop. A document is ready when every source of it that asked for the
    /// analysis has its WAV (or its read failed); a document that lost a source or whose
    /// regions changed is ready at once. Ready documents wait for a short quiet time (a DAW
    /// adds the sources one after the other, so eight requests are one round, not eight),
    /// then one round runs over the document: ALL its tracks are analysed together.
    /// Removals, restores and closes of a document run on this thread in the order they
    /// arrive, so they never overlap a round. Returns when the channel is closed or on
    /// `Msg::Quit`.
    fn worker_loop(&'static self, rx: &Receiver<Msg>, st: &mut WorkerState) {
        loop {
            // a request that waits for audio has a deadline: the wait for a message ends then
            // (a document that is ready now, for example one with a late source, runs first)
            self.poll_evidence();
            self.run_due_retries(st);
            let got = if !ready_documents(st).is_empty() {
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            } else {
                match [wait_left(&st.requested), retry_left(st)].into_iter().flatten().min() {
                    Some(left) => rx.recv_timeout(left.min(std::time::Duration::from_secs(1))),
                    // the wake of one second lets the worker write the evidence rows of the audio thread
                    None => rx.recv_timeout(std::time::Duration::from_secs(1)),
                }
            };
            match got {
                Ok(msg) => {
                    if self.absorb(msg, st) {
                        return;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
            let mut settled = false;
            loop {
                while let Ok(msg) = rx.try_recv() {
                    if self.absorb(msg, st) {
                        return;
                    }
                }
                let ready = ready_documents(st);
                if ready.is_empty() {
                    break;
                }
                if !settled {
                    if self.settle(rx, st) {
                        return;
                    }
                    settled = true;
                    continue;
                }
                for (doc, list) in ready {
                    st.requested.retain(|s| !Arc::ptr_eq(&s.doc, &doc));
                    let reason = st
                        .stale
                        .iter()
                        .position(|(d, _)| Arc::ptr_eq(d, &doc))
                        .map(|i| st.stale.remove(i).1);
                    let targets: Vec<Arc<SourceState>> = list.into_iter().filter(needs_analysis).collect();
                    if doc.closing.load(Ordering::SeqCst) || (targets.is_empty() && reason.is_none()) {
                        continue; // nothing changed: no round
                    }
                    st.current = targets.clone();
                    test_panic("worker");
                    let why = if targets.is_empty() {
                        reason.unwrap_or("change")
                    } else {
                        "request"
                    };
                    // a source that needs analysis starts a round even when the service sees no
                    // change (a stale track after a restore: its snapshot did not change)
                    let force = round_forced(&targets);
                    // S2: a job of the page runs: the round goes back to the queue (a timer
                    // brings it again) and the worker serves the other documents
                    let since = *st
                        .deferred_since
                        .entry(doc.id())
                        .or_insert_with(std::time::Instant::now);
                    if crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() != Ok("no-page-wait")
                        && doc.page.job_running()
                        && since.elapsed() < std::time::Duration::from_secs(600)
                    {
                        defer_msg(Msg::Defer {
                            doc: doc.clone(),
                            targets: targets.clone(),
                            reason: why,
                            since,
                        });
                        st.current.clear();
                        continue;
                    }
                    let allow = since.elapsed() < std::time::Duration::from_secs(600)
                        && crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() != Ok("no-page-wait");
                    if self.run_job(&doc, &targets, why, force, allow) {
                        // R4: the 409 after the short retry goes back to the queue like a job of the page
                        defer_msg(Msg::Defer {
                            doc: doc.clone(),
                            targets: targets.clone(),
                            reason: why,
                            since,
                        });
                        st.current.clear();
                        continue;
                    }
                    st.deferred_since.remove(&doc.id());
                    st.current.clear();
                    self.plan_retry(&doc, st);
                }
                settled = false;
            }
        }
    }

    /// Wait until no message came for [`SETTLE_QUIET`] (at most [`SETTLE_MAX`] in all).
    /// True when the worker must end (the channel is closed, or `Msg::Quit` came).
    fn settle(&'static self, rx: &Receiver<Msg>, st: &mut WorkerState) -> bool {
        // the fault run: no settle time, every request starts a round of its own
        if crate::test_switch("DS_ARA_SETTLE_FAULT").as_deref() == Ok("none") {
            return false;
        }
        let deadline = std::time::Instant::now() + SETTLE_MAX;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return false;
            }
            match rx.recv_timeout(SETTLE_QUIET.min(left)) {
                Ok(msg) => {
                    if self.absorb(msg, st) {
                        return true;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return false,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return true,
            }
        }
    }

    /// Queue a source that needs analysis and is not queued (and no round works on it) yet.
    fn want(&self, s: &Arc<SourceState>, st: &mut WorkerState) {
        // the fault run `DS_ARA_ROUND_FAULT=read-again`: the read of a source that has its
        // result (one restored from an archive) starts a round of the document all the same
        if crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() == Ok("read-again")
            && !self.fault
            && !s.removed.load(Ordering::SeqCst)
            && !needs_analysis(s)
            && !st.stale.iter().any(|(d, _)| Arc::ptr_eq(d, &s.doc))
        {
            st.stale.push((s.doc.clone(), "removal"));
            return;
        }
        if self.fault
            || !needs_analysis(s)
            || s.doc.closing.load(Ordering::SeqCst)
            || st.requested.iter().any(|r| Arc::ptr_eq(r, s))
            || st.current.iter().any(|r| Arc::ptr_eq(r, s))
        {
            return;
        }
        // a new source, new audio: the count of retries starts again
        st.retry.retain(|e| !Arc::ptr_eq(&e.doc, &s.doc));
        s.requested.store(true, Ordering::SeqCst);
        *s.requested_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
        st.requested.push(s.clone());
    }

    /// After a round: plan a retry when a source of the document is left with a failed analysis.
    fn plan_retry(&self, doc: &Arc<DocShared>, st: &mut WorkerState) {
        if self.fault
            || crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() == Ok("no-retry")
            || doc.closing.load(Ordering::SeqCst)
        {
            return;
        }
        let failed_left = doc
            .members
            .lock()
            .map(|m| m.iter().filter_map(std::sync::Weak::upgrade).any(|s| retryable(&s)))
            .unwrap_or(false);
        let at = st.retry.iter().position(|e| Arc::ptr_eq(&e.doc, doc));
        if !failed_left {
            if let Some(i) = at {
                st.retry.remove(i);
            }
            return;
        }
        let now = std::time::Instant::now();
        match at {
            None => st.retry.push(RetryEntry {
                doc: doc.clone(),
                tries: 1,
                due: Some(now + retry_base()),
                gave_up: false,
            }),
            Some(i) => {
                let e = &mut st.retry[i];
                if e.gave_up {
                    return;
                }
                if e.tries >= RETRY_MAX && crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() != Ok("retry-forever") {
                    e.gave_up = true;
                    e.due = None;
                    self.log(json!({"event": "retry_gave_up", "doc": doc.id(), "tries": e.tries}).to_string());
                    self.write_out();
                    return;
                }
                e.due = Some(now + retry_base().saturating_mul(2u32.pow(e.tries.min(6))));
                e.tries += 1;
            }
        }
    }

    /// Start the retries that are due: the failed sources go back to the queue.
    fn run_due_retries(&self, st: &mut WorkerState) {
        let now = std::time::Instant::now();
        let mut again: Vec<Arc<SourceState>> = Vec::new();
        for e in st.retry.iter_mut() {
            if e.due.is_some_and(|d| d <= now) {
                e.due = None;
                if e.doc.closing.load(Ordering::SeqCst) {
                    continue;
                }
                self.log(json!({"event": "retry", "doc": e.doc.id(), "try": e.tries}).to_string());
                if let Ok(m) = e.doc.members.lock() {
                    again.extend(m.iter().filter_map(std::sync::Weak::upgrade).filter(retryable));
                }
            }
        }
        for s in again {
            if !st.requested.iter().any(|r| Arc::ptr_eq(r, &s)) && !st.current.iter().any(|r| Arc::ptr_eq(r, &s)) {
                *s.requested_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
                st.requested.push(s);
            }
        }
    }

    /// Take one message. True for `Msg::Quit`.
    fn absorb(&'static self, msg: Msg, st: &mut WorkerState) -> bool {
        match msg {
            Msg::Request(s) => {
                // the fault run brings back the old way: a request for a complete source
                // starts a full round as well
                let always = crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() == Ok("always");
                if s.removed.load(Ordering::SeqCst) || (!needs_analysis(&s) && !always) {
                    return false; // a source that is complete: nothing changed, no round
                }
                if always && !st.stale.iter().any(|(d, _)| Arc::ptr_eq(d, &s.doc)) {
                    st.stale.push((s.doc.clone(), "removal"));
                }
                st.retry.retain(|e| !Arc::ptr_eq(&e.doc, &s.doc)); // a request tries at once, the count starts again
                if !st.requested.iter().any(|r| Arc::ptr_eq(r, &s)) {
                    *s.requested_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
                    st.requested.push(s);
                }
            }
            Msg::Failed(s, gen) => {
                // T3: a read that a content update cancelled belongs to an older generation: the
                // new read follows, nothing failed. (`DS_ARA_ROUND_FAULT=cancel-fails` counts it.)
                if gen != s.audio_gen.load(Ordering::SeqCst)
                    && crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() != Ok("cancel-fails")
                {
                    return false;
                }
                // The read of the NEW audio of a content update failed (state table in the design
                // note). With the samples access on and tries left, a retry is planned and the
                // source stays incomplete (something is still coming). Else it fails for good:
                // `analysis_failed` makes it complete for the host (the old hits stay), and the row
                // names it. `DS_ARA_ROUND_FAULT=no-read-retry`: no retry.
                if s.reanalyse.load(Ordering::SeqCst) && !s.removed.load(Ordering::SeqCst) {
                    let tries = s.read_tries.load(Ordering::SeqCst);
                    let retry = s.access.load(Ordering::SeqCst)
                        && tries < RETRY_MAX
                        && crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() != Ok("no-read-retry");
                    if retry {
                        s.read_tries.store(tries + 1, Ordering::SeqCst);
                        let wait = retry_base().saturating_mul(2u32.pow(tries.min(6)));
                        *s.read_retry_at.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(std::time::Instant::now() + wait);
                    } else {
                        s.analysis_failed.store(true, Ordering::SeqCst);
                        tell_host_of_failure(&s);
                    }
                    self.log(
                        json!({"event": "source_read_failed", "name": s.props().name, "tries": tries, "retry": retry})
                            .to_string(),
                    );
                    self.write_out();
                }
            }
            Msg::Reading(s) => {
                // M1b4: the host reads the audio of a source and may never ask for its analysis
                // (LUNA). The plugin takes the source as wanted when it has no result, so one
                // round covers all the sources whose audio is on the way (the deadline of
                // `audio_deadline` holds). A later request for it finds it queued or done.
                // `DS_ARA_ROUND_FAULT=request-only` brings back the old way (a round only on
                // a request); `ready-only` queues a source only when its audio is here, so
                // sources that arrive one after the other get a round each.
                let fault = crate::test_switch("DS_ARA_ROUND_FAULT").unwrap_or_default();
                if fault != "request-only" && fault != "ready-only" && fault != "no-late-round" {
                    self.want(&s, st);
                }
            }
            Msg::Ready(s) => {
                // The audio of a source that has no result is here now. That is a source the
                // host never asked for (M1b4), or one that has no result after a late audio, a
                // failed round or a stale track after a restore: a round takes it, the host
                // need not ask again. A restored source with a result is not queued.
                // (`DS_ARA_ROUND_FAULT=no-late-round` brings back the first version: nothing
                // happens; `request-only`: only a source whose analysis failed is queued.)
                let fault = crate::test_switch("DS_ARA_ROUND_FAULT").unwrap_or_default();
                if fault != "no-late-round" {
                    if fault == "request-only" {
                        if s.analysis_failed.load(Ordering::SeqCst) {
                            self.want(&s, st);
                        }
                    } else {
                        self.want(&s, st);
                    }
                }
            }
            Msg::Remove {
                src,
                sources_left,
                tries,
            } => {
                st.requested.retain(|r| !Arc::ptr_eq(r, &src));
                st.retry.retain(|e| !Arc::ptr_eq(&e.doc, &src.doc));
                let busy_fault = crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() == Ok("no-page-wait");
                // M3: a job of the page blocks the removal of a track (the app answers 409). The
                // worker does not wait for it: the removal comes back to the queue, the events
                // of the document stay as they are, and other documents are served meanwhile.
                if !busy_fault && tries < DEFER_MAX_TRIES && src.doc.page.job_running() {
                    defer_msg(Msg::Remove {
                        src,
                        sources_left,
                        tries: tries + 1,
                    });
                    return false;
                }
                if let Err(e) = self.remove_track(&src, sources_left) {
                    if !busy_fault && tries < DEFER_MAX_TRIES && !src.doc.closing.load(Ordering::SeqCst) {
                        self.log(json!({"event": "source_destroy_retry", "doc": src.doc.id(), "error": e}).to_string());
                        defer_msg(Msg::Remove {
                            src,
                            sources_left,
                            tries: tries + 1,
                        });
                        return false;
                    }
                }
                // R4: the files of a destroyed source go (its current WAV and the old generations)
                if crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() != Ok("keep-wavs") {
                    let mut files: Vec<PathBuf> =
                        std::mem::take(&mut *src.old_wavs.lock().unwrap_or_else(|e| e.into_inner()));
                    files.extend(src.wav.lock().unwrap_or_else(|e| e.into_inner()).clone());
                    for f in files {
                        let _ = std::fs::remove_file(f);
                    }
                }
                // the tracks that are left are analysed together again
                if !src.doc.closing.load(Ordering::SeqCst)
                    && crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() != Ok("new-only")
                {
                    // a removal outranks a change that came before it (the host destroys the
                    // regions of a source before the source)
                    match st.stale.iter_mut().find(|(d, _)| Arc::ptr_eq(d, &src.doc)) {
                        Some(entry) => entry.1 = "removal",
                        None => st.stale.push((src.doc.clone(), "removal")),
                    }
                }
            }
            Msg::Defer {
                doc,
                targets,
                reason,
                since,
            } => {
                if doc.closing.load(Ordering::SeqCst) {
                    return false;
                }
                // back in the queue: the sources ask again, a round without sources is a stale mark
                for s in targets {
                    if !s.removed.load(Ordering::SeqCst) && !st.requested.iter().any(|r| Arc::ptr_eq(r, &s)) {
                        st.requested.push(s);
                    }
                }
                if !st.stale.iter().any(|(d, _)| Arc::ptr_eq(d, &doc)) {
                    st.stale.push((doc.clone(), reason));
                }
                st.deferred_since.insert(doc.id(), since);
            }
            Msg::Touch(doc) => {
                st.retry.retain(|e| !Arc::ptr_eq(&e.doc, &doc));
                if !doc.closing.load(Ordering::SeqCst) && !st.stale.iter().any(|(d, _)| Arc::ptr_eq(d, &doc)) {
                    st.stale.push((doc, "change"));
                }
            }
            Msg::PageSync(doc) => {
                st.retry.retain(|e| !Arc::ptr_eq(&e.doc, &doc));
                self.page_sync(&doc);
            }
            Msg::Restore { doc, id, tracks } => {
                st.retry.retain(|e| !Arc::ptr_eq(&e.doc, &doc));
                self.restore_project(&doc, id, tracks);
                // a source that was queued by its read before the restore and has its result now
                // (S3: access, then restore) leaves the queue: it is not analysed
                drop_done(st, &doc);
            }
            Msg::Close { doc, ack } => {
                st.retry.retain(|e| !Arc::ptr_eq(&e.doc, &doc));
                st.requested.retain(|r| !Arc::ptr_eq(&r.doc, &doc));
                st.stale.retain(|(d, _)| !Arc::ptr_eq(d, &doc));
                self.close_document(&doc);
                let _ = ack.send(());
            }
            Msg::Quit => return true,
        }
        false
    }

    /// One analysis round of a document over `targets` (the sources that asked; none after a
    /// removal or a change), then the report rows. Every track of the project whose audio is
    /// here is analysed, with a fresh snapshot of every source (so a change of the regions or
    /// of the properties of an earlier source is seen). `force` runs the round even when
    /// nothing changed (the fault run of the check).
    fn run_job(
        &'static self,
        doc: &Arc<DocShared>,
        targets: &[Arc<SourceState>],
        reason: &'static str,
        force: bool,
        allow_defer: bool,
    ) -> bool {
        // a target whose read failed has no audio: its analysis ended without a result (the
        // host does not wait for it; a new request reads it again)
        let (usable, unread): (Vec<_>, Vec<_>) = targets.iter().cloned().partition(has_wav);
        // a source whose audio never came (the deadline passed, see `audio_deadline`) is left
        // out of the round and named: the result is partial
        let mut late_names: Vec<String> = Vec::new();
        for s in &unread {
            // U2: a source that waits for new audio (a read runs, or a retry is planned) is left out of
            // this round and keeps its old result: it is neither failed nor late
            if waits_for_new_audio(s) {
                continue;
            }
            s.analysis_failed.store(true, Ordering::SeqCst);
            if !s.failed.load(Ordering::SeqCst) {
                late_names.push(s.props().name);
                self.log(json!({"event": "source_late", "name": s.props().name}).to_string());
            }
        }
        let members: Vec<Arc<SourceState>> = doc
            .members
            .lock()
            .map(|m| m.iter().filter_map(std::sync::Weak::upgrade).collect())
            .unwrap_or_default();
        let snaps: Vec<service::SourceSnapshot> =
            members.iter().filter(|s| takes_part(s)).map(|s| s.snapshot()).collect();
        let id_before = doc.id();
        self.log(
            json!({"event": "round_start", "doc": id_before, "reason": reason, "tracks": snaps.len(), "unix_ns": now_ns().to_string()})
                .to_string(),
        );
        self.write_out();
        let snap_gens: HashMap<usize, (u64, bool)> = snaps.iter().map(|x| (x.seq, (x.audio_gen, x.ready))).collect();
        let snap_counts: HashMap<usize, i64> = snaps.iter().map(|x| (x.seq, x.sample_count)).collect();
        let snap_regions: Vec<(usize, Vec<service::RegionRec>, f64)> = snaps
            .iter()
            .map(|x| (x.seq, x.regions.clone(), x.sample_rate))
            .collect();
        let worker_doc = doc.clone();
        let outcome = guard("service job", Err("the service job panicked".to_string()), move || {
            let mut svc = worker_doc.lock_svc();
            let result = svc.analyse(&snaps, force)?;
            Ok((result, svc.id().to_string(), svc.folder(), svc.home()))
        });
        let (result, id, folder, home) = match outcome {
            Ok(r) => r,
            Err(e) if allow_defer && e.starts_with("409") => {
                // R4: a job of the page started after the check: the round goes back to the queue
                self.log(json!({"event": "round_deferred", "doc": id_before, "reason": e}).to_string());
                self.write_out();
                return true;
            }
            Err(e) => {
                // the round ended without a result: the sources of the round are complete
                // for the host (it must not wait for ever) and are tried again at the next
                // request; their tracks stay (a source is one track for its life)
                for s in &usable {
                    s.analysis_failed.store(true, Ordering::SeqCst);
                }
                let outcome = if e.contains("cancelled") { "cancelled" } else { "failed" };
                self.log(
                    json!({"event": "round_end", "doc": id_before, "outcome": outcome, "unix_ns": now_ns().to_string()})
                        .to_string(),
                );
                self.log(json!({"event": "job_failed", "doc": id_before, "error": e}).to_string());
                self.write_out();
                return false;
            }
        };
        if !result.ran {
            self.log(
                json!({"event": "round_end", "doc": id, "outcome": "idle", "unix_ns": now_ns().to_string()})
                    .to_string(),
            );
            self.write_out();
            return false; // nothing had changed
        }
        doc.region_gen.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut rr) = doc.round_regions.lock() {
            // the regions of the snapshots the notes were made with (R6), not of the live ones
            for (seq, r) in &result.regions {
                rr.insert(*seq, r.clone());
            }
        }
        if let Ok(mut rs) = doc.round_samples.lock() {
            rs.extend(snap_counts.iter().map(|(k, v)| (*k, *v)));
        }
        let seq_of = |name: &str| result.tracks.iter().find(|(n, _)| n == name).map(|(_, s)| *s);
        let by_seq = |seq: usize| members.iter().find(|s| s.seq == seq);
        // a track the service could not analyse, and a target that was left out (its audio
        // is not the file the reader wrote): the source has no result. It is complete for
        // the host (no wait for ever) and has no notes; a new request tries again.
        let mut failed_seqs: Vec<usize> = result.errors.iter().filter_map(|(n, _)| seq_of(n)).collect();
        for (n, _) in &result.skipped {
            if let Some(seq) = seq_of(n) {
                if usable.iter().any(|u| u.seq == seq) {
                    failed_seqs.push(seq);
                }
            }
        }
        let mut changed: Vec<String> = Vec::new();
        for (seq, notes) in &result.notes {
            let Some(s) = by_seq(*seq) else { continue };
            let was_complete = s.has_events.load(Ordering::SeqCst);
            let was_failed = s.analysis_failed.load(Ordering::SeqCst);
            let failed = failed_seqs.contains(seq);
            // a failed track that had events keeps them and their notes as STALE (the `job`
            // row says so); one that had none has none
            let stale = failed && result.stale.iter().any(|n| seq_of(n) == Some(*seq));
            let notes = notes.clone();
            let differs = s.notes.lock().map(|n| *n != notes).unwrap_or(false);
            if let Ok(mut n) = s.notes.lock() {
                *n = notes;
            }
            if let (Ok(mut t), Some(name)) = (s.track.lock(), result.tracks.iter().find(|(_, q)| q == seq)) {
                *t = Some(name.0.clone());
            }
            if failed {
                if !stale {
                    s.has_events.store(false, Ordering::SeqCst);
                }
                s.stale.store(stale, Ordering::SeqCst);
                s.analysis_failed.store(true, Ordering::SeqCst);
            } else if result.analysed.iter().any(|n| seq_of(n) == Some(*seq)) {
                // R1: only a round that really analysed the audio of the live generation ends the
                // new analysis: a snapshot that was not ready was analysed on the OLD audio (the
                // service keeps the old snapshot), the new read has still to come or has failed
                // for good. Then the state of the source does not change (T1: `analysis_failed`
                // stays, so a source that failed for good stays complete-and-failed, not
                // incomplete for ever). `clear-stale` and `clear-failed` bring back the two
                // first versions.
                let live = s.audio_gen.load(Ordering::SeqCst);
                let fresh = snap_gens.get(seq).is_some_and(|(g, ready)| *g == live && *ready);
                let fault = crate::test_switch("DS_ARA_ROUND_FAULT").unwrap_or_default();
                let old_audio = s.reanalyse.load(Ordering::SeqCst) && !fresh && fault != "clear-stale";
                if !old_audio || fault == "clear-failed" {
                    s.has_events.store(true, Ordering::SeqCst);
                    s.analysis_failed.store(false, Ordering::SeqCst);
                    s.stale.store(false, Ordering::SeqCst);
                }
                if fresh || fault == "clear-stale" {
                    s.reanalyse.store(false, Ordering::SeqCst);
                    // T4: the track points to the new file now: the files of the older generations go
                    let old: Vec<PathBuf> = std::mem::take(&mut *s.old_wavs.lock().unwrap_or_else(|e| e.into_inner()));
                    for p in old {
                        let _ = std::fs::remove_file(p);
                    }
                }
            }
            // a complete source whose notes changed, or one that had no result and has one
            // now, is told to the host
            // a source the host never asked for does not wait for its notes: it is told when they
            // come (M1b4)
            let first = !was_complete && !was_failed && !failed && !s.asked.load(Ordering::SeqCst);
            let on_old_audio = s.reanalyse.load(Ordering::SeqCst)
                && !snap_gens
                    .get(seq)
                    .is_some_and(|(g, ready)| *ready && *g == s.audio_gen.load(Ordering::SeqCst));
            let told = (was_complete && differs) || (was_failed && !failed && !on_old_audio) || first;
            if told && !s.removed.load(Ordering::SeqCst) {
                changed.push(s.props().name);
                if let Ok(mut p) = doc.pending.lock() {
                    if !p.iter().any(|x| Arc::ptr_eq(x, s)) {
                        p.push(s.clone());
                    }
                }
            }
        }
        if let Ok(mut m) = doc.events.lock() {
            *m = result.events.clone();
        }
        if result.analysed.is_empty() && usable.is_empty() {
            // a removal that left nothing to analyse
            self.log(
                json!({"event": "round_end", "doc": id, "outcome": "idle", "unix_ns": now_ns().to_string()})
                    .to_string(),
            );
            self.write_out();
            return false;
        }
        let round = doc.rounds.fetch_add(1, Ordering::SeqCst) + 1;
        ROUNDS.fetch_add(1, Ordering::SeqCst);
        let counts: serde_json::Map<String, Value> = result
            .decode_counts
            .iter()
            .map(|(p, n)| (p.display().to_string(), json!(n)))
            .collect();
        // the events of every track of the project as the bytes the service gave: the
        // check compares them between rounds, between documents and with the reference
        let track_json: serde_json::Map<String, Value> = result
            .events
            .iter()
            .map(|(name, ev)| (name.clone(), json!(serde_json::to_string(ev).unwrap_or_default())))
            .collect();
        let skipped: Vec<Value> = result
            .skipped
            .iter()
            .map(|(t, why)| json!({"track": t, "why": why}))
            .collect();
        let errors: Vec<Value> = result
            .errors
            .iter()
            .map(|(t, e)| json!({"track": t, "error": e}))
            .collect();
        self.log(
            json!({
                "event": "round_end", "doc": id, "outcome": "done", "unix_ns": now_ns().to_string(),
            })
            .to_string(),
        );
        self.log(
            json!({
                "event": "job",
                "doc": id,
                "folder": folder.display().to_string(),
                "round": round,
                "reason": reason,
                "added": result.added,
                "analysed": result.analysed,
                "skipped": skipped,
                "errors": errors,
                "stale": result.stale,
                "late": late_names,
                "partial": result.partial() || !late_names.is_empty(),
                "changed_notes": changed,
                "project_tracks": result.project_tracks,
                "earlier_changed": result.earlier_changed,
                "decodes": result.decode_counts.values().sum::<usize>(),
                "service_home": home.display().to_string(),
                "events_json": service::events_line(&result.events),
                "track_json": track_json,
            })
            .to_string(),
        );
        let row = format!(
            "{{\"event\":\"service\",\"analysis_ms\":{},\"analysis_thread\":\"{:?}\",\"tracks\":\"{}\",\"doc\":\"{}\",\"decode_counts\":{},\"events\":{}}}",
            result.analysis_ms,
            // the thread the job ran on: the gate proves it is not the request caller's
            std::thread::current().id(),
            result.project_tracks.join(", "),
            id,
            serde_json::to_string(&counts).unwrap_or_else(|_| "{}".into()),
            service::events_line(&result.events)
        );
        if let Ok(mut sr) = self.service_row.lock() {
            *sr = Some(row);
        }
        self.write_out();
        // the page reloads its project: it holds the tracks and events of this round (F1, F8)
        doc.announce_reload();
        false
    }

    /// The page changed the project (an edit): the events of the tracks are read again, the
    /// project is saved, the document's events (the archive stores them) and the notes follow,
    /// and the host is told about the sources whose notes changed.
    fn page_sync(&'static self, doc: &Arc<DocShared>) {
        let worker_doc = doc.clone();
        let outcome = guard("page sync", Err("the page sync panicked".to_string()), move || {
            worker_doc.lock_svc().sync_page_edits()
        });
        match outcome {
            Ok((None, saved)) => {
                // R3: the events were already in the document (the request thread recorded them);
                // the worker saved the project
                self.log(json!({"event": "page_sync", "doc": doc.id(), "saved": saved, "changed_notes": [], "unix_ns": now_ns().to_string()}).to_string());
                self.write_out();
            }
            Ok((Some((events, notes)), saved)) => {
                let members: Vec<Arc<SourceState>> = doc
                    .members
                    .lock()
                    .map(|m| m.iter().filter_map(std::sync::Weak::upgrade).collect())
                    .unwrap_or_default();
                let mut changed: Vec<String> = Vec::new();
                let notes = if crate::test_switch("DS_ARA_EDITOR_FAULT").as_deref() == Ok("no-notes") {
                    HashMap::new() // the fault run: the notes do not follow
                } else {
                    notes
                };
                for (seq, n) in notes {
                    let Some(s) = members.iter().find(|s| s.seq == seq) else {
                        continue;
                    };
                    let differs = s.notes.lock().map(|old| *old != n).unwrap_or(false);
                    if !differs {
                        continue;
                    }
                    if let Ok(mut old) = s.notes.lock() {
                        *old = n;
                    }
                    if s.has_events.load(Ordering::SeqCst) && !s.removed.load(Ordering::SeqCst) {
                        changed.push(s.props().name);
                        if let Ok(mut p) = doc.pending.lock() {
                            if !p.iter().any(|x| Arc::ptr_eq(x, s)) {
                                p.push(s.clone());
                            }
                        }
                    }
                }
                let line = service::events_line(&events);
                if let Ok(mut m) = doc.events.lock() {
                    *m = events;
                }
                self.log(
                    json!({"event": "page_sync", "doc": doc.id(), "saved": saved, "changed_notes": changed,
                        "events_json": line, "unix_ns": now_ns().to_string()})
                    .to_string(),
                );
                self.write_out();
            }
            Err(e) => {
                self.log(json!({"event": "page_sync_failed", "doc": doc.id(), "error": e}).to_string());
                self.write_out();
            }
        }
    }

    /// A destroyed source: its track leaves the project, its cached audio goes.
    fn remove_track(&'static self, src: &Arc<SourceState>, sources_left: usize) -> Result<(), String> {
        let doc = &src.doc;
        let removed = {
            let worker_doc = doc.clone();
            let seq = src.seq;
            guard("remove track", Err("the removal panicked".to_string()), move || {
                worker_doc.lock_svc().remove(seq)
            })
        };
        let busy_fault = crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() == Ok("no-page-wait");
        match (&removed, busy_fault) {
            (Ok(left), _) => {
                if let Ok(mut m) = doc.events.lock() {
                    m.retain(|name, _| left.contains(name));
                }
            }
            (Err(_), true) => {
                // the fault run (the first version): an error was taken as "no track is left"
                if let Ok(mut m) = doc.events.lock() {
                    m.clear();
                }
            }
            // M3: on an error the events stay (the caller retries the removal)
            (Err(e), false) => {
                self.log(
                    json!({"event": "source_destroy_failed", "doc": doc.id(), "name": src.props().name, "error": e})
                        .to_string(),
                );
                self.write_out();
                return Err(e.clone());
            }
        }
        let tracks_left = removed.clone().unwrap_or_default();
        if removed.is_ok() {
            doc.region_gen.fetch_add(1, Ordering::SeqCst);
            if let Ok(mut rr) = doc.round_regions.lock() {
                rr.remove(&src.seq);
            }
        }
        if let Some(wav) = src.wav.lock().ok().and_then(|w| w.clone()) {
            let _ = std::fs::remove_file(wav);
        }
        self.log(
            json!({
                "event": "source_destroyed",
                "doc": doc.id(),
                "name": src.props().name,
                "sources_left": sources_left,
                "project_tracks": tracks_left,
                "error": removed.err(),
            })
            .to_string(),
        );
        Ok(())
    }

    /// The archive was restored: the project of the stored document id opens with the
    /// stored events. Nothing is decoded.
    fn restore_project(&'static self, doc: &Arc<DocShared>, id: Option<String>, tracks: Vec<service::RestoredTrack>) {
        doc.region_gen.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut rr) = doc.round_regions.lock() {
            for t in &tracks {
                rr.insert(t.snapshot.seq, (t.snapshot.regions.clone(), t.snapshot.sample_rate));
            }
        }
        let worker_doc = doc.clone();
        let outcome = guard("restore project", Err("the restore panicked".to_string()), move || {
            let mut svc = worker_doc.lock_svc();
            if let Some(id) = &id {
                svc.set_id(id);
            }
            let result = svc.restore(&tracks)?;
            Ok((result, svc.id().to_string(), svc.folder(), svc.home()))
        });
        match outcome {
            Ok((result, id, folder, home)) => {
                if crate::test_switch("DS_ARA_RESTORE_FAULT").as_deref() == Ok("late-events") {
                    // the fault run: the events of the document are set here, on the worker
                    // thread, as the first version did
                    if let Ok(mut m) = doc.events.lock() {
                        *m = result.events.clone();
                    }
                }
                self.log(
                    json!({
                        "event": "doc_restored",
                        "doc": id,
                        "folder": folder.display().to_string(),
                        "project_tracks": result.project_tracks,
                        "decodes": result.decode_counts.values().sum::<usize>(),
                        "service_home": home.display().to_string(),
                        "events_json": service::events_line(&result.events),
                    })
                    .to_string(),
                );
            }
            Err(e) => self.log(format!("{{\"event\":\"restore_failed\",\"error\":{e:?}}}")),
        }
        self.write_out();
        doc.announce_reload();
    }

    /// The document controller ends: the project is saved and closed, the service dropped.
    fn close_document(&'static self, doc: &Arc<DocShared>) {
        let worker_doc = doc.clone();
        guard("close document", (), move || worker_doc.lock_svc().shutdown());
        self.log(json!({"event": "doc_closed", "doc": doc.id(), "unix_ns": now_ns().to_string()}).to_string());
        self.write_out();
    }

    /// The fault path: the analysis runs inside `process` (the audio thread), once. The
    /// reader threads write the WAVs; the audio thread waits for them (the point of the
    /// proof: the block overruns its budget).
    fn run_fault_job(&'static self) {
        let Some((doc, sources)) = current_document() else {
            return;
        };
        let targets: Vec<Arc<SourceState>> = sources
            .into_iter()
            .filter(|s| s.requested.load(Ordering::SeqCst))
            .collect();
        if targets.is_empty() {
            return;
        }
        loop {
            let missing = targets
                .iter()
                .any(|s| s.wav.lock().map(|w| w.is_none()).unwrap_or(true) && !s.failed.load(Ordering::SeqCst));
            if !missing {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        self.run_job(&doc, &targets, "request", false, false);
    }
}

/// What the editor of a plugin instance needs from its document.
fn backend_of_doc(doc: &Arc<DocShared>) -> editor::Backend {
    let d = doc.clone();
    let r = doc.clone();
    let g = doc.clone();
    let g2 = doc.clone();
    editor::Backend {
        page: doc.page.clone(),
        changed: Arc::new(move || {
            let _ = send(session(), Msg::PageSync(d.clone()));
        }),
        record: Arc::new(move |svc, sid| r.record_page_edit(svc, sid)),
        regions: Arc::new(move || g.round_region_list()),
        regions_gen: Arc::new(move || g2.region_gen.load(Ordering::SeqCst)),
        doc_id: doc.id(),
    }
}

/// The newest document controller (the callbacks carry their own ref; this is for the
/// fault job, which runs from `process` and has none).
static CURRENT: SyncPtr<Mutex<Option<*const Controller>>> = SyncPtr(Mutex::new(None));

fn current_store(c: *const Controller) {
    if let Ok(mut g) = CURRENT.0.lock() {
        *g = Some(c);
    }
}

/// The document and the sources of the newest document controller.
fn current_document() -> Option<(Arc<DocShared>, Vec<Arc<SourceState>>)> {
    CURRENT
        .0
        .lock()
        .ok()
        .and_then(|g| *g)
        .and_then(|c| unsafe { (*c).sources.lock().ok().map(|l| ((*c).doc.clone(), l.clone())) })
}

/// Start the worker thread once.
fn start_worker(s: &'static Session) {
    let mut tx = s.tx.lock().unwrap_or_else(|e| e.into_inner());
    // no worker, or one that has ended (its receiver is dead): a new one
    if tx.is_none() || !WORKER_RUNNING.load(Ordering::SeqCst) {
        let (tx_, rx) = channel::<Msg>();
        let generation = WORKER_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        WORKER_RUNNING.store(true, Ordering::SeqCst);
        let spawned = std::thread::Builder::new()
            .name("drumspike-ara-analysis".into())
            .spawn(move || s.worker_main(rx, generation));
        match spawned {
            Ok(handle) => {
                *tx = Some(tx_);
                *s.worker.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
            }
            Err(_) => WORKER_RUNNING.store(false, Ordering::SeqCst),
        }
    }
}

/// The host unloads the plugin library (`clap_entry.deinit`): no round may run on in a
/// thread of code that is about to be unmapped. Every live document is closed for new work
/// and its running round is cancelled (the cancel takes effect between two tracks), the
/// worker gets `Quit` and the unload waits for it at most [`UNLOAD_WAIT`]. A worker that is
/// still inside one track after that stays behind: the plugin cannot stop a thread, so the
/// report says `ended: false` and the unload goes on. A host that loads the library again in
/// the same process gets a new worker. `DS_ARA_UNLOAD_FAULT=skip` (the check's fault run)
/// does nothing here.
fn unload_worker() {
    if crate::test_switch("DS_ARA_UNLOAD_FAULT").as_deref() == Ok("skip") {
        return;
    }
    let Some(s) = SESSION.get() else { return };
    let started = std::time::Instant::now();
    let docs: Vec<Arc<DocShared>> = LIVE_DOCS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter_map(std::sync::Weak::upgrade)
        .collect();
    for d in &docs {
        d.closing.store(true, Ordering::SeqCst);
        d.canceller.cancel();
    }
    if !send(s, Msg::Quit) {
        return; // no worker was started
    }
    while WORKER_RUNNING.load(Ordering::SeqCst) && started.elapsed() < UNLOAD_WAIT {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let ended = !WORKER_RUNNING.load(Ordering::SeqCst);
    if ended {
        if let Some(h) = s.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = h.join();
        }
    }
    // a worker that is left behind gets no more messages through this sender: a later init
    // makes a new worker
    *s.tx.lock().unwrap_or_else(|e| e.into_inner()) = None;
    s.log(
        json!({"event": "worker_unloaded", "ended": ended, "ms": started.elapsed().as_millis() as u64,
            "unix_ns": now_ns().to_string()})
        .to_string(),
    );
    s.write_out();
}

/// Send a message to the worker; false when there is no worker.
fn send(s: &'static Session, msg: Msg) -> bool {
    if let Ok(tx) = s.tx.lock() {
        if let Some(tx) = tx.as_ref() {
            return tx.send(msg).is_ok();
        }
    }
    false
}

// ---------------------------------------------------------------- ARA factory

static CONTENT_TYPES: [ARAContentType; 1] = [kARAContentTypeNotes as ARAContentType];

static FACTORY: SyncPtr<ARAFactory> = SyncPtr(ARAFactory {
    structSize: std::mem::size_of::<ARAFactory>(),
    lowestSupportedApiGeneration: kARAAPIGeneration_2_0_Final as ARAAPIGeneration,
    highestSupportedApiGeneration: kARAAPIGeneration_2_0_Final as ARAAPIGeneration,
    factoryID: cs!("com.cherryblossomvalley.drumspike.ara.arafactory"),
    initializeARAWithConfiguration: Some(ara_init),
    uninitializeARA: Some(ara_uninit),
    plugInName: cs!("Drum Spike ARA"),
    manufacturerName: cs!("Cherry Blossom Valley"),
    informationURL: cs!("https://github.com/Cherry-Blossom-Valley/drumspike"),
    version: version_cs!(),
    createDocumentControllerWithDocument: Some(create_document_controller),
    documentArchiveID: cs!("com.cherryblossomvalley.drumspike.ara.archive.v1"),
    compatibleDocumentArchiveIDsCount: 0,
    compatibleDocumentArchiveIDs: std::ptr::null(),
    analyzeableContentTypesCount: 1,
    analyzeableContentTypes: CONTENT_TYPES.as_ptr(),
    supportedPlaybackTransformationFlags: 0,
    supportsStoringAudioFileChunks: 0,
});

unsafe extern "C" fn ara_init(config: *const ARAInterfaceConfiguration) {
    guard("ara_init", (), move || {
        // the ARA interface structs are packed: copy the field out, never reference it
        let api_generation = (*config).desiredApiGeneration;
        session().log(format!(
            "{{\"event\":\"ara_init\",\"api_generation\":{api_generation}}}"
        ));
    })
}
unsafe extern "C" fn ara_uninit() {
    guard("ara_uninit", (), move || {})
}

// ---------------------------------------------------------------- document controller

struct Controller {
    /// The host instance the readers read through.
    host: *const ARADocumentControllerHostInstance,
    /// The ARA instance struct the host holds.
    instance: ARADocumentControllerInstance,
    /// The sources of this document, in creation order.
    sources: Mutex<Vec<Arc<SourceState>>>,
    /// The service, the project and the id of this document.
    doc: Arc<DocShared>,
}

/// The document controller interface the instance carries. Under `DS_ARA_TABLES_FAULT`
/// the plugin serves a copy with one emptied slot inside its declared structSize — the
/// deliberate bug that proves the host's table walk can fail.
fn dc_interface_of_instance() -> *const ARADocumentControllerInterface {
    if crate::test_switch("DS_ARA_TABLES_FAULT").as_deref() != Ok("1") {
        return &DC_INTERFACE.0;
    }
    let mut incomplete: ARADocumentControllerInterface = unsafe { std::ptr::read(&DC_INTERFACE.0) };
    incomplete.beginEditing = None;
    Box::into_raw(Box::new(incomplete))
}

unsafe extern "C" fn create_document_controller(
    host: *const ARADocumentControllerHostInstance,
    _properties: *const ARADocumentProperties,
) -> *const ARADocumentControllerInstance {
    guard("create_document_controller", std::ptr::null(), move || {
        let s = session();
        start_worker(s);
        // the service of the document is made here, with the controller
        let doc = DocShared::new();
        let doc_id = doc.id();
        let c = Box::into_raw(Box::new(Controller {
            host,
            instance: std::mem::zeroed(),
            sources: Mutex::new(Vec::new()),
            doc,
        }));
        (*c).instance = ARADocumentControllerInstance {
            structSize: std::mem::size_of::<ARADocumentControllerInstance>(),
            documentControllerRef: c as ARADocumentControllerRef,
            documentControllerInterface: dc_interface_of_instance(),
        };
        current_store(c);
        s.log(format!(
            "{{\"event\":\"create_document_controller\",\"doc\":\"{doc_id}\"}}"
        ));
        &(*c).instance
    })
}

unsafe fn controller_of(controllerRef: ARADocumentControllerRef) -> &'static Controller {
    &*(controllerRef as *const Controller)
}

unsafe extern "C" fn dc_destroyDocumentController(controllerRef: ARADocumentControllerRef) {
    guard("destroyDocumentController", (), move || {
        let ctl = Box::from_raw(controllerRef as *mut Controller);
        // the fault job must not reach a controller that is gone
        if let Ok(mut g) = CURRENT.0.lock() {
            if *g == Some(controllerRef as *const Controller) {
                *g = None;
            }
        }
        // the service of the document ends with it: a running job is cancelled, the
        // worker saves and closes the project and drops the service (in the order of the
        // messages, so every removal before this call is done first). The worker lives for
        // the process.
        let s = session();
        // the 500 ms of the wait below count from here, so the joins of the readers are part
        // of them; what the plugin does not bound is one host read call inside a join
        let started = std::time::Instant::now();
        // no reader of this controller may run once the host takes the controller away
        let sources: Vec<Arc<SourceState>> = ctl.sources.lock().map(|l| l.clone()).unwrap_or_default();
        for src in &sources {
            stop_worker(src);
        }
        let close_fault = crate::test_switch("DS_ARA_CLOSE_FAULT").unwrap_or_default();
        let long_wait = close_fault == "wait";
        ctl.doc.closing.store(true, Ordering::SeqCst);
        // `DS_ARA_CLOSE_FAULT=nocancel` (the check's fault run): the running round is not
        // cancelled and runs on behind the close
        if !long_wait && close_fault != "nocancel" {
            ctl.doc.canceller.cancel();
        }
        let (ack, done) = channel::<()>();
        if send(
            s,
            Msg::Close {
                doc: ctl.doc.clone(),
                ack,
            },
        ) {
            // bounded and short, on the host's main thread: the worker owns the document
            // through shared ownership and finishes the save (or drops it) on its own. The
            // fault run brings back the long wait of the first version.
            let limit = if long_wait {
                std::time::Duration::from_secs(20)
            } else {
                std::time::Duration::from_millis(500).saturating_sub(started.elapsed())
            };
            let _ = done.recv_timeout(limit);
        } else {
            // no worker: nothing runs, the document closes here
            ctl.doc.lock_svc().shutdown();
            s.write_out();
        }
        // the id is free again (a document that restores it later may take it)
        release_id(&ctl.doc.id());
        LIVE_DOCS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|w| w.upgrade().is_some_and(|d| !Arc::ptr_eq(&d, &ctl.doc)));
        drop(ctl);
    })
}

unsafe extern "C" fn dc_getFactory(_c: ARADocumentControllerRef) -> *const ARAFactory {
    guard("getFactory", std::ptr::null(), move || &FACTORY.0)
}

unsafe extern "C" fn dc_isLicensedForCapabilities(
    _c: ARADocumentControllerRef,
    _dialog: ARABool,
    _n: ARASize,
    _t: *const ARAContentType,
    _f: ARAPlaybackTransformationFlags,
) -> ARABool {
    guard("isLicensedForCapabilities", 0, move || 1)
}

// The no-op stubs for the members M1a does not implement: the ARA test host of the SDK
// examples calls them without a check (the same stub set the S1 spike passes with, see
// plugins/ara-spike/src/stubs.rs). Every one runs inside the boundary guard.
unsafe extern "C" fn dc_beginEditing(_c: ARADocumentControllerRef) {
    guard("beginEditing", (), move || {})
}
unsafe extern "C" fn dc_endEditing(_c: ARADocumentControllerRef) {
    guard("endEditing", (), move || {})
}
/// The host asks for the pending model updates (the only call in which the plugin may call
/// back into the host's model update controller): every source whose notes changed in a
/// later round is reported as changed, so the host reads the notes again.
unsafe extern "C" fn dc_notifyModelUpdates(c: ARADocumentControllerRef) {
    guard("notifyModelUpdates", (), move || {
        let ctl = controller_of(c);
        // R3: a read of new audio that failed is tried again here, on the host's main thread (the
        // reader is made there); the worker only sets the time
        let due = ctl.sources.lock().map(|l| due_read_retries(&l)).unwrap_or_default();
        for s in due {
            start_read(&s);
        }
        let pending: Vec<Arc<SourceState>> = match ctl.doc.pending.lock() {
            Ok(mut p) => std::mem::take(&mut *p),
            Err(_) => return,
        };
        if pending.is_empty() || ctl.host.is_null() {
            return;
        }
        let host = ctl.host;
        // the optional interface: the struct of the host must reach it, and carry a function
        let declared = std::ptr::addr_of!((*host).structSize).read_unaligned();
        let end = std::mem::offset_of!(ARADocumentControllerHostInstance, modelUpdateControllerInterface)
            + std::mem::size_of::<usize>();
        if declared < end {
            return;
        }
        let iface = std::ptr::addr_of!((*host).modelUpdateControllerInterface).read_unaligned();
        let host_ref = std::ptr::addr_of!((*host).modelUpdateControllerHostRef).read_unaligned();
        if iface.is_null() {
            return;
        }
        let Some(changed) = (*iface).notifyAudioSourceContentChanged else {
            return;
        };
        for s in pending {
            if s.removed.load(Ordering::SeqCst) {
                continue;
            }
            // the notes changed, the signal did not: a null range is the whole source
            changed(
                host_ref,
                s.host_ref.0,
                std::ptr::null(),
                kARAContentUpdateSignalScopeRemainsUnchanged as ARAContentUpdateFlags,
            );
        }
    })
}
unsafe extern "C" fn dc_beginRestoringDocumentFromArchive(
    _c: ARADocumentControllerRef,
    _r: ARAArchiveReaderHostRef,
) -> ARABool {
    guard("beginRestoringDocumentFromArchive", 0, move || 0)
}
unsafe extern "C" fn dc_endRestoringDocumentFromArchive(
    _c: ARADocumentControllerRef,
    _r: ARAArchiveReaderHostRef,
) -> ARABool {
    guard("endRestoringDocumentFromArchive", 0, move || 0)
}
unsafe extern "C" fn dc_storeDocumentToArchive(_c: ARADocumentControllerRef, _w: ARAArchiveWriterHostRef) -> ARABool {
    guard("storeDocumentToArchive", 0, move || 0)
}
unsafe extern "C" fn dc_updateDocumentProperties(_c: ARADocumentControllerRef, _p: *const ARADocumentProperties) {
    guard("updateDocumentProperties", (), move || {})
}
unsafe extern "C" fn dc_createMusicalContext(
    _c: ARADocumentControllerRef,
    _h: ARAMusicalContextHostRef,
    _p: *const ARAMusicalContextProperties,
) -> ARAMusicalContextRef {
    guard("createMusicalContext", std::ptr::null_mut(), move || {
        Box::into_raw(Box::new(0u64)) as ARAMusicalContextRef
    })
}
unsafe extern "C" fn dc_updateMusicalContextProperties(
    _c: ARADocumentControllerRef,
    _m: ARAMusicalContextRef,
    _p: *const ARAMusicalContextProperties,
) {
    guard("updateMusicalContextProperties", (), move || {})
}
unsafe extern "C" fn dc_updateMusicalContextContent(
    _c: ARADocumentControllerRef,
    _m: ARAMusicalContextRef,
    _r: *const ARAContentTimeRange,
    _f: ARAContentUpdateFlags,
) {
    guard("updateMusicalContextContent", (), move || {})
}
unsafe extern "C" fn dc_destroyMusicalContext(_c: ARADocumentControllerRef, _m: ARAMusicalContextRef) {
    guard("destroyMusicalContext", (), move || {})
}
/// The host changed the content of a source. When the signal changed (the flag
/// `kARAContentUpdateSignalScopeRemainsUnchanged` is NOT set; no flags at all means everything)
/// the samples are new (M1b4): the audio of the source gets a new generation, the old WAV stays
/// until the new one is committed, the reader reads again at once when the host's samples access
/// is on (else at the next enable), and the source needs a round although it has a result.
/// The round keeps the hits the user placed (the rule of #538); the hits the detection placed are
/// made again from the new audio. `DS_ARA_ROUND_FAULT=ignore-update` brings back the old way: the
/// update does nothing.
unsafe extern "C" fn dc_updateAudioSourceContent(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    _r: *const ARAContentTimeRange,
    flags: ARAContentUpdateFlags,
) {
    guard("updateAudioSourceContent", (), move || {
        let Some(src) = state_of(audioSourceRef) else { return };
        let signal_kept = flags & (kARAContentUpdateSignalScopeRemainsUnchanged as ARAContentUpdateFlags) != 0;
        if signal_kept
            || crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() == Ok("ignore-update")
            || src.removed.load(Ordering::SeqCst)
        {
            return;
        }
        // The generation is raised FIRST, then the old read is stopped: the failure that the cancel
        // makes carries the old generation and is ignored (T3), and a commit of the old read is
        // refused under the same lock (see `read_source`).
        let old = {
            // under the lock the snapshots read (R2); a restored source has no path of this
            // process: the file an earlier process left for it goes too (R4)
            let mut w = src.wav.lock().unwrap_or_else(|e| e.into_inner());
            let old_gen = src.audio_gen.fetch_add(1, Ordering::SeqCst);
            w.take()
                .unwrap_or_else(|| src.doc.wav_path(src.seq, &src.props().name, old_gen))
        };
        src.old_wavs.lock().unwrap_or_else(|e| e.into_inner()).push(old);
        src.read_tries.store(0, Ordering::SeqCst);
        *src.read_retry_at.lock().unwrap_or_else(|e| e.into_inner()) = None; // T2: an old retry time is void
        src.analysis_failed.store(false, Ordering::SeqCst);
        src.reanalyse.store(true, Ordering::SeqCst);
        // no read of the old samples may run on: stop it (the reader is made and dropped on this thread)
        stop_worker(&src);
        src.failed.store(false, Ordering::SeqCst);
        if src.access.load(Ordering::SeqCst) {
            start_read(&src);
        }
    })
}
unsafe extern "C" fn dc_deactivateAudioSourceForUndoHistory(
    _c: ARADocumentControllerRef,
    _s: ARAAudioSourceRef,
    _d: ARABool,
) {
    guard("deactivateAudioSourceForUndoHistory", (), move || {})
}
unsafe extern "C" fn dc_updateAudioModificationProperties(
    _c: ARADocumentControllerRef,
    _m: ARAAudioModificationRef,
    _p: *const ARAAudioModificationProperties,
) {
    guard("updateAudioModificationProperties", (), move || {})
}
unsafe extern "C" fn dc_deactivateAudioModificationForUndoHistory(
    _c: ARADocumentControllerRef,
    _m: ARAAudioModificationRef,
    _d: ARABool,
) {
    guard("deactivateAudioModificationForUndoHistory", (), move || {})
}
unsafe extern "C" fn dc_isAudioModificationContentAvailable(
    _c: ARADocumentControllerRef,
    _m: ARAAudioModificationRef,
    _t: ARAContentType,
) -> ARABool {
    guard("isAudioModificationContentAvailable", 0, move || 0)
}
unsafe extern "C" fn dc_getAudioModificationContentGrade(
    _c: ARADocumentControllerRef,
    _m: ARAAudioModificationRef,
    _t: ARAContentType,
) -> ARAContentGrade {
    guard("getAudioModificationContentGrade", 0, move || 0)
}
unsafe extern "C" fn dc_createAudioModificationContentReader(
    _c: ARADocumentControllerRef,
    _m: ARAAudioModificationRef,
    _t: ARAContentType,
    _r: *const ARAContentTimeRange,
) -> ARAContentReaderRef {
    guard(
        "createAudioModificationContentReader",
        std::ptr::null_mut(),
        move || Box::into_raw(Box::new(Reader { notes: Vec::new() })) as ARAContentReaderRef,
    )
}
unsafe extern "C" fn dc_isPlaybackRegionContentAvailable(
    _c: ARADocumentControllerRef,
    _r: ARAPlaybackRegionRef,
    _t: ARAContentType,
) -> ARABool {
    guard("isPlaybackRegionContentAvailable", 0, move || 0)
}
unsafe extern "C" fn dc_getPlaybackRegionContentGrade(
    _c: ARADocumentControllerRef,
    _r: ARAPlaybackRegionRef,
    _t: ARAContentType,
) -> ARAContentGrade {
    guard("getPlaybackRegionContentGrade", 0, move || 0)
}
unsafe extern "C" fn dc_createPlaybackRegionContentReader(
    _c: ARADocumentControllerRef,
    _r: ARAPlaybackRegionRef,
    _t: ARAContentType,
    _range: *const ARAContentTimeRange,
) -> ARAContentReaderRef {
    guard("createPlaybackRegionContentReader", std::ptr::null_mut(), move || {
        Box::into_raw(Box::new(Reader { notes: Vec::new() })) as ARAContentReaderRef
    })
}
unsafe extern "C" fn dc_createRegionSequence(
    _c: ARADocumentControllerRef,
    _h: ARARegionSequenceHostRef,
    _p: *const ARARegionSequenceProperties,
) -> ARARegionSequenceRef {
    guard("createRegionSequence", std::ptr::null_mut(), move || {
        Box::into_raw(Box::new(0u64)) as ARARegionSequenceRef
    })
}
unsafe extern "C" fn dc_updateRegionSequenceProperties(
    _c: ARADocumentControllerRef,
    _r: ARARegionSequenceRef,
    _p: *const ARARegionSequenceProperties,
) {
    guard("updateRegionSequenceProperties", (), move || {})
}
unsafe extern "C" fn dc_destroyRegionSequence(_c: ARADocumentControllerRef, _r: ARARegionSequenceRef) {
    guard("destroyRegionSequence", (), move || {})
}
unsafe extern "C" fn dc_getPlaybackRegionHeadAndTailTime(
    _c: ARADocumentControllerRef,
    _r: ARAPlaybackRegionRef,
    _head: *mut ARATimeDuration,
    _tail: *mut ARATimeDuration,
) {
    guard("getPlaybackRegionHeadAndTailTime", (), move || {})
}
unsafe extern "C" fn dc_getProcessingAlgorithmsCount(_c: ARADocumentControllerRef) -> ARAInt32 {
    guard("getProcessingAlgorithmsCount", 0, move || 0)
}
unsafe extern "C" fn dc_getProcessingAlgorithmProperties(
    _c: ARADocumentControllerRef,
    _i: ARAInt32,
) -> *const ARAProcessingAlgorithmProperties {
    guard("getProcessingAlgorithmProperties", std::ptr::null(), std::ptr::null)
}
unsafe extern "C" fn dc_getProcessingAlgorithmForAudioSource(
    _c: ARADocumentControllerRef,
    _s: ARAAudioSourceRef,
) -> ARAInt32 {
    guard("getProcessingAlgorithmForAudioSource", 0, move || 0)
}
unsafe extern "C" fn dc_requestProcessingAlgorithmForAudioSource(
    _c: ARADocumentControllerRef,
    _s: ARAAudioSourceRef,
    _i: ARAInt32,
) {
    guard("requestProcessingAlgorithmForAudioSource", (), move || {})
}
unsafe extern "C" fn dc_storeAudioSourceToAudioFileChunk(
    _c: ARADocumentControllerRef,
    _w: ARAArchiveWriterHostRef,
    _s: ARAAudioSourceRef,
    _id: *mut ARAPersistentID,
    _open: *mut ARABool,
) -> ARABool {
    guard("storeAudioSourceToAudioFileChunk", 0, move || 0)
}
unsafe extern "C" fn dc_isAudioModificationPreservingAudioSourceSignal(
    _c: ARADocumentControllerRef,
    _m: ARAAudioModificationRef,
) -> ARABool {
    guard("isAudioModificationPreservingAudioSourceSignal", 0, move || 0)
}

/// The document controller interface: the M1a surface (sources, content, archiving); the
/// calls M1a does not answer run the no-op stubs above.
static DC_INTERFACE: SyncPtr<ARADocumentControllerInterface> = SyncPtr(ARADocumentControllerInterface {
    structSize: std::mem::size_of::<ARADocumentControllerInterface>(),
    destroyDocumentController: Some(dc_destroyDocumentController),
    getFactory: Some(dc_getFactory),
    beginEditing: Some(dc_beginEditing),
    endEditing: Some(dc_endEditing),
    notifyModelUpdates: Some(dc_notifyModelUpdates),
    beginRestoringDocumentFromArchive: Some(dc_beginRestoringDocumentFromArchive),
    endRestoringDocumentFromArchive: Some(dc_endRestoringDocumentFromArchive),
    storeDocumentToArchive: Some(dc_storeDocumentToArchive),
    updateDocumentProperties: Some(dc_updateDocumentProperties),
    createMusicalContext: Some(dc_createMusicalContext),
    updateMusicalContextProperties: Some(dc_updateMusicalContextProperties),
    updateMusicalContextContent: Some(dc_updateMusicalContextContent),
    destroyMusicalContext: Some(dc_destroyMusicalContext),
    createAudioSource: Some(dc_createAudioSource),
    updateAudioSourceProperties: Some(dc_updateAudioSourceProperties),
    updateAudioSourceContent: Some(dc_updateAudioSourceContent),
    enableAudioSourceSamplesAccess: Some(dc_enableAudioSourceSamplesAccess),
    deactivateAudioSourceForUndoHistory: Some(dc_deactivateAudioSourceForUndoHistory),
    destroyAudioSource: Some(dc_destroyAudioSource),
    createAudioModification: Some(dc_createAudioModification),
    cloneAudioModification: Some(dc_cloneAudioModification),
    updateAudioModificationProperties: Some(dc_updateAudioModificationProperties),
    deactivateAudioModificationForUndoHistory: Some(dc_deactivateAudioModificationForUndoHistory),
    destroyAudioModification: Some(dc_destroyAudioModification),
    createPlaybackRegion: Some(dc_createPlaybackRegion),
    updatePlaybackRegionProperties: Some(dc_updatePlaybackRegionProperties),
    destroyPlaybackRegion: Some(dc_destroyPlaybackRegion),
    isAudioSourceContentAvailable: Some(dc_isAudioSourceContentAvailable),
    isAudioSourceContentAnalysisIncomplete: Some(dc_isAudioSourceContentAnalysisIncomplete),
    requestAudioSourceContentAnalysis: Some(dc_requestAudioSourceContentAnalysis),
    getAudioSourceContentGrade: Some(dc_getAudioSourceContentGrade),
    createAudioSourceContentReader: Some(dc_createAudioSourceContentReader),
    isAudioModificationContentAvailable: Some(dc_isAudioModificationContentAvailable),
    getAudioModificationContentGrade: Some(dc_getAudioModificationContentGrade),
    createAudioModificationContentReader: Some(dc_createAudioModificationContentReader),
    isPlaybackRegionContentAvailable: Some(dc_isPlaybackRegionContentAvailable),
    getPlaybackRegionContentGrade: Some(dc_getPlaybackRegionContentGrade),
    createPlaybackRegionContentReader: Some(dc_createPlaybackRegionContentReader),
    getContentReaderEventCount: Some(dc_getContentReaderEventCount),
    getContentReaderDataForEvent: Some(dc_getContentReaderDataForEvent),
    destroyContentReader: Some(dc_destroyContentReader),
    createRegionSequence: Some(dc_createRegionSequence),
    updateRegionSequenceProperties: Some(dc_updateRegionSequenceProperties),
    destroyRegionSequence: Some(dc_destroyRegionSequence),
    getPlaybackRegionHeadAndTailTime: Some(dc_getPlaybackRegionHeadAndTailTime),
    restoreObjectsFromArchive: Some(dc_restoreObjectsFromArchive),
    storeObjectsToArchive: Some(dc_storeObjectsToArchive),
    getProcessingAlgorithmsCount: Some(dc_getProcessingAlgorithmsCount),
    getProcessingAlgorithmProperties: Some(dc_getProcessingAlgorithmProperties),
    getProcessingAlgorithmForAudioSource: Some(dc_getProcessingAlgorithmForAudioSource),
    requestProcessingAlgorithmForAudioSource: Some(dc_requestProcessingAlgorithmForAudioSource),
    isLicensedForCapabilities: Some(dc_isLicensedForCapabilities),
    storeAudioSourceToAudioFileChunk: Some(dc_storeAudioSourceToAudioFileChunk),
    isAudioModificationPreservingAudioSourceSignal: Some(dc_isAudioModificationPreservingAudioSourceSignal),
});

unsafe extern "C" fn dc_createAudioSource(
    c: ARADocumentControllerRef,
    hostRef: ARAAudioSourceHostRef,
    properties: *const ARAAudioSourceProperties,
) -> ARAAudioSourceRef {
    guard("createAudioSource", std::ptr::null_mut(), move || {
        let ctl = controller_of(c);
        let src = {
            let mut list = ctl.sources.lock().unwrap_or_else(|e| e.into_inner());
            let seq = ctl.doc.next_seq.fetch_add(1, Ordering::SeqCst);
            let src = Arc::new(SourceState {
                seq,
                doc: ctl.doc.clone(),
                track: Mutex::new(None),
                removed: AtomicBool::new(false),
                props: Mutex::new(SourceProps {
                    name: text((*properties).name),
                    persistent_id: text((*properties).persistentID),
                    sample_count: (*properties).sampleCount,
                    sample_rate: (*properties).sampleRate,
                    channels: (*properties).channelCount,
                }),
                host: SyncPtr(ctl.host),
                host_ref: SyncPtr(hostRef),
                reader: Mutex::new(SyncPtr(std::ptr::null_mut())),
                worker: Mutex::new(None),
                cancel: AtomicBool::new(false),
                wav: Mutex::new(None),
                failed: AtomicBool::new(false),
                regions: Mutex::new(Vec::new()),
                notes: Mutex::new(Vec::new()),
                has_events: AtomicBool::new(false),
                requested: AtomicBool::new(false),
                asked: AtomicBool::new(false),
                analysis_failed: AtomicBool::new(false),
                wav_len: AtomicU64::new(0),
                audio_gen: AtomicU64::new(0),
                reanalyse: AtomicBool::new(false),
                access: AtomicBool::new(false),
                old_wavs: Mutex::new(Vec::new()),
                read_retry_at: Mutex::new(None),
                read_tries: std::sync::atomic::AtomicU32::new(0),
                stale: AtomicBool::new(false),
                requested_at: Mutex::new(None),
            });
            list.push(src.clone());
            if let Ok(mut m) = ctl.doc.members.lock() {
                m.push(Arc::downgrade(&src));
            }
            src
        };
        Box::into_raw(Box::new(SyncPtr(src))) as ARAAudioSourceRef
    })
}

unsafe extern "C" fn dc_updateAudioSourceProperties(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    properties: *const ARAAudioSourceProperties,
) {
    guard("updateAudioSourceProperties", (), move || {
        let Some(src) = state_of(audioSourceRef) else { return };
        if let Ok(mut p) = src.props.lock() {
            p.name = text((*properties).name);
            p.persistent_id = text((*properties).persistentID);
            p.sample_count = (*properties).sampleCount;
            p.sample_rate = (*properties).sampleRate;
            p.channels = (*properties).channelCount;
        };
        touch(&src);
    })
}

/// The regions or the properties of a source that took part in the analysis changed: the
/// document needs a round with fresh snapshots (it runs after the settle time; a change
/// that alters nothing ends in a round that does nothing). `DS_ARA_TOUCH_FAULT=ignore` (the
/// check's fault run) brings back the first version: the change is not seen.
fn touch(src: &Arc<SourceState>) {
    src.doc.region_gen.fetch_add(1, Ordering::SeqCst);
    if crate::test_switch("DS_ARA_TOUCH_FAULT").as_deref() == Ok("ignore") {
        return;
    }
    if src.removed.load(Ordering::SeqCst) {
        return;
    }
    if src.requested.load(Ordering::SeqCst) || src.has_events.load(Ordering::SeqCst) {
        send(session(), Msg::Touch(src.doc.clone()));
    }
}

unsafe fn state_of(audioSourceRef: ARAAudioSourceRef) -> Option<Arc<SourceState>> {
    let ptr = audioSourceRef as *const SyncPtr<Arc<SourceState>>;
    Some((*ptr).0.clone())
}

/// ARA rule: a reader is created and destroyed on the host's main thread. Only the reads
/// run elsewhere.
unsafe fn stop_worker(src: &Arc<SourceState>) {
    if let Ok(mut w) = src.worker.lock() {
        if let Some(h) = w.take() {
            src.cancel.store(true, Ordering::SeqCst);
            let _ = h.join();
            src.cancel.store(false, Ordering::SeqCst);
        }
    }
    destroy_reader(src);
}

unsafe fn destroy_reader(src: &Arc<SourceState>) {
    let reader = src.reader.lock().unwrap_or_else(|e| e.into_inner()).0;
    if reader.is_null() || src.host.0.is_null() {
        return;
    }
    // the host's tables are optional input: a missing table or function is skipped, never unwrapped
    let table = (*src.host.0).audioAccessControllerInterface;
    if table.is_null() {
        return;
    }
    if let Some(destroy) = (*table).destroyAudioReader {
        destroy((*src.host.0).audioAccessControllerHostRef, reader);
        *src.reader.lock().unwrap_or_else(|e| e.into_inner()) = SyncPtr(std::ptr::null_mut());
    }
}

unsafe extern "C" fn dc_destroyAudioSource(c: ARADocumentControllerRef, audioSourceRef: ARAAudioSourceRef) {
    guard("destroyAudioSource", (), move || {
        let ptr = audioSourceRef as *mut SyncPtr<Arc<SourceState>>;
        let src = Box::from_raw(ptr);
        stop_worker(&src.0);
        *src.0.read_retry_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
        // the fault run keeps the destroyed source (the M1a way)
        if crate::test_switch("DS_ARA_DESTROY_FAULT").as_deref() == Ok("keep") {
            return;
        }
        src.0.removed.store(true, Ordering::SeqCst);
        src.0.doc.region_gen.fetch_add(1, Ordering::SeqCst);
        // the source leaves the controller's list now; its track leaves the project on the
        // worker thread (the service does disk work, never the host's main thread)
        let ctl = controller_of(c);
        let left = {
            let mut list = ctl.sources.lock().unwrap_or_else(|e| e.into_inner());
            list.retain(|s| !Arc::ptr_eq(s, &src.0));
            list.len()
        };
        let msg = Msg::Remove {
            src: src.0.clone(),
            sources_left: left,
            tries: 0,
        };
        if !send(session(), msg) {
            // no worker: nothing was analysed, so no track exists
        }
    })
}

unsafe extern "C" fn dc_enableAudioSourceSamplesAccess(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    enable: ARABool,
) {
    guard("enableAudioSourceSamplesAccess", (), move || {
        let Some(src) = state_of(audioSourceRef) else { return };
        src.access.store(enable != 0, Ordering::SeqCst);
        // T2: the retry of an earlier failed read is void; a new enable gives a new set of tries
        *src.read_retry_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if enable != 0 {
            src.read_tries.store(0, Ordering::SeqCst);
            // the reader is created on the host's main thread; the reads run on the worker
            start_read(&src);
        } else {
            // no read may run after this call returns
            stop_worker(&src);
            leave_for_access_off(&src);
        }
    })
}

/// A read of this source is running now (its thread has not ended).
fn reader_running(src: &Arc<SourceState>) -> bool {
    src.worker
        .lock()
        .map(|w| w.as_ref().is_some_and(|h| !h.is_finished()))
        .unwrap_or(false)
}

/// The sources whose retry of a failed read is due (T2). A time that is due is taken; one whose
/// reader still runs keeps its time (a healthy read is never touched, nothing is joined here); a
/// source whose access is off or that has its audio needs no read. The caller reads again.
fn due_read_retries(sources: &[Arc<SourceState>]) -> Vec<Arc<SourceState>> {
    sources
        .iter()
        .filter(|s| {
            let mut at = s.read_retry_at.lock().unwrap_or_else(|e| e.into_inner());
            if !at.is_some_and(|t| t <= std::time::Instant::now()) || reader_running(s) {
                return false;
            }
            *at = None;
            s.access.load(Ordering::SeqCst) && !has_wav(s) && !s.removed.load(Ordering::SeqCst)
        })
        .cloned()
        .collect()
}

/// The reader is created on the calling (the host's main) thread and the reads run on
/// their own thread (the ARA rule). A source that has its WAV is not read again; one whose
/// read is still running is left alone; one whose read failed is read again.
unsafe fn start_read(src: &Arc<SourceState>) {
    if src.host.0.is_null() || has_wav(src) || reader_running(src) {
        return;
    }
    // a finished read that failed leaves its reader and its thread: clear them first
    stop_worker(src);
    src.failed.store(false, Ordering::SeqCst);
    let table = (*src.host.0).audioAccessControllerInterface;
    let create = if table.is_null() {
        None
    } else {
        (*table).createAudioReaderForSource
    };
    let Some(create) = create else {
        // no reader can be made: the read failed, and the worker is told (the round of the
        // document must not wait for this source)
        src.failed.store(true, Ordering::SeqCst);
        send(
            session(),
            Msg::Failed(src.clone(), src.audio_gen.load(Ordering::SeqCst)),
        );
        return;
    };
    let reader = create((*src.host.0).audioAccessControllerHostRef, src.host_ref.0, 0);
    *src.reader.lock().unwrap_or_else(|e| e.into_inner()) = SyncPtr(reader);
    let s = session();
    // before the reader thread starts, so the worker hears of the read before it hears that the
    // audio is here
    send(s, Msg::Reading(src.clone()));
    let worker_src = src.clone();
    let gen = src.audio_gen.load(Ordering::SeqCst);
    // `DS_ARA_SCENARIO=no-reader-thread`: the reader thread cannot be made
    let spawned = if crate::test_switch("DS_ARA_SCENARIO").as_deref() == Ok("no-reader-thread") {
        Err(std::io::Error::other("test: no reader thread"))
    } else {
        std::thread::Builder::new()
            .name(format!("drumspike-ara-read-{}", src.seq))
            .spawn(move || read_source(worker_src, s, gen))
    };
    if spawned.is_err() && crate::test_switch("DS_ARA_ROUND_FAULT").as_deref() != Ok("no-spawn-failed") {
        // neither `Ready` nor `Failed` would follow: the source would wait for its audio until the
        // deadline and be named late. The read failed: say so at once.
        src.failed.store(true, Ordering::SeqCst);
        send(s, Msg::Failed(src.clone(), src.audio_gen.load(Ordering::SeqCst)));
    }
    if let Ok(mut w) = src.worker.lock() {
        *w = spawned.ok();
    }
}

/// What one read gave: whether it worked, the hash of the samples, and its time.
struct ReadOutcome {
    ok: bool,
    hash: u64,
}

/// The reader thread: read every sample through the host's audio access controller,
/// write the source's WAV into the cache folder block by block, hash the samples on the
/// way, and tell the worker. The read runs inside the panic guard: a panic ends the read
/// as a failure (the worker is told, the round of the document does not wait for the
/// source), it never ends the thread without a message.
/// `gen` is the generation of the audio when the read was STARTED (on the host's thread): a thread
/// that starts late must not take the generation of a later update.
fn read_source(src: Arc<SourceState>, s: &'static Session, gen: u64) {
    let started = std::time::Instant::now();
    let props = src.props();
    let (count, rate, ch) = (props.sample_count, props.sample_rate, props.channels.max(1) as usize);
    let dest = src.doc.wav_path(src.seq, &props.name, gen);
    let body_src = src.clone();
    let body_dest = dest.clone();
    let unguarded = crate::test_switch("DS_ARA_GUARD_FAULT").as_deref() == Ok("off");
    let body = move || read_samples(&body_src, &body_dest, count, rate, ch);
    let outcome = if unguarded {
        Some(body())
    } else {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).ok()
    };
    let ms = started.elapsed().as_millis() as u64;
    let panicked = outcome.is_none();
    if panicked {
        eprintln!("drumspike-ara: caught panic in the reader thread");
        s.log(json!({"event": "reader_panic", "name": props.name}).to_string());
    }
    let outcome = outcome.unwrap_or(ReadOutcome { ok: false, hash: 0 });
    let mut outcome = outcome;
    if outcome.ok {
        // commit under the lock the snapshots and the content update use, and only for the live
        // generation: a content update that came while this read ran has raised it (T3), and
        // the file is of the old audio
        let kept = {
            let mut w = src.wav.lock().unwrap_or_else(|e| e.into_inner());
            let live = src.audio_gen.load(Ordering::SeqCst) == gen;
            if live {
                src.wav_len
                    .store(wav::file_len(count.max(0) as usize, ch as u16), Ordering::SeqCst);
                *w = Some(dest.clone());
            }
            live
        };
        if !kept {
            let _ = std::fs::remove_file(&dest);
            outcome.ok = false;
        }
    }
    if outcome.ok {
        s.log(format!(
            "{{\"event\":\"read\",\"name\":{:?},\"sample_count\":{},\"sample_rate\":{},\"channels\":{},\"read_ok\":true,\"fnv64\":\"{:016x}\",\"wav\":{:?},\"ms\":{},\"caller_thread\":\"main\",\"reader_thread\":\"{:?}\"}}",
            props.name,
            count,
            rate,
            ch,
            outcome.hash,
            dest.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
            ms,
            std::thread::current().id()
        ));
        src.read_tries.store(0, Ordering::SeqCst);
        // the files of the older generations stay until a round has moved the track to this one (T4)
        corrupt_for_scenario(&src, &dest);
        send(s, Msg::Ready(src));
    } else {
        src.failed.store(true, Ordering::SeqCst);
        s.log(format!(
            "{{\"event\":\"read\",\"name\":{:?},\"read_ok\":false}}",
            props.name
        ));
        send(s, Msg::Failed(src, gen));
    }
}

/// `DS_ARA_SCENARIO=corrupt-wav:<n>` (the check's scenario switch): after the read of the
/// source with the creation index `n` the plugin writes garbage over the first bytes of its
/// WAV (the size stays), so the service cannot read that track.
fn corrupt_for_scenario(src: &Arc<SourceState>, dest: &std::path::Path) {
    let Ok(v) = crate::test_switch("DS_ARA_SCENARIO") else {
        return;
    };
    if v.strip_prefix("corrupt-wav:").and_then(|n| n.parse::<usize>().ok()) != Some(src.seq) {
        return;
    }
    use std::io::{Seek, Write};
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(dest) {
        let _ = f.seek(std::io::SeekFrom::Start(0));
        let _ = f.write_all(b"NOTAWAVE-NOTAWAVE-NOTAWAVE");
    }
}

/// The samples of one source into its WAV (the body of the reader thread).
fn read_samples(src: &Arc<SourceState>, dest: &std::path::Path, count: i64, rate: f64, ch: usize) -> ReadOutcome {
    let hash = std::cell::Cell::new(0xcbf29ce484222325u64);
    test_panic("reader");
    let ok = (|| -> bool {
        let Ok(mut writer) = wav::WavWriter::create(dest, rate as u32, ch as u16, count.max(0) as usize) else {
            return false;
        };
        if src.host.0.is_null() {
            return false;
        }
        let table = unsafe { (*src.host.0).audioAccessControllerInterface };
        if table.is_null() {
            return false;
        }
        let iface = unsafe { &*table };
        let Some(read_samples) = iface.readAudioSamples else {
            return false;
        };
        let ctl = unsafe { (*src.host.0).audioAccessControllerHostRef };
        let reader = src.reader.lock().unwrap_or_else(|e| e.into_inner()).0;
        const BLOCK: usize = 65536;
        let mut bufs: Vec<Vec<f32>> = (0..ch).map(|_| vec![0f32; BLOCK]).collect();
        let mut pos = 0i64;
        while pos < count && !src.cancel.load(Ordering::SeqCst) {
            let n = BLOCK.min((count - pos).max(0) as usize);
            if n == 0 {
                break;
            }
            let ptrs: Vec<*mut c_void> = bufs.iter_mut().map(|b| b.as_mut_ptr() as *mut c_void).collect();
            let read = unsafe { read_samples(ctl, reader, pos, n as i64, ptrs.as_ptr()) };
            if read == 0 {
                return false;
            }
            let mut inter = Vec::with_capacity(n * ch);
            for i in 0..n {
                for b in bufs.iter() {
                    let v = b[i];
                    inter.push(v);
                    let mut h = hash.get();
                    for byte in v.to_le_bytes() {
                        h = (h ^ u64::from(byte)).wrapping_mul(0x100000001b3);
                    }
                    hash.set(h);
                }
            }
            if writer.write_interleaved_f32(&inter).is_err() {
                return false;
            }
            pos += n as i64;
        }
        // a read that was cancelled is no finished file: the temporary is never committed
        !src.cancel.load(Ordering::SeqCst) && writer.commit().is_ok()
    })();
    ReadOutcome { ok, hash: hash.get() }
}

unsafe extern "C" fn dc_requestAudioSourceContentAnalysis(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    _n: ARASize,
    _types: *const ARAContentType,
) {
    guard("requestAudioSourceContentAnalysis", (), move || {
        let s = session();
        // the caller thread goes into the report: the gate proves that the analysis does
        // not run on it (finding 4 of the review)
        let name = state_of(audioSourceRef).map(|src| src.props().name).unwrap_or_default();
        s.log(format!(
            "{{\"event\":\"request\",\"name\":{:?},\"thread\":\"{:?}\"}}",
            name,
            std::thread::current().id()
        ));
        let Some(src) = state_of(audioSourceRef) else { return };
        src.requested.store(true, Ordering::SeqCst);
        src.asked.store(true, Ordering::SeqCst);
        // T1: a source that failed for good (new audio, no retry left) reads again on a request
        // while the samples access is on
        if src.reanalyse.load(Ordering::SeqCst) && src.access.load(Ordering::SeqCst) {
            src.read_tries.store(0, Ordering::SeqCst);
            start_read(&src);
        }
        if s.fault {
            return; // the fault switch runs the job inside process instead
        }
        send(s, Msg::Request(src));
    })
}

unsafe extern "C" fn dc_isAudioSourceContentAnalysisIncomplete(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    _t: ARAContentType,
) -> ARABool {
    guard("isAudioSourceContentAnalysisIncomplete", 1, move || {
        let Some(src) = state_of(audioSourceRef) else { return 1 };
        // incomplete until the source has a result or its analysis ended without one (a failed
        // round, a track the service could not analyse): the host never waits for ever, and a
        // new request tries again
        let failed = src.analysis_failed.load(Ordering::SeqCst);
        // new samples (a content update) make a source with a result incomplete again
        let pending =
            !(src.has_events.load(Ordering::SeqCst) || failed) || (src.reanalyse.load(Ordering::SeqCst) && !failed);
        pending as ARABool
    })
}

unsafe extern "C" fn dc_isAudioSourceContentAvailable(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    t: ARAContentType,
) -> ARABool {
    guard("isAudioSourceContentAvailable", 0, move || {
        let Some(src) = state_of(audioSourceRef) else { return 0 };
        (t == kARAContentTypeNotes as ARAContentType && src.has_events.load(Ordering::SeqCst)) as ARABool
    })
}

unsafe extern "C" fn dc_getAudioSourceContentGrade(
    _c: ARADocumentControllerRef,
    _s: ARAAudioSourceRef,
    _t: ARAContentType,
) -> ARAContentGrade {
    guard("getAudioSourceContentGrade", 0, move || {
        kARAContentGradeDetected as ARAContentGrade
    })
}

fn note_of(sample: f64, volume: f64, rate: f64) -> ARAContentNote {
    ARAContentNote {
        frequency: kARAInvalidFrequency as f32,
        pitchNumber: kARAInvalidPitchNumber,
        volume: volume as f32,
        startPosition: sample / rate,
        attackDuration: 0.0,
        noteDuration: 0.01,
        signalDuration: 0.01,
    }
}

unsafe extern "C" fn dc_createAudioSourceContentReader(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    _t: ARAContentType,
    _range: *const ARAContentTimeRange,
) -> ARAContentReaderRef {
    guard("createAudioSourceContentReader", std::ptr::null_mut(), move || {
        let Some(src) = state_of(audioSourceRef) else {
            return Box::into_raw(Box::new(Reader { notes: Vec::new() })) as ARAContentReaderRef;
        };
        let rate = src.props().sample_rate;
        let notes: Vec<ARAContentNote> = src
            .notes
            .lock()
            .map(|n| {
                n.iter()
                    .map(|[sample, volume]| note_of(*sample, *volume, rate))
                    .collect()
            })
            .unwrap_or_default();
        let s = session();
        s.log(format!(
            "{{\"event\":\"notes\",\"name\":{:?},\"count\":{}}}",
            src.props().name,
            notes.len()
        ));
        Box::into_raw(Box::new(Reader { notes })) as ARAContentReaderRef
    })
}

unsafe extern "C" fn dc_getContentReaderEventCount(_c: ARADocumentControllerRef, r: ARAContentReaderRef) -> ARAInt32 {
    guard("getContentReaderEventCount", 0, move || {
        let reader: &Reader = &*(r as *mut Reader);
        reader.notes.len() as ARAInt32
    })
}

unsafe extern "C" fn dc_getContentReaderDataForEvent(
    _c: ARADocumentControllerRef,
    r: ARAContentReaderRef,
    i: ARAInt32,
) -> *const c_void {
    guard("getContentReaderDataForEvent", std::ptr::null(), move || {
        let reader: &Reader = &*(r as *mut Reader);
        &reader.notes[i as usize] as *const ARAContentNote as *const c_void
    })
}

unsafe extern "C" fn dc_destroyContentReader(_c: ARADocumentControllerRef, r: ARAContentReaderRef) {
    guard("destroyContentReader", (), move || {
        drop(Box::from_raw(r as *mut Reader));
    })
}

unsafe extern "C" fn dc_createAudioModification(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    _h: ARAAudioModificationHostRef,
    _p: *const ARAAudioModificationProperties,
) -> ARAAudioModificationRef {
    guard("createAudioModification", std::ptr::null_mut(), move || {
        let seq = state_of(audioSourceRef).map(|s| s.seq).unwrap_or(usize::MAX);
        Box::into_raw(Box::new(Modification { seq })) as ARAAudioModificationRef
    })
}

unsafe extern "C" fn dc_cloneAudioModification(
    _c: ARADocumentControllerRef,
    m: ARAAudioModificationRef,
    _h: ARAAudioModificationHostRef,
    _p: *const ARAAudioModificationProperties,
) -> ARAAudioModificationRef {
    guard("cloneAudioModification", std::ptr::null_mut(), move || {
        let seq = (*(m as *mut Modification)).seq;
        Box::into_raw(Box::new(Modification { seq })) as ARAAudioModificationRef
    })
}

unsafe extern "C" fn dc_destroyAudioModification(_c: ARADocumentControllerRef, m: ARAAudioModificationRef) {
    guard("destroyAudioModification", (), move || {
        drop(Box::from_raw(m as *mut Modification));
    })
}

unsafe fn region_times(p: *const ARAPlaybackRegionProperties) -> service::RegionRec {
    service::RegionRec {
        start_mod: (*p).startInModificationTime,
        dur_mod: (*p).durationInModificationTime,
        start_play: (*p).startInPlaybackTime,
    }
}

unsafe extern "C" fn dc_createPlaybackRegion(
    c: ARADocumentControllerRef,
    m: ARAAudioModificationRef,
    _h: ARAPlaybackRegionHostRef,
    p: *const ARAPlaybackRegionProperties,
) -> ARAPlaybackRegionRef {
    guard("createPlaybackRegion", std::ptr::null_mut(), move || {
        let seq = (*(m as *mut Modification)).seq;
        let rec = region_times(p);
        let id = NEXT_REGION_ID.fetch_add(1, Ordering::SeqCst);
        let ctl = controller_of(c);
        if let Ok(list) = ctl.sources.lock() {
            if let Some(src) = list.iter().find(|s| s.seq == seq) {
                if let Ok(mut rl) = src.regions.lock() {
                    rl.push(RegionSlot { id, rec });
                }
                touch(src);
            }
        }
        Box::into_raw(Box::new(PlaybackRegion { seq, id })) as ARAPlaybackRegionRef
    })
}

unsafe extern "C" fn dc_updatePlaybackRegionProperties(
    c: ARADocumentControllerRef,
    r: ARAPlaybackRegionRef,
    p: *const ARAPlaybackRegionProperties,
) {
    guard("updatePlaybackRegionProperties", (), move || {
        // the update names its region: the record with the region's id is rewritten, the
        // other regions of the source keep their times
        let region: &PlaybackRegion = &*(r as *const PlaybackRegion);
        let rec = region_times(p);
        let ctl = controller_of(c);
        let mut moved = false;
        if let Ok(list) = ctl.sources.lock() {
            if let Some(src) = list.iter().find(|s| s.seq == region.seq) {
                if let Ok(mut rl) = src.regions.lock() {
                    if let Some(slot) = rl.iter_mut().find(|slot| slot.id == region.id) {
                        // a move in playback time alone changes nothing the analysis uses: the
                        // record is kept, no round starts (`DS_ARA_TOUCH_FAULT=playback-time`
                        // brings back the round)
                        moved = slot.rec.start_mod != rec.start_mod
                            || slot.rec.dur_mod != rec.dur_mod
                            || crate::test_switch("DS_ARA_TOUCH_FAULT").as_deref() == Ok("playback-time");
                        slot.rec = rec;
                    }
                }
                if moved {
                    touch(src);
                }
            }
        }
    })
}

unsafe extern "C" fn dc_destroyPlaybackRegion(c: ARADocumentControllerRef, r: ARAPlaybackRegionRef) {
    guard("destroyPlaybackRegion", (), move || {
        // the region record leaves the source with the ref (the destroyed region is no
        // longer part of the document)
        let region = Box::from_raw(r as *mut PlaybackRegion);
        let ctl = controller_of(c);
        if let Ok(list) = ctl.sources.lock() {
            if let Some(src) = list.iter().find(|s| s.seq == region.seq) {
                if let Ok(mut rl) = src.regions.lock() {
                    rl.retain(|slot| slot.id != region.id);
                }
                touch(src);
            }
        }
        drop(region);
    })
}

// ---------------------------------------------------------------- archiving

/// The document archive ID of this plugin's factory.
const ARCHIVE_ID: &str = "com.cherryblossomvalley.drumspike.ara.archive.v1";

/// The name the archive stores: never an absolute path. The ARA name of a source may be
/// the host's file path; a stored absolute path would carry the host's folders into the
/// archive (and the committed fixture). A name with a path separator is reduced to its
/// final component; a clean name is kept as it is.
fn archive_name_of(name: &str) -> String {
    if name.starts_with('/') || name.contains('\\') || name.contains('/') {
        std::path::Path::new(name)
            .file_name()
            .map_or_else(|| name.to_string(), |s| s.to_string_lossy().into_owned())
    } else {
        name.to_string()
    }
}

/// The archived source of a host source: the persistent ID first, then the name. The
/// archive stores the reduced name (`archive_name_of`), so the host's own name (it may be
/// a path, with no persistent ID) is reduced the same way before the compare. An entry that
/// an earlier source took (`used`, by position) is not taken again: two sources of one name
/// and no persistent ID get two different entries.
fn find_archived<'a>(
    sources: &'a [state::ArchiveSource],
    persistent_id: &str,
    name: &str,
    used: &[usize],
) -> Option<&'a state::ArchiveSource> {
    let reduced = archive_name_of(name);
    let free = |i: usize| !used.contains(&i);
    sources
        .iter()
        .enumerate()
        .find(|(i, a)| free(*i) && !a.persistent_id.is_empty() && a.persistent_id == persistent_id)
        .or_else(|| sources.iter().enumerate().find(|(i, a)| free(*i) && a.name == reduced))
        .map(|(_, a)| a)
}

/// The upper bound for an archive the host hands over: a hostile or broken host value
/// must not become an unbounded allocation.
const MAX_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;

unsafe extern "C" fn dc_storeObjectsToArchive(
    _c: ARADocumentControllerRef,
    writer: ARAArchiveWriterHostRef,
    _filter: *const ARAStoreObjectsFilter,
) -> ARABool {
    guard("storeObjectsToArchive", 0, move || {
        let s = session();
        let sources: Vec<state::ArchiveSource> = {
            let ctl = controller_of(_c);
            let Ok(list) = ctl.sources.lock() else { return 0 };
            list.iter()
                .map(|src| {
                    let p = src.props();
                    state::ArchiveSource {
                        persistent_id: p.persistent_id,
                        name: archive_name_of(&p.name),
                        sample_rate: p.sample_rate,
                        notes: src.notes.lock().map(|n| n.clone()).unwrap_or_default(),
                        track: src.track.lock().ok().and_then(|t| t.clone()).unwrap_or_default(),
                        stale: src.stale.load(Ordering::SeqCst),
                    }
                })
                .collect()
        };
        let ctl = controller_of(_c);
        // the events of the project of this document (not of the last job of the process)
        let events = ctl.doc.events.lock().map(|e| e.clone()).unwrap_or_default();
        let blob = state::encode(&state::ArchiveState {
            version: state::ARCHIVE_VERSION,
            archive_id: ARCHIVE_ID.into(),
            doc_id: ctl.doc.id(),
            events,
            sources,
        });
        if ctl.host.is_null() {
            return 0;
        }
        let table = (*ctl.host).archivingControllerInterface;
        if table.is_null() {
            return 0;
        }
        let a = &*table;
        let Some(write_bytes) = a.writeBytesToArchive else {
            return 0;
        };
        let host_ref = (*ctl.host).archivingControllerHostRef;
        let ok = write_bytes(host_ref, writer, 0, blob.len(), blob.as_ptr());
        ok as ARABool
    })
}

unsafe extern "C" fn dc_restoreObjectsFromArchive(
    c: ARADocumentControllerRef,
    reader: ARAArchiveReaderHostRef,
    _filter: *const ARARestoreObjectsFilter,
) -> ARABool {
    guard("restoreObjectsFromArchive", 0, move || {
        let s = session();
        let ctl = controller_of(c);
        if ctl.host.is_null() {
            return 0;
        }
        let table = (*ctl.host).archivingControllerInterface;
        if table.is_null() {
            return 0;
        }
        let a = &*table;
        let Some(get_size) = a.getArchiveSize else { return 0 };
        let Some(read_bytes) = a.readBytesFromArchive else {
            return 0;
        };
        let host_ref = (*ctl.host).archivingControllerHostRef;
        let size = get_size(host_ref, reader);
        if size == 0 || size > MAX_ARCHIVE_BYTES {
            return 0;
        }
        let mut bytes = vec![0u8; size];
        if read_bytes(host_ref, reader, 0, size, bytes.as_mut_ptr()) == 0 {
            return 0;
        }
        let Ok(st) = state::decode(&bytes) else { return 0 };
        let fault = crate::test_switch("DS_ARA_RESTORE_FAULT").unwrap_or_default();
        // The stored document id names the project folder again. It comes from a file the
        // user may have damaged or edited: only the form the plugin makes (16 lowercase hex
        // digits) is taken, anything else gets the fresh id the document already has, and
        // the report says so. An id another live document of this process holds (a session
        // file that was copied, or one that is open twice) is not taken either: the second
        // claimant keeps its fresh id and its folder starts from the archive data. An
        // archive of M1a has no id, and a fresh one stays. (Fault `newid`: the stored id is
        // ignored.)
        let mut id: Option<String> = None;
        if !st.doc_id.is_empty() && fault != "newid" {
            if !service::accept_doc_id(&st.doc_id) {
                s.log(json!({"event": "doc_id_refused", "value": st.doc_id}).to_string());
            } else if !claim_id(&ctl.doc.id(), &st.doc_id) {
                s.log(json!({"event": "doc_id_taken", "value": st.doc_id}).to_string());
            } else {
                *ctl.doc.id.lock().unwrap_or_else(|e| e.into_inner()) = st.doc_id.clone();
                id = Some(st.doc_id.clone());
            }
        }
        // the events of the archive that name a track the plugin can keep (a name that is
        // no path); the others are dropped and the report says how many
        let events_ok: Vec<(&String, &Value)> = st
            .events
            .iter()
            .filter(|(k, _)| service::accept_track_name(k))
            .collect();
        if events_ok.len() != st.events.len() {
            s.log(
                json!({"event": "restore_refused", "what": "events keyed by a bad track name",
                    "count": st.events.len() - events_ok.len()})
                .to_string(),
            );
        }
        let mut restored: Vec<service::RestoredTrack> = Vec::new();
        let mut used: Vec<usize> = Vec::new();
        if let Ok(list) = ctl.sources.lock() {
            for src in list.iter() {
                let p = src.props();
                // two sources of one name and no persistent ID match two different entries
                let found = find_archived(&st.sources, &p.persistent_id, &p.name, &used);
                if let Some(a) = found {
                    let at = st.sources.iter().position(|x| std::ptr::eq(x, a)).unwrap_or(0);
                    used.push(at);
                    if let Ok(mut n) = src.notes.lock() {
                        *n = a.notes.clone();
                    }
                    src.has_events.store(true, Ordering::SeqCst);
                    // a stale track needs a new analysis: the next round with audio takes it
                    src.stale.store(a.stale, Ordering::SeqCst);
                    src.analysis_failed.store(a.stale, Ordering::SeqCst);
                    // the track of the source: stored (M1b1), else derived from the stored
                    // name the way the analysis named it (an archive of M1a); a stored name
                    // that is a path is not trusted
                    let track = if a.track.is_empty() || !service::accept_track_name(&a.track) {
                        if !a.track.is_empty() {
                            s.log(
                                json!({"event": "restore_refused", "what": "bad track name", "value": a.track})
                                    .to_string(),
                            );
                        }
                        drumspike_app::tracks::safe_name(&service::stem_of(&a.name), at)
                    } else {
                        a.track.clone()
                    };
                    if let Some((_, events)) = events_ok.iter().find(|(k, _)| **k == track) {
                        if let Ok(mut t) = src.track.lock() {
                            *t = Some(track.clone());
                        }
                        let mut snapshot = src.snapshot();
                        if snapshot.wav.as_os_str().is_empty() {
                            // where the reader would write it: nothing is decoded now
                            snapshot.wav = ctl.doc.wav_path(src.seq, &p.name, 0);
                        }
                        restored.push(service::RestoredTrack {
                            snapshot,
                            track,
                            events: if fault == "empty" { json!([]) } else { (*events).clone() },
                        });
                    }
                }
            }
        }
        // the tracks in the order of the stored events
        let order: Vec<&String> = st.events.keys().collect();
        restored.sort_by_key(|t| order.iter().position(|k| **k == t.track).unwrap_or(usize::MAX));
        // The events of the document are set here, on the calling thread, not by the
        // worker: the worker is shared by all documents and may be busy for minutes, and a
        // host that stores the document right after the restore (an autosave) must get the
        // events, or the next restore would find none. (Fault `late-events`: the worker
        // sets them, as the first version did.)
        if fault != "late-events" {
            let mut m = serde_json::Map::new();
            for t in &restored {
                m.insert(t.track.clone(), t.events.clone());
            }
            *ctl.doc.events.lock().unwrap_or_else(|e| e.into_inner()) = m;
        }
        send(
            s,
            Msg::Restore {
                doc: ctl.doc.clone(),
                id,
                tracks: restored,
            },
        );
        // the fault switch of the no-decode check: decode one source during the restore
        // on purpose, so the gate's "no read row, no cache WAV" proof can fail
        if crate::test_switch("DS_ARA_RESTORE_FAULT").as_deref() == Ok("decode") {
            let first = ctl.sources.lock().ok().and_then(|l| l.first().cloned());
            if let Some(src) = first {
                unsafe { start_read(&src) };
                loop {
                    let done =
                        src.wav.lock().map(|w| w.is_some()).unwrap_or(false) || src.failed.load(Ordering::SeqCst);
                    if done {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                unsafe { stop_worker(&src) };
            }
        }
        let row = format!(
            "{{\"event\":\"service\",\"restored\":true,\"version\":{},\"doc\":\"{}\",\"events\":{}}}",
            st.version,
            ctl.doc.id(),
            serde_json::to_string(&st.events).unwrap_or_else(|_| "{}".into()),
        );
        if let Ok(mut sr) = s.service_row.lock() {
            *sr = Some(row);
        }
        s.write_out();
        1
    })
}

// ---------------------------------------------------------------- the CLAP plugin

const PLUGIN_ID: *const c_char = cs!("com.cherryblossomvalley.drumspike.ara");

const EXT_ARA_FACTORY: &[u8] = b"org.ara-audio.ara.factory/2";
const EXT_ARA_FACTORY_DRAFT: &[u8] = b"org.ara-audio.ara.factory.draft/2";
const EXT_ARA_PLUGIN: &[u8] = b"org.ara-audio.ara.pluginextension/2";
const EXT_ARA_PLUGIN_DRAFT: &[u8] = b"org.ara-audio.ara.pluginextension.draft/2";

static FEATURES: SyncPtr<[*const c_char; 4]> = SyncPtr([
    cs!("audio-effect"),
    cs!("ara:supported"),
    cs!("ara:required"),
    std::ptr::null(),
]);

static DESCRIPTOR: clap_plugin_descriptor = clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: PLUGIN_ID,
    name: cs!("Drum Spike ARA"),
    vendor: cs!("Cherry Blossom Valley"),
    url: cs!("https://github.com/Cherry-Blossom-Valley/drumspike"),
    manual_url: cs!(""),
    support_url: cs!(""),
    version: version_cs!(),
    description: cs!("The Drum Spike service on the host's tracks (ARA, no editor)"),
    features: &FEATURES.0 as *const [*const c_char; 4] as *const *const c_char,
};

/// The CLAP ARA factory extension (`get_factory("org.ara-audio.ara.factory/2")`); the
/// test host reads it to reach the ARA factory.
#[repr(C)]
pub struct clap_ara_factory {
    pub get_factory_count: Option<unsafe extern "C" fn(*const clap_ara_factory) -> u32>,
    pub get_ara_factory: Option<unsafe extern "C" fn(*const clap_ara_factory, u32) -> *const ARAFactory>,
    pub get_plugin_id: Option<unsafe extern "C" fn(*const clap_ara_factory, u32) -> *const c_char>,
}
#[repr(C)]
pub struct clap_ara_plugin_extension {
    pub get_factory: Option<unsafe extern "C" fn(*const clap_plugin) -> *const ARAFactory>,
    pub bind_to_document_controller: Option<
        unsafe extern "C" fn(
            *const clap_plugin,
            ARADocumentControllerRef,
            ARAPlugInInstanceRoleFlags,
            ARAPlugInInstanceRoleFlags,
        ) -> *const ARAPlugInExtensionInstance,
    >,
}

unsafe extern "C" fn araf_count(_f: *const clap_ara_factory) -> u32 {
    guard("ara_factory_count", 0, move || 1)
}
unsafe extern "C" fn araf_get(_f: *const clap_ara_factory, i: u32) -> *const ARAFactory {
    guard("ara_factory_get", std::ptr::null(), move || {
        if i == 0 {
            &FACTORY.0
        } else {
            std::ptr::null()
        }
    })
}
unsafe extern "C" fn araf_id(_f: *const clap_ara_factory, i: u32) -> *const c_char {
    guard("ara_factory_id", std::ptr::null(), move || {
        if i == 0 {
            PLUGIN_ID
        } else {
            std::ptr::null()
        }
    })
}
static ARA_FACTORY_EXT: clap_ara_factory = clap_ara_factory {
    get_factory_count: Some(araf_count),
    get_ara_factory: Some(araf_get),
    get_plugin_id: Some(araf_id),
};

struct Plugin {
    clap: clap_plugin,
    ext: ARAPlugInExtensionInstance,
    sample_rate: Mutex<f64>,
    /// The document the ARA bind call joined this instance to (the editor shows its service).
    doc: Mutex<Option<Arc<DocShared>>>,
    /// The editor; made and dropped on the host's main thread only.
    editor: std::cell::RefCell<Option<editor::Editor>>,
    /// The size the editor asks for, packed (width << 32 | height): read by any thread the host calls
    /// `get_size` from.
    gui_size: std::sync::atomic::AtomicU64,
}

unsafe extern "C" fn pext_factory(_p: *const clap_plugin) -> *const ARAFactory {
    guard("pext_factory", std::ptr::null(), move || &FACTORY.0)
}
unsafe extern "C" fn pext_bind(
    p: *const clap_plugin,
    dc: ARADocumentControllerRef,
    known: ARAPlugInInstanceRoleFlags,
    assigned: ARAPlugInInstanceRoleFlags,
) -> *const ARAPlugInExtensionInstance {
    guard("bind_to_document_controller", std::ptr::null(), move || {
        let plugin = (*p).plugin_data as *mut Plugin;
        if !dc.is_null() {
            *(*plugin).doc.lock().unwrap_or_else(|e| e.into_inner()) = Some(controller_of(dc).doc.clone());
        }
        let me = plugin as *mut c_void;
        let has = |role: u32| assigned & role as i32 != 0;
        (*plugin).ext = ARAPlugInExtensionInstance {
            structSize: std::mem::size_of::<ARAPlugInExtensionInstance>(),
            plugInExtensionRef: std::ptr::null_mut(),
            plugInExtensionInterface: std::ptr::null(),
            playbackRendererRef: if has(kARAPlaybackRendererRole) {
                me as _
            } else {
                std::ptr::null_mut()
            },
            playbackRendererInterface: if has(kARAPlaybackRendererRole) {
                &PR_INTERFACE.0
            } else {
                std::ptr::null()
            },
            editorRendererRef: if has(kARAEditorRendererRole) {
                me as _
            } else {
                std::ptr::null_mut()
            },
            editorRendererInterface: if has(kARAEditorRendererRole) {
                &ER_INTERFACE.0
            } else {
                std::ptr::null()
            },
            editorViewRef: if has(kARAEditorViewRole) {
                me as _
            } else {
                std::ptr::null_mut()
            },
            editorViewInterface: if has(kARAEditorViewRole) {
                &EV_INTERFACE.0
            } else {
                std::ptr::null()
            },
        };
        session().log(format!(
            "{{\"event\":\"bind_to_document_controller\",\"known_roles\":{known},\"assigned_roles\":{assigned}}}"
        ));
        &(*plugin).ext
    })
}
static ARA_PLUGIN_EXT: clap_ara_plugin_extension = clap_ara_plugin_extension {
    get_factory: Some(pext_factory),
    bind_to_document_controller: Some(pext_bind),
};

unsafe extern "C" fn ports_count(_p: *const clap_plugin, _is_input: bool) -> u32 {
    guard("ports_count", 1u32, move || 1u32)
}

unsafe extern "C" fn ports_get(
    _p: *const clap_plugin,
    index: u32,
    is_input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    guard("ports_get", false, move || {
        if index != 0 {
            return false;
        }
        let mut name = [0 as c_char; 256];
        for (i, b) in (if is_input { "In" } else { "Out" }).bytes().enumerate() {
            name[i] = b as c_char;
        }
        *info = clap_audio_port_info {
            id: if is_input { 0 } else { 1 },
            name,
            flags: CLAP_AUDIO_PORT_IS_MAIN,
            channel_count: 1,
            port_type: CLAP_PORT_MONO.as_ptr(),
            in_place_pair: if is_input { 1 } else { 0 },
        };
        true
    })
}

static AUDIO_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(ports_count),
    get: Some(ports_get),
};

unsafe extern "C" fn p_init(_p: *const clap_plugin) -> bool {
    guard("init", true, move || {
        start_worker(session());
        true
    })
}

unsafe extern "C" fn p_destroy(p: *const clap_plugin) {
    guard("destroy", (), move || {
        drop(Box::from_raw((*p).plugin_data as *mut Plugin));
    })
}

unsafe extern "C" fn p_activate(p: *const clap_plugin, sr: f64, _min: u32, _max: u32) -> bool {
    guard("activate", false, move || {
        let plugin: &Plugin = &*((*p).plugin_data as *mut Plugin);
        *plugin.sample_rate.lock().unwrap_or_else(|e| e.into_inner()) = sr;
        PROCESS_RATE_BITS.store(sr.to_bits(), Ordering::Relaxed);
        transport::read_switches(); // off the audio thread: the process call reads atomics only
        let _ = transport::now_us(); // makes the clock the process call stamps with
        true
    })
}

unsafe extern "C" fn p_deactivate(p: *const clap_plugin) {
    guard("deactivate", (), move || transport::clear(p as u64))
}
unsafe extern "C" fn p_start(_p: *const clap_plugin) -> bool {
    guard("start_processing", true, move || true)
}
unsafe extern "C" fn p_stop(_p: *const clap_plugin) {
    guard("stop_processing", (), move || {})
}
unsafe extern "C" fn p_reset(p: *const clap_plugin) {
    guard("reset", (), move || transport::clear(p as u64))
}

// ---- the evidence rows (M1b4): what a host sends. They change no behaviour.

/// The time since the first call of this function, in ms.
fn mono_ms() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

const UNSET: u64 = u64::MAX;

/// True for the first call, and then at most once per `gap_ms` (the rate limit of the rows
/// that a host may send many times a second).
fn rate_gate(last: &AtomicU64, now_ms: u64, gap_ms: u64) -> bool {
    let prev = last.load(Ordering::SeqCst);
    (prev == UNSET || now_ms.saturating_sub(prev) >= gap_ms)
        && last
            .compare_exchange(prev, now_ms, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
}

/// The rows of one kind of host call, one a second at most. A row that the limit holds back is
/// not lost: the newest one waits and the worker writes it when the second is over.
struct RateRows {
    last: AtomicU64,
    pending: Mutex<Option<String>>,
}

impl RateRows {
    const fn new() -> Self {
        RateRows {
            last: AtomicU64::new(UNSET),
            pending: Mutex::new(None),
        }
    }

    /// The row to write now, or `None` when it must wait (the newest waiting row is kept).
    fn offer(&self, row: String, now_ms: u64, gap_ms: u64) -> Option<String> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if rate_gate(&self.last, now_ms, gap_ms) {
            *pending = None;
            Some(row)
        } else {
            *pending = Some(row);
            None
        }
    }

    /// The waiting row, when the second is over.
    fn flush(&self, now_ms: u64, gap_ms: u64) -> Option<String> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending.is_some() && rate_gate(&self.last, now_ms, gap_ms) {
            pending.take()
        } else {
            None
        }
    }
}

static SELECTION_ROWS: RateRows = RateRows::new();
static HIDE_ROWS: RateRows = RateRows::new();

/// What the audio thread saw. It only sets atomics here: the row is written by the worker
/// ([`Session::poll_evidence`]), never on the audio thread. 0: not seen, 1: seen, 2: written.
static PROCESS_SEEN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static PROCESS_FRAMES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static PROCESS_RATE_BITS: AtomicU64 = AtomicU64::new(0);
static TRANSPORT_SEEN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static TRANSPORT_FLAGS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// The position as the host gave it: CLAP seconds in fixed point (2^31 per second).
static TRANSPORT_SECONDS_BITS: AtomicU64 = AtomicU64::new(0);

/// The audio thread's part of the evidence: a few atomic stores, no lock, no allocation.
unsafe fn note_process(process: *const clap_process) {
    let pr = &*process;
    if PROCESS_SEEN.load(Ordering::Relaxed) == 0 {
        PROCESS_FRAMES.store(pr.frames_count, Ordering::Relaxed);
        PROCESS_SEEN.store(1, Ordering::Release);
    }
    if TRANSPORT_SEEN.load(Ordering::Relaxed) == 0 && !pr.transport.is_null() {
        let t = &*pr.transport;
        TRANSPORT_FLAGS.store(t.flags, Ordering::Relaxed);
        TRANSPORT_SECONDS_BITS.store(t.song_pos_seconds as u64, Ordering::Relaxed);
        TRANSPORT_SEEN.store(1, Ordering::Release);
    }
}

impl Session {
    /// Write the rows `process_first` and `transport_seen` once, when the audio thread saw
    /// them. Called by the worker, never by the audio thread.
    fn poll_evidence(&self) {
        use clap_sys::events::*;
        let mut wrote = false;
        let writer = format!("{:?}", std::thread::current().id());
        if PROCESS_SEEN
            .compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.log(
                json!({
                    "event": "process_first",
                    "sample_rate": f64::from_bits(PROCESS_RATE_BITS.load(Ordering::Relaxed)),
                    "frames": PROCESS_FRAMES.load(Ordering::Relaxed),
                    "writer_thread": writer,
                })
                .to_string(),
            );
            wrote = true;
        }
        if TRANSPORT_SEEN
            .compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let f = TRANSPORT_FLAGS.load(Ordering::Relaxed);
            self.log(
                json!({
                    "event": "transport_seen",
                    // M1b5: where the position of the page comes from. ARA gives a plugin no
                    // transport, so the process call is the only source.
                    "source": "clap_process",
                    "has_seconds": f & CLAP_TRANSPORT_HAS_SECONDS_TIMELINE != 0,
                    "has_beats": f & CLAP_TRANSPORT_HAS_BEATS_TIMELINE != 0,
                    "has_tempo": f & CLAP_TRANSPORT_HAS_TEMPO != 0,
                    "is_playing": f & CLAP_TRANSPORT_IS_PLAYING != 0,
                    "position_seconds": (TRANSPORT_SECONDS_BITS.load(Ordering::Relaxed) as i64) as f64 / 2_147_483_648.0,
                    "writer_thread": writer,
                })
                .to_string(),
            );
            wrote = true;
        }
        // the rows that the rate limit held back, and the rows the host's main thread left
        for pending in [SELECTION_ROWS.flush(mono_ms(), 1000), HIDE_ROWS.flush(mono_ms(), 1000)]
            .into_iter()
            .flatten()
        {
            self.log(pending);
            wrote = true;
        }
        if ROWS_DIRTY.swap(false, Ordering::SeqCst) || wrote {
            self.write_out();
        }
    }
}

// ---- the state extension (M1b4): LUNA asks every plugin for its state when it makes an
// instance, and logs "loadPreset failed" when the plugin has none. The ARA archive holds the
// data, so the state is only a small versioned blob.

const STATE_MAGIC: &[u8; 6] = b"DSARA\0";
const STATE_VERSION: u32 = 1;

fn state_blob() -> Vec<u8> {
    let mut b = STATE_MAGIC.to_vec();
    b.extend_from_slice(&STATE_VERSION.to_le_bytes());
    b
}

/// Load never fails the host's instance creation: an empty stream, a newer version and
/// garbage are all accepted and ignored. `DS_ARA_STATE_FAULT=strict` (the check's fault run)
/// refuses what is not a blob of version 1.
fn state_accepts(bytes: &[u8]) -> bool {
    if crate::test_switch("DS_ARA_STATE_FAULT").as_deref() == Ok("strict") {
        return bytes == state_blob().as_slice();
    }
    true
}

unsafe extern "C" fn state_save(_p: *const clap_plugin, stream: *const clap_sys::stream::clap_ostream) -> bool {
    guard("state_save", false, move || {
        let Some(write) = (*stream).write else { return false };
        let blob = state_blob();
        let mut done = 0usize;
        while done < blob.len() {
            let n = write(
                stream,
                blob[done..].as_ptr() as *const c_void,
                (blob.len() - done) as u64,
            );
            if n <= 0 {
                return false;
            }
            done += n as usize;
        }
        true
    })
}

unsafe extern "C" fn state_load(_p: *const clap_plugin, stream: *const clap_sys::stream::clap_istream) -> bool {
    guard("state_load", false, move || {
        let Some(read) = (*stream).read else {
            return state_accepts(&[]);
        };
        let mut got: Vec<u8> = Vec::new();
        let mut buf = [0u8; 256];
        // a bound: no state of this plugin is larger, and a host stream must not hold the thread
        while got.len() < 65_536 {
            let n = read(stream, buf.as_mut_ptr() as *mut c_void, buf.len() as u64);
            if n <= 0 {
                break;
            }
            got.extend_from_slice(&buf[..(n as usize).min(buf.len())]);
        }
        state_accepts(&got)
    })
}

static STATE_EXT: clap_sys::ext::state::clap_plugin_state = clap_sys::ext::state::clap_plugin_state {
    save: Some(state_save),
    load: Some(state_load),
};

/// The audio thread: pass the first mono pair through (silence without an input), touch
/// nothing else. The analysis runs on its own threads and is never reached here — except
/// under the fault switch, which runs it inside `process` on purpose (the overrun proof).
unsafe extern "C" fn p_process(p: *const clap_plugin, process: *const clap_process) -> clap_process_status {
    guard("process", CLAP_PROCESS_CONTINUE, move || {
        let s = session();
        if s.fault && !s.fault_ran.swap(true, Ordering::SeqCst) {
            s.run_fault_job();
        }
        note_process(process);
        // M1b5: the transport for the page, atomics only (see transport.rs)
        transport::note(
            process,
            f64::from_bits(PROCESS_RATE_BITS.load(Ordering::Relaxed)),
            p as u64,
        );
        let pr = &*process;
        let frames = pr.frames_count as usize;
        if pr.audio_outputs_count > 0 {
            let out = &*pr.audio_outputs;
            let input = if pr.audio_inputs_count > 0 {
                &*pr.audio_inputs
            } else {
                std::ptr::null()
            };
            for c in 0..out.channel_count as usize {
                let dst = *out.data32.add(c);
                if dst.is_null() {
                    continue;
                }
                let src: *const f32 = if !input.is_null() && ((*input).channel_count as usize) > c {
                    *(*input).data32.add(c)
                } else {
                    std::ptr::null()
                };
                if src.is_null() {
                    std::ptr::write_bytes(dst, 0, frames);
                } else {
                    std::ptr::copy_nonoverlapping(src, dst, frames);
                }
            }
        }
        CLAP_PROCESS_CONTINUE
    })
}

unsafe extern "C" fn p_get_extension(_p: *const clap_plugin, id: *const c_char) -> *const c_void {
    guard("get_extension", std::ptr::null(), move || {
        let id = CStr::from_ptr(id).to_bytes();
        if id == CLAP_EXT_AUDIO_PORTS.to_bytes() {
            return &AUDIO_PORTS as *const _ as *const c_void;
        }
        if id == EXT_ARA_PLUGIN || id == EXT_ARA_PLUGIN_DRAFT {
            return &ARA_PLUGIN_EXT as *const _ as *const c_void;
        }
        if id == clap_sys::ext::gui::CLAP_EXT_GUI.to_bytes() {
            return &editor::PLUGIN_GUI as *const _ as *const c_void;
        }
        if id == clap_sys::ext::state::CLAP_EXT_STATE.to_bytes() {
            return &STATE_EXT as *const _ as *const c_void;
        }
        std::ptr::null()
    })
}

unsafe extern "C" fn p_on_main_thread(_p: *const clap_plugin) {
    guard("on_main_thread", (), move || {})
}

// the playback renderer of M1a passes audio through (the notes reach the host through
// the content readers; the timeline render of the notes arrives with the editor part)
unsafe extern "C" fn pr_addPlaybackRegion(_rendererRef: ARAPlaybackRendererRef, _r: ARAPlaybackRegionRef) {
    guard("addPlaybackRegion", (), move || {})
}
unsafe extern "C" fn pr_removePlaybackRegion(_rendererRef: ARAPlaybackRendererRef, _r: ARAPlaybackRegionRef) {
    guard("removePlaybackRegion", (), move || {})
}
static PR_INTERFACE: SyncPtr<ARAPlaybackRendererInterface> = SyncPtr(ARAPlaybackRendererInterface {
    structSize: std::mem::size_of::<ARAPlaybackRendererInterface>(),
    addPlaybackRegion: Some(pr_addPlaybackRegion),
    removePlaybackRegion: Some(pr_removePlaybackRegion),
});

unsafe extern "C" fn er_addPlaybackRegion(
    _editorRendererRef: ARAEditorRendererRef,
    _playbackRegionRef: ARAPlaybackRegionRef,
) {
    guard("er_addPlaybackRegion", (), move || {})
}
unsafe extern "C" fn er_removePlaybackRegion(
    _editorRendererRef: ARAEditorRendererRef,
    _playbackRegionRef: ARAPlaybackRegionRef,
) {
    guard("er_removePlaybackRegion", (), move || {})
}
unsafe extern "C" fn er_addRegionSequence(
    _editorRendererRef: ARAEditorRendererRef,
    _regionSequenceRef: ARARegionSequenceRef,
) {
    guard("er_addRegionSequence", (), move || {})
}
unsafe extern "C" fn er_removeRegionSequence(
    _editorRendererRef: ARAEditorRendererRef,
    _regionSequenceRef: ARARegionSequenceRef,
) {
    guard("er_removeRegionSequence", (), move || {})
}
static ER_INTERFACE: SyncPtr<ARAEditorRendererInterface> = SyncPtr(ARAEditorRendererInterface {
    structSize: std::mem::size_of::<ARAEditorRendererInterface>(),
    addPlaybackRegion: Some(er_addPlaybackRegion),
    removePlaybackRegion: Some(er_removePlaybackRegion),
    addRegionSequence: Some(er_addRegionSequence),
    removeRegionSequence: Some(er_removeRegionSequence),
});

/// The report row of a selection (M1b4, evidence only): the counts of what the host selected
/// and whether it gave a time range. The first call and then one a second; a row the limit holds
/// back waits for the worker (the newest one is kept).
unsafe extern "C" fn ev_notifySelection(_editorViewRef: ARAEditorViewRef, selection: *const ARAViewSelection) {
    guard("notifySelection", (), move || {
        let (mut regions, mut sequences, mut range) = (0usize, 0usize, false);
        if !selection.is_null() {
            // the struct of the host must reach the field before the plugin reads it
            let declared = std::ptr::addr_of!((*selection).structSize).read_unaligned();
            let end = |off: usize| off + std::mem::size_of::<usize>();
            if declared >= end(std::mem::offset_of!(ARAViewSelection, playbackRegionRefsCount)) {
                regions = std::ptr::addr_of!((*selection).playbackRegionRefsCount).read_unaligned();
            }
            if declared >= end(std::mem::offset_of!(ARAViewSelection, regionSequenceRefsCount)) {
                sequences = std::ptr::addr_of!((*selection).regionSequenceRefsCount).read_unaligned();
            }
            if declared >= end(std::mem::offset_of!(ARAViewSelection, timeRange)) {
                range = !std::ptr::addr_of!((*selection).timeRange).read_unaligned().is_null();
            }
        }
        let row = json!({"event": "view_selection", "playback_regions": regions, "region_sequences": sequences, "time_range": range})
            .to_string();
        // the callback only keeps the row; the worker writes the file (S1)
        if let Some(row) = SELECTION_ROWS.offer(row, mono_ms(), 1000) {
            session().log_deferred(row);
        }
    })
}
unsafe extern "C" fn ev_notifyHideRegionSequences(
    _editorViewRef: ARAEditorViewRef,
    regionSequenceRefsCount: ARASize,
    _regionSequenceRefs: *const ARARegionSequenceRef,
) {
    guard("notifyHideRegionSequences", (), move || {
        let row = json!({"event": "view_hide_sequences", "count": regionSequenceRefsCount}).to_string();
        if let Some(row) = HIDE_ROWS.offer(row, mono_ms(), 1000) {
            session().log_deferred(row);
        }
    })
}
static EV_INTERFACE: SyncPtr<ARAEditorViewInterface> = SyncPtr(ARAEditorViewInterface {
    structSize: std::mem::size_of::<ARAEditorViewInterface>(),
    notifySelection: Some(ev_notifySelection),
    notifyHideRegionSequences: Some(ev_notifyHideRegionSequences),
});

unsafe extern "C" fn f_count(_f: *const clap_plugin_factory) -> u32 {
    guard("plugin_factory_count", 0, move || 1)
}
unsafe extern "C" fn f_desc(_f: *const clap_plugin_factory, i: u32) -> *const clap_plugin_descriptor {
    guard("plugin_factory_descriptor", std::ptr::null(), move || {
        if i == 0 {
            &DESCRIPTOR
        } else {
            std::ptr::null()
        }
    })
}
unsafe extern "C" fn f_create(
    _f: *const clap_plugin_factory,
    _host: *const clap_host,
    id: *const c_char,
) -> *const clap_plugin {
    guard("create_plugin", std::ptr::null(), move || {
        if CStr::from_ptr(id) != CStr::from_ptr(PLUGIN_ID) {
            return std::ptr::null();
        }
        let p = Box::into_raw(Box::new(Plugin {
            clap: clap_plugin {
                desc: &DESCRIPTOR,
                plugin_data: std::ptr::null_mut(),
                init: Some(p_init),
                destroy: Some(p_destroy),
                activate: Some(p_activate),
                deactivate: Some(p_deactivate),
                start_processing: Some(p_start),
                stop_processing: Some(p_stop),
                reset: Some(p_reset),
                process: Some(p_process),
                get_extension: Some(p_get_extension),
                on_main_thread: Some(p_on_main_thread),
            },
            ext: std::mem::zeroed(),
            sample_rate: Mutex::new(0.0),
            doc: Mutex::new(None),
            editor: std::cell::RefCell::new(None),
            gui_size: std::sync::atomic::AtomicU64::new((1000u64 << 32) | 640),
        }));
        (*p).clap.plugin_data = p as *mut c_void;
        &(*p).clap
    })
}

static PLUGIN_FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(f_count),
    get_plugin_descriptor: Some(f_desc),
    create_plugin: Some(f_create),
};

/// The CLAP entry of the CLAP 1.x spec (`clap/entry.h`): the version, then `init`,
/// `deinit` and `get_factory`. No name/vendor/... strings: the spec has none - not in
/// clap-sys 0.5.0, not in the CLAP 1.1.9 headers, not on free-audio/clap main. A draft
/// with strings sat here and sent every spec-conforming host (clap-validator, the
/// wrapped VST3, the ARATestHost) into a string constant: SIGSEGV before the first log
/// line, while ara-host only worked because it read the same wrong struct.
#[repr(C)]
pub struct ClapEntry {
    pub clap_version: clap_sys::version::clap_version,
    pub init: Option<unsafe extern "C" fn(plugin_path: *const c_char) -> bool>,
    pub deinit: Option<unsafe extern "C" fn()>,
    pub get_factory: Option<unsafe extern "C" fn(factory_id: *const c_char) -> *const c_void>,
}
// the host may call the entry from any thread
unsafe impl Send for ClapEntry {}
unsafe impl Sync for ClapEntry {}

unsafe extern "C" fn e_init(plugin_path: *const c_char) -> bool {
    guard("entry init", true, move || {
        editor::set_bundle_hint(plugin_path);
        true
    })
}
unsafe extern "C" fn e_deinit() {
    guard("entry deinit", (), unload_worker)
}
unsafe extern "C" fn e_get_factory(id: *const c_char) -> *const c_void {
    guard("entry get_factory", std::ptr::null(), move || {
        let id = CStr::from_ptr(id).to_bytes();
        if id == CLAP_PLUGIN_FACTORY_ID.to_bytes() {
            return &PLUGIN_FACTORY as *const _ as *const c_void;
        }
        if id == EXT_ARA_FACTORY || id == EXT_ARA_FACTORY_DRAFT {
            return &ARA_FACTORY_EXT as *const _ as *const c_void;
        }
        std::ptr::null()
    })
}

/// The CLAP entry the loader looks for (the bundle carries it in its binary).
#[no_mangle]
#[used]
pub static clap_entry: ClapEntry = ClapEntry {
    clap_version: CLAP_VERSION,
    init: Some(e_init),
    deinit: Some(e_deinit),
    get_factory: Some(e_get_factory),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_map_with_the_regions_of_the_round_not_the_live_ones() {
        let rec = |a: f64, d: f64| service::RegionRec {
            start_mod: a,
            dur_mod: d,
            start_play: 0.0,
        };
        let round = (vec![rec(0.0, 1.0)], 48_000.0);
        let live = (vec![rec(0.0, 0.5)], 48_000.0);
        assert_eq!(regions_for_notes(Some(&round), live.clone()).0[0].dur_mod, 1.0);
        assert_eq!(regions_for_notes(None, live).0[0].dur_mod, 0.5);
        // the same list of events gives the same notes through both paths: a hit at 0.75 s
        // is a note with the round's regions and none with the live ones
        let list = vec![json!({"t": 0.75, "v": 100})];
        assert_eq!(service::notes_for(&list, &round.0, round.1, i64::MAX).len(), 1);
        assert_eq!(service::notes_for(&list, &[rec(0.0, 0.5)], 48_000.0, i64::MAX).len(), 0);
    }

    fn entry(id: &str, name: &str) -> state::ArchiveSource {
        state::ArchiveSource {
            persistent_id: id.into(),
            name: name.into(),
            sample_rate: 48_000.0,
            notes: Vec::new(),
            track: String::new(),
            stale: false,
        }
    }

    #[test]
    fn a_path_name_with_no_persistent_id_finds_its_archived_source() {
        let stored = [entry("a", "kick.wav"), entry("b", "snare.wav")];
        let found = find_archived(&stored, "", "/some/folder/snare.wav", &[]).expect("matched by name");
        assert_eq!(found.name, "snare.wav");
        assert!(find_archived(&stored, "", "/some/folder/tom.wav", &[]).is_none());
    }

    #[test]
    fn the_persistent_id_wins_over_the_name() {
        let stored = [entry("a", "kick.wav"), entry("b", "snare.wav")];
        let found = find_archived(&stored, "a", "/x/snare.wav", &[]).expect("matched by id");
        assert_eq!(found.name, "kick.wav");
    }

    #[test]
    fn the_cache_folder_default_is_the_one_the_readme_names() {
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() {
            assert_eq!(
                cache_default(),
                PathBuf::from(home).join("Library/Caches/DrumSpike/ARA")
            );
        }
    }

    #[test]
    fn two_sources_of_one_name_without_persistent_id_get_two_entries() {
        let stored = [entry("", "kick.wav"), entry("", "kick.wav")];
        let first = find_archived(&stored, "", "/a/kick.wav", &[]).expect("first");
        let at = stored.iter().position(|x| std::ptr::eq(x, first)).unwrap();
        let second = find_archived(&stored, "", "/b/kick.wav", &[at]).expect("second");
        assert!(!std::ptr::eq(first, second));
        assert!(find_archived(&stored, "", "/c/kick.wav", &[0, 1]).is_none());
    }

    #[test]
    fn a_second_document_cannot_take_an_id_a_live_document_holds() {
        let a = fresh_id();
        let b = fresh_id();
        assert_ne!(a, b);
        // b may not take a's id; a keeps it
        assert!(!claim_id(&b, &a));
        assert!(claim_id(&a, &a));
        // when a ends, its id is free for b
        release_id(&a);
        assert!(claim_id(&b, &a));
        release_id(&a);
    }

    fn bare_source(doc: &Arc<DocShared>, seq: usize) -> Arc<SourceState> {
        Arc::new(SourceState {
            seq,
            doc: doc.clone(),
            track: Mutex::new(None),
            removed: AtomicBool::new(false),
            props: Mutex::new(SourceProps::default()),
            host: SyncPtr(std::ptr::null()),
            host_ref: SyncPtr(std::ptr::null_mut()),
            reader: Mutex::new(SyncPtr(std::ptr::null_mut())),
            worker: Mutex::new(None),
            cancel: AtomicBool::new(false),
            wav: Mutex::new(None),
            failed: AtomicBool::new(false),
            regions: Mutex::new(Vec::new()),
            notes: Mutex::new(Vec::new()),
            has_events: AtomicBool::new(false),
            requested: AtomicBool::new(true),
            asked: AtomicBool::new(false),
            analysis_failed: AtomicBool::new(false),
            wav_len: AtomicU64::new(0),
            audio_gen: AtomicU64::new(0),
            reanalyse: AtomicBool::new(false),
            access: AtomicBool::new(false),
            old_wavs: Mutex::new(Vec::new()),
            read_retry_at: Mutex::new(None),
            read_tries: std::sync::atomic::AtomicU32::new(0),
            stale: AtomicBool::new(false),
            requested_at: Mutex::new(None),
        })
    }

    #[test]
    fn a_source_past_its_deadline_does_not_make_the_worker_spin() {
        let doc = DocShared::new();
        let late = bare_source(&doc, 0);
        let reading = bare_source(&doc, 1);
        let now = std::time::Instant::now();
        *late.requested_at.lock().unwrap() = now.checked_sub(std::time::Duration::from_secs(40));
        *reading.requested_at.lock().unwrap() = Some(now);
        assert_eq!(
            deadline_left(&late),
            Some(std::time::Duration::ZERO),
            "past its deadline"
        );
        let wait = wait_left(&[late.clone(), reading.clone()]).expect("the other source still waits");
        assert!(
            !wait.is_zero() && wait <= std::time::Duration::from_secs(30),
            "{wait:?}"
        );
        // the document is not ready while the other source reads; with only the late one it is
        let st = WorkerState {
            requested: vec![late.clone(), reading],
            ..WorkerState::default()
        };
        assert!(ready_documents(&st).is_empty());
        assert_eq!(wait_left(std::slice::from_ref(&late)), None, "nothing to wait for");
        let st = WorkerState {
            requested: vec![late],
            ..WorkerState::default()
        };
        assert_eq!(ready_documents(&st).len(), 1);
        release_id(&doc.id());
    }

    #[test]
    fn a_second_init_gets_a_worker_when_the_first_has_ended() {
        let s = session();
        start_worker(s);
        assert!(WORKER_RUNNING.load(Ordering::SeqCst));
        assert!(send(s, Msg::Quit));
        let t0 = std::time::Instant::now();
        while WORKER_RUNNING.load(Ordering::SeqCst) && t0.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!WORKER_RUNNING.load(Ordering::SeqCst), "the worker ended on Quit");
        // the sender is still there with a dead receiver: the new init must not trust it
        start_worker(s);
        assert!(WORKER_RUNNING.load(Ordering::SeqCst));
        assert!(send(s, Msg::Quit));
    }

    /// Held by every test that calls `p_process` (it sets process-global flags).
    static EVIDENCE_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn the_selection_rows_are_rate_limited() {
        let last = AtomicU64::new(UNSET);
        assert!(rate_gate(&last, 0, 1000), "the first call writes a row");
        assert!(!rate_gate(&last, 10, 1000));
        assert!(!rate_gate(&last, 999, 1000));
        assert!(rate_gate(&last, 1000, 1000), "one row a second");
        assert!(!rate_gate(&last, 1500, 1000));
        assert!(rate_gate(&last, 2600, 1000));
    }

    #[test]
    fn the_newest_row_of_a_burst_is_not_lost() {
        let rows = RateRows::new();
        assert_eq!(rows.offer("a".into(), 0, 1000).as_deref(), Some("a"));
        assert_eq!(rows.offer("b".into(), 100, 1000), None);
        assert_eq!(rows.offer("c".into(), 200, 1000), None, "held back");
        assert_eq!(rows.flush(500, 1000), None, "the second is not over");
        assert_eq!(
            rows.flush(1000, 1000).as_deref(),
            Some("c"),
            "the newest one is written when it is"
        );
        assert_eq!(rows.flush(3000, 1000), None, "nothing waits any more");
        assert_eq!(rows.offer("d".into(), 3100, 1000).as_deref(), Some("d"));
    }

    #[test]
    fn without_a_report_file_the_rows_are_a_small_ring() {
        let s = session();
        if s.out.is_some() {
            return; // a run with a report file keeps every row on purpose
        }
        for i in 0..(RING_ROWS * 2) {
            s.log(format!("{{\"event\":\"ring_test\",\"i\":{i}}}"));
        }
        assert!(s.rows.lock().unwrap().len() <= RING_ROWS);
    }

    #[test]
    fn a_round_for_a_cancelled_read_is_not_forced() {
        let doc = DocShared::new();
        let cancelled = bare_source(&doc, 0);
        cancelled.failed.store(true, Ordering::SeqCst);
        assert!(
            !round_forced(std::slice::from_ref(&cancelled)),
            "no new audio: no forced round"
        );
        assert!(!round_forced(&[]));
        *cancelled.wav.lock().unwrap() = Some(PathBuf::from("x.wav"));
        assert!(round_forced(&[cancelled]), "a source with audio forces the round");
    }

    #[test]
    fn a_source_queued_by_its_read_and_then_restored_leaves_the_queue() {
        let doc = DocShared::new();
        let src = bare_source(&doc, 0);
        let mut st = WorkerState::default();
        assert!(!session().absorb(Msg::Reading(src.clone()), &mut st));
        assert_eq!(st.requested.len(), 1, "the read queued it");
        // the restore gives it its result (what restore_objects does)
        src.has_events.store(true, Ordering::SeqCst);
        drop_done(&mut st, &doc);
        assert!(
            st.requested.is_empty(),
            "a restored source with a result is not analysed"
        );
        // its read ends later: still not queued
        assert!(!session().absorb(Msg::Ready(src.clone()), &mut st));
        assert!(st.requested.is_empty());
    }

    #[test]
    fn a_restored_stale_source_is_analysed_again_when_its_audio_is_read() {
        let doc = DocShared::new();
        let src = bare_source(&doc, 0);
        src.requested.store(false, Ordering::SeqCst);
        // what a restore of a stale track sets
        src.has_events.store(true, Ordering::SeqCst);
        src.stale.store(true, Ordering::SeqCst);
        src.analysis_failed.store(true, Ordering::SeqCst);
        let mut st = WorkerState::default();
        assert!(!session().absorb(Msg::Ready(src.clone()), &mut st));
        assert_eq!(st.requested.len(), 1, "no request needed");
        assert!(needs_analysis(&src));
    }

    #[test]
    fn new_samples_make_a_source_with_a_result_need_a_round_until_it_is_done() {
        let doc = DocShared::new();
        let src = bare_source(&doc, 0);
        src.has_events.store(true, Ordering::SeqCst);
        assert!(!needs_analysis(&src));
        src.reanalyse.store(true, Ordering::SeqCst);
        assert!(needs_analysis(&src), "the content update marked it");
        let mut st = WorkerState::default();
        assert!(!session().absorb(Msg::Reading(src.clone()), &mut st));
        assert_eq!(st.requested.len(), 1, "the read of the new audio queues it");
    }

    #[test]
    fn an_old_retry_time_never_touches_a_healthy_read() {
        let doc = DocShared::new();
        let src = bare_source(&doc, 0);
        let past = std::time::Instant::now().checked_sub(std::time::Duration::from_secs(1));
        src.access.store(true, Ordering::SeqCst);
        // no time planned: nothing is due
        assert!(due_read_retries(std::slice::from_ref(&src)).is_empty());
        // a time that is due, a reader that still runs (the new read): not touched, the time stays
        *src.read_retry_at.lock().unwrap() = past;
        let running = std::thread::spawn(|| std::thread::sleep(std::time::Duration::from_millis(300)));
        *src.worker.lock().unwrap() = Some(running);
        assert!(due_read_retries(std::slice::from_ref(&src)).is_empty());
        assert!(
            src.read_retry_at.lock().unwrap().is_some(),
            "the time is kept for the next call"
        );
        // the reader has ended: the retry is due and taken
        let ended = src.worker.lock().unwrap().take().unwrap();
        ended.join().unwrap();
        let done = std::thread::spawn(|| {});
        *src.worker.lock().unwrap() = Some(done);
        while !src.worker.lock().unwrap().as_ref().unwrap().is_finished() {
            std::thread::yield_now();
        }
        assert_eq!(due_read_retries(std::slice::from_ref(&src)).len(), 1);
        assert!(src.read_retry_at.lock().unwrap().is_none(), "taken");
        // the host turned the access off: the time is void
        *src.read_retry_at.lock().unwrap() = past;
        src.access.store(false, Ordering::SeqCst);
        assert!(due_read_retries(std::slice::from_ref(&src)).is_empty());
        assert!(src.read_retry_at.lock().unwrap().is_none());
    }

    #[test]
    fn a_round_for_another_reason_leaves_a_source_that_waits_for_new_audio_alone() {
        let doc = DocShared::new();
        let a = bare_source(&doc, 0);
        a.has_events.store(true, Ordering::SeqCst);
        assert!(!waits_for_new_audio(&a), "no update: not waiting");
        a.reanalyse.store(true, Ordering::SeqCst);
        assert!(!waits_for_new_audio(&a), "no read, no retry: the deadline rule applies");
        // state 2: the read failed, a retry is planned
        *a.read_retry_at.lock().unwrap() = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
        assert!(waits_for_new_audio(&a));
        // state 1: a read runs
        *a.read_retry_at.lock().unwrap() = None;
        let running = std::thread::spawn(|| std::thread::sleep(std::time::Duration::from_millis(200)));
        *a.worker.lock().unwrap() = Some(running);
        assert!(waits_for_new_audio(&a));
        a.worker.lock().unwrap().take().unwrap().join().unwrap();
        assert!(!waits_for_new_audio(&a));
    }

    #[test]
    fn access_off_in_state_two_leaves_the_source_failed_not_incomplete_for_ever() {
        let doc = DocShared::new();
        let a = bare_source(&doc, 0);
        a.has_events.store(true, Ordering::SeqCst);
        a.reanalyse.store(true, Ordering::SeqCst);
        // state 2: the read failed once and a retry was planned; the host now turns the access off
        // (the retry time is cleared by the handler first)
        assert!(leave_for_access_off(&a), "state 2 goes to state 4");
        assert!(
            a.analysis_failed.load(Ordering::SeqCst),
            "complete for the host: the wait ends"
        );
        assert_eq!(doc.pending.lock().unwrap().len(), 1, "the host is told");
        assert!(!leave_for_access_off(&a), "once only");
        // a source with its audio, or one that is not waiting, is left alone
        let b = bare_source(&doc, 1);
        assert!(!leave_for_access_off(&b));
        let c = bare_source(&doc, 2);
        c.reanalyse.store(true, Ordering::SeqCst);
        *c.wav.lock().unwrap() = Some(PathBuf::from("x.wav"));
        assert!(!leave_for_access_off(&c));
    }

    #[test]
    fn a_snapshot_is_never_torn_by_a_content_update() {
        // the update takes the WAV and raises the generation under the lock the snapshot reads under:
        // a snapshot that is ready holds the path of ITS generation, never (old path, new generation)
        let doc = DocShared::new();
        let src = bare_source(&doc, 0);
        let path_of = |g: u64| doc.wav_path(0, "", g);
        *src.wav.lock().unwrap() = Some(path_of(0));
        let stop = Arc::new(AtomicBool::new(false));
        let writer = {
            let (src, stop) = (src.clone(), stop.clone());
            let doc = doc.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    {
                        let mut w = src.wav.lock().unwrap();
                        src.audio_gen.fetch_add(1, Ordering::SeqCst);
                        *w = None; // what the update does
                    }
                    let mut w = src.wav.lock().unwrap();
                    let g = src.audio_gen.load(Ordering::SeqCst);
                    *w = Some(doc.wav_path(0, "", g)); // what the commit of the new read does
                }
            })
        };
        for _ in 0..20_000 {
            let snap = src.snapshot();
            assert_eq!(
                snap.wav,
                path_of(snap.audio_gen),
                "path and generation belong together (ready: {})",
                snap.ready
            );
        }
        stop.store(true, Ordering::SeqCst);
        writer.join().unwrap();
    }

    #[test]
    fn a_failed_round_is_retried_three_times_then_the_plugin_gives_up() {
        let doc = DocShared::new();
        let src = bare_source(&doc, 0);
        *src.wav.lock().unwrap() = Some(PathBuf::from("x.wav"));
        src.analysis_failed.store(true, Ordering::SeqCst);
        doc.members.lock().unwrap().push(Arc::downgrade(&src));
        let s = session();
        let mut st = WorkerState::default();
        s.plan_retry(&doc, &mut st);
        assert_eq!(
            (st.retry.len(), st.retry[0].tries),
            (1, 1),
            "the first retry is planned"
        );
        for want in [2, 3] {
            st.retry[0].due = None; // the retry ran and failed again
            s.plan_retry(&doc, &mut st);
            assert_eq!(st.retry[0].tries, want);
            assert!(st.retry[0].due.is_some());
        }
        st.retry[0].due = None;
        s.plan_retry(&doc, &mut st);
        assert!(
            st.retry[0].gave_up && st.retry[0].due.is_none(),
            "after three tries: stop"
        );
        // a retry that works ends the count
        src.analysis_failed.store(false, Ordering::SeqCst);
        st.retry[0].gave_up = false;
        s.plan_retry(&doc, &mut st);
        assert!(st.retry.is_empty());
        // anything that changes for the document starts the count again
        src.analysis_failed.store(true, Ordering::SeqCst);
        s.plan_retry(&doc, &mut st);
        assert_eq!(st.retry.len(), 1);
        assert!(!s.absorb(Msg::Touch(doc.clone()), &mut st));
        assert!(st.retry.is_empty(), "a change of the document resets the retry count");
    }

    #[test]
    fn process_writes_no_row_on_the_audio_thread() {
        // the evidence flags are process-global: one test at a time may call `process`
        let _one = EVIDENCE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _slot = transport::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for flag in [&PROCESS_SEEN, &TRANSPORT_SEEN] {
            flag.store(0, Ordering::SeqCst);
        }
        let s = session();
        let rows_with = |ev: &str| -> usize {
            s.rows
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.contains(&format!("\"event\":\"{ev}\"")))
                .count()
        };
        let before = (rows_with("process_first"), rows_with("transport_seen"));
        let mut transport: clap_sys::events::clap_event_transport = unsafe { std::mem::zeroed() };
        transport.flags =
            clap_sys::events::CLAP_TRANSPORT_HAS_SECONDS_TIMELINE | clap_sys::events::CLAP_TRANSPORT_IS_PLAYING;
        let mut process: clap_process = unsafe { std::mem::zeroed() };
        process.frames_count = 64;
        process.transport = &transport;
        let audio_thread = format!("{:?}", std::thread::current().id());
        unsafe { p_process(std::ptr::null(), &process) };
        // the audio thread wrote no row (the worker of another test may have written it already,
        // so look at the thread that wrote it, not at the time)
        let writer = std::thread::spawn(move || {
            session().poll_evidence();
            format!("{:?}", std::thread::current().id())
        })
        .join()
        .unwrap();
        assert_ne!(writer, audio_thread);
        assert_eq!(
            (rows_with("process_first"), rows_with("transport_seen")),
            (before.0 + 1, before.1 + 1)
        );
        // a second call writes nothing more
        unsafe { p_process(std::ptr::null(), &process) };
        s.poll_evidence();
        assert_eq!(rows_with("process_first"), before.0 + 1);
    }

    #[test]
    fn the_state_saves_a_small_blob_and_loads_anything() {
        assert_eq!(state_blob().len(), 10);
        assert!(state_accepts(&state_blob()));
        assert!(state_accepts(&[]), "an empty stream");
        let mut newer = STATE_MAGIC.to_vec();
        newer.extend_from_slice(&99u32.to_le_bytes());
        assert!(state_accepts(&newer), "an unknown newer version");
        assert!(state_accepts(b"garbage"));
    }

    #[test]
    fn a_clean_name_still_matches() {
        let stored = [entry("", "kick.wav")];
        assert!(find_archived(&stored, "", "kick.wav", &[]).is_some());
    }
}
