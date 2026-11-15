//! Drum Spike ARA plugin (M1a, #525): a CLAP plugin in Rust that speaks ARA 2 through the
//! plain C interface and analyses the host's tracks with the Drum Spike service
//! (`drumspike-app`), wrapped as a self-contained VST3 by `clap-wrapper`.
//!
//! One chain: host -> CLAP -> ARA document controller in Rust -> the samples of every
//! audio source written into the cache folder -> the app service on the project built
//! from the sources and the playback regions -> the hits back to the host as ARA notes.
//!
//! Environment (the test switches of the check; a real host sets none of them):
//!
//! - `DS_ARA_CACHE`: the cache folder (default `~/Library/Caches/DrumSpike/ARA`).
//! - `DS_ARA_OUT`: where the plugin writes its report (one JSON object per line).
//! - `DS_ARA_FAULT=in-process`: the fault switch — the analysis runs inside `process`
//!   (the audio thread) on purpose, so the overrun check of the host can fail.
//!
//! No analysis runs on the audio thread and none on the host's main thread: the audio
//! readers are created and destroyed on the main thread (the ARA rule), the reads and
//! the service job run on their own threads, and `process` only passes audio through —
//! except under the fault switch, which is the deliberate exception that proves the
//! overrun check of the host can fail. Every CLAP entry point runs inside a panic guard.
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
pub mod service;
pub mod state;
pub mod wav;

use ara::*;
use clap_sys::ext::audio_ports::*;
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};
use clap_sys::version::CLAP_VERSION;
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

pub(crate) struct SyncPtr<T>(pub T);
unsafe impl<T> Sync for SyncPtr<T> {}
unsafe impl<T> Send for SyncPtr<T> {}

macro_rules! cs {
    ($s:literal) => {
        concat!($s, "\0").as_ptr() as *const c_char
    };
}

/// The boundary guard: every entry point runs its body inside `catch_unwind`. A panic
/// that reached the C ABI would abort the host; caught here, the entry point answers its
/// default. The default is explicit (`Result` and the CLAP status carry no `Default`).
fn guard<T>(where_: &'static str, default: T, body: impl FnOnce() -> T + std::panic::UnwindSafe) -> T {
    match std::panic::catch_unwind(body) {
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
    /// Creation index (stable key in the results).
    seq: usize,
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
    /// The playback regions of the source (the segments of the project).
    regions: Mutex<Vec<service::RegionRec>>,
    /// The notes of the source: `(sample, volume)` per event.
    notes: Mutex<Vec<[f64; 2]>>,
    /// A job covered this source (its content is available).
    has_events: AtomicBool,
    /// The host asked for the analysis of this source.
    requested: AtomicBool,
}

impl SourceState {
    fn props(&self) -> SourceProps {
        self.props.lock().map(|p| p.clone()).unwrap_or_default()
    }

    /// The snapshot the service job works on.
    fn snapshot(&self) -> service::SourceSnapshot {
        let p = self.props();
        service::SourceSnapshot {
            seq: self.seq,
            name: p.name,
            persistent_id: p.persistent_id,
            sample_rate: p.sample_rate,
            sample_count: p.sample_count,
            wav: self
                .wav
                .lock()
                .map(|w| w.clone().unwrap_or_default())
                .unwrap_or_default(),
            regions: self.regions.lock().map(|r| r.clone()).unwrap_or_default(),
        }
    }
}

/// One audio modification: the creation index of its source (the ARA graph links regions
/// to sources through their modifications).
struct Modification {
    seq: usize,
}

/// One content reader handed to the host.
struct Reader {
    notes: Vec<ARAContentNote>,
}

/// One message to the analysis worker.
enum Msg {
    /// A source's analysis was requested.
    Request(Arc<SourceState>),
    /// A source's WAV is written.
    Ready(Arc<SourceState>),
    /// A source's read failed.
    Failed(Arc<SourceState>),
}

/// The session: the state of the process that the document controllers share (the cache
/// folder, the report, the analysis worker). M1a runs one analysis per document
/// controller; a second controller (the restore run of the archive check) starts its own
/// requests and reuses the worker.
struct Session {
    /// The cache folder (everything the plugin writes lives below it).
    cache: PathBuf,
    /// Where the report goes (`DS_ARA_OUT`).
    out: Option<PathBuf>,
    /// The rows of the report (read and notes rows, in order).
    rows: Mutex<Vec<String>>,
    /// The last service row (rewritten by every job and by a restore; the last line).
    service_row: Mutex<Option<String>>,
    /// To the worker.
    tx: Mutex<Option<Sender<Msg>>>,
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
        rows: Mutex::new(Vec::new()),
        service_row: Mutex::new(None),
        tx: Mutex::new(None),
        fault_ran: AtomicBool::new(false),
        fault: std::env::var("DS_ARA_FAULT").as_deref() == Ok("in-process"),
    })
}

impl Session {
    fn log(&self, line: String) {
        if let Ok(mut rows) = self.rows.lock() {
            rows.push(line);
        }
    }

