//! The test host of the ARA plugin (`ara-host`): loads the CLAP bundle with dlopen,
//! speaks ARA 2 to it from the host side (the audio access controller and the archive
//! controller are the two interfaces the plugin calls back into), builds documents from
//! WAV files, and drives the modes of the check:
//!
//! ```text
//! ara-host <bundle> session  --files <wav>...                     analyse whole files
//! ara-host <bundle> regions  --file <wav> --region <a>,<d>,<p>... the regions fixture
//!                             [--update-region <i>:<a>,<d>,<p>] [--destroy-region <i>]
//! ara-host <bundle> store    --files <wav>... --archive <file>    store the ARA archive
//! ara-host <bundle> restore  --archive <file> --files <wav>...    restore in a NEW process
//! ara-host <bundle> restore  ... --no-persistent-id               sources without a persistent ID
//! ara-host <bundle> blocks   --files <wav>... [--frames N]        the audio-thread check
//! ara-host <bundle> tables                                        the interface-table walk
//! ara-host <bundle> version                                       the version of the ARA factory and of the CLAP descriptor
//! ara-host <bundle> state                                         save and load the plugin state (M1b4)
//! ara-host <bundle> grow     --files <wav>... [--batch N] [--destroy I]
//!                             the sources arrive in batches of N: each batch is added,
//!                             analysed and waited for before the next one; then source I
//!                             (an index) is destroyed (M1b1: one service and one project
//!                             for the whole document)
//!                             [--gap-ms G]: no wait between the batches, G ms between the
//!                             requests, one wait for all at the end (a DAW sets the plugin
//!                             on one track after the other)
//! ara-host <bundle> close-busy --files synth:<secs>... [--lead-ms N]  destroy the controller
//!                             while its analysis runs; prints the time of the destroy call
//! ara-host <bundle> docs     --files <wav>... --files2 <wav>...   two document controllers
//!                             in one process: the first document works, then the second
//! ara-host <bundle> editor ... --transport script|loop|null|none|deactivate   the host transport (M1b5):
//!                             `script`: stop at 2.5 s, play, stop, locate, play, locate while
//!                             playing, stop, locate before and after the region; `loop`: play
//!                             from 0 and wrap every 2.5 s; `null`: process calls with no
//!                             transport; `none`: no process call at all. The thread of the
//!                             process calls starts when the page has booted and prints the row
//!                             `transport_script` (the host's own truth, on the wall clock).
//!                             `deactivate`: plays 2 s, then deactivates the plugin with no stopped
//!                             process call (the segments end with a stopped one at that time).
//!                             `--regions-all a,d,p`: every source gets this one region.
//! ```
//!
//! Options of the modes that request an analysis (M1b4, the finding from LUNA):
//! `--no-request`: the host enables the samples access and never calls
//! `requestAudioSourceContentAnalysis`; `--request-order read-first|request-first`: the
//! request comes after the audio was read and analysed, or 300 ms before the audio;
//! `--slow-read <k>:<ms>`: the audio of source k comes ms later than the audio of the others;
//! `--cancel-read <phase>`: the host turns the access off 100 ms after it turned it on in that
//! phase; `grow ... --update-content <k>:<wav>`: after the analysis the host replaces the samples
//! of source k and sends `updateAudioSourceContent` (`--update-add <wav>`: with the access of
//! the source off, and a new source analysed before the access is on again;
//! `--update-twice`: a second update at once; `--update-fail-reads <n>`: the first n reads of the
//! new audio fail); `--add-late <wav>`: a source is added at the end and the host waits for all; `grow ... --linger-ms <n>`: the host keeps
//! pumping for n ms at the end;
//! `restore ... --enable-access`: after the restore the host enables the samples access of
//! every source (no request) and waits 3 s.
//!
//! A file `synth:<secs>` is a source the host makes itself (a kick-like figure, 48 kHz mono):
//! long sources for the modes that need an analysis that takes seconds.
//!
//! The host implements the model update controller: every `notifyAudioSourceContentChanged`
//! of the plugin is printed as a `host_content_changed` row (it comes only inside the
//! host's `notifyModelUpdates` calls, which the host makes while it waits).
//!
//! Region spec: `<startInModificationTime>,<durationInModificationTime>,<startInPlaybackTime>`
//! in seconds. Every mode prints one JSON object per line (the read and service rows of
//! the plugin come through `DS_ARA_OUT`); the report of `blocks` is the last stdout line
//! and its exit code is 0 only with 0 overruns and a written event file. The host itself
//! never writes below the cache folder of the plugin.
//!
//! usage: ara-host <path-to-.clap-bundle> <mode> [options]

