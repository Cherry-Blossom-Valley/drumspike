//! The test host of the ARA plugin (`ara-host`): loads the CLAP bundle with dlopen,
//! speaks ARA 2 to it from the host side (the audio access controller and the archive
//! controller are the two interfaces the plugin calls back into), builds documents from
//! WAV files, and drives the modes of the check:
//!
//! ```text
//! ara-host <bundle> session  --files <wav>...                     analyse whole files
//! ara-host <bundle> regions  --file <wav> --region <a>,<d>,<p>... the regions fixture
//! ara-host <bundle> store    --files <wav>... --archive <file>    store the ARA archive
//! ara-host <bundle> restore  --archive <file> --files <wav>...    restore in a NEW process
//! ara-host <bundle> blocks   --files <wav>... [--frames N]        the audio-thread check
//! ```
//!
//! Region spec: `<startInModificationTime>,<durationInModificationTime>,<startInPlaybackTime>`
//! in seconds. Every mode prints one JSON object per line (the read and service rows of
//! the plugin come through `DS_ARA_OUT`); the report of `blocks` is the last stdout line
//! and its exit code is 0 only with 0 overruns and a written event file. The host itself
//! never writes below the cache folder of the plugin.
//!
//! usage: ara-host <path-to-.clap-bundle> <mode> [options]

use drumspike_ara::ara::*;
use libloading::os::unix::{Library, Symbol};
use clap_sys::entry::clap_plugin_entry;
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::process::clap_process;
use clap_sys::version::CLAP_VERSION;
use std::ffi::{c_char, c_void, CString};
use std::sync::Mutex;
use std::time::{Duration, Instant};

macro_rules! cs {
    ($s:literal) => {
        concat!($s, "\0").as_ptr() as *const c_char
    };
}

pub(crate) struct SyncPtr<T>(pub T);
unsafe impl<T> Sync for SyncPtr<T> {}
unsafe impl<T> Send for SyncPtr<T> {}

const PLUGIN_CLAP_ID: &[u8] = b"com.cherryblossomvalley.drumspike.ara\0";
const FRAMES: usize = 512;
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
}