    /// Rewrite the report: the rows, then the last service row. A reader never sees half
    /// a file (write to a temporary, rename).
    fn write_out(&self) {
        let Some(out) = &self.out else { return };
        let mut text = self.rows.lock().map(|r| r.join("\n")).unwrap_or_default();
        if let Some(row) = self.service_row.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(row);
        }
        if !text.is_empty() {
            text.push('\n');
        }
        let mut tmp = out.clone().into_os_string();
        tmp.push(".tmp");
        if std::fs::write(&tmp, text.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, out);
        }
    }

    /// The worker loop: run the service job over the requested sources whenever a new
    /// request or a finished WAV arrives. A job covers every requested source that has
    /// no events yet and waits for its WAV first (the ARA test host enables the samples
    /// access of one source after the other and blocks on each analysis request, so a
    /// job that covered an earlier subset is superseded by the next one).
    fn worker_loop(&'static self, rx: Receiver<Msg>) {
        let mut requested: Vec<Arc<SourceState>> = Vec::new();
        let mut run_pending = false;
        let absorb = |msg: Msg, requested: &mut Vec<Arc<SourceState>>, run_pending: &mut bool| match msg {
            Msg::Request(s) => {
                if !requested.iter().any(|r| r.seq == s.seq) {
                    requested.push(s);
                }
                *run_pending = true;
            }
            Msg::Ready(_) | Msg::Failed(_) => *run_pending = true,
        };
        loop {
            match rx.recv() {
                Ok(msg) => absorb(msg, &mut requested, &mut run_pending),
                Err(_) => return,
            }
            while run_pending {
                run_pending = false;
                // wait until every requested source has its WAV (or its read failed)
                loop {
                    let missing = requested
                        .iter()
                        .any(|s| s.wav.lock().map(|w| w.is_none()).unwrap_or(true) && !s.failed.load(Ordering::SeqCst));
                    if !missing {
                        break;
                    }
                    match rx.recv() {
                        Ok(msg) => absorb(msg, &mut requested, &mut run_pending),
                        Err(_) => return,
                    }
                }
                let targets: Vec<Arc<SourceState>> = requested
                    .iter()
                    .filter(|s| !s.has_events.load(Ordering::SeqCst))
                    .cloned()
                    .collect();
                if targets.is_empty() {
                    break;
                }
                self.run_job(&targets);
                // a request that arrived while the job ran needs another round
                for s in &requested {
                    if s.requested.load(Ordering::SeqCst) && !s.has_events.load(Ordering::SeqCst) {
                        run_pending = true;
                    }
                }
            }
        }
    }

    /// One service job over `targets`, then the report row.
    fn run_job(&'static self, targets: &[Arc<SourceState>]) {
        let snaps: Vec<service::SourceSnapshot> = targets.iter().map(|s| s.snapshot()).collect();
        let cache = self.cache.clone();
        let result = guard("service job", Err("the service job panicked".to_string()), move || {
            service::run(&snaps, &cache)
        });
        let Ok(result) = result else { return };
        for (seq, notes) in &result.notes {
            if let Some(s) = targets.iter().find(|s| s.seq == *seq) {
                if let Ok(mut n) = s.notes.lock() {
                    *n = notes.clone();
                }
                s.has_events.store(true, Ordering::SeqCst);
            }
        }
        let counts: serde_json::Map<String, Value> = result
            .decode_counts
            .iter()
            .map(|(p, n)| (p.display().to_string(), json!(n)))
            .collect();
        let tracks = result.track_of.values().cloned().collect::<Vec<_>>().join(", ");
        let row = format!(
            "{{\"event\":\"service\",\"analysis_ms\":{},\"tracks\":\"{}\",\"decode_counts\":{},\"events\":{}}}",
            result.analysis_ms,
            tracks,
            serde_json::to_string(&counts).unwrap_or_else(|_| "{}".into()),
            service::events_line(&result.events)
        );
        if let Ok(mut sr) = self.service_row.lock() {
            *sr = Some(row);
        }
        self.write_out();
    }

    /// The fault path: the analysis runs inside `process` (the audio thread), once. The
    /// reader threads write the WAVs; the audio thread waits for them (the point of the
    /// proof: the block overruns its budget).
    fn run_fault_job(&'static self) {
        let targets: Vec<Arc<SourceState>> = self
            .sources_of_current()
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
        self.run_job(&targets);
    }

    /// The sources of the newest document controller (the fault job runs before the
    /// process of the host; the current document is the last one created).
    fn sources_of_current(&self) -> Vec<Arc<SourceState>> {
        CURRENT
            .0
            .lock()
            .ok()
            .and_then(|g| *g)
            .and_then(|c| unsafe { (*c).sources.lock().ok().map(|l| l.clone()) })
            .unwrap_or_default()
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

/// Start the worker thread once.
fn start_worker(s: &'static Session) {
    let mut tx = s.tx.lock().unwrap_or_else(|e| e.into_inner());
    if tx.is_none() {
        let (tx_, rx) = channel::<Msg>();
        let spawned = std::thread::Builder::new()
            .name("drumspike-ara-analysis".into())
            .spawn(move || s.worker_loop(rx));
        if spawned.is_ok() {
            *tx = Some(tx_);
        }
    }
}

fn send(s: &'static Session, msg: Msg) {
    if let Ok(tx) = s.tx.lock() {
        if let Some(tx) = tx.as_ref() {
            let _ = tx.send(msg);
        }
    }
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
    version: cs!("0.1.0"),
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
    // the ARA interface structs are packed: copy the field out, never reference it
    let api_generation = (*config).desiredApiGeneration;
    session().log(format!(
        "{{\"event\":\"ara_init\",\"api_generation\":{api_generation}}}"
    ));
}
unsafe extern "C" fn ara_uninit() {}

// ---------------------------------------------------------------- document controller

struct Controller {
    /// The host instance the readers read through.
    host: *const ARADocumentControllerHostInstance,
    /// The ARA instance struct the host holds.
    instance: ARADocumentControllerInstance,
    /// The sources of this document, in creation order.
    sources: Mutex<Vec<Arc<SourceState>>>,
}

unsafe extern "C" fn create_document_controller(
    host: *const ARADocumentControllerHostInstance,
    _properties: *const ARADocumentProperties,
) -> *const ARADocumentControllerInstance {
    let s = session();
    start_worker(s);
    let c = Box::into_raw(Box::new(Controller {
        host,
        instance: std::mem::zeroed(),
        sources: Mutex::new(Vec::new()),
    }));
    (*c).instance = ARADocumentControllerInstance {
        structSize: std::mem::size_of::<ARADocumentControllerInstance>(),
        documentControllerRef: c as ARADocumentControllerRef,
        documentControllerInterface: &DC_INTERFACE.0,
    };
    current_store(c);
    s.log("{\"event\":\"create_document_controller\"}".into());
    &(*c).instance
}

unsafe fn controller_of(controllerRef: ARADocumentControllerRef) -> &'static Controller {
    &*(controllerRef as *const Controller)
}