use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::clap_process;
use clap_sys::version::CLAP_VERSION;
use drumspike_ara::ara::*;
use drumspike_ara::ClapEntry;
use libloading::os::unix::{Library, Symbol};
use std::ffi::{c_char, c_void, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[path = "ara-host/editor_mode.rs"]
mod editor_mode;

macro_rules! cs {
    ($s:literal) => {
        concat!($s, "\0").as_ptr() as *const c_char
    };
}

pub(crate) struct SyncPtr<T>(pub T);
unsafe impl<T> Sync for SyncPtr<T> {}
unsafe impl<T> Send for SyncPtr<T> {}

const PLUGIN_CLAP_ID: &[u8] = b"com.cherryblossomvalley.drumspike.ara\0";
const ARA_PLUGIN_EXT_ID: &[u8] = b"org.ara-audio.ara.pluginextension/2\0";
const FRAMES: usize = 512;

/// `--no-persistent-id`: the host gives every source and modification an empty persistent
/// ID, as a host that has none does. A restore then has only the name to match on, and the
/// name is the host's own (here the file path), not the reduced name the archive stores.
static NO_REGIONS: AtomicBool = AtomicBool::new(false);
/// `--regions-all a,d,p` (M1b5): every source gets this one region instead of the back-to-back layout,
/// so the sources of a session sit at the same place of the song (the usual drum session).
static REGIONS_ALL: Mutex<Option<(f64, f64, f64)>> = Mutex::new(None);
static NO_PERSISTENT_ID: AtomicBool = AtomicBool::new(false);

/// `--names <n>...`: the ARA names the host gives the sources, by creation index, instead of
/// the file paths (hostile names for the check: `a/..`, an empty name).
static NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `--no-access <k>`: the host never enables the samples access of the k-th source of a
/// request (and still requests its analysis): its audio never comes.
static NO_ACCESS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

/// `--no-request` (M1b4): the host enables the samples access and never calls
/// `requestAudioSourceContentAnalysis`, as LUNA does. It still asks whether the analysis is
/// incomplete and reads the notes.
static NO_REQUEST: AtomicBool = AtomicBool::new(false);

/// `--request-order read-first|request-first` (M1b4): with `read-first` the host enables the
/// samples access of every source and asks for the analysis only after the analysis is complete
/// (the audio was read before the request); with `request-first` it asks first and enables the
/// access 300 ms later (the request was before the audio). 0 is the old way: access, then the
/// request at once.
static REQUEST_ORDER: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// `--slow-read <k>:<ms>` (M1b4): the first read of source k takes ms more, so its audio comes
/// after the audio of the other sources (the reads run on the plugin's reader threads).
static SLOW_READ: Mutex<Option<(usize, u64)>> = Mutex::new(None);

/// `--update-fail-reads <n>` (with `--update-content`): the first n reads of the new audio fail
/// (the host's read call answers false), so the plugin has to read again.
static FAIL_READS: Mutex<Option<(usize, usize)>> = Mutex::new(None);

/// `--cancel-read <phase>` (M1b4): for the sources enabled in that phase the host turns the
/// samples access off again 100 ms after it turned it on (the read is cancelled while it runs;
/// use it with `--slow-read`).
static CANCEL_READ_PHASE: Mutex<Option<String>> = Mutex::new(None);

/// How long `analyse` waits for the plugin (seconds; `--wait N` changes it, the check gives
/// the new document modes a short bound so a broken plugin fails fast).
static WAIT_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(300);
const SAMPLE_RATE: f64 = 48_000.0;

/// One audio source of the host: the samples per channel, loaded from the WAV.
struct HostSource {
    name: CString,
    rate: u32,
    channels: Vec<Vec<f32>>,
}

/// The state the ARA callbacks read and write (the controller host refs point at it).
struct HostState {
    sources: Vec<HostSource>,
    /// The bytes written per archive writer (one entry per writer index).
    writers: Vec<Vec<u8>>,
    /// The bytes of the loaded archive (the readers read it).
    loaded: Vec<u8>,
    /// The playback regions in creation order (the update and destroy calls of the
    /// regions mode address them by index).
    regions: Vec<ARAPlaybackRegionRef>,
}

/// The bundle binary of a `.clap` bundle path.
fn binary_of(path: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(path);
    if p.is_dir() {
        let macos = p.join("Contents").join("MacOS");
        let bin = std::fs::read_dir(&macos)
            .ok()
            .and_then(|d| d.flatten().next())
            .map(|e| e.path());
        return bin.unwrap_or_else(|| p.to_path_buf());
    }
    p.to_path_buf()
}

/// The bundle path (the `init` argument): two levels above the binary.
fn bundle_of(path: &str) -> String {
    let p = std::path::Path::new(path);
    if p.is_dir() {
        return p.display().to_string();
    }
    p.ancestors()
        .nth(2)
        .map(|a| a.display().to_string())
        .unwrap_or_else(|| p.display().to_string())
}

// ---------------------------------------------------------------- the WAV loader

/// A RIFF walk over `fmt ` and `data`: the samples per channel as f32 (float 32 as it
/// is, the PCM widths scaled to full scale).
fn load_wav(path: &str) -> Result<(u32, Vec<Vec<f32>>), String> {
    let raw = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if raw.len() < 12 || &raw[0..4] != b"RIFF" || &raw[8..12] != b"WAVE" {
        return Err(format!("{path}: not a RIFF/WAVE file"));
    }
    let (mut rate, mut channels, mut bits, mut tag) = (0u32, 0u16, 0u16, 0u16);
    let mut data: Option<&[u8]> = None;
    let mut i = 12usize;
    while i + 8 <= raw.len() {
        let id = &raw[i..i + 4];
        let size = u32::from_le_bytes(raw[i + 4..i + 8].try_into().unwrap()) as usize;
        let body = raw
            .get(i + 8..i + 8 + size)
            .ok_or_else(|| format!("{path}: truncated {id:?} chunk"))?;
        match id {
            b"fmt " => {
                tag = u16::from_le_bytes(body[0..2].try_into().unwrap());
                channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
                rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
                if tag == 0xFFFE && body.len() >= 40 {
                    // WAVE_FORMAT_EXTENSIBLE: the sub-format is the real tag
                    tag = u16::from_le_bytes(body[24..26].try_into().unwrap());
                }
            }
            b"data" => data = Some(body),
            _ => {}
        }
        i += 8 + size + (size & 1);
    }
    let data = data.ok_or_else(|| format!("{path}: no data chunk"))?;
    let bytes_per = usize::from(bits / 8);
    let frames = data.len() / (bytes_per * usize::from(channels));
    let mut out: Vec<Vec<f32>> = vec![Vec::with_capacity(frames); usize::from(channels)];
    for (c, out_c) in out.iter_mut().enumerate().take(usize::from(channels)) {
        for f in 0..frames {
            let at = f * bytes_per * usize::from(channels) + c * bytes_per;
            let v = match (tag, bits) {
                (3, 32) => f32::from_le_bytes(data[at..at + 4].try_into().unwrap()),
                (1, 16) => (i16::from_le_bytes(data[at..at + 2].try_into().unwrap()) as f32) / 32768.0,
                (1, 24) => {
                    let b = [data[at], data[at + 1], data[at + 2], 0];
                    let x = i32::from_le_bytes(b) >> 8;
                    x as f32 / 8_388_608.0
                }
                (1, 32) => (i32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as f32) / 2_147_483_648.0,
                _ => return Err(format!("{path}: unsupported format tag {tag} with {bits} bits")),
            };
            out_c.push(v);
        }
    }
    Ok((rate, out))
}

// ---------------------------------------------------------------- the ARA host side

static AUDIO_ACCESS: SyncPtr<ARAAudioAccessControllerInterface> = SyncPtr(ARAAudioAccessControllerInterface {
    structSize: std::mem::size_of::<ARAAudioAccessControllerInterface>(),
    createAudioReaderForSource: Some(aa_create_reader),
    readAudioSamples: Some(aa_read),
    destroyAudioReader: Some(aa_destroy_reader),
});

/// The reader ref carries the index of the source.
unsafe extern "C" fn aa_create_reader(
    _ctl: ARAAudioAccessControllerHostRef,
    source: ARAAudioSourceHostRef,
    _all: ARABool,
) -> ARAAudioReaderHostRef {
    Box::into_raw(Box::new(source as usize)) as ARAAudioReaderHostRef
}

unsafe extern "C" fn aa_destroy_reader(_ctl: ARAAudioAccessControllerHostRef, reader: ARAAudioReaderHostRef) {
    drop(Box::from_raw(reader as *mut usize));
}

/// The block read: the samples of `samplesToRead` frames from `samplePosition`, one
/// buffer per channel.
unsafe extern "C" fn aa_read(
    _ctl: ARAAudioAccessControllerHostRef,
    reader: ARAAudioReaderHostRef,
    sample_position: ARASamplePosition,
    samples_to_read: ARASampleCount,
    buffers: *const *mut c_void,
) -> ARABool {
    let idx = *(reader as *mut usize);
    if sample_position == 0 {
        let mut fail = FAIL_READS.lock().unwrap();
        if let Some((k, left)) = fail.as_mut() {
            if *k == idx && *left > 0 {
                *left -= 1;
                return 0;
            }
        }
        drop(fail);
        let slow = *SLOW_READ.lock().unwrap();
        if let Some((k, ms)) = slow.filter(|(k, _)| *k == idx) {
            let _ = k;
            std::thread::sleep(Duration::from_millis(ms));
        }
    }
    let state = &*(_ctl as *const HostState);
    let Some(src) = state.sources.get(idx) else { return 0 };
    for (c, chan) in src.channels.iter().enumerate() {
        let buf = *buffers.add(c) as *mut f32;
        if buf.is_null() {
            continue;
        }
        for i in 0..samples_to_read.max(0) as usize {
            let at = sample_position.max(0) as usize + i;
            let v = chan.get(at).copied().unwrap_or(0.0);
            *buf.add(i) = v;
        }
    }
    1
}

static ARCHIVING: SyncPtr<ARAArchivingControllerInterface> = SyncPtr(ARAArchivingControllerInterface {
    structSize: std::mem::size_of::<ARAArchivingControllerInterface>(),
    getArchiveSize: Some(ar_size),
    readBytesFromArchive: Some(ar_read),
    writeBytesToArchive: Some(ar_write),
    notifyDocumentArchivingProgress: Some(ar_progress),
    notifyDocumentUnarchivingProgress: Some(ar_progress),
    getDocumentArchiveID: Some(ar_id),
});

/// The phase label of the rows the model update callbacks print.
static PHASE: Mutex<String> = Mutex::new(String::new());

fn set_phase(p: &str) {
    *PHASE.lock().unwrap() = p.to_string();
}

static MODEL_UPDATE: SyncPtr<ARAModelUpdateControllerInterface> = SyncPtr(ARAModelUpdateControllerInterface {
    structSize: std::mem::size_of::<ARAModelUpdateControllerInterface>(),
    notifyAudioSourceAnalysisProgress: Some(mu_progress),
    notifyAudioSourceContentChanged: Some(mu_source_changed),
    notifyAudioModificationContentChanged: Some(mu_other_changed),
    notifyPlaybackRegionContentChanged: Some(mu_region_changed),
    notifyDocumentDataChanged: Some(mu_document_changed),
});

unsafe extern "C" fn mu_document_changed(_ctl: ARAModelUpdateControllerHostRef) {}

unsafe extern "C" fn mu_progress(
    _ctl: ARAModelUpdateControllerHostRef,
    _source: ARAAudioSourceHostRef,
    _state: ARAAnalysisProgressState,
    _value: f32,
) {
}

/// The content of a source changed: the row names the source (the host ref is its index).
unsafe extern "C" fn mu_source_changed(
    ctl: ARAModelUpdateControllerHostRef,
    source: ARAAudioSourceHostRef,
    _range: *const ARAContentTimeRange,
    flags: ARAContentUpdateFlags,
) {
    let state = &*(ctl as *const HostState);
    let idx = source as usize;
    let name = state
        .sources
        .get(idx)
        .map(|s| s.name.to_string_lossy().into_owned())
        .unwrap_or_default();
    println!(
        "{{\"event\":\"host_content_changed\",\"phase\":{:?},\"index\":{idx},\"name\":{name:?},\"flags\":{flags}}}",
        PHASE.lock().unwrap().as_str()
    );
}

unsafe extern "C" fn mu_other_changed(
    _ctl: ARAModelUpdateControllerHostRef,
    _m: ARAAudioModificationHostRef,
    _range: *const ARAContentTimeRange,
    _flags: ARAContentUpdateFlags,
) {
}

unsafe extern "C" fn mu_region_changed(
    _ctl: ARAModelUpdateControllerHostRef,
    _r: ARAPlaybackRegionHostRef,
    _range: *const ARAContentTimeRange,
    _flags: ARAContentUpdateFlags,
) {
}

unsafe extern "C" fn ar_size(_ctl: ARAArchivingControllerHostRef, _reader: ARAArchiveReaderHostRef) -> ARASize {
    let state = &*(_ctl as *const HostState);
    state.loaded.len() as ARASize
}

unsafe extern "C" fn ar_read(
    _ctl: ARAArchivingControllerHostRef,
    _reader: ARAArchiveReaderHostRef,
    position: ARASize,
    length: ARASize,
    buffer: *mut u8,
) -> ARABool {
    let state = &*(_ctl as *const HostState);
    let start = position;
    let end = start + length;
    if end > state.loaded.len() {
        return 0;
    }
    std::ptr::copy_nonoverlapping(state.loaded[start..end].as_ptr(), buffer, length);
    1
}

unsafe extern "C" fn ar_write(
    _ctl: ARAArchivingControllerHostRef,
    writer: ARAArchiveWriterHostRef,
    position: ARASize,
    length: ARASize,
    buffer: *const u8,
) -> ARABool {
    let state = &mut *(_ctl as *mut HostState);
    let idx = writer as usize;
    if state.writers.len() <= idx {
        state.writers.resize(idx + 1, Vec::new());
    }
    let blob = &mut state.writers[idx];
    let end = position + length;
    if blob.len() < end {
        blob.resize(end, 0);
    }
    blob[position..end].copy_from_slice(std::slice::from_raw_parts(buffer, length));
    1
}

unsafe extern "C" fn ar_progress(_ctl: ARAArchivingControllerHostRef, _value: f32) {}

/// The document archive ID of the factory (the plugin reads its own ID back).
unsafe extern "C" fn ar_id(_ctl: ARAArchivingControllerHostRef, _reader: ARAArchiveReaderHostRef) -> ARAPersistentID {
    factory().documentArchiveID
}

static ARA_FACTORY: SyncPtr<Mutex<*const ARAFactory>> = SyncPtr(Mutex::new(std::ptr::null()));

fn factory() -> &'static ARAFactory {
    unsafe { &*(*ARA_FACTORY.0.lock().unwrap()) }
}

/// `initializeARAWithConfiguration` runs once per process, however many document
/// controllers the host makes.
static ARA_INITIALISED: AtomicBool = AtomicBool::new(false);

// ------------------------------------------------------------ the playback controller (M1b7)

#[link(name = "System")]
extern "C" {
    fn pthread_main_np() -> i32;
}

/// `--playback full|ignore|partial` of the editor mode: `none` (the default) gives no playback
/// controller (the instance's pointer stays null); `full`: the three calls, recorded, and the
/// transport of the host moves; `ignore`: recorded, the transport stays; `partial`: the host
/// declares an interface struct that ends before `requestSetPlaybackPosition` (a host that
/// does not offer the call).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PbMode {
    None,
    Full,
    Ignore,
    Partial,
}

static PB_MODE: Mutex<PbMode> = Mutex::new(PbMode::None);

/// The transport of the test host when the playback controller moves it: the position of the next
/// block and whether the host plays. The process thread (`--transport follow`) reads it every block.
pub struct PbState {
    pub playing: bool,
    pub song: f64,
    /// `--snap <s>`: a position request lands on the nearest multiple of this many seconds (a DAW
    /// grid). 0: exactly where asked.
    pub snap: f64,
}

pub static PB_STATE: Mutex<PbState> = Mutex::new(PbState {
    playing: false,
    song: 0.0,
    snap: 0.0,
});

/// One request of the plugin, as the host recorded it.
pub struct PbReq {
    pub kind: &'static str,
    pub seconds: Option<f64>,
    pub wall_ms: f64,
    /// The song position of the host right after the request (where it landed, or where it stands).
    pub landed: f64,
    pub playing_after: bool,
    pub main_thread: bool,
}

pub static PB_LOG: Mutex<Vec<PbReq>> = Mutex::new(Vec::new());

pub fn set_playback(mode: PbMode, snap: f64) {
    *PB_MODE.lock().unwrap() = mode;
    PB_STATE.lock().unwrap().snap = snap;
}

fn pb_note(kind: &'static str, seconds: Option<f64>) {
    let main_thread = unsafe { pthread_main_np() == 1 };
    let mode = *PB_MODE.lock().unwrap();
    let mut st = PB_STATE.lock().unwrap();
    if mode == PbMode::Full || mode == PbMode::Partial {
        match kind {
            "play" => st.playing = true,
            "stop" => st.playing = false,
            _ => {
                if let Some(t) = seconds {
                    st.song = if st.snap > 0.0 {
                        (t / st.snap).round() * st.snap
                    } else {
                        t
                    };
                }
            }
        }
    }
    let wall_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    PB_LOG.lock().unwrap().push(PbReq {
        kind,
        seconds,
        wall_ms,
        landed: st.song,
        playing_after: st.playing,
        main_thread,
    });
}

unsafe extern "C" fn pb_start(_r: ARAPlaybackControllerHostRef) {
    pb_note("play", None);
}
unsafe extern "C" fn pb_stop(_r: ARAPlaybackControllerHostRef) {
    pb_note("stop", None);
}
unsafe extern "C" fn pb_position(_r: ARAPlaybackControllerHostRef, t: ARATimePosition) {
    pb_note("locate", Some(t));
}
unsafe extern "C" fn pb_cycle_range(_r: ARAPlaybackControllerHostRef, _s: ARATimePosition, _d: ARATimeDuration) {}
unsafe extern "C" fn pb_cycle(_r: ARAPlaybackControllerHostRef, _e: ARABool) {}

static PLAYBACK_FULL: SyncPtr<ARAPlaybackControllerInterface> = SyncPtr(ARAPlaybackControllerInterface {
    structSize: std::mem::size_of::<ARAPlaybackControllerInterface>(),
    requestStartPlayback: Some(pb_start),
    requestStopPlayback: Some(pb_stop),
    requestSetPlaybackPosition: Some(pb_position),
    requestSetCycleRange: Some(pb_cycle_range),
    requestEnableCycle: Some(pb_cycle),
});