/// The bundle binary of a `.clap` bundle path.
fn binary_of(path: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(path);
    if p.is_dir() {
        let macos = p.join("Contents").join("MacOS");
        let bin = std::fs::read_dir(&macos).ok().and_then(|d| d.flatten().next()).map(|e| e.path());
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
    p.ancestors().nth(2).map(|a| a.display().to_string()).unwrap_or_else(|| p.display().to_string())
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
        let body = raw.get(i + 8..i + 8 + size).ok_or_else(|| format!("{path}: truncated {id:?} chunk"))?;
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
    for f in 0..frames {
        for c in 0..usize::from(channels) {
            let at = f * bytes_per * usize::from(channels) + c * bytes_per;
            let v = match (tag, bits) {
                (3, 32) => f32::from_le_bytes(data[at..at + 4].try_into().unwrap()),
                (1, 16) => {
                    (i16::from_le_bytes(data[at..at + 2].try_into().unwrap()) as f32) / 32768.0
                }
                (1, 24) => {
                    let b = [data[at], data[at + 1], data[at + 2], 0];
                    let x = i32::from_le_bytes(b) >> 8;
                    x as f32 / 8_388_608.0
                }
                (1, 32) => (i32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as f32) / 2_147_483_648.0,
                _ => return Err(format!("{path}: unsupported format tag {tag} with {bits} bits")),
            };
            out[c].push(v);
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
    _ctl: ARAAudioAccessControllerHostRef, source: ARAAudioSourceHostRef, _all: ARABool,
) -> ARAAudioReaderHostRef {
    Box::into_raw(Box::new(source as usize)) as ARAAudioReaderHostRef
}

unsafe extern "C" fn aa_destroy_reader(_ctl: ARAAudioAccessControllerHostRef, reader: ARAAudioReaderHostRef) {
    drop(Box::from_raw(reader as *mut usize));
}

/// The block read: the samples of `samplesToRead` frames from `samplePosition`, one
/// buffer per channel.
unsafe extern "C" fn aa_read(
    _ctl: ARAAudioAccessControllerHostRef, reader: ARAAudioReaderHostRef, sample_position: ARASamplePosition,
    samples_to_read: ARASampleCount, buffers: *const *mut c_void,
) -> ARABool {
    let idx = *(reader as *mut usize);
    let state = &*(_ctl as *const HostState);
    let Some(src) = state.sources.get(idx) else { return 0 };
    for c in 0..src.channels.len() {
        let buf = *buffers.add(c) as *mut f32;
        if buf.is_null() {
            continue;
        }
        for i in 0..samples_to_read.max(0) as usize {
            let at = sample_position.max(0) as usize + i;
            let v = src.channels[c].get(at).copied().unwrap_or(0.0);
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

unsafe extern "C" fn ar_size(_ctl: ARAArchivingControllerHostRef, _reader: ARAArchiveReaderHostRef) -> ARASize {
    let state = &*(_ctl as *const HostState);
    state.loaded.len() as ARASize
}

unsafe extern "C" fn ar_read(
    _ctl: ARAArchivingControllerHostRef, _reader: ARAArchiveReaderHostRef, position: ARASize, length: ARASize,
    buffer: *mut u8,
) -> ARABool {
    let state = &*(_ctl as *const HostState);
    let start = position as usize;
    let end = start + length as usize;
    if end > state.loaded.len() {
        return 0;
    }
    std::ptr::copy_nonoverlapping(state.loaded[start..end].as_ptr(), buffer, length as usize);
    1
}

unsafe extern "C" fn ar_write(
    _ctl: ARAArchivingControllerHostRef, writer: ARAArchiveWriterHostRef, position: ARASize, length: ARASize,
    buffer: *const u8,
) -> ARABool {
    let state = &mut *(_ctl as *mut HostState);
    let idx = writer as usize;
    if state.writers.len() <= idx {
        state.writers.resize(idx + 1, Vec::new());
    }
    let blob = &mut state.writers[idx];
    let end = position as usize + length as usize;
    if blob.len() < end {
        blob.resize(end, 0);
    }
    blob[position as usize..end].copy_from_slice(std::slice::from_raw_parts(buffer, length as usize));
    1
}

unsafe extern "C" fn ar_progress(_ctl: ARAArchivingControllerHostRef, _value: f32) {}

/// The document archive ID of the factory (the plugin reads its own ID back).
unsafe extern "C" fn ar_id(
    _ctl: ARAArchivingControllerHostRef, _reader: ARAArchiveReaderHostRef,
) -> ARAPersistentID {
    factory().documentArchiveID
}

static ARA_FACTORY: SyncPtr<Mutex<*const ARAFactory>> = SyncPtr(Mutex::new(std::ptr::null()));

fn factory() -> &'static ARAFactory {
    unsafe { &*(*ARA_FACTORY.0.lock().unwrap()) }
}

static HOST_INSTANCE: SyncPtr<Mutex<Option<ARADocumentControllerHostInstance>>> =
    SyncPtr(Mutex::new(None));

/// The host instance: the two mandatory interfaces, the optional ones stay null (the
/// plugin must check before calling them).
fn host_instance(state: *mut HostState) -> ARADocumentControllerHostInstance {
    ARADocumentControllerHostInstance {
        structSize: std::mem::size_of::<ARADocumentControllerHostInstance>(),
        audioAccessControllerHostRef: state as ARAAudioAccessControllerHostRef,
        audioAccessControllerInterface: &AUDIO_ACCESS.0,
        archivingControllerHostRef: state as ARAArchivingControllerHostRef,
        archivingControllerInterface: &ARCHIVING.0,
        contentAccessControllerHostRef: std::ptr::null_mut(),
        contentAccessControllerInterface: std::ptr::null(),
        modelUpdateControllerHostRef: std::ptr::null_mut(),
        modelUpdateControllerInterface: std::ptr::null(),
        playbackControllerHostRef: std::ptr::null_mut(),
        playbackControllerInterface: std::ptr::null(),
    }
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
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
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

/// The loaded plugin: the library keeps the code alive, the entry and the ARA factory
/// are stored.
struct Loaded {
    _lib: Library,
    entry: *const clap_plugin_entry,
}

/// dlopen the bundle, run the CLAP entry init, and take the ARA factory.
unsafe fn load(bundle: &str) -> Result<Loaded, String> {
    let bin = binary_of(bundle);
    let lib = Library::new(&bin).map_err(|e| format!("dlopen {bin:?}: {e}"))?;
    let sym: Symbol<*mut c_void> = lib.get(b"clap_entry").map_err(|e| format!("no clap_entry: {e}"))?;
    let entry = sym.into_raw() as *const clap_plugin_entry;
    let path = CString::new(bundle_of(bundle)).unwrap();
    let factory_id = CString::new(CLAP_PLUGIN_FACTORY_ID.to_bytes()).unwrap();
    let init: unsafe extern "C" fn(*const c_char, *const c_char) -> bool =
        std::mem::transmute((*entry).init.expect("entry.init"));
    if !init(path.as_ptr(), factory_id.as_ptr()) {
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
unsafe fn make_controller(
    _loaded: &Loaded, state: *mut HostState,
) -> Result<ARADocumentControllerRef, String> {
    let f = factory();
    let config = ARAInterfaceConfiguration {
        structSize: std::mem::size_of::<ARAInterfaceConfiguration>(),
        desiredApiGeneration: kARAAPIGeneration_2_0_Final as ARAAPIGeneration,
        assertFunctionAddress: std::ptr::null_mut(),
    };
    ((*f).initializeARAWithConfiguration.expect("initializeARAWithConfiguration"))(&config);
    *HOST_INSTANCE.0.lock().unwrap() = Some(host_instance(state));
    let host =
        HOST_INSTANCE.0.lock().unwrap().as_ref().unwrap() as *const ARADocumentControllerHostInstance;
    let dc = ((*f).createDocumentControllerWithDocument.expect("createDocumentControllerWithDocument"))(
        host,
        std::ptr::null(),
    );
    if dc.is_null() {
        return Err("createDocumentControllerWithDocument returned null".into());
    }
    Ok((*dc).documentControllerRef)
}

/// One source and one modification per file, registered in `state`.
unsafe fn add_sources(
    dc_ref: ARADocumentControllerRef, state: *mut HostState, files: &[String],
) -> Result<Vec<(ARAAudioSourceRef, ARAAudioModificationRef)>, String> {
    let dc = iface_of(dc_ref);
    let mut pairs = Vec::new();
    for (i, file) in files.iter().enumerate() {
        let (rate, channels) = load_wav(file)?;
        let name = CString::new(file.clone()).unwrap();
        let pid = CString::new(format!("ara-host-source-{i}")).unwrap();
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
        state.sources.push(HostSource {
            name,
            rate,
            channels,
        });
    }
    Ok(pairs)
}

/// The instance struct behind a document controller ref (the host keeps the layout).
unsafe fn document_controller_of(dc_ref: ARADocumentControllerRef) -> *const ARADocumentControllerInstance {
    dc_ref as *const ARADocumentControllerInstance
}

/// The interface the host calls: the plugin's document controller interface, out of the
/// instance struct (the ARA rule — the instance carries the ref and the interface).
unsafe fn iface_of(dc_ref: ARADocumentControllerRef) -> &'static ARADocumentControllerInterface {
    &*(*document_controller_of(dc_ref)).documentControllerInterface
}

/// The playback regions of one modification, inside the edit cycle.
unsafe fn add_regions(
    dc_ref: ARADocumentControllerRef, modification: ARAAudioModificationRef, specs: &[(f64, f64, f64)],
) {
    let dc = iface_of(dc_ref);
    for (a, d, p) in specs {
        let props = ARAPlaybackRegionProperties {
            structSize: std::mem::size_of::<ARAPlaybackRegionProperties>(),
            transformationFlags: kARAPlaybackTransformationNoChanges as i32,
            startInModificationTime: *a,
            durationInModificationTime: *d,
            startInPlaybackTime: *p,
            durationInPlaybackTime: *d,
            musicalContextRef: std::ptr::null_mut(),
            regionSequenceRef: std::ptr::null_mut(),
            name: std::ptr::null(),
            color: std::mem::zeroed(),
        };
        let r = (dc.createPlaybackRegion.expect("createPlaybackRegion"))(
            dc_ref,
            modification,
            std::ptr::null_mut(),
            &props,
        );
        if r.is_null() {
            eprintln!("createPlaybackRegion failed");
        }
    }
}

/// Samples access on (the reader is created on this thread, the reads run on the
/// plugin's threads), then the analysis request, then the wait.
unsafe fn analyse(
    dc_ref: ARADocumentControllerRef, pairs: &[(ARAAudioSourceRef, ARAAudioModificationRef)],
) -> Result<(), String> {
    let dc = iface_of(dc_ref);
    let types = [kARAContentTypeNotes as ARAContentType];
    for (src, _) in pairs {
        (dc.enableAudioSourceSamplesAccess.expect("enableAudioSourceSamplesAccess"))(dc_ref, *src, 1);
        (dc.requestAudioSourceContentAnalysis.expect("requestAudioSourceContentAnalysis"))(
            dc_ref,
            *src,
            types.len(),
            types.as_ptr(),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let incomplete = pairs.iter().any(|(s, _)| {
            (dc.isAudioSourceContentAnalysisIncomplete.expect("isAudioSourceContentAnalysisIncomplete"))(
                dc_ref,
                *s,
                kARAContentTypeNotes as ARAContentType,
            ) != 0
        });
        if !incomplete {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err("the analysis did not finish in 300 s".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The notes of every source through the content readers, one JSON line per source.
unsafe fn print_notes(
    dc_ref: ARADocumentControllerRef, state: &HostState, pairs: &[(ARAAudioSourceRef, ARAAudioModificationRef)],
) {
    let dc = iface_of(dc_ref);
    for (i, (s, _)) in pairs.iter().enumerate() {
        let reader = (dc.createAudioSourceContentReader.expect("createAudioSourceContentReader"))(
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
        let exact: Vec<f64> = notes.iter().map(|[pos, _]| (pos * f64::from(src.rate)).round()).collect();
        println!(
            "{{\"event\":\"host_notes\",\"name\":{:?},\"rate\":{},\"count\":{},\"samples\":{:?}}}",
            src.name.to_string_lossy(),
            src.rate,
            notes.len(),
            exact
        );
    }
}

/// Build the document of `files` and run `step` on it; every mode shares this.
fn with_document(
    bundle: &str, files: &[String], regions_of: &dyn Fn(&HostState, usize) -> Vec<(f64, f64, f64)>,
    step: &dyn Fn(ARADocumentControllerRef, &HostState, &[(ARAAudioSourceRef, ARAAudioModificationRef)]) -> Result<(), String>,
) -> Result<(), String> {
    let state = Box::into_raw(Box::new(HostState {
        sources: Vec::new(),
        writers: Vec::new(),
        loaded: Vec::new(),
    }));
    unsafe {
        let loaded = load(bundle)?;
        let dc_ref = make_controller(&loaded, state)?;
        let dc = iface_of(dc_ref);
        (dc.beginEditing.expect("beginEditing"))(dc_ref);
        let pairs = add_sources(dc_ref, state, files)?;
        for (i, (_, modification)) in pairs.iter().enumerate() {
            let specs = regions_of(&*state, i);
            add_regions(dc_ref, *modification, &specs);
        }
        (dc.endEditing.expect("endEditing"))(dc_ref);
        let result = step(dc_ref, &*state, &pairs);
        (dc.destroyDocumentController.expect("destroyDocumentController"))(dc_ref);
        result
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
    let mut files = args_list(&rest, "--files");
    if files.is_empty() {
        files = args_value(&rest, "--file").into_iter().collect();
    }
    let rc = match mode.as_str() {
        "session" => run_session(bundle, &files),
        "regions" => run_regions(bundle, &files, &rest),
        "store" => run_store(bundle, &files, args_value(&rest, "--archive")),
        "restore" => run_restore(bundle, &files, args_value(&rest, "--archive")),
        "blocks" => run_blocks(bundle, &files, &rest),
        other => {
            eprintln!("unknown mode {other:?}");
            64
        }
    };
    std::process::exit(rc);
}

/// The full-length region layout (the ARA test host's: the regions sit back to back).
fn full_regions(state: &HostState, i: usize) -> Vec<(f64, f64, f64)> {
    let mut position = 0.0;
    for s in state.sources.iter().take(i) {
        position += s.channels.first().map_or(0.0, |c| c.len() as f64) / f64::from(s.rate);
    }
    let duration = state.sources[i].channels.first().map_or(0.0, |c| c.len() as f64)
        / f64::from(state.sources[i].rate);
    vec![(0.0, duration, position)]
}

/// `session`: whole files, one full-length region per source.
fn run_session(bundle: &str, files: &[String]) -> i32 {
    if files.is_empty() {
        eprintln!("session: no --files");
        return 64;
    }
    let rc = with_document(bundle, files, &full_regions, &|dc_ref, state, pairs| {
        unsafe { analyse(dc_ref, pairs)? };
        unsafe { print_notes(dc_ref, state, pairs) };
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

/// `regions`: one file, the regions of the CLI specs.
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
    let regions_of = |_state: &HostState, i: usize| if i == 0 { specs.clone() } else { Vec::new() };
    let rc = with_document(bundle, files, &regions_of, &|dc_ref, state, pairs| {
        unsafe { analyse(dc_ref, pairs)? };
        unsafe { print_notes(dc_ref, state, pairs) };
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

/// `store`: analyse, then write the archive of the session.
fn run_store(bundle: &str, files: &[String], archive: Option<String>) -> i32 {
    let Some(archive) = archive else {
        eprintln!("store: no --archive");
        return 64;
    };
    let rc = with_document(bundle, files, &full_regions, &|dc_ref, state, pairs| unsafe {
        analyse(dc_ref, pairs)?;
        print_notes(dc_ref, state, pairs);
        // one writer; the plugin writes its state blob through the archive controller
        let dc = iface_of(dc_ref);
        let ok = (dc.storeObjectsToArchive.expect("storeObjectsToArchive"))(dc_ref, 0 as ARAArchiveWriterHostRef, std::ptr::null());
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

/// `restore`: a new process loads the archive, no analysis, notes from the blob.
fn run_restore(bundle: &str, files: &[String], archive: Option<String>) -> i32 {
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
    let rc = with_document(bundle, files, &full_regions, &|dc_ref, state, pairs| unsafe {
        let dc = iface_of(dc_ref);
        // the loaded archive feeds the readers; no request, no analysis, no decode
        let ok = (dc.restoreObjectsFromArchive.expect("restoreObjectsFromArchive"))(dc_ref, 0 as ARAArchiveReaderHostRef, std::ptr::null());
        if ok == 0 {
            return Err("restoreObjectsFromArchive refused".into());
        }
        print_notes(dc_ref, state, pairs);
        Ok(())
    });
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
    let frames = args_value(rest, "--frames").and_then(|v| v.parse::<usize>().ok()).unwrap_or(FRAMES);
    let max_blocks = args_value(rest, "--blocks").and_then(|v| v.parse::<usize>().ok()).unwrap_or(200_000);
    let wait_file = std::env::var("DS_ARA_OUT").ok().filter(|s| !s.is_empty());

    let state = Box::into_raw(Box::new(HostState {
        sources: Vec::new(),
        writers: Vec::new(),
        loaded: Vec::new(),
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
        let dc_ref = match make_controller(&lib, state) {
            Ok(dc) => dc,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        let dc = iface_of(dc_ref);
        (dc.beginEditing.expect("beginEditing"))(dc_ref);
        let pairs = match add_sources(dc_ref, state, files) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        for (i, (_, modification)) in pairs.iter().enumerate() {
            add_regions(dc_ref, *modification, &full_regions(&*state, i));
        }
        (dc.endEditing.expect("endEditing"))(dc_ref);
        let types = [kARAContentTypeNotes as ARAContentType];
        for (src, _) in &pairs {
            (dc.enableAudioSourceSamplesAccess.expect("enableAudioSourceSamplesAccess"))(dc_ref, *src, 1);
            (dc.requestAudioSourceContentAnalysis.expect("requestAudioSourceContentAnalysis"))(
                dc_ref,
                *src,
                types.len(),
                types.as_ptr(),
            );
        }

        // the CLAP plugin instance for the audio blocks
        let entry = lib.entry;
        let factory_id = CString::new(CLAP_PLUGIN_FACTORY_ID.to_bytes()).unwrap();
        let clap_factory =
            ((*entry).get_factory.expect("entry.get_factory"))(factory_id.as_ptr()) as *const clap_plugin_factory;
        if clap_factory.is_null() {
            eprintln!("no CLAP plugin factory");
            return 2;
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
            eprintln!("create_plugin returned null");
            return 2;
        }
        if !((*plugin).init.expect("init"))(plugin) {
            eprintln!("plugin init refused");
            return 2;
        }
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
        let mut saw_event_file = wait_file.as_deref().map(std::path::Path::new).map(|p| p.exists()).unwrap_or(true);
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
    _list: *const clap_sys::events::clap_input_events, _index: u32,
) -> *const clap_sys::events::clap_event_header {
    std::ptr::null()
}
unsafe extern "C" fn out_events_try_push(
    _list: *const clap_sys::events::clap_output_events, _event: *const clap_sys::events::clap_event_header,
) -> bool {
    false
}