unsafe extern "C" fn dc_destroyDocumentController(controllerRef: ARADocumentControllerRef) {
    // the sources die with the controller box; the worker lives for the process
    drop(Box::from_raw(controllerRef as *mut Controller));
}

unsafe extern "C" fn dc_getFactory(_c: ARADocumentControllerRef) -> *const ARAFactory {
    &FACTORY.0
}

unsafe extern "C" fn dc_isLicensedForCapabilities(
    _c: ARADocumentControllerRef,
    _dialog: ARABool,
    _n: ARASize,
    _t: *const ARAContentType,
    _f: ARAPlaybackTransformationFlags,
) -> ARABool {
    1
}

// The no-op stubs for the members M1a does not implement: the ARA test host of the SDK
// examples calls them without a check (the same stub set the S1 spike passes with, see
// plugins/ara-spike/src/stubs.rs).
unsafe extern "C" fn dc_beginEditing(_c: ARADocumentControllerRef) {}
unsafe extern "C" fn dc_endEditing(_c: ARADocumentControllerRef) {}
unsafe extern "C" fn dc_notifyModelUpdates(_c: ARADocumentControllerRef) {}
unsafe extern "C" fn dc_beginRestoringDocumentFromArchive(
    _c: ARADocumentControllerRef,
    _r: ARAArchiveReaderHostRef,
) -> ARABool {
    0
}
unsafe extern "C" fn dc_endRestoringDocumentFromArchive(
    _c: ARADocumentControllerRef,
    _r: ARAArchiveReaderHostRef,
) -> ARABool {
    0
}
unsafe extern "C" fn dc_storeDocumentToArchive(_c: ARADocumentControllerRef, _w: ARAArchiveWriterHostRef) -> ARABool {
    0
}
unsafe extern "C" fn dc_updateDocumentProperties(_c: ARADocumentControllerRef, _p: *const ARADocumentProperties) {}
unsafe extern "C" fn dc_createMusicalContext(
    _c: ARADocumentControllerRef,
    _h: ARAMusicalContextHostRef,
    _p: *const ARAMusicalContextProperties,
) -> ARAMusicalContextRef {
    Box::into_raw(Box::new(0u64)) as ARAMusicalContextRef
}
unsafe extern "C" fn dc_updateMusicalContextProperties(
    _c: ARADocumentControllerRef,
    _m: ARAMusicalContextRef,
    _p: *const ARAMusicalContextProperties,
) {
}
unsafe extern "C" fn dc_updateMusicalContextContent(
    _c: ARADocumentControllerRef,
    _m: ARAMusicalContextRef,
    _r: *const ARAContentTimeRange,
    _f: ARAContentUpdateFlags,
) {
}
unsafe extern "C" fn dc_destroyMusicalContext(_c: ARADocumentControllerRef, _m: ARAMusicalContextRef) {}
unsafe extern "C" fn dc_updateAudioSourceContent(
    _c: ARADocumentControllerRef,
    _s: ARAAudioSourceRef,
    _r: *const ARAContentTimeRange,
    _f: ARAContentUpdateFlags,
) {
}
unsafe extern "C" fn dc_deactivateAudioSourceForUndoHistory(
    _c: ARADocumentControllerRef,
    _s: ARAAudioSourceRef,
    _d: ARABool,
) {
}
unsafe extern "C" fn dc_updateAudioModificationProperties(
    _c: ARADocumentControllerRef,
    _m: ARAAudioModificationRef,
    _p: *const ARAAudioModificationProperties,
) {
}
unsafe extern "C" fn dc_deactivateAudioModificationForUndoHistory(
    _c: ARADocumentControllerRef,
    _m: ARAAudioModificationRef,
    _d: ARABool,
) {
}
unsafe extern "C" fn dc_isAudioModificationContentAvailable(
    _c: ARADocumentControllerRef,
    _m: ARAAudioModificationRef,
    _t: ARAContentType,
) -> ARABool {
    0
}
unsafe extern "C" fn dc_getAudioModificationContentGrade(
    _c: ARADocumentControllerRef,
    _m: ARAAudioModificationRef,
    _t: ARAContentType,
) -> ARAContentGrade {
    0
}
unsafe extern "C" fn dc_createAudioModificationContentReader(
    _c: ARADocumentControllerRef,
    _m: ARAAudioModificationRef,
    _t: ARAContentType,
    _r: *const ARAContentTimeRange,
) -> ARAContentReaderRef {
    Box::into_raw(Box::new(Reader { notes: Vec::new() })) as ARAContentReaderRef
}
unsafe extern "C" fn dc_isPlaybackRegionContentAvailable(
    _c: ARADocumentControllerRef,
    _r: ARAPlaybackRegionRef,
    _t: ARAContentType,
) -> ARABool {
    0
}
unsafe extern "C" fn dc_getPlaybackRegionContentGrade(
    _c: ARADocumentControllerRef,
    _r: ARAPlaybackRegionRef,
    _t: ARAContentType,
) -> ARAContentGrade {
    0
}
unsafe extern "C" fn dc_createPlaybackRegionContentReader(
    _c: ARADocumentControllerRef,
    _r: ARAPlaybackRegionRef,
    _t: ARAContentType,
    _range: *const ARAContentTimeRange,
) -> ARAContentReaderRef {
    Box::into_raw(Box::new(Reader { notes: Vec::new() })) as ARAContentReaderRef
}
unsafe extern "C" fn dc_createRegionSequence(
    _c: ARADocumentControllerRef,
    _h: ARARegionSequenceHostRef,
    _p: *const ARARegionSequenceProperties,
) -> ARARegionSequenceRef {
    Box::into_raw(Box::new(0u64)) as ARARegionSequenceRef
}
unsafe extern "C" fn dc_updateRegionSequenceProperties(
    _c: ARADocumentControllerRef,
    _r: ARARegionSequenceRef,
    _p: *const ARARegionSequenceProperties,
) {
}
unsafe extern "C" fn dc_destroyRegionSequence(_c: ARADocumentControllerRef, _r: ARARegionSequenceRef) {}
unsafe extern "C" fn dc_getPlaybackRegionHeadAndTailTime(
    _c: ARADocumentControllerRef,
    _r: ARAPlaybackRegionRef,
    _head: *mut ARATimeDuration,
    _tail: *mut ARATimeDuration,
) {
}
unsafe extern "C" fn dc_getProcessingAlgorithmsCount(_c: ARADocumentControllerRef) -> ARAInt32 {
    0
}
unsafe extern "C" fn dc_getProcessingAlgorithmProperties(
    _c: ARADocumentControllerRef,
    _i: ARAInt32,
) -> *const ARAProcessingAlgorithmProperties {
    std::ptr::null()
}
unsafe extern "C" fn dc_getProcessingAlgorithmForAudioSource(
    _c: ARADocumentControllerRef,
    _s: ARAAudioSourceRef,
) -> ARAInt32 {
    0
}
unsafe extern "C" fn dc_requestProcessingAlgorithmForAudioSource(
    _c: ARADocumentControllerRef,
    _s: ARAAudioSourceRef,
    _i: ARAInt32,
) {
}
unsafe extern "C" fn dc_storeAudioSourceToAudioFileChunk(
    _c: ARADocumentControllerRef,
    _w: ARAArchiveWriterHostRef,
    _s: ARAAudioSourceRef,
    _id: *mut ARAPersistentID,
    _open: *mut ARABool,
) -> ARABool {
    0
}
unsafe extern "C" fn dc_isAudioModificationPreservingAudioSourceSignal(
    _c: ARADocumentControllerRef,
    _m: ARAAudioModificationRef,
) -> ARABool {
    0
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
    let ctl = controller_of(c);
    let src = {
        let mut list = ctl.sources.lock().unwrap_or_else(|e| e.into_inner());
        let seq = list.len();
        let src = Arc::new(SourceState {
            seq,
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
        });
        list.push(src.clone());
        src
    };
    Box::into_raw(Box::new(SyncPtr(src))) as ARAAudioSourceRef
}