/// The struct size ends before `requestSetPlaybackPosition`: the pointer is there, the host does
/// not offer the call.
static PLAYBACK_PARTIAL: SyncPtr<ARAPlaybackControllerInterface> = SyncPtr(ARAPlaybackControllerInterface {
    structSize: std::mem::offset_of!(ARAPlaybackControllerInterface, requestSetPlaybackPosition),
    requestStartPlayback: Some(pb_start),
    requestStopPlayback: Some(pb_stop),
    requestSetPlaybackPosition: Some(pb_position),
    requestSetCycleRange: Some(pb_cycle_range),
    requestEnableCycle: Some(pb_cycle),
});

/// The row `playback_record`: what the host was asked, in the order it was asked, and where its
/// transport stands now. The check compares it with the song positions it computes itself.
pub fn playback_row(mode: &str, snap: f64) -> String {
    let log = PB_LOG.lock().unwrap();
    let st = PB_STATE.lock().unwrap();
    let list: Vec<String> = log
        .iter()
        .map(|r| {
            format!(
                "{{\"kind\":{:?},\"seconds\":{},\"samples\":{},\"wall_ms\":{:.3},\"landed_song\":{:.6},\"playing_after\":{},\"main_thread\":{}}}",
                r.kind,
                r.seconds.map_or("null".to_string(), |v| format!("{v:.9}")),
                r.seconds.map_or("null".to_string(), |v| ((v * SAMPLE_RATE).round() as i64).to_string()),
                r.wall_ms,
                r.landed,
                r.playing_after,
                r.main_thread
            )
        })
        .collect();
    format!(
        "{{\"event\":\"playback_record\",\"mode\":{mode:?},\"snap\":{snap},\"rate\":{SAMPLE_RATE},\"requests\":[{}],\"final_playing\":{},\"final_song\":{:.6}}}",
        list.join(","),
        st.playing,
        st.song
    )
}

/// The host instance: the two mandatory interfaces, the optional ones stay null (the
/// plugin must check before calling them). The playback controller is given only with
/// `--playback` (`set_playback`).
fn host_instance(state: *mut HostState) -> ARADocumentControllerHostInstance {
    let (pb_ref, pb_iface): (ARAPlaybackControllerHostRef, *const ARAPlaybackControllerInterface) =
        match *PB_MODE.lock().unwrap() {
            PbMode::None => (std::ptr::null_mut(), std::ptr::null()),
            PbMode::Full | PbMode::Ignore => (state as ARAPlaybackControllerHostRef, &PLAYBACK_FULL.0),
            PbMode::Partial => (state as ARAPlaybackControllerHostRef, &PLAYBACK_PARTIAL.0),
        };
    ARADocumentControllerHostInstance {
        structSize: std::mem::size_of::<ARADocumentControllerHostInstance>(),
        audioAccessControllerHostRef: state as ARAAudioAccessControllerHostRef,
        audioAccessControllerInterface: &AUDIO_ACCESS.0,
        archivingControllerHostRef: state as ARAArchivingControllerHostRef,
        archivingControllerInterface: &ARCHIVING.0,
        contentAccessControllerHostRef: std::ptr::null_mut(),
        contentAccessControllerInterface: std::ptr::null(),
        modelUpdateControllerHostRef: state as ARAModelUpdateControllerHostRef,
        modelUpdateControllerInterface: &MODEL_UPDATE.0,
        playbackControllerHostRef: pb_ref,
        playbackControllerInterface: pb_iface,
    }
}

/// A dense kick-like figure of `secs` seconds at 48 kHz: a decaying 60 Hz burst every 40 ms,
/// with a level that varies (many hits, so the analysis takes long; the source index shifts
/// the pattern, so every source differs).
fn synth_samples(i: usize, secs: f64) -> Vec<f32> {
    let rate = 48_000.0f64;
    let n = (rate * secs) as usize;
    let mut out = vec![0f32; n];
    let step = (rate * 0.04) as usize;
    let first = (rate * (0.1 + 0.03 * i as f64)) as usize;
    let burst = (rate * 0.035) as usize;
    let mut at = first;
    let mut k_hit = 0usize;
    let mut lcg = 12345u32 + i as u32;
    while at < n {
        for k in 0..burst.min(n - at) {
            let dt = k as f64 / rate;
            let level = 0.5 + 0.4 * (((k_hit * 7 + i) % 5) as f64 / 4.0);
            out[at + k] = ((2.0 * std::f64::consts::PI * 60.0 * dt).sin() * (-dt * 60.0).exp() * level) as f32;
        }
        k_hit += 1;
        at += step;
    }
    for v in out.iter_mut() {
        lcg = lcg.wrapping_mul(1_103_515_245).wrapping_add(12_345) & 0x7fff_ffff;
        *v += ((f64::from(lcg) / f64::from(0x7fff_ffffu32) - 0.5) * 0.003) as f32;
    }
    out
}

/// One CLI region spec: `startInModificationTime,durationInModificationTime,startInPlaybackTime`.
fn parse_region(spec: &str) -> Result<(f64, f64, f64), String> {
    let parts: Vec<&str> = spec.split(',').collect();
    if parts.len() != 3 {
        return Err(format!("bad region {spec:?}: want start,length,playback"));
    }
    let f = |s: &str| s.trim().parse::<f64>().map_err(|_| format!("bad region number {s:?}"));
    Ok((f(parts[0])?, f(parts[1])?, f(parts[2])?))
}