unsafe extern "C" fn dc_updateAudioSourceProperties(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    properties: *const ARAAudioSourceProperties,
) {
    let Some(src) = state_of(audioSourceRef) else { return };
    if let Ok(mut p) = src.props.lock() {
        p.name = text((*properties).name);
        p.persistent_id = text((*properties).persistentID);
        p.sample_count = (*properties).sampleCount;
        p.sample_rate = (*properties).sampleRate;
        p.channels = (*properties).channelCount;
    };
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
    if !reader.is_null() && !src.host.0.is_null() {
        let iface = &*(*src.host.0).audioAccessControllerInterface;
        (iface.destroyAudioReader.unwrap())((*src.host.0).audioAccessControllerHostRef, reader);
        *src.reader.lock().unwrap_or_else(|e| e.into_inner()) = SyncPtr(std::ptr::null_mut());
    }
}

unsafe extern "C" fn dc_destroyAudioSource(_c: ARADocumentControllerRef, audioSourceRef: ARAAudioSourceRef) {
    let ptr = audioSourceRef as *mut SyncPtr<Arc<SourceState>>;
    let src = Box::from_raw(ptr);
    stop_worker(&src.0);
}

unsafe extern "C" fn dc_enableAudioSourceSamplesAccess(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    enable: ARABool,
) {
    let Some(src) = state_of(audioSourceRef) else { return };
    if enable != 0 {
        // the reader is created on the host's main thread; the reads run on the worker
        if src.host.0.is_null() {
            return;
        }
        let iface = &*(*src.host.0).audioAccessControllerInterface;
        let reader =
            (iface.createAudioReaderForSource.unwrap())((*src.host.0).audioAccessControllerHostRef, src.host_ref.0, 0);
        *src.reader.lock().unwrap_or_else(|e| e.into_inner()) = SyncPtr(reader);
        let s = session();
        let worker_src = src.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("drumspike-ara-read-{}", src.seq))
            .spawn(move || read_source(worker_src, s));
        if let Ok(mut w) = src.worker.lock() {
            *w = spawned.ok();
        }
    } else {
        // no read may run after this call returns
        stop_worker(&src);
    }
}

/// The reader thread: read every sample through the host's audio access controller,
/// write the source's WAV into the cache folder block by block, hash the samples on the
/// way, and tell the worker.
fn read_source(src: Arc<SourceState>, s: &'static Session) {
    let started = std::time::Instant::now();
    let props = src.props();
    let (count, rate, ch) = (props.sample_count, props.sample_rate, props.channels.max(1) as usize);
    let dest = s
        .cache
        .join("audio")
        .join(format!("{:03}-{}.wav", src.seq, service::stem_of(&props.name)));
    let hash = std::cell::Cell::new(0xcbf29ce484222325u64);
    let ok = (|| -> bool {
        let Ok(mut writer) = wav::WavWriter::create(&dest, rate as u32, ch as u16, count.max(0) as usize) else {
            return false;
        };
        if src.host.0.is_null() {
            return false;
        }
        let iface = unsafe { &*(*src.host.0).audioAccessControllerInterface };
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
            let read = unsafe { (iface.readAudioSamples.unwrap())(ctl, reader, pos, n as i64, ptrs.as_ptr()) };
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
        writer.commit().is_ok()
    })();
    let ms = started.elapsed().as_millis() as u64;
    if ok {
        if let Ok(mut w) = src.wav.lock() {
            *w = Some(dest.clone());
        }
        s.log(format!(
            "{{\"event\":\"read\",\"name\":{:?},\"sample_count\":{},\"sample_rate\":{},\"channels\":{},\"read_ok\":true,\"fnv64\":\"{:016x}\",\"wav\":{:?},\"ms\":{},\"caller_thread\":\"main\",\"reader_thread\":\"{:?}\"}}",
            props.name,
            count,
            rate,
            ch,
            hash.get(),
            dest.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
            ms,
            std::thread::current().id()
        ));
        send(s, Msg::Ready(src));
    } else {
        src.failed.store(true, Ordering::SeqCst);
        s.log(format!(
            "{{\"event\":\"read\",\"name\":{:?},\"read_ok\":false}}",
            props.name
        ));
        send(s, Msg::Failed(src));
    }
}

unsafe extern "C" fn dc_requestAudioSourceContentAnalysis(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    _n: ARASize,
    _types: *const ARAContentType,
) {
    let Some(src) = state_of(audioSourceRef) else { return };
    src.requested.store(true, Ordering::SeqCst);
    let s = session();
    if s.fault {
        return; // the fault switch runs the job inside process instead
    }
    send(s, Msg::Request(src));
}

unsafe extern "C" fn dc_isAudioSourceContentAnalysisIncomplete(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    _t: ARAContentType,
) -> ARABool {
    let Some(src) = state_of(audioSourceRef) else { return 1 };
    (!src.has_events.load(Ordering::SeqCst)) as ARABool
}

unsafe extern "C" fn dc_isAudioSourceContentAvailable(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    t: ARAContentType,
) -> ARABool {
    let Some(src) = state_of(audioSourceRef) else { return 0 };
    (t == kARAContentTypeNotes as ARAContentType && src.has_events.load(Ordering::SeqCst)) as ARABool
}

unsafe extern "C" fn dc_getAudioSourceContentGrade(
    _c: ARADocumentControllerRef,
    _s: ARAAudioSourceRef,
    _t: ARAContentType,
) -> ARAContentGrade {
    kARAContentGradeDetected as ARAContentGrade
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
}

unsafe extern "C" fn dc_getContentReaderEventCount(_c: ARADocumentControllerRef, r: ARAContentReaderRef) -> ARAInt32 {
    let reader: &Reader = &*(r as *mut Reader);
    reader.notes.len() as ARAInt32
}

unsafe extern "C" fn dc_getContentReaderDataForEvent(
    _c: ARADocumentControllerRef,
    r: ARAContentReaderRef,
    i: ARAInt32,
) -> *const c_void {
    let reader: &Reader = &*(r as *mut Reader);
    &reader.notes[i as usize] as *const ARAContentNote as *const c_void
}