fn args_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn args_list(args: &[String], name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            i += 1;
            while i < args.len() && !args[i].starts_with("--") {
                out.push(args[i].clone());
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------- the interface walk

/// The function slots of an ARA interface table as (name, offset). The walk reads every
/// slot up to the structSize the plugin declares: ARA marks optional parts by structSize
/// (see ARAInterface.h, Versioned_Structs_) — a table may end early, but every slot
/// inside its declared size must carry a function. The hosts of the SDK call the slots
/// unchecked (ARA_Library/Dispatch/ARAHostDispatch.cpp), so an empty slot inside the
/// declared size is a crash on a real host, not a harmless gap.
macro_rules! fn_slots {
    ($t:ty: $($m:ident),+ $(,)?) => {
        &[$((stringify!($m), ::std::mem::offset_of!($t, $m))),+]
    };
}

/// The function pointers of the ARA factory.
static FACTORY_SLOTS: &[(&str, usize)] = fn_slots!(ARAFactory:
    initializeARAWithConfiguration, uninitializeARA, createDocumentControllerWithDocument);

/// The slots of the document controller interface (the full ARA 2.0-final table, 440
/// bytes = structSize + 54 slots).
static DC_SLOTS: &[(&str, usize)] = fn_slots!(ARADocumentControllerInterface:
    destroyDocumentController, getFactory, beginEditing, endEditing, notifyModelUpdates,
    beginRestoringDocumentFromArchive, endRestoringDocumentFromArchive, storeDocumentToArchive,
    updateDocumentProperties, createMusicalContext, updateMusicalContextProperties,
    updateMusicalContextContent, destroyMusicalContext, createAudioSource,
    updateAudioSourceProperties, updateAudioSourceContent, enableAudioSourceSamplesAccess,
    deactivateAudioSourceForUndoHistory, destroyAudioSource, createAudioModification,
    cloneAudioModification, updateAudioModificationProperties,
    deactivateAudioModificationForUndoHistory, destroyAudioModification, createPlaybackRegion,
    updatePlaybackRegionProperties, destroyPlaybackRegion, isAudioSourceContentAvailable,
    isAudioSourceContentAnalysisIncomplete, requestAudioSourceContentAnalysis,
    getAudioSourceContentGrade, createAudioSourceContentReader,
    isAudioModificationContentAvailable, getAudioModificationContentGrade,
    createAudioModificationContentReader, isPlaybackRegionContentAvailable,
    getPlaybackRegionContentGrade, createPlaybackRegionContentReader,
    getContentReaderEventCount, getContentReaderDataForEvent, destroyContentReader,
    createRegionSequence, updateRegionSequenceProperties, destroyRegionSequence,
    getPlaybackRegionHeadAndTailTime, restoreObjectsFromArchive, storeObjectsToArchive,
    getProcessingAlgorithmsCount, getProcessingAlgorithmProperties,
    getProcessingAlgorithmForAudioSource, requestProcessingAlgorithmForAudioSource,
    isLicensedForCapabilities, storeAudioSourceToAudioFileChunk,
    isAudioModificationPreservingAudioSourceSignal);

/// The slots of the role tables the plug-in extension carries (ARA 2.0-final).
static PR_SLOTS: &[(&str, usize)] = fn_slots!(ARAPlaybackRendererInterface: addPlaybackRegion, removePlaybackRegion);
static ER_SLOTS: &[(&str, usize)] = fn_slots!(ARAEditorRendererInterface:
    addPlaybackRegion, removePlaybackRegion, addRegionSequence, removeRegionSequence);
static EV_SLOTS: &[(&str, usize)] = fn_slots!(ARAEditorViewInterface: notifySelection, notifyHideRegionSequences);

/// Walk one interface table: count the slots inside the declared structSize and fail on
/// the first empty one. A structSize that ends before a slot skips it (the ARA rule).
unsafe fn walk_table(
    name: &str,
    iface: *const c_void,
    declared: usize,
    slots: &[(&str, usize)],
) -> Result<usize, String> {
    let mut checked = 0usize;
    for (slot, off) in slots {
        if off + std::mem::size_of::<*const ()>() > declared {
            continue;
        }
        checked += 1;
        let f = (iface as *const u8).add(*off) as *const *const ();
        if f.read_unaligned().is_null() {
            return Err(format!(
                "{name}: slot {slot} (offset {off}) is empty inside the declared structSize of {declared} bytes"
            ));
        }
    }
    Ok(checked)
}

/// The loaded plugin: the library keeps the code alive, the entry and the ARA factory
/// are stored.
struct Loaded {
    _lib: Library,
    entry: *const ClapEntry,
}

/// dlopen the bundle, run the CLAP entry init, and take the ARA factory.
unsafe fn load(bundle: &str) -> Result<Loaded, String> {
    let bin = binary_of(bundle);
    let lib = Library::new(&bin).map_err(|e| format!("dlopen {bin:?}: {e}"))?;
    let sym: Symbol<*mut c_void> = lib.get(b"clap_entry").map_err(|e| format!("no clap_entry: {e}"))?;
    let entry = sym.into_raw() as *const ClapEntry;
    let path = CString::new(bundle_of(bundle)).unwrap();
    if !((*entry).init.expect("entry.init"))(path.as_ptr()) {
        return Err("clap_entry.init refused".into());
    }
    let ext_id = CString::new(b"org.ara-audio.ara.factory/2".to_vec()).unwrap();
    let ext = ((*entry).get_factory.expect("entry.get_factory"))(ext_id.as_ptr());
    if ext.is_null() {
        return Err("the plugin exposes no ARA factory extension".into());
    }
    let fext = &*(ext as *const drumspike_ara::clap_ara_factory);
    let n = (fext.get_factory_count.expect("get_factory_count"))(ext as *const drumspike_ara::clap_ara_factory);
    if n == 0 {
        return Err("the ARA factory extension is empty".into());
    }
    let ara = (fext.get_ara_factory.expect("get_ara_factory"))(ext as *const drumspike_ara::clap_ara_factory, 0);
    if ara.is_null() {
        return Err("the ARA factory is null".into());
    }
    *ARA_FACTORY.0.lock().unwrap() = ara;
    Ok(Loaded { _lib: lib, entry })
}

/// `initializeARAWithConfiguration` and one document controller over the host state.
unsafe fn make_controller(_loaded: &Loaded, state: *mut HostState) -> Result<Inst, String> {
    let f = factory();
    let config = ARAInterfaceConfiguration {
        structSize: std::mem::size_of::<ARAInterfaceConfiguration>(),
        desiredApiGeneration: kARAAPIGeneration_2_0_Final as ARAAPIGeneration,
        assertFunctionAddress: std::ptr::null_mut(),
    };
    if !ARA_INITIALISED.swap(true, Ordering::SeqCst) {
        (f.initializeARAWithConfiguration
            .expect("initializeARAWithConfiguration"))(&config);
    }
    // one host instance per document controller (the plugin keeps the pointer for the
    // life of the controller; the host never frees it before the process ends)
    let host = Box::leak(Box::new(host_instance(state))) as *const ARADocumentControllerHostInstance;
    let dc = (f
        .createDocumentControllerWithDocument
        .expect("createDocumentControllerWithDocument"))(host, std::ptr::null());
    if dc.is_null() {
        return Err("createDocumentControllerWithDocument returned null".into());
    }
    Ok(dc)
}

/// One source and one modification per file, registered in `state`.
unsafe fn add_sources(
    inst: Inst,
    state: *mut HostState,
    files: &[String],
) -> Result<Vec<(ARAAudioSourceRef, ARAAudioModificationRef)>, String> {
    let dc = iface_of(inst);
    let dc_ref = dc_ref_of(inst);
    let mut pairs = Vec::new();
    // the host ref of a source is its index in `state.sources`: a later batch of the same
    // document continues the numbering
    let base = (*state).sources.len();
    for (k, file) in files.iter().enumerate() {
        let i = base + k;
        let (rate, channels, name) = if let Some(secs) = file.strip_prefix("synth:") {
            let secs: f64 = secs.parse().map_err(|_| format!("bad {file:?}: want synth:<secs>"))?;
            (
                48_000u32,
                vec![synth_samples(i, secs)],
                CString::new(format!("synth_kick_{i}.wav")).unwrap(),
            )
        } else {
            let (rate, channels) = load_wav(file)?;
            let shown = NAMES.lock().unwrap().get(i).cloned().unwrap_or_else(|| file.clone());
            (rate, channels, CString::new(shown).unwrap())
        };
        let pid = if NO_PERSISTENT_ID.load(Ordering::SeqCst) {
            CString::default()
        } else {
            CString::new(format!("ara-host-source-{i}")).unwrap()
        };
        let props = ARAAudioSourceProperties {
            structSize: std::mem::size_of::<ARAAudioSourceProperties>(),
            name: name.as_ptr(),
            persistentID: pid.as_ptr(),
            sampleCount: channels.first().map_or(0, |c| c.len() as ARASampleCount),
            sampleRate: f64::from(rate),
            channelCount: channels.len() as ARAChannelCount,
            merits64BitSamples: 0,
            channelArrangementDataType: kARAChannelArrangementUndefined as i32,
            channelArrangement: std::ptr::null(),
        };
        let src = (dc.createAudioSource.expect("createAudioSource"))(dc_ref, i as ARAAudioSourceHostRef, &props);
        if src.is_null() {
            return Err(format!("createAudioSource failed for {file}"));
        }
        let mod_props = ARAAudioModificationProperties {
            structSize: std::mem::size_of::<ARAAudioModificationProperties>(),
            name: name.as_ptr(),
            persistentID: pid.as_ptr(),
        };
        let modification = (dc.createAudioModification.expect("createAudioModification"))(
            dc_ref,
            src,
            i as ARAAudioModificationHostRef,
            &mod_props,
        );
        if modification.is_null() {
            return Err(format!("createAudioModification failed for {file}"));
        }
        pairs.push((src, modification));
        let state = &mut *state;
        state.sources.push(HostSource { name, rate, channels });
    }
    Ok(pairs)
}

/// The instance struct the host got from `createDocumentControllerWithDocument`. The
/// ARA rule: the host holds the instance and reads the interface out of it, while the
/// calls into the plugin carry the instance's `documentControllerRef` — never the
/// instance pointer itself.
type Inst = *const ARADocumentControllerInstance;

/// The document controller ref of an instance (the handle the plugin's own functions
/// take).
unsafe fn dc_ref_of(inst: Inst) -> ARADocumentControllerRef {
    std::ptr::addr_of!((*inst).documentControllerRef).read_unaligned()
}

/// The interface the host calls: the plugin's document controller interface, out of the
/// instance struct (the ARA rule — the instance carries the ref and the interface).
unsafe fn iface_of(inst: Inst) -> &'static ARADocumentControllerInterface {
    &*std::ptr::addr_of!((*inst).documentControllerInterface).read_unaligned()
}

/// The properties of one playback region spec (shared by the create and the update call).
fn playback_props(a: f64, d: f64, p: f64) -> ARAPlaybackRegionProperties {
    ARAPlaybackRegionProperties {
        structSize: std::mem::size_of::<ARAPlaybackRegionProperties>(),
        transformationFlags: kARAPlaybackTransformationNoChanges as i32,
        startInModificationTime: a,
        durationInModificationTime: d,
        startInPlaybackTime: p,
        durationInPlaybackTime: d,
        musicalContextRef: std::ptr::null_mut(),
        regionSequenceRef: std::ptr::null_mut(),
        name: std::ptr::null(),
        color: unsafe { std::mem::zeroed() },
    }
}

/// The playback regions of one modification, inside the edit cycle; the refs land in
/// `state` (the regions mode updates and destroys them by index afterwards).
unsafe fn add_regions(
    inst: Inst,
    state: *mut HostState,
    modification: ARAAudioModificationRef,
    specs: &[(f64, f64, f64)],
) {
    let dc = iface_of(inst);
    let dc_ref = dc_ref_of(inst);
    for (a, d, p) in specs {
        let props = playback_props(*a, *d, *p);
        let r = (dc.createPlaybackRegion.expect("createPlaybackRegion"))(
            dc_ref,
            modification,
            std::ptr::null_mut(),
            &props,
        );
        if r.is_null() {
            eprintln!("createPlaybackRegion failed");
        }
        (*state).regions.push(r);
    }
}

/// Samples access on (the reader is created on this thread, the reads run on the
/// plugin's threads), then the analysis request, then the wait.
unsafe fn analyse(inst: Inst, pairs: &[(ARAAudioSourceRef, ARAAudioModificationRef)]) -> Result<(), String> {
    request(inst, pairs);
    wait_for(inst, pairs)
}

/// The host asks the plugin to send its pending model updates (outside of editing).
unsafe fn pump(inst: Inst) {
    let dc = iface_of(inst);
    if let Some(f) = dc.notifyModelUpdates {
        f(dc_ref_of(inst));
    }
}

/// The samples access on for every source of `pairs` (not the one `--no-access` names).
unsafe fn enable_access(inst: Inst, pairs: &[(ARAAudioSourceRef, ARAAudioModificationRef)]) {
    let dc = iface_of(inst);
    let dc_ref = dc_ref_of(inst);
    for (k, (src, _)) in pairs.iter().enumerate() {
        if k != NO_ACCESS.load(Ordering::SeqCst) {
            (dc.enableAudioSourceSamplesAccess
                .expect("enableAudioSourceSamplesAccess"))(dc_ref, *src, 1);
        }
    }
    let cancel = CANCEL_READ_PHASE.lock().unwrap().clone();
    if cancel.is_some() && cancel.as_deref() == Some(PHASE.lock().unwrap().as_str()) {
        std::thread::sleep(Duration::from_millis(100));
        for (k, (src, _)) in pairs.iter().enumerate() {
            if k != NO_ACCESS.load(Ordering::SeqCst) {
                (dc.enableAudioSourceSamplesAccess
                    .expect("enableAudioSourceSamplesAccess"))(dc_ref, *src, 0);
            }
        }
    }
}

/// The analysis request for every source of `pairs`.
unsafe fn request_only(inst: Inst, pairs: &[(ARAAudioSourceRef, ARAAudioModificationRef)]) {
    let dc = iface_of(inst);
    let dc_ref = dc_ref_of(inst);
    let types = [kARAContentTypeNotes as ARAContentType];
    for (src, _) in pairs {
        (dc.requestAudioSourceContentAnalysis
            .expect("requestAudioSourceContentAnalysis"))(dc_ref, *src, types.len(), types.as_ptr());
    }
}

/// The host pumps the model updates for `ms` ms.
unsafe fn pump_for(inst: Inst, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        pump(inst);
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Samples access on and the analysis request for every source of `pairs` (no wait), in the
/// order and with the options the command line names (see `NO_REQUEST`, `REQUEST_ORDER`).
unsafe fn request(inst: Inst, pairs: &[(ARAAudioSourceRef, ARAAudioModificationRef)]) {
    if NO_REQUEST.load(Ordering::SeqCst) {
        enable_access(inst, pairs);
        return;
    }
    match REQUEST_ORDER.load(Ordering::SeqCst) {
        1 => {
            // the audio first; the request only after the plugin finished the analysis
            enable_access(inst, pairs);
            if wait_for(inst, pairs).is_ok() {
                request_only(inst, pairs);
                pump_for(inst, 1500);
            }
        }
        2 => {
            // the request first; the audio 300 ms later
            request_only(inst, pairs);
            pump_for(inst, 300);
            enable_access(inst, pairs);
        }
        _ => {
            // the old way: for each source the access, then the request at once
            let dc = iface_of(inst);
            let dc_ref = dc_ref_of(inst);
            let types = [kARAContentTypeNotes as ARAContentType];
            for (k, (src, _)) in pairs.iter().enumerate() {
                if k != NO_ACCESS.load(Ordering::SeqCst) {
                    (dc.enableAudioSourceSamplesAccess
                        .expect("enableAudioSourceSamplesAccess"))(dc_ref, *src, 1);
                }
                (dc.requestAudioSourceContentAnalysis
                    .expect("requestAudioSourceContentAnalysis"))(
                    dc_ref, *src, types.len(), types.as_ptr()
                );
            }
        }
    }
}

/// Wait until the analysis of every source of `pairs` is complete; the host pumps the model
/// updates while it waits and once more at the end.
unsafe fn wait_for(inst: Inst, pairs: &[(ARAAudioSourceRef, ARAAudioModificationRef)]) -> Result<(), String> {
    let dc = iface_of(inst);
    let dc_ref = dc_ref_of(inst);
    let wait = WAIT_SECS.load(Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(wait);
    loop {
        let incomplete = pairs.iter().any(|(s, _)| {
            (dc.isAudioSourceContentAnalysisIncomplete
                .expect("isAudioSourceContentAnalysisIncomplete"))(
                dc_ref, *s, kARAContentTypeNotes as ARAContentType
            ) != 0
        });
        if !incomplete {
            pump(inst);
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("the analysis did not finish in {wait} s"));
        }
        pump(inst);
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The notes of every source through the content readers, one JSON line per source.
unsafe fn print_notes(inst: Inst, state: &HostState, pairs: &[(ARAAudioSourceRef, ARAAudioModificationRef)]) {
    print_notes_at(inst, state, pairs, 0, None);
}

/// [`print_notes`] for the sources `base..` of the host state; a `phase` puts a label on
/// the rows (the modes with more than one round print the notes of a source more than once).
unsafe fn print_notes_at(
    inst: Inst,
    state: &HostState,
    pairs: &[(ARAAudioSourceRef, ARAAudioModificationRef)],
    base: usize,
    phase: Option<&str>,
) {
    let dc = iface_of(inst);
    let dc_ref = dc_ref_of(inst);
    let label = phase.map_or(String::new(), |p| format!("\"phase\":{p:?},"));
    for (k, (s, _)) in pairs.iter().enumerate() {
        let i = base + k;
        let reader = (dc
            .createAudioSourceContentReader
            .expect("createAudioSourceContentReader"))(
            dc_ref,
            *s,
            kARAContentTypeNotes as ARAContentType,
            std::ptr::null(),
        );
        let count = (dc.getContentReaderEventCount.expect("getContentReaderEventCount"))(dc_ref, reader);
        let mut notes = Vec::new();
        for e in 0..count {
            let p = (dc.getContentReaderDataForEvent.expect("getContentReaderDataForEvent"))(dc_ref, reader, e)
                as *const ARAContentNote;
            if p.is_null() {
                continue;
            }
            let n = &*p;
            notes.push([n.startPosition, f64::from(n.volume)]);
        }
        (dc.destroyContentReader.expect("destroyContentReader"))(dc_ref, reader);
        let src = &state.sources[i];
        let exact: Vec<f64> = notes
            .iter()
            .map(|[pos, _]| (pos * f64::from(src.rate)).round())
            .collect();
        println!(
            "{{\"event\":\"host_notes\",{label}\"name\":{:?},\"rate\":{},\"count\":{},\"samples\":{:?}}}",
            src.name.to_string_lossy(),
            src.rate,
            notes.len(),
            exact
        );
    }
}

/// The regions spec callback and the mode step of the shared document builder.
type RegionsOf<'a> = &'a dyn Fn(&HostState, usize) -> Vec<(f64, f64, f64)>;
type ModeStep<'a> = &'a dyn Fn(Inst, &HostState, &[(ARAAudioSourceRef, ARAAudioModificationRef)]) -> Result<(), String>;

/// Build the document of `files` and run `step` on it; every mode shares this.
fn with_document(
    bundle: &str,
    files: &[String],
    regions_of: RegionsOf<'_>,
    loaded: Vec<u8>,
    step: ModeStep<'_>,
) -> Result<(), String> {
    let state = Box::into_raw(Box::new(HostState {
        sources: Vec::new(),
        writers: Vec::new(),
        loaded,
        regions: Vec::new(),
    }));
    unsafe {
        let loaded = load(bundle)?;
        let inst = make_controller(&loaded, state)?;
        let dc = iface_of(inst);
        let dc_ref = dc_ref_of(inst);
        // the ARA rule: the optional interface members may be None, the host checks
        if let Some(f) = dc.beginEditing {
            f(dc_ref);
        }
        let pairs = add_sources(inst, state, files)?;
        for (i, (_, modification)) in pairs.iter().enumerate() {
            let specs = regions_of(&*state, i);
            add_regions(inst, state, *modification, &specs);
        }
        if let Some(f) = dc.endEditing {
            f(dc_ref);
        }
        let result = step(inst, &*state, &pairs);
        (dc.destroyDocumentController.expect("destroyDocumentController"))(dc_ref);
        result
    }
}

/// The CLAP plugin instance of the bundle: create + init. The modes that need the
/// companion-API instance (blocks, tables) share this.
unsafe fn clap_plugin_of(loaded: &Loaded) -> Result<*const clap_plugin, String> {
    let entry = loaded.entry;
    let factory_id = CString::new(CLAP_PLUGIN_FACTORY_ID.to_bytes()).unwrap();
    let clap_factory =
        ((*entry).get_factory.expect("entry.get_factory"))(factory_id.as_ptr()) as *const clap_plugin_factory;
    if clap_factory.is_null() {
        return Err("no CLAP plugin factory".into());
    }
    let id = CString::new(&PLUGIN_CLAP_ID[..PLUGIN_CLAP_ID.len() - 1]).unwrap();
    let host = clap_host {
        clap_version: CLAP_VERSION,
        host_data: std::ptr::null_mut(),
        name: cs!("ara-host"),
        vendor: cs!("Cherry Blossom Valley"),
        url: cs!("https://github.com/Cherry-Blossom-Valley/drumspike"),
        version: cs!("0.1.0"),
        get_extension: None,
        request_restart: None,
        request_process: None,
        request_callback: None,
    };
    let plugin = ((*clap_factory).create_plugin.expect("create_plugin"))(clap_factory, &host, id.as_ptr());
    if plugin.is_null() {
        return Err("create_plugin returned null".into());
    }
    if !((*plugin).init.expect("init"))(plugin) {
        return Err("plugin init refused".into());
    }
    Ok(plugin)
}

/// `version`: the version text of the ARA factory and of the CLAP plugin descriptor, one JSON line
/// (the check compares both with the version of the crate and the bundle files).
fn run_version(bundle: &str) -> i32 {
    unsafe {
        let loaded = match load(bundle) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        let text = |p: *const c_char| {
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        let ara = text(factory().version);
        let factory_id = CString::new(CLAP_PLUGIN_FACTORY_ID.to_bytes()).unwrap();
        let clap_factory = ((*loaded.entry).get_factory.expect("entry.get_factory"))(factory_id.as_ptr())
            as *const clap_plugin_factory;
        if clap_factory.is_null() {
            eprintln!("no CLAP plugin factory");
            return 2;
        }
        let desc = ((*clap_factory).get_plugin_descriptor.expect("get_plugin_descriptor"))(clap_factory, 0);
        if desc.is_null() {
            eprintln!("no plugin descriptor");
            return 2;
        }
        let clap = text((*desc).version);
        println!("{{\"event\":\"version\",\"ara_factory\":{ara:?},\"clap_descriptor\":{clap:?}}}");
    }
    0
}

/// `tables`: walk every ARA interface table the plugin exposes, up to the structSize it
/// declares, and fail on an empty slot or on 0 slots checked (the gate line for the
/// empty-slot class: ARA hosts call the slots inside the declared size unchecked). The
/// factory comes from the ARA factory extension, the document controller interface from
/// the instance the host holds, and the role tables from the plugin extension bound for
/// all three roles — an assigned role with a null interface is a failure on its own.
fn run_tables(bundle: &str) -> i32 {
    let state = Box::into_raw(Box::new(HostState {
        sources: Vec::new(),
        writers: Vec::new(),
        loaded: Vec::new(),
        regions: Vec::new(),
    }));
    unsafe {
        let loaded = match load(bundle) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        let inst = match make_controller(&loaded, state) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        let mut slots = 0usize;
        let mut tables: Vec<(String, usize)> = Vec::new();

        // 1. the ARA factory (its three function pointers)
        let f = factory();
        let f_declared = std::ptr::addr_of!(f.structSize).read_unaligned();
        let n = match walk_table(
            "ARAFactory",
            f as *const ARAFactory as *const c_void,
            f_declared,
            FACTORY_SLOTS,
        ) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        slots += n;
        tables.push(("ARAFactory".into(), n));

        // 2. the document controller interface, out of the instance the host holds.
        //    Under DS_ARA_TABLES_FAULT the plugin serves a table with one emptied slot:
        //    the walk must refuse it (the proof that this check can fail).
        let fault = std::env::var("DS_ARA_TABLES_FAULT").as_deref() == Ok("1");
        let dc_ptr = iface_of(inst) as *const ARADocumentControllerInterface;
        let dc_declared = std::ptr::addr_of!((*dc_ptr).structSize).read_unaligned();
        let walked_dc = walk_table("document controller", dc_ptr as *const c_void, dc_declared, DC_SLOTS);
        if fault {
            match walked_dc {
                Ok(_) => {
                    eprintln!("tables-fault: the walk passed despite the emptied slot");
                    return 2;
                }
                Err(e) => {
                    println!("{{\"event\":\"tables-fault\",\"detected\":true,\"error\":{:?}}}", e);
                    return 0;
                }
            }
        }
        let n = match walked_dc {
            Ok(n) => n,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        slots += n;
        tables.push(("documentController".into(), n));
        let dc_ref = dc_ref_of(inst);

        // 3. the plug-in extension: bind all three roles, the tables it returns must be
        //    complete (an assigned role with a null interface is a failure on its own)
        let plugin = match clap_plugin_of(&loaded) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        let ext_id = CString::new(&ARA_PLUGIN_EXT_ID[..ARA_PLUGIN_EXT_ID.len() - 1]).unwrap();
        let ext = ((*plugin).get_extension.expect("get_extension"))(plugin, ext_id.as_ptr());
        if ext.is_null() {
            eprintln!("the plugin exposes no ARA plugin extension");
            return 2;
        }
        let aext = ext as *const drumspike_ara::clap_ara_plugin_extension;
        let bind = (*aext)
            .bind_to_document_controller
            .expect("bind_to_document_controller");
        let roles: ARAPlugInInstanceRoleFlags =
            (kARAPlaybackRendererRole | kARAEditorRendererRole | kARAEditorViewRole) as ARAPlugInInstanceRoleFlags;
        let pext = bind(plugin, dc_ref, roles, roles);
        if pext.is_null() {
            eprintln!("bind_to_document_controller returned null");
            return 2;
        }
        for (label, iface, slot_list) in [
            (
                "playbackRenderer",
                std::ptr::addr_of!((*pext).playbackRendererInterface).read_unaligned() as *const c_void,
                PR_SLOTS,
            ),
            (
                "editorRenderer",
                std::ptr::addr_of!((*pext).editorRendererInterface).read_unaligned() as *const c_void,
                ER_SLOTS,
            ),
            (
                "editorView",
                std::ptr::addr_of!((*pext).editorViewInterface).read_unaligned() as *const c_void,
                EV_SLOTS,
            ),
        ] {
            if iface.is_null() {
                eprintln!("the plugin bound no {label} interface for the assigned role");
                return 2;
            }
            // structSize is the first member of every ARA table (an ARASize)
            let declared = (iface as *const ARASize).read_unaligned();
            let n = match walk_table(label, iface, declared, slot_list) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("{e}");
                    return 2;
                }
            };
            slots += n;
            tables.push((label.into(), n));
        }
        ((*plugin).destroy.expect("destroy"))(plugin);
        ((*loaded.entry).deinit.expect("deinit"))();
        ((iface_of(inst).destroyDocumentController).expect("destroyDocumentController"))(dc_ref);
        if slots == 0 {
            eprintln!("tables: no slot was inside any declared structSize");
            return 2;
        }
        let list: Vec<String> = tables.iter().map(|(t, n)| format!("\"{t}\":{n}")).collect();
        println!(
            "{{\"event\":\"tables\",\"slots\":{slots},\"tables\":{{{}}}}}",
            list.join(",")
        );
        0
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(bundle) = args.first() else {
        eprintln!("usage: ara-host <bundle> session|regions|store|restore|blocks [...]");
        std::process::exit(64);
    };
    let mode = args.get(1).cloned().unwrap_or_default();
    let rest: Vec<String> = args[2.min(args.len())..].to_vec();
    NO_REGIONS.store(rest.iter().any(|a| a == "--no-regions"), Ordering::SeqCst);
    NO_PERSISTENT_ID.store(rest.iter().any(|a| a == "--no-persistent-id"), Ordering::SeqCst);
    *NAMES.lock().unwrap() = args_list(&rest, "--names");
    if let Some(k) = args_value(&rest, "--no-access").and_then(|v| v.parse::<usize>().ok()) {
        NO_ACCESS.store(k, Ordering::SeqCst);
    }
    NO_REQUEST.store(rest.iter().any(|a| a == "--no-request"), Ordering::SeqCst);
    match args_value(&rest, "--request-order").as_deref() {
        Some("read-first") => REQUEST_ORDER.store(1, Ordering::SeqCst),
        Some("request-first") => REQUEST_ORDER.store(2, Ordering::SeqCst),
        _ => {}
    }
    if let Some(v) = args_value(&rest, "--cancel-read") {
        *CANCEL_READ_PHASE.lock().unwrap() = Some(v);
    }
    if let Some(v) = args_value(&rest, "--slow-read") {
        if let Some((k, ms)) = v
            .split_once(':')
            .and_then(|(k, ms)| Some((k.parse().ok()?, ms.parse().ok()?)))
        {
            *SLOW_READ.lock().unwrap() = Some((k, ms));
        }
    }
    if let Some(r) = args_value(&rest, "--regions-all").and_then(|v| parse_region(&v).ok()) {
        *REGIONS_ALL.lock().unwrap() = Some(r);
    }
    if let Some(w) = args_value(&rest, "--wait").and_then(|v| v.parse::<u64>().ok()) {
        WAIT_SECS.store(w, Ordering::SeqCst);
    }
    let mut files = args_list(&rest, "--files");
    if files.is_empty() {
        files = args_value(&rest, "--file").into_iter().collect();
    }
    let rc = match mode.as_str() {
        "session" => run_session(bundle, &files),
        "regions" => run_regions(bundle, &files, &rest),
        "store" => run_store(bundle, &files, args_value(&rest, "--archive")),
        "restore" => run_restore(
            bundle,
            &files,
            args_value(&rest, "--archive"),
            rest.iter().any(|a| a == "--enable-access"),
        ),
        "blocks" => run_blocks(bundle, &files, &rest),
        "tables" => run_tables(bundle),
        "version" => run_version(bundle),
        "state" => run_state(bundle),
        "grow" => run_grow(bundle, &files, &rest),
        "close-busy" => run_close_busy(bundle, &files, &rest),
        "restore2" => run_restore2(bundle, &files, &rest),
        "restore-busy" => run_restore_busy(bundle, &files, &rest),
        "editor" => editor_mode::run(bundle, &files, &rest),
        "docs" => run_docs(bundle, &files, &args_list(&rest, "--files2")),
        other => {
            eprintln!("unknown mode {other:?}");
            64
        }
    };
    std::process::exit(rc);
}

/// The full-length region layout (the ARA test host's: the regions sit back to back).
fn full_regions(state: &HostState, i: usize) -> Vec<(f64, f64, f64)> {
    if let Some(all) = *REGIONS_ALL.lock().unwrap() {
        return vec![all];
    }
    let mut position = 0.0;
    for s in state.sources.iter().take(i) {
        position += s.channels.first().map_or(0.0, |c| c.len() as f64) / f64::from(s.rate);
    }
    let duration = state.sources[i].channels.first().map_or(0.0, |c| c.len() as f64) / f64::from(state.sources[i].rate);
    vec![(0.0, duration, position)]
}

/// `session`: whole files, one full-length region per source.
fn run_session(bundle: &str, files: &[String]) -> i32 {
    if files.is_empty() {
        eprintln!("session: no --files");
        return 64;
    }
    let rc = with_document(bundle, files, &full_regions, Vec::new(), &|inst, state, pairs| {
        unsafe {
            analyse(inst, pairs)?;
        }
        unsafe {
            print_notes(inst, state, pairs);
        }
        Ok(())
    });
    match rc {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

/// `regions`: one file, the regions of the CLI specs. `--update-region <i>:<a>,<d>,<p>`
/// rewrites the properties of the i-th created region (the ARA update call lands on that
/// region, not on the first of the source) and `--destroy-region <i>` destroys it.
fn run_regions(bundle: &str, files: &[String], rest: &[String]) -> i32 {
    if files.len() != 1 {
        eprintln!("regions: exactly one --file");
        return 64;
    }
    let mut specs = Vec::new();
    for spec in args_list(rest, "--region") {
        match parse_region(&spec) {
            Ok(r) => specs.push(r),
            Err(e) => {
                eprintln!("{e}");
                return 64;
            }
        }
    }
    if specs.is_empty() {
        eprintln!("regions: no --region");
        return 64;
    }
    let mut updates: Vec<(usize, (f64, f64, f64))> = Vec::new();
    for spec in args_list(rest, "--update-region") {
        let Some((idx, region)) = spec.split_once(':') else {
            eprintln!("bad --update-region {spec:?}: want <index>:<start>,<length>,<playback>");
            return 64;
        };
        let Ok(i) = idx.parse::<usize>() else {
            eprintln!("bad --update-region index {idx:?}");
            return 64;
        };
        match parse_region(region) {
            Ok(r) => updates.push((i, r)),
            Err(e) => {
                eprintln!("{e}");
                return 64;
            }
        }
    }
    let mut destroys: Vec<usize> = Vec::new();
    for spec in args_list(rest, "--destroy-region") {
        match spec.parse::<usize>() {
            Ok(i) => destroys.push(i),
            Err(_) => {
                eprintln!("bad --destroy-region {spec:?}");
                return 64;
            }
        }
    }
    let regions_of = |_state: &HostState, i: usize| if i == 0 { specs.clone() } else { Vec::new() };
    // `--update-after`: the analysis first, then the updates, then the host waits (pumping the
    // model updates) for the plugin to analyse again
    let after = rest.iter().any(|a| a == "--update-after");
    let pump_ms = args_value(rest, "--pump-ms")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(3000);
    let rc = with_document(bundle, files, &regions_of, Vec::new(), &|inst, state, pairs| {
        unsafe {
            if after {
                analyse(inst, pairs)?;
                print_notes_at(inst, state, pairs, 0, Some("before-update"));
            }
            // the ARA update and destroy calls on the created regions, by index
            let dc = iface_of(inst);
            let dc_ref = dc_ref_of(inst);
            for (i, (a, d, p)) in &updates {
                let Some(r) = state.regions.get(*i) else {
                    return Err(format!("--update-region {i}: no region with that index"));
                };
                let props = playback_props(*a, *d, *p);
                (dc.updatePlaybackRegionProperties
                    .expect("updatePlaybackRegionProperties"))(dc_ref, *r, &props);
            }
            for i in &destroys {
                let Some(r) = state.regions.get(*i) else {
                    return Err(format!("--destroy-region {i}: no region with that index"));
                };
                (dc.destroyPlaybackRegion.expect("destroyPlaybackRegion"))(dc_ref, *r);
            }
            if !updates.is_empty() || !destroys.is_empty() {
                println!(
                    "{{\"event\":\"regions_mutated\",\"regions\":{},\"updated\":{:?},\"destroyed\":{:?}}}",
                    state.regions.len(),
                    updates.iter().map(|(i, _)| i).collect::<Vec<_>>(),
                    destroys,
                );
            }
            if after {
                set_phase("update");
                let until = Instant::now() + Duration::from_millis(pump_ms);
                while Instant::now() < until {
                    pump(inst);
                    std::thread::sleep(Duration::from_millis(20));
                }
                pump(inst);
                print_notes_at(inst, state, pairs, 0, Some("after-update"));
                return Ok(());
            }
            analyse(inst, pairs)?;
        }
        unsafe {
            print_notes(inst, state, pairs);
        }
        Ok(())
    });
    match rc {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

/// The unix time in nanoseconds (the phase markers of `docs`: the check compares them with
/// the modification times of the files).
fn unix_ns() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

/// One host document: its state, its controller instance and its sources.
struct HostDoc {
    state: *mut HostState,
    inst: Inst,
    pairs: Vec<(ARAAudioSourceRef, ARAAudioModificationRef)>,
}

impl HostDoc {
    unsafe fn new(loaded: &Loaded) -> Result<HostDoc, String> {
        HostDoc::with_archive(loaded, Vec::new())
    }

    /// A document whose host holds an archive (the restore reads it).
    unsafe fn with_archive(loaded: &Loaded, archive: Vec<u8>) -> Result<HostDoc, String> {
        let state = Box::into_raw(Box::new(HostState {
            sources: Vec::new(),
            writers: Vec::new(),
            loaded: archive,
            regions: Vec::new(),
        }));
        let inst = make_controller(loaded, state)?;
        Ok(HostDoc {
            state,
            inst,
            pairs: Vec::new(),
        })
    }

    /// One batch: add the sources with one full-length region each (inside the edit
    /// cycle), then ask for the analysis and wait for it. Prints the notes of the batch
    /// through the content readers under `phase`.
    unsafe fn add_and_analyse(&mut self, files: &[String], phase: &str) -> Result<(), String> {
        let (pairs, base) = self.add_and_request(files, phase)?;
        wait_for(self.inst, &pairs)?;
        print_notes_at(self.inst, &*self.state, &pairs, base, Some(phase));
        self.pairs.extend(pairs);
        Ok(())
    }

    /// The sources are added and their analysis is requested; nothing is waited for.
    unsafe fn add_and_request(
        &mut self,
        files: &[String],
        phase: &str,
    ) -> Result<(Vec<(ARAAudioSourceRef, ARAAudioModificationRef)>, usize), String> {
        let (pairs, base) = self.add_only(files, phase)?;
        request(self.inst, &pairs);
        Ok((pairs, base))
    }

    /// The sources are added (one full-length region each); no request.
    unsafe fn add_only(
        &mut self,
        files: &[String],
        phase: &str,
    ) -> Result<(Vec<(ARAAudioSourceRef, ARAAudioModificationRef)>, usize), String> {
        set_phase(phase);
        let dc = iface_of(self.inst);
        let dc_ref = dc_ref_of(self.inst);
        let base = (*self.state).sources.len();
        if let Some(f) = dc.beginEditing {
            f(dc_ref);
        }
        let pairs = add_sources(self.inst, self.state, files)?;
        for (k, (_, modification)) in pairs.iter().enumerate() {
            if NO_REGIONS.load(Ordering::SeqCst) {
                // `--no-regions`: sources without playback regions are plain tracks
                (*self.state).regions.push(std::ptr::null_mut());
                continue;
            }
            add_regions(
                self.inst,
                self.state,
                *modification,
                &full_regions(&*self.state, base + k),
            );
        }
        if let Some(f) = dc.endEditing {
            f(dc_ref);
        }
        Ok((pairs, base))
    }

    /// Restore the archive the host holds; the notes are read right after.
    unsafe fn restore(&self) -> Result<(), String> {
        let dc = iface_of(self.inst);
        let ok = (dc.restoreObjectsFromArchive.expect("restoreObjectsFromArchive"))(
            dc_ref_of(self.inst),
            0 as ARAArchiveReaderHostRef,
            std::ptr::null(),
        );
        if ok == 0 {
            return Err("restoreObjectsFromArchive refused".into());
        }
        Ok(())
    }

    /// Store the archive of the document; the bytes the plugin wrote.
    unsafe fn store(&self) -> Result<Vec<u8>, String> {
        let dc = iface_of(self.inst);
        let ok = (dc.storeObjectsToArchive.expect("storeObjectsToArchive"))(
            dc_ref_of(self.inst),
            0 as ARAArchiveWriterHostRef,
            std::ptr::null(),
        );
        if ok == 0 {
            return Err("storeObjectsToArchive refused".into());
        }
        (*self.state)
            .writers
            .first()
            .cloned()
            .ok_or_else(|| "the plugin wrote no archive bytes".to_string())
    }

    /// The notes of every source of the document that was not destroyed.
    unsafe fn print_all_notes(&self, phase: &str, skip: Option<usize>) {
        for (i, pair) in self.pairs.iter().enumerate() {
            if Some(i) == skip {
                continue;
            }
            print_notes_at(self.inst, &*self.state, std::slice::from_ref(pair), i, Some(phase));
        }
    }

    /// The ARA destroy order for one source: samples access off, its playback region, its
    /// modification, then the source.
    unsafe fn destroy_source(&self, i: usize) -> Result<(), String> {
        let dc = iface_of(self.inst);
        let dc_ref = dc_ref_of(self.inst);
        let (src, modification) = *self
            .pairs
            .get(i)
            .ok_or_else(|| format!("--destroy {i}: no such source"))?;
        let st = &*self.state;
        let region = *st.regions.get(i).ok_or_else(|| format!("--destroy {i}: no region"))?;
        (dc.enableAudioSourceSamplesAccess
            .expect("enableAudioSourceSamplesAccess"))(dc_ref, src, 0);
        (dc.destroyPlaybackRegion.expect("destroyPlaybackRegion"))(dc_ref, region);
        (dc.destroyAudioModification.expect("destroyAudioModification"))(dc_ref, modification);
        (dc.destroyAudioSource.expect("destroyAudioSource"))(dc_ref, src);
        println!(
            "{{\"event\":\"host_destroyed\",\"index\":{i},\"name\":{:?}}}",
            st.sources[i].name.to_string_lossy()
        );
        Ok(())
    }

    unsafe fn close(&self) {
        let dc = iface_of(self.inst);
        (dc.destroyDocumentController.expect("destroyDocumentController"))(dc_ref_of(self.inst));
    }
}

/// `grow`: the sources of one document arrive in batches (`--batch`, default all at once);
/// every batch is analysed and waited for before the next one. `--destroy I` destroys the
/// I-th source at the end.
fn run_grow(bundle: &str, files: &[String], rest: &[String]) -> i32 {
    if files.is_empty() {
        eprintln!("grow: no --files");
        return 64;
    }
    let batch = args_value(rest, "--batch")
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|b| *b > 0)
        .unwrap_or(files.len());
    let destroy = args_value(rest, "--destroy").and_then(|v| v.parse::<usize>().ok());
    let gap = args_value(rest, "--gap-ms").and_then(|v| v.parse::<u64>().ok());
    let pump_ms = args_value(rest, "--pump-ms")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let result = (|| -> Result<(), String> {
        unsafe {
            let loaded = load(bundle)?;
            let mut doc = HostDoc::new(&loaded)?;
            if let Some(gap) = gap {
                // the sources are requested one after the other with `gap` ms between them,
                // and waited for once at the end
                let mut all = Vec::new();
                for chunk in files.chunks(batch) {
                    let (pairs, _base) = doc.add_and_request(chunk, "burst")?;
                    all.extend(pairs);
                    std::thread::sleep(Duration::from_millis(gap));
                }
                wait_for(doc.inst, &all)?;
                doc.pairs.extend(all);
            } else {
                for (round, chunk) in files.chunks(batch).enumerate() {
                    doc.add_and_analyse(chunk, &format!("round-{}", round + 1))?;
                }
            }
            doc.print_all_notes("final", None);
            if let Some(v) = args_value(rest, "--update-content") {
                // M1b4: the host replaces the samples of source k (the same length and rate, other
                // content) and tells the plugin: updateAudioSourceContent, everything changed. The
                // samples access stays on. The host waits until the analysis is complete again.
                set_phase("update");
                let (k, path) = v
                    .split_once(':')
                    .and_then(|(k, p)| Some((k.parse::<usize>().ok()?, p.to_string())))
                    .ok_or_else(|| format!("--update-content {v}: want <k>:<wav>"))?;
                let (rate, channels) = load_wav(&path)?;
                let src = doc
                    .pairs
                    .get(k)
                    .ok_or_else(|| format!("--update-content {k}: no such source"))?
                    .0;
                {
                    let st = &mut *doc.state;
                    let hs = st
                        .sources
                        .get_mut(k)
                        .ok_or_else(|| format!("--update-content {k}: no source"))?;
                    hs.rate = rate;
                    hs.channels = channels;
                }
                let dc = iface_of(doc.inst);
                if let Some(n) = args_value(rest, "--update-fail-reads").and_then(|v| v.parse::<usize>().ok()) {
                    *FAIL_READS.lock().unwrap() = Some((k, n));
                }
                // `--update-add <wav>`: the samples access of the source is off while the host
                // updates the content; a new source is added and analysed (a round runs while the
                // new audio of source k is not read); then the access is turned on again
                let add = args_value(rest, "--update-add");
                let enable = dc
                    .enableAudioSourceSamplesAccess
                    .expect("enableAudioSourceSamplesAccess");
                if add.is_some() {
                    enable(dc_ref_of(doc.inst), src, 0);
                }
                (dc.updateAudioSourceContent.expect("updateAudioSourceContent"))(
                    dc_ref_of(doc.inst),
                    src,
                    std::ptr::null(),
                    kARAContentUpdateEverythingChanged as ARAContentUpdateFlags,
                );
                if rest.iter().any(|a| a == "--update-twice") {
                    // a second update at once, while the read of the first one runs (use `--slow-read`)
                    (dc.updateAudioSourceContent.expect("updateAudioSourceContent"))(
                        dc_ref_of(doc.inst),
                        src,
                        std::ptr::null(),
                        kARAContentUpdateEverythingChanged as ARAContentUpdateFlags,
                    );
                }
                if let Some(extra) = add {
                    doc.add_and_analyse(&[extra], "update-add")?;
                    set_phase("update");
                    enable(dc_ref_of(doc.inst), src, 1);
                }
                let pairs = doc.pairs.clone();
                wait_for(doc.inst, &pairs)?;
                doc.print_all_notes("after-update", None);
            }
            if let Some(ms) = args_value(rest, "--linger-ms").and_then(|v| v.parse::<u64>().ok()) {
                // the host keeps running and pumping (the plugin's own retries happen now)
                set_phase("linger");
                pump_for(doc.inst, ms);
                doc.print_all_notes("after-linger", None);
            }
            if let Some(extra) = args_value(rest, "--add-late") {
                // a source is added after everything else (a round runs over the document), and the
                // host waits until every source is complete
                doc.add_and_analyse(&[extra], "late-add")?;
                let pairs = doc.pairs.clone();
                wait_for(doc.inst, &pairs)?;
                doc.print_all_notes("after-late-add", None);
            }
            if rest.iter().any(|a| a == "--rerequest") {
                // the host asks again for sources that are complete (or whose analysis
                // failed): the plugin starts a round only when something changed
                set_phase("rerequest");
                let pairs = doc.pairs.clone();
                request(doc.inst, &pairs);
                let until = Instant::now() + Duration::from_millis(pump_ms.max(1500));
                while Instant::now() < until {
                    pump(doc.inst);
                    std::thread::sleep(Duration::from_millis(20));
                }
                wait_for(doc.inst, &pairs)?;
                doc.print_all_notes("after-rerequest", None);
            }
            if let Some(k) = args_value(rest, "--enable-late").and_then(|v| v.parse::<usize>().ok()) {
                // the samples access of a source that had none comes now, after the first round
                set_phase("late");
                let dc = iface_of(doc.inst);
                let src = doc
                    .pairs
                    .get(k)
                    .ok_or_else(|| format!("--enable-late {k}: no such source"))?
                    .0;
                (dc.enableAudioSourceSamplesAccess
                    .expect("enableAudioSourceSamplesAccess"))(dc_ref_of(doc.inst), src, 1);
                let until = Instant::now() + Duration::from_millis(pump_ms.max(3000));
                while Instant::now() < until {
                    pump(doc.inst);
                    std::thread::sleep(Duration::from_millis(20));
                }
                pump(doc.inst);
                doc.print_all_notes("after-late", None);
            }
            if let Some(i) = destroy {
                set_phase("destroy");
                doc.destroy_source(i)?;
                // the plugin analyses the tracks that are left again; the host pumps the
                // model updates while it waits for that
                let until = Instant::now() + Duration::from_millis(pump_ms);
                while Instant::now() < until {
                    pump(doc.inst);
                    std::thread::sleep(Duration::from_millis(20));
                }
                pump(doc.inst);
                doc.print_all_notes("after-destroy", Some(i));
            }
            doc.close();
        }
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

/// `close-busy`: the sources are requested and the controller is destroyed while the
/// analysis runs (`--lead-ms` after the requests); prints the time of the destroy call and
/// leaves the process at once (the plugin's worker still holds what it needs).
fn run_close_busy(bundle: &str, files: &[String], rest: &[String]) -> i32 {
    if files.is_empty() {
        eprintln!("close-busy: no --files");
        return 64;
    }
    let lead = args_value(rest, "--lead-ms")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1500);
    let after = args_value(rest, "--after-ms")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let deinit = rest.iter().any(|a| a == "--deinit");
    let result = (|| -> Result<f64, String> {
        unsafe {
            let loaded = load(bundle)?;
            let mut doc = HostDoc::new(&loaded)?;
            let (_pairs, _base) = doc.add_and_request(files, "busy")?;
            std::thread::sleep(Duration::from_millis(lead));
            let t0 = Instant::now();
            let at = unix_ns();
            doc.close();
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            println!("{{\"event\":\"host_close\",\"ms\":{ms:.1},\"unix_ns\":{at}}}");
            if deinit {
                // the host unloads the plugin library: the entry's deinit, right after the destroy
                let t1 = Instant::now();
                if let Some(f) = (*loaded.entry).deinit {
                    f();
                }
                println!(
                    "{{\"event\":\"host_deinit\",\"ms\":{:.1},\"unix_ns\":{}}}",
                    t1.elapsed().as_secs_f64() * 1000.0,
                    unix_ns()
                );
            }
            // the host stays alive: the plugin's worker keeps running in the process
            std::thread::sleep(Duration::from_millis(after));
            Ok(ms)
        }
    })();
    match result {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

/// `restore2`: the same archive restored into two document controllers of one process.
fn run_restore2(bundle: &str, files: &[String], rest: &[String]) -> i32 {
    let Some(archive) = args_value(rest, "--archive").and_then(|a| std::fs::read(a).ok()) else {
        eprintln!("restore2: no readable --archive");
        return 64;
    };
    let result = (|| -> Result<(), String> {
        unsafe {
            let loaded = load(bundle)?;
            let mut docs = Vec::new();
            for label in ["doc-1", "doc-2"] {
                let mut d = HostDoc::with_archive(&loaded, archive.clone())?;
                let (pairs, base) = d.add_only(files, label)?;
                d.restore()?;
                print_notes_at(d.inst, &*d.state, &pairs, base, Some(label));
                d.pairs.extend(pairs);
                docs.push(d);
            }
            for d in &docs {
                d.close();
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

/// `restore-busy`: another document starts a long analysis (`--busy synth:<secs>...`), then
/// this one restores `--archive` and stores at once (a host autosave right after the
/// restore); the archive goes to `--out`. The process leaves without closing anything.
fn run_restore_busy(bundle: &str, files: &[String], rest: &[String]) -> i32 {
    let (Some(archive), Some(out)) = (
        args_value(rest, "--archive").and_then(|a| std::fs::read(a).ok()),
        args_value(rest, "--out"),
    ) else {
        eprintln!("restore-busy: want --archive <readable> --out <file>");
        return 64;
    };
    let busy = args_list(rest, "--busy");
    let result = (|| -> Result<(), String> {
        unsafe {
            let loaded = load(bundle)?;
            let mut other = HostDoc::new(&loaded)?;
            other.add_and_request(&busy, "busy")?;
            std::thread::sleep(Duration::from_millis(1200));
            let mut d = HostDoc::with_archive(&loaded, archive)?;
            let (pairs, base) = d.add_only(files, "restore")?;
            println!("{{\"event\":\"host_restore\",\"unix_ns\":{}}}", unix_ns());
            d.restore()?;
            print_notes_at(d.inst, &*d.state, &pairs, base, Some("restore"));
            d.pairs.extend(pairs);
            let blob = d.store()?;
            std::fs::write(&out, &blob).map_err(|e| format!("{out}: {e}"))?;
            println!("{{\"event\":\"stored\",\"bytes\":{}}}", blob.len());
        }
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

/// `docs`: two document controllers in one process. Both exist before the first one
/// works; the first document is analysed and waited for, a phase marker is printed with
/// the time, then the second one works.
fn run_docs(bundle: &str, first: &[String], second: &[String]) -> i32 {
    if first.is_empty() || second.is_empty() {
        eprintln!("docs: want --files <wav>... --files2 <wav>...");
        return 64;
    }
    let result = (|| -> Result<(), String> {
        unsafe {
            let loaded = load(bundle)?;
            let mut a = HostDoc::new(&loaded)?;
            let mut b = HostDoc::new(&loaded)?;
            a.add_and_analyse(first, "doc-a")?;
            println!(
                "{{\"event\":\"host_phase\",\"doc\":\"b\",\"phase\":\"start\",\"unix_ns\":{}}}",
                unix_ns()
            );
            b.add_and_analyse(second, "doc-b")?;
            println!(
                "{{\"event\":\"host_phase\",\"doc\":\"b\",\"phase\":\"done\",\"unix_ns\":{}}}",
                unix_ns()
            );
            a.close();
            b.close();
        }
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

/// `store`: analyse, then write the archive of the session.
fn run_store(bundle: &str, files: &[String], archive: Option<String>) -> i32 {
    let Some(archive) = archive else {
        eprintln!("store: no --archive");
        return 64;
    };
    let rc = with_document(bundle, files, &full_regions, Vec::new(), &|inst, state, pairs| unsafe {
        analyse(inst, pairs)?;
        print_notes(inst, state, pairs);
        // one writer; the plugin writes its state blob through the archive controller
        let dc = iface_of(inst);
        let dc_ref = dc_ref_of(inst);
        let ok = (dc.storeObjectsToArchive.expect("storeObjectsToArchive"))(
            dc_ref,
            0 as ARAArchiveWriterHostRef,
            std::ptr::null(),
        );
        if ok == 0 {
            return Err("storeObjectsToArchive refused".into());
        }
        let blob = state.writers.first().cloned().unwrap_or_default();
        if blob.is_empty() {
            return Err("the plugin wrote no archive bytes".into());
        }
        if std::fs::write(&archive, &blob).is_err() {
            return Err(format!("{}: cannot write", archive));
        }
        println!("{{\"event\":\"stored\",\"bytes\":{}}}", blob.len());
        Ok(())
    });
    match rc {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

unsafe extern "C" fn state_write(stream: *const clap_sys::stream::clap_ostream, buf: *const c_void, size: u64) -> i64 {
    let out = &mut *((*stream).ctx as *mut Vec<u8>);
    out.extend_from_slice(std::slice::from_raw_parts(buf as *const u8, size as usize));
    size as i64
}

struct StateIn {
    bytes: Vec<u8>,
    at: usize,
}

unsafe extern "C" fn state_read(stream: *const clap_sys::stream::clap_istream, buf: *mut c_void, size: u64) -> i64 {
    let input = &mut *((*stream).ctx as *mut StateIn);
    let n = (input.bytes.len() - input.at).min(size as usize).min(7);
    std::ptr::copy_nonoverlapping(input.bytes[input.at..].as_ptr(), buf as *mut u8, n);
    input.at += n;
    n as i64
}

/// `state` (M1b4): the host saves the state of the plugin and loads it back, then loads an
/// empty stream, a blob of a newer version and garbage, as a host does when it makes an
/// instance. The plugin must accept all of them. Prints one `plugin_state` row; the exit code
/// is 0 when the plugin has the extension and every call answered true.
fn run_state(bundle: &str) -> i32 {
    unsafe {
        let lib = match load(bundle) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        let plugin = match clap_plugin_of(&lib) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        let id = CString::new("clap.state").unwrap();
        let ext = ((*plugin).get_extension.expect("get_extension"))(plugin, id.as_ptr())
            as *const clap_sys::ext::state::clap_plugin_state;
        if ext.is_null() {
            println!("{{\"event\":\"plugin_state\",\"has_state\":false}}");
            return 1;
        }
        let mut saved: Vec<u8> = Vec::new();
        let ostream = clap_sys::stream::clap_ostream {
            ctx: &mut saved as *mut Vec<u8> as *mut c_void,
            write: Some(state_write),
        };
        let save_ok = ((*ext).save.expect("save"))(plugin, &ostream);
        let load = |bytes: Vec<u8>| -> bool {
            let mut input = StateIn { bytes, at: 0 };
            let istream = clap_sys::stream::clap_istream {
                ctx: &mut input as *mut StateIn as *mut c_void,
                read: Some(state_read),
            };
            ((*ext).load.expect("load"))(plugin, &istream)
        };
        let mut newer = saved.clone();
        if newer.len() >= 4 {
            let at = newer.len() - 4;
            newer[at..].copy_from_slice(&99u32.to_le_bytes());
        }
        let own = load(saved.clone());
        let empty = load(Vec::new());
        let newer_ok = load(newer);
        let garbage = load(b"not a state of this plugin".to_vec());
        println!(
            "{}",
            serde_json::json!({
                "event": "plugin_state", "has_state": true, "saved_bytes": saved.len(),
                "versioned": saved.len() == 10 && saved.starts_with(b"DSARA\0"),
                "save_ok": save_ok, "load_own": own, "load_empty": empty, "load_newer": newer_ok, "load_garbage": garbage,
            })
        );
        i32::from(!(save_ok && own && empty && newer_ok && garbage))
    }
}

/// `restore`: a new process loads the archive, no analysis, notes from the blob.
fn run_restore(bundle: &str, files: &[String], archive: Option<String>, with_access: bool) -> i32 {
    let Some(archive) = archive else {
        eprintln!("restore: no --archive");
        return 64;
    };
    let bytes = match std::fs::read(&archive) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{archive}: {e}");
            return 2;
        }
    };
    if files.is_empty() {
        eprintln!("restore: no --files");
        return 64;
    }
    let rc = with_document(
        bundle,
        files,
        &full_regions,
        bytes.clone(),
        &|inst, state, pairs| unsafe {
            let dc = iface_of(inst);
            let dc_ref = dc_ref_of(inst);
            // the loaded archive reaches the plugin through the archiving controller
            // callbacks, which read the host state; no request, no analysis, no decode
            let ok = (dc.restoreObjectsFromArchive.expect("restoreObjectsFromArchive"))(
                dc_ref,
                0 as ARAArchiveReaderHostRef,
                std::ptr::null(),
            );
            if ok == 0 {
                return Err("restoreObjectsFromArchive refused".into());
            }
            if with_access {
                // M1b4: the host reads the audio of the restored sources and never asks for an
                // analysis; they have their result, so the plugin must start no round (the
                // pump runs longer than the settle time and a round would need)
                enable_access(inst, pairs);
                pump_for(inst, 3000);
            }
            print_notes(inst, state, pairs);
            Ok(())
        },
    );
    match rc {
        Ok(()) => {
            let _ = bytes;
            0
        }
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

/// `blocks`: audio blocks while the analysis runs (the S3 overrun check). One block over
/// its own duration is an overrun; the run ends when the plugin's event file appears.
fn run_blocks(bundle: &str, files: &[String], rest: &[String]) -> i32 {
    if files.is_empty() {
        eprintln!("blocks: no --files");
        return 64;
    }
    let frames = args_value(rest, "--frames")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(FRAMES);
    let max_blocks = args_value(rest, "--blocks")
        .and_then(|v| v.parse::<usize>().ok())
        // the event file is the normal end of the loop (the report of the finished
        // analysis); the cap is only the belt: the loop runs far faster than real time,
        // so a cap in blocks would cut the run before the analysis is done
        .unwrap_or(20_000_000);
    let wait_file = std::env::var("DS_ARA_OUT").ok().filter(|s| !s.is_empty());

    let state = Box::into_raw(Box::new(HostState {
        sources: Vec::new(),
        writers: Vec::new(),
        loaded: Vec::new(),
        regions: Vec::new(),
    }));
    unsafe {
        let lib = match load(bundle) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        // the document and the analysis request first: the job runs on the plugin's own
        // threads while the blocks are processed below
        let inst = match make_controller(&lib, state) {
            Ok(dc) => dc,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        let dc = iface_of(inst);
        let dc_ref = dc_ref_of(inst);
        if let Some(f) = dc.beginEditing {
            f(dc_ref);
        }
        let pairs = match add_sources(inst, state, files) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        for (i, (_, modification)) in pairs.iter().enumerate() {
            add_regions(inst, state, *modification, &full_regions(&*state, i));
        }
        if let Some(f) = dc.endEditing {
            f(dc_ref);
        }
        let types = [kARAContentTypeNotes as ARAContentType];
        for (src, _) in &pairs {
            (dc.enableAudioSourceSamplesAccess
                .expect("enableAudioSourceSamplesAccess"))(dc_ref, *src, 1);
            (dc.requestAudioSourceContentAnalysis
                .expect("requestAudioSourceContentAnalysis"))(dc_ref, *src, types.len(), types.as_ptr());
        }

        // the CLAP plugin instance for the audio blocks
        let plugin = match clap_plugin_of(&lib) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        let entry = lib.entry;
        if !((*plugin).activate.expect("activate"))(plugin, SAMPLE_RATE, 64, 4096) {
            eprintln!("activate refused");
            return 2;
        }
        if !((*plugin).start_processing.expect("start_processing"))(plugin) {
            eprintln!("start_processing refused");
            return 2;
        }
        let mut input_data: Vec<f32> = vec![0.0; frames];
        let mut output_data: Vec<f32> = vec![0.0; frames];
        let in_buf = clap_sys::audio_buffer::clap_audio_buffer {
            data32: &mut input_data.as_mut_ptr(),
            data64: std::ptr::null_mut(),
            channel_count: 1,
            latency: 0,
            constant_mask: 0,
        };
        let mut out_buf = clap_sys::audio_buffer::clap_audio_buffer {
            data32: &mut output_data.as_mut_ptr(),
            data64: std::ptr::null_mut(),
            channel_count: 1,
            latency: 0,
            constant_mask: 0,
        };
        let in_events = clap_sys::events::clap_input_events {
            ctx: std::ptr::null_mut(),
            size: Some(in_events_size),
            get: Some(in_events_get),
        };
        let out_events = clap_sys::events::clap_output_events {
            ctx: std::ptr::null_mut(),
            try_push: Some(out_events_try_push),
        };
        let budget = Duration::from_secs_f64(frames as f64 / SAMPLE_RATE);
        let (mut blocks, mut overruns, mut worst) = (0usize, 0usize, Duration::ZERO);
        let mut saw_event_file = wait_file
            .as_deref()
            .map(std::path::Path::new)
            .map(|p| p.exists())
            .unwrap_or(true);
        // the wall-clock bound: a hung analysis must end the loop with rc 2, not spin
        let deadline = Instant::now() + Duration::from_secs(120);
        while blocks < max_blocks {
            let process = clap_process {
                steady_time: blocks as i64,
                frames_count: frames as u32,
                transport: std::ptr::null(),
                audio_inputs: &in_buf,
                audio_outputs: &mut out_buf,
                audio_inputs_count: 1,
                audio_outputs_count: 1,
                in_events: &in_events,
                out_events: &out_events,
            };
            let t0 = Instant::now();
            let _ = ((*plugin).process.expect("process"))(plugin, &process);
            let took = t0.elapsed();
            if took > worst {
                worst = took;
            }
            if took > budget {
                overruns += 1;
            }
            blocks += 1;
            if let Some(f) = &wait_file {
                if std::path::Path::new(f).exists() {
                    saw_event_file = true;
                    break;
                }
            } else if blocks >= 4096 {
                break;
            }
            if Instant::now() > deadline {
                break;
            }
        }
        ((*plugin).stop_processing.expect("stop_processing"))(plugin);
        ((*plugin).deactivate.expect("deactivate"))(plugin);
        ((*plugin).destroy.expect("destroy"))(plugin);
        ((*entry).deinit.expect("deinit"))();
        let budget_us = (budget.as_secs_f64() * 1e6).round() as u64;
        let worst_us = (worst.as_secs_f64() * 1e6).round() as u64;
        println!(
            "{{\"event\":\"blocks\",\"blocks\":{blocks},\"frames\":{frames},\"sample_rate\":{SAMPLE_RATE},\"overruns\":{overruns},\"worst_block_us\":{worst_us},\"budget_us\":{budget_us},\"event_file\":{saw_event_file}}}"
        );
        if overruns == 0 && saw_event_file {
            0
        } else {
            2
        }
    }
}

unsafe extern "C" fn in_events_size(_list: *const clap_sys::events::clap_input_events) -> u32 {
    0
}
unsafe extern "C" fn in_events_get(
    _list: *const clap_sys::events::clap_input_events,
    _index: u32,
) -> *const clap_sys::events::clap_event_header {
    std::ptr::null()
}
unsafe extern "C" fn out_events_try_push(
    _list: *const clap_sys::events::clap_output_events,
    _event: *const clap_sys::events::clap_event_header,
) -> bool {
    false
}