unsafe extern "C" fn dc_destroyContentReader(_c: ARADocumentControllerRef, r: ARAContentReaderRef) {
    drop(Box::from_raw(r as *mut Reader));
}

unsafe extern "C" fn dc_createAudioModification(
    _c: ARADocumentControllerRef,
    audioSourceRef: ARAAudioSourceRef,
    _h: ARAAudioModificationHostRef,
    _p: *const ARAAudioModificationProperties,
) -> ARAAudioModificationRef {
    let seq = state_of(audioSourceRef).map(|s| s.seq).unwrap_or(usize::MAX);
    Box::into_raw(Box::new(Modification { seq })) as ARAAudioModificationRef
}

unsafe extern "C" fn dc_cloneAudioModification(
    _c: ARADocumentControllerRef,
    m: ARAAudioModificationRef,
    _h: ARAAudioModificationHostRef,
    _p: *const ARAAudioModificationProperties,
) -> ARAAudioModificationRef {
    let seq = (*(m as *mut Modification)).seq;
    Box::into_raw(Box::new(Modification { seq })) as ARAAudioModificationRef
}

unsafe extern "C" fn dc_destroyAudioModification(_c: ARADocumentControllerRef, m: ARAAudioModificationRef) {
    drop(Box::from_raw(m as *mut Modification));
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
    let seq = (*(m as *mut Modification)).seq;
    let rec = region_times(p);
    let ctl = controller_of(c);
    if let Ok(list) = ctl.sources.lock() {
        if let Some(src) = list.iter().find(|s| s.seq == seq) {
            if let Ok(mut rl) = src.regions.lock() {
                rl.push(rec);
            }
        }
    }
    Box::into_raw(Box::new(Modification { seq })) as ARAPlaybackRegionRef
}

unsafe extern "C" fn dc_updatePlaybackRegionProperties(
    c: ARADocumentControllerRef,
    r: ARAPlaybackRegionRef,
    p: *const ARAPlaybackRegionProperties,
) {
    let seq = (*(r as *mut Modification)).seq;
    let rec = region_times(p);
    let ctl = controller_of(c);
    if let Ok(list) = ctl.sources.lock() {
        if let Some(src) = list.iter().find(|s| s.seq == seq) {
            // an update rewrites the region's times: the last one wins
            if let Ok(mut rl) = src.regions.lock() {
                if let Some(first) = rl.first_mut() {
                    *first = rec;
                }
            }
        }
    }
}

unsafe extern "C" fn dc_destroyPlaybackRegion(_c: ARADocumentControllerRef, r: ARAPlaybackRegionRef) {
    drop(Box::from_raw(r as *mut Modification));
}

// ---------------------------------------------------------------- archiving

/// The document archive ID of this plugin's factory.
const ARCHIVE_ID: &str = "com.cherryblossomvalley.drumspike.ara.archive.v1";

unsafe extern "C" fn dc_storeObjectsToArchive(
    _c: ARADocumentControllerRef,
    writer: ARAArchiveWriterHostRef,
    _filter: *const ARAStoreObjectsFilter,
) -> ARABool {
    let s = session();
    let sources: Vec<state::ArchiveSource> = {
        let ctl = controller_of(_c);
        let Ok(list) = ctl.sources.lock() else { return 0 };
        list.iter()
            .map(|src| {
                let p = src.props();
                state::ArchiveSource {
                    persistent_id: p.persistent_id,
                    name: p.name,
                    sample_rate: p.sample_rate,
                    notes: src.notes.lock().map(|n| n.clone()).unwrap_or_default(),
                }
            })
            .collect()
    };
    let events = s
        .service_row
        .lock()
        .ok()
        .and_then(|r| r.as_ref().and_then(|row| extract_events(row)));
    let blob = state::encode(&state::ArchiveState {
        version: state::ARCHIVE_VERSION,
        archive_id: ARCHIVE_ID.into(),
        events: events.unwrap_or_default(),
        sources,
    });
    let ctl = controller_of(_c);
    if ctl.host.is_null() {
        return 0;
    }
    let a = &*(*ctl.host).archivingControllerInterface;
    let host_ref = (*ctl.host).archivingControllerHostRef;
    let ok = (a.writeBytesToArchive.unwrap())(host_ref, writer, 0, blob.len(), blob.as_ptr());
    ok as ARABool
}

/// The events object of a service row (the last JSON key of the line).
fn extract_events(row: &str) -> Option<serde_json::Map<String, Value>> {
    let v: Value = serde_json::from_str(row).ok()?;
    v.get("events")?.as_object().cloned()
}

unsafe extern "C" fn dc_restoreObjectsFromArchive(
    c: ARADocumentControllerRef,
    reader: ARAArchiveReaderHostRef,
    _filter: *const ARARestoreObjectsFilter,
) -> ARABool {
    let s = session();
    let ctl = controller_of(c);
    if ctl.host.is_null() {
        return 0;
    }
    let a = &*(*ctl.host).archivingControllerInterface;
    let host_ref = (*ctl.host).archivingControllerHostRef;
    let size = (a.getArchiveSize.unwrap())(host_ref, reader);
    let mut bytes = vec![0u8; size];
    if (a.readBytesFromArchive.unwrap())(host_ref, reader, 0, size, bytes.as_mut_ptr()) == 0 {
        return 0;
    }
    let Ok(st) = state::decode(&bytes) else { return 0 };
    // a fresh service counts its own decodes: the restore must not have decoded a file
    let svc = drumspike_app::Service::new(drumspike_app::ServiceConfig::from_env(std::path::Path::new("/")));
    let decode_counts: serde_json::Map<String, Value> = svc
        .decode_counts()
        .into_iter()
        .map(|(p, n)| (p.display().to_string(), json!(n)))
        .collect();
    if let Ok(list) = ctl.sources.lock() {
        for src in list.iter() {
            let p = src.props();
            let found = st
                .sources
                .iter()
                .find(|a| !a.persistent_id.is_empty() && a.persistent_id == p.persistent_id)
                .or_else(|| st.sources.iter().find(|a| a.name == p.name));
            if let Some(a) = found {
                if let Ok(mut n) = src.notes.lock() {
                    *n = a.notes.clone();
                }
                src.has_events.store(true, Ordering::SeqCst);
            }
        }
    }
    let row = format!(
        "{{\"event\":\"service\",\"restored\":true,\"version\":{},\"decode_counts\":{},\"events\":{}}}",
        st.version,
        serde_json::to_string(&decode_counts).unwrap_or_else(|_| "{}".into()),
        serde_json::to_string(&st.events).unwrap_or_else(|_| "{}".into()),
    );
    if let Ok(mut sr) = s.service_row.lock() {
        *sr = Some(row);
    }
    s.write_out();
    1
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
    version: cs!("0.1.0"),
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
    1
}
unsafe extern "C" fn araf_get(_f: *const clap_ara_factory, i: u32) -> *const ARAFactory {
    if i == 0 {
        &FACTORY.0
    } else {
        std::ptr::null()
    }
}
unsafe extern "C" fn araf_id(_f: *const clap_ara_factory, i: u32) -> *const c_char {
    if i == 0 {
        PLUGIN_ID
    } else {
        std::ptr::null()
    }
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
}

unsafe extern "C" fn pext_factory(_p: *const clap_plugin) -> *const ARAFactory {
    &FACTORY.0
}
unsafe extern "C" fn pext_bind(
    p: *const clap_plugin,
    _dc: ARADocumentControllerRef,
    known: ARAPlugInInstanceRoleFlags,
    assigned: ARAPlugInInstanceRoleFlags,
) -> *const ARAPlugInExtensionInstance {
    let plugin = (*p).plugin_data as *mut Plugin;
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
    drop(Box::from_raw((*p).plugin_data as *mut Plugin));
}

unsafe extern "C" fn p_activate(p: *const clap_plugin, sr: f64, _min: u32, _max: u32) -> bool {
    guard("activate", false, move || {
        let plugin: &Plugin = &*((*p).plugin_data as *mut Plugin);
        *plugin.sample_rate.lock().unwrap_or_else(|e| e.into_inner()) = sr;
        true
    })
}

unsafe extern "C" fn p_deactivate(_p: *const clap_plugin) {}
unsafe extern "C" fn p_start(_p: *const clap_plugin) -> bool {
    guard("start_processing", true, move || true)
}
unsafe extern "C" fn p_stop(_p: *const clap_plugin) {}
unsafe extern "C" fn p_reset(_p: *const clap_plugin) {}

/// The audio thread: pass the first mono pair through (silence without an input), touch
/// nothing else. The analysis runs on its own threads and is never reached here — except
/// under the fault switch, which runs it inside `process` on purpose (the overrun proof).
unsafe extern "C" fn p_process(_p: *const clap_plugin, process: *const clap_process) -> clap_process_status {
    guard("process", CLAP_PROCESS_CONTINUE, move || {
        let s = session();
        if s.fault && !s.fault_ran.swap(true, Ordering::SeqCst) {
            s.run_fault_job();
        }
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
        std::ptr::null()
    })
}

unsafe extern "C" fn p_on_main_thread(_p: *const clap_plugin) {}

// the playback renderer of M1a passes audio through (the notes reach the host through
// the content readers; the timeline render of the notes arrives with the editor part)
unsafe extern "C" fn pr_addPlaybackRegion(_rendererRef: ARAPlaybackRendererRef, _r: ARAPlaybackRegionRef) {}
unsafe extern "C" fn pr_removePlaybackRegion(_rendererRef: ARAPlaybackRendererRef, _r: ARAPlaybackRegionRef) {}
static PR_INTERFACE: SyncPtr<ARAPlaybackRendererInterface> = SyncPtr(ARAPlaybackRendererInterface {
    structSize: std::mem::size_of::<ARAPlaybackRendererInterface>(),
    addPlaybackRegion: Some(pr_addPlaybackRegion),
    removePlaybackRegion: Some(pr_removePlaybackRegion),
});

unsafe extern "C" fn er_addPlaybackRegion(
    _editorRendererRef: ARAEditorRendererRef,
    _playbackRegionRef: ARAPlaybackRegionRef,
) {
}
unsafe extern "C" fn er_removePlaybackRegion(
    _editorRendererRef: ARAEditorRendererRef,
    _playbackRegionRef: ARAPlaybackRegionRef,
) {
}
unsafe extern "C" fn er_addRegionSequence(
    _editorRendererRef: ARAEditorRendererRef,
    _regionSequenceRef: ARARegionSequenceRef,
) {
}
unsafe extern "C" fn er_removeRegionSequence(
    _editorRendererRef: ARAEditorRendererRef,
    _regionSequenceRef: ARARegionSequenceRef,
) {
}
static ER_INTERFACE: SyncPtr<ARAEditorRendererInterface> = SyncPtr(ARAEditorRendererInterface {
    structSize: std::mem::size_of::<ARAEditorRendererInterface>(),
    addPlaybackRegion: Some(er_addPlaybackRegion),
    removePlaybackRegion: Some(er_removePlaybackRegion),
    addRegionSequence: Some(er_addRegionSequence),
    removeRegionSequence: Some(er_removeRegionSequence),
});

unsafe extern "C" fn ev_notifySelection(_editorViewRef: ARAEditorViewRef, _selection: *const ARAViewSelection) {}
unsafe extern "C" fn ev_notifyHideRegionSequences(
    _editorViewRef: ARAEditorViewRef,
    _regionSequenceRefsCount: ARASize,
    _regionSequenceRefs: *const ARARegionSequenceRef,
) {
}
static EV_INTERFACE: SyncPtr<ARAEditorViewInterface> = SyncPtr(ARAEditorViewInterface {
    structSize: std::mem::size_of::<ARAEditorViewInterface>(),
    notifySelection: Some(ev_notifySelection),
    notifyHideRegionSequences: Some(ev_notifyHideRegionSequences),
});

unsafe extern "C" fn f_count(_f: *const clap_plugin_factory) -> u32 {
    1
}
unsafe extern "C" fn f_desc(_f: *const clap_plugin_factory, i: u32) -> *const clap_plugin_descriptor {
    if i == 0 {
        &DESCRIPTOR
    } else {
        std::ptr::null()
    }
}
unsafe extern "C" fn f_create(
    _f: *const clap_plugin_factory,
    _host: *const clap_host,
    id: *const c_char,
) -> *const clap_plugin {
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
    }));
    (*p).clap.plugin_data = p as *mut c_void;
    &(*p).clap
}

static PLUGIN_FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(f_count),
    get_plugin_descriptor: Some(f_desc),
    create_plugin: Some(f_create),
};

/// The CLAP entry in the layout of the CLAP 1.x spec (`clap/entry.h`). `clap-sys` 0.5.0
/// ships a draft `clap_plugin_entry` (no name/vendor/... strings, `init` without the
/// factory id, the function pointers at the wrong offsets), so the struct is defined
/// here; the export must carry it so that any spec-conforming host can load the bundle.
#[repr(C)]
pub struct ClapEntry {
    pub clap_version: clap_sys::version::clap_version,
    pub name: *const c_char,
    pub vendor: *const c_char,
    pub url: *const c_char,
    pub manual_url: *const c_char,
    pub version: *const c_char,
    pub description: *const c_char,
    pub clap_version_supported: Option<unsafe extern "C" fn(version: *const clap_sys::version::clap_version) -> bool>,
    pub get_factory: Option<unsafe extern "C" fn(factory_id: *const c_char) -> *const c_void>,
    pub init: Option<unsafe extern "C" fn(plugin_path: *const c_char, factory_id: *const c_char) -> bool>,
    pub deinit: Option<unsafe extern "C" fn()>,
}
// the strings point at literals with the process lifetime, the functions are reentrant
unsafe impl Send for ClapEntry {}
unsafe impl Sync for ClapEntry {}

unsafe extern "C" fn e_version_supported(version: *const clap_sys::version::clap_version) -> bool {
    !version.is_null() && (*version).major == CLAP_VERSION.major
}

unsafe extern "C" fn e_init(_path: *const c_char, _factory_id: *const c_char) -> bool {
    guard("entry init", true, move || true)
}
unsafe extern "C" fn e_deinit() {}
unsafe extern "C" fn e_get_factory(id: *const c_char) -> *const c_void {
    let id = CStr::from_ptr(id).to_bytes();
    if id == CLAP_PLUGIN_FACTORY_ID.to_bytes() {
        return &PLUGIN_FACTORY as *const _ as *const c_void;
    }
    if id == EXT_ARA_FACTORY || id == EXT_ARA_FACTORY_DRAFT {
        return &ARA_FACTORY_EXT as *const _ as *const c_void;
    }
    std::ptr::null()
}

/// The CLAP entry the loader looks for (the bundle carries it in its binary).
#[no_mangle]
#[used]
pub static clap_entry: ClapEntry = ClapEntry {
    clap_version: CLAP_VERSION,
    name: cs!("Drum Spike ARA"),
    vendor: cs!("Cherry Blossom Valley"),
    url: cs!("https://github.com/Cherry-Blossom-Valley/drumspike"),
    manual_url: cs!(""),
    version: cs!("0.1.0"),
    description: cs!("The Drum Spike service on the host's tracks (ARA, no editor)"),
    clap_version_supported: Some(e_version_supported),
    get_factory: Some(e_get_factory),
    init: Some(e_init),
    deinit: Some(e_deinit),
};
