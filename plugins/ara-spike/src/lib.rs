//! Drum Spike ARA spike: a CLAP plugin in Rust that speaks ARA 2 through the plain C interface.
//! Throwaway code. It proves one chain: host -> CLAP -> ARA document controller in Rust ->
//! audio samples of a source read on a background thread -> "hits" back to the host as notes.
#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case, unused_variables, dead_code)]

pub mod ara {
    #![allow(non_upper_case_globals, non_camel_case_types, non_snake_case, dead_code)]
    include!(concat!(env!("OUT_DIR"), "/ara.rs"));
}
use ara::*;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::ext::audio_ports::*;
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};
use clap_sys::version::CLAP_VERSION;
use std::ffi::{c_char, c_void, CStr};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub(crate) struct SyncPtr<T>(pub T);
unsafe impl<T> Sync for SyncPtr<T> {}
unsafe impl<T> Send for SyncPtr<T> {}

macro_rules! cs {
    ($s:literal) => {
        concat!($s, "\0").as_ptr() as *const c_char
    };
}

include!("stubs.rs");

// ---------------------------------------------------------------- ARA factory

static CONTENT_TYPES: [ARAContentType; 1] = [kARAContentTypeNotes as ARAContentType];

static FACTORY: SyncPtr<ARAFactory> = SyncPtr(ARAFactory {
    structSize: std::mem::size_of::<ARAFactory>(),
    lowestSupportedApiGeneration: kARAAPIGeneration_2_0_Final as ARAAPIGeneration,
    highestSupportedApiGeneration: kARAAPIGeneration_2_0_Final as ARAAPIGeneration,
    factoryID: cs!("com.cherryblossomvalley.drumspike.araspike.arafactory"),
    initializeARAWithConfiguration: Some(ara_init),
    uninitializeARA: Some(ara_uninit),
    plugInName: cs!("Drum Spike ARA Spike"),
    manufacturerName: cs!("Cherry Blossom Valley"),
    informationURL: cs!("https://github.com/Cherry-Blossom-Valley/drumspike"),
    version: cs!("0.0.1"),
    createDocumentControllerWithDocument: Some(create_document_controller),
    documentArchiveID: cs!("com.cherryblossomvalley.drumspike.araspike.archive.v1"),
    compatibleDocumentArchiveIDsCount: 0,
    compatibleDocumentArchiveIDs: std::ptr::null(),
    analyzeableContentTypesCount: 1,
    analyzeableContentTypes: CONTENT_TYPES.as_ptr(),
    supportedPlaybackTransformationFlags: 0,
    supportsStoringAudioFileChunks: 0,
});

unsafe extern "C" fn ara_init(config: *const ARAInterfaceConfiguration) {
    log(&format!("{{\"event\":\"ara_init\",\"api_generation\":{}}}", (*config).desiredApiGeneration));
}
unsafe extern "C" fn ara_uninit() {
    log("{\"event\":\"ara_uninit\"}");
}

fn log(line: &str) {
    if let Ok(path) = std::env::var("DS_ARA_SPIKE_OUT") {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

// ---------------------------------------------------------------- document controller

struct Controller {
    host: *const ARADocumentControllerHostInstance,
    instance: ARADocumentControllerInstance,
}

struct Shared {
    done: AtomicBool,
    cancel: AtomicBool,
    notes: Mutex<Vec<ARAContentNote>>,
}

struct Source {
    host_ref: ARAAudioSourceHostRef,
    name: String,
    sample_count: i64,
    sample_rate: f64,
    channels: i32,
    access: bool,
    host: *const ARADocumentControllerHostInstance,
    reader: ARAAudioReaderHostRef,
    worker: Option<std::thread::JoinHandle<()>>,
    shared: Arc<Shared>,
}

struct Reader {
    notes: Vec<ARAContentNote>,
}

struct Modification {
    source: *mut Source,
}

#[derive(Clone, Copy)]
struct RegionTimes {
    start_mod: f64,
    dur_mod: f64,
    start_play: f64,
}

struct Region {
    modification: *mut Modification,
    times: Mutex<RegionTimes>,
}

unsafe extern "C" fn create_document_controller(
    host: *const ARADocumentControllerHostInstance,
    _properties: *const ARADocumentProperties,
) -> *const ARADocumentControllerInstance {
    let c = Box::into_raw(Box::new(Controller { host, instance: std::mem::zeroed() }));
    (*c).instance = ARADocumentControllerInstance {
        structSize: std::mem::size_of::<ARADocumentControllerInstance>(),
        documentControllerRef: c as ARADocumentControllerRef,
        documentControllerInterface: &DC_INTERFACE.0,
    };
    log("{\"event\":\"create_document_controller\"}");
    &(*c).instance
}

unsafe extern "C" fn dc_destroyDocumentController(controllerRef: ARADocumentControllerRef) {
    drop(Box::from_raw(controllerRef as *mut Controller));
    log("{\"event\":\"destroy_document_controller\"}");
}
unsafe extern "C" fn dc_getFactory(_c: ARADocumentControllerRef) -> *const ARAFactory {
    &FACTORY.0
}
unsafe extern "C" fn dc_isLicensedForCapabilities(
    _c: ARADocumentControllerRef, _dialog: ARABool, _n: ARASize, _t: *const ARAContentType, _f: ARAPlaybackTransformationFlags,
) -> ARABool {
    1
}

unsafe fn text(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
}

unsafe fn apply(src: &mut Source, p: *const ARAAudioSourceProperties) {
    src.name = text((*p).name);
    src.sample_count = (*p).sampleCount;
    src.sample_rate = (*p).sampleRate;
    src.channels = (*p).channelCount;
}

unsafe extern "C" fn dc_createAudioSource(
    c: ARADocumentControllerRef, hostRef: ARAAudioSourceHostRef, properties: *const ARAAudioSourceProperties,
) -> ARAAudioSourceRef {
    let mut src = Source {
        host_ref: hostRef, name: String::new(), sample_count: 0, sample_rate: 0.0, channels: 0, access: false,
        host: (*(c as *mut Controller)).host, reader: std::ptr::null_mut(), worker: None,
        shared: Arc::new(Shared { done: AtomicBool::new(false), cancel: AtomicBool::new(false), notes: Mutex::new(Vec::new()) }),
    };
    apply(&mut src, properties);
    Box::into_raw(Box::new(src)) as ARAAudioSourceRef
}
unsafe extern "C" fn dc_updateAudioSourceProperties(
    _c: ARADocumentControllerRef, audioSourceRef: ARAAudioSourceRef, properties: *const ARAAudioSourceProperties,
) {
    apply(&mut *(audioSourceRef as *mut Source), properties);
}

unsafe fn stop_worker(src: &mut Source) {
    if let Some(h) = src.worker.take() {
        src.shared.cancel.store(true, Ordering::SeqCst);
        let _ = h.join();
        src.shared.cancel.store(false, Ordering::SeqCst);
    }
    destroy_reader(src);
}

/// ARA rule: a reader is created and destroyed on the host's main thread. Only the reads run elsewhere.
unsafe fn destroy_reader(src: &mut Source) {
    if !src.reader.is_null() {
        let iface = &*(*src.host).audioAccessControllerInterface;
        (iface.destroyAudioReader.unwrap())((*src.host).audioAccessControllerHostRef, src.reader);
        src.reader = std::ptr::null_mut();
    }
}

unsafe extern "C" fn dc_enableAudioSourceSamplesAccess(_c: ARADocumentControllerRef, audioSourceRef: ARAAudioSourceRef, enable: ARABool) {
    let src = &mut *(audioSourceRef as *mut Source);
    src.access = enable != 0;
    if src.access {
        start_analysis(src); // like a real plugin: analyse as soon as the samples are readable
    } else {
        stop_worker(src); // no read may run after this call returns
    }
}
unsafe extern "C" fn dc_destroyAudioSource(_c: ARADocumentControllerRef, audioSourceRef: ARAAudioSourceRef) {
    let mut src = Box::from_raw(audioSourceRef as *mut Source);
    stop_worker(&mut src);
}

/// The analysis: read every sample of the source through the host's audio access controller,
/// on a background thread, hash all samples, and take each rising edge above 0.5 on channel 0 as a hit.
unsafe extern "C" fn dc_requestAudioSourceContentAnalysis(
    controllerRef: ARADocumentControllerRef, audioSourceRef: ARAAudioSourceRef, _n: ARASize, _types: *const ARAContentType,
) {
    start_analysis(&mut *(audioSourceRef as *mut Source));
}

unsafe fn start_analysis(src: &mut Source) {
    if !src.access || src.worker.is_some() || src.shared.done.load(Ordering::SeqCst) {
        return;
    }
    let host = SyncPtr(src.host);
    {
        let iface = &*(*src.host).audioAccessControllerInterface;
        src.reader = (iface.createAudioReaderForSource.unwrap())((*src.host).audioAccessControllerHostRef, src.host_ref, 0);
    }
    let reader_ref = SyncPtr(src.reader);
    let (name, count, rate, ch) = (src.name.clone(), src.sample_count, src.sample_rate, src.channels.max(1) as usize);
    let shared = src.shared.clone();
    let caller = format!("{:?}", std::thread::current().id());
    src.worker = Some(std::thread::spawn(move || {
        let host = host;
        let reader_ref = reader_ref;
        let iface = &*(*host.0).audioAccessControllerInterface;
        let ctl = (*host.0).audioAccessControllerHostRef;
        let reader = reader_ref.0;
        const BLOCK: usize = 65536;
        let mut bufs: Vec<Vec<f32>> = (0..ch).map(|_| vec![0f32; BLOCK]).collect();
        let (mut pos, mut hash, mut ok, mut prev) = (0i64, 0xcbf29ce484222325u64, true, 0f32);
        let mut hits: Vec<i64> = Vec::new();
        let mut notes = Vec::new();
        while pos < count && !shared.cancel.load(Ordering::SeqCst) {
            let n = BLOCK.min((count - pos) as usize);
            let ptrs: Vec<*mut c_void> = bufs.iter_mut().map(|b| b.as_mut_ptr() as *mut c_void).collect();
            if (iface.readAudioSamples.unwrap())(ctl, reader, pos, n as i64, ptrs.as_ptr()) == 0 {
                ok = false;
                break;
            }
            for i in 0..n {
                for b in bufs.iter() {
                    for byte in b[i].to_le_bytes() {
                        hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
                    }
                }
                let v = bufs[0][i].abs();
                if v >= 0.5 && prev < 0.5 {
                    let idx = pos + i as i64;
                    hits.push(idx);
                    notes.push(ARAContentNote {
                        frequency: 0.0, // kARAInvalidFrequency: unpitched
                        pitchNumber: kARAInvalidPitchNumber,
                        volume: v,
                        startPosition: idx as f64 / rate,
                        attackDuration: 0.0,
                        noteDuration: 0.01,
                        signalDuration: 0.01,
                    });
                }
                prev = v;
            }
            pos += n as i64;
        }
        let hits_json: Vec<String> = hits.iter().map(|h| h.to_string()).collect();
        log(&format!(
            "{{\"event\":\"read\",\"name\":{:?},\"sample_count\":{},\"samples_read\":{},\"sample_rate\":{},\"channels\":{},\"read_ok\":{},\"fnv64\":\"{:016x}\",\"hits\":[{}],\"caller_thread\":{:?},\"reader_thread\":{:?}}}",
            name, count, pos, rate, ch, ok, hash, hits_json.join(","), caller, format!("{:?}", std::thread::current().id())
        ));
        *shared.notes.lock().unwrap() = notes;
        shared.done.store(true, Ordering::SeqCst);
    }));
}

unsafe extern "C" fn dc_isAudioSourceContentAnalysisIncomplete(_c: ARADocumentControllerRef, audioSourceRef: ARAAudioSourceRef, _t: ARAContentType) -> ARABool {
    let src = &mut *(audioSourceRef as *mut Source);
    if src.shared.done.load(Ordering::SeqCst) {
        stop_worker(src); // joins the finished thread and frees the reader, on the main thread
        return 0;
    }
    src.worker.is_some() as ARABool
}
unsafe extern "C" fn dc_isAudioSourceContentAvailable(_c: ARADocumentControllerRef, audioSourceRef: ARAAudioSourceRef, t: ARAContentType) -> ARABool {
    let src = &*(audioSourceRef as *mut Source);
    (t == kARAContentTypeNotes as ARAContentType && src.shared.done.load(Ordering::SeqCst)) as ARABool
}
unsafe extern "C" fn dc_getAudioSourceContentGrade(_c: ARADocumentControllerRef, _s: ARAAudioSourceRef, _t: ARAContentType) -> ARAContentGrade {
    kARAContentGradeDetected as ARAContentGrade
}
unsafe extern "C" fn dc_createAudioSourceContentReader(
    _c: ARADocumentControllerRef, audioSourceRef: ARAAudioSourceRef, _t: ARAContentType, _range: *const ARAContentTimeRange,
) -> ARAContentReaderRef {
    let src = &*(audioSourceRef as *mut Source);
    let notes = src.shared.notes.lock().unwrap().clone();
    log(&format!("{{\"event\":\"content_reader\",\"name\":{:?},\"notes\":{}}}", src.name, notes.len()));
    Box::into_raw(Box::new(Reader { notes })) as ARAContentReaderRef
}
unsafe extern "C" fn dc_getContentReaderEventCount(_c: ARADocumentControllerRef, r: ARAContentReaderRef) -> ARAInt32 {
    let reader: &Reader = &*(r as *mut Reader);
    reader.notes.len() as ARAInt32
}
unsafe extern "C" fn dc_getContentReaderDataForEvent(_c: ARADocumentControllerRef, r: ARAContentReaderRef, i: ARAInt32) -> *const c_void {
    let reader: &Reader = &*(r as *mut Reader);
    &reader.notes[i as usize] as *const ARAContentNote as *const c_void
}
unsafe extern "C" fn dc_destroyContentReader(_c: ARADocumentControllerRef, r: ARAContentReaderRef) {
    drop(Box::from_raw(r as *mut Reader));
}

unsafe extern "C" fn dc_createAudioModification(
    _c: ARADocumentControllerRef, audioSourceRef: ARAAudioSourceRef, _h: ARAAudioModificationHostRef, _p: *const ARAAudioModificationProperties,
) -> ARAAudioModificationRef {
    Box::into_raw(Box::new(Modification { source: audioSourceRef as *mut Source })) as ARAAudioModificationRef
}
unsafe extern "C" fn dc_cloneAudioModification(
    _c: ARADocumentControllerRef, m: ARAAudioModificationRef, _h: ARAAudioModificationHostRef, _p: *const ARAAudioModificationProperties,
) -> ARAAudioModificationRef {
    Box::into_raw(Box::new(Modification { source: (*(m as *mut Modification)).source })) as ARAAudioModificationRef
}
unsafe extern "C" fn dc_destroyAudioModification(_c: ARADocumentControllerRef, m: ARAAudioModificationRef) {
    drop(Box::from_raw(m as *mut Modification));
}
unsafe fn region_times(p: *const ARAPlaybackRegionProperties) -> RegionTimes {
    RegionTimes { start_mod: (*p).startInModificationTime, dur_mod: (*p).durationInModificationTime, start_play: (*p).startInPlaybackTime }
}
unsafe extern "C" fn dc_createPlaybackRegion(
    _c: ARADocumentControllerRef, m: ARAAudioModificationRef, _h: ARAPlaybackRegionHostRef, p: *const ARAPlaybackRegionProperties,
) -> ARAPlaybackRegionRef {
    Box::into_raw(Box::new(Region { modification: m as *mut Modification, times: Mutex::new(region_times(p)) })) as ARAPlaybackRegionRef
}
unsafe extern "C" fn dc_updatePlaybackRegionProperties(_c: ARADocumentControllerRef, r: ARAPlaybackRegionRef, p: *const ARAPlaybackRegionProperties) {
    let region: &Region = &*(r as *mut Region);
    *region.times.lock().unwrap() = region_times(p);
}
unsafe extern "C" fn dc_destroyPlaybackRegion(_c: ARADocumentControllerRef, r: ARAPlaybackRegionRef) {
    drop(Box::from_raw(r as *mut Region));
}

// playback renderer: the regions this plugin instance must render
unsafe extern "C" fn pr_addPlaybackRegion(rendererRef: ARAPlaybackRendererRef, r: ARAPlaybackRegionRef) {
    let plugin: &Plugin = &*(rendererRef as *mut Plugin);
    let region = r as *mut Region;
    // spike shortcut: wait here (main thread) until the analysis of this region's source is done
    let shared = (*(*(*region).modification).source).shared.clone();
    while !shared.done.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    plugin.regions.lock().unwrap().push(SyncPtr(region));
}
unsafe extern "C" fn pr_removePlaybackRegion(rendererRef: ARAPlaybackRendererRef, r: ARAPlaybackRegionRef) {
    let plugin: &Plugin = &*(rendererRef as *mut Plugin);
    plugin.regions.lock().unwrap().retain(|x| x.0 != r as *mut Region);
}

// ---------------------------------------------------------------- CLAP plugin

const PLUGIN_ID: *const c_char = cs!("com.cherryblossomvalley.drumspike.araspike");
const EXT_ARA_FACTORY: &[u8] = b"org.ara-audio.ara.factory/2";
const EXT_ARA_FACTORY_DRAFT: &[u8] = b"org.ara-audio.ara.factory.draft/2";
const EXT_ARA_PLUGIN: &[u8] = b"org.ara-audio.ara.pluginextension/2";
const EXT_ARA_PLUGIN_DRAFT: &[u8] = b"org.ara-audio.ara.pluginextension.draft/2";

static FEATURES: SyncPtr<[*const c_char; 4]> = SyncPtr([cs!("audio-effect"), cs!("ara:supported"), cs!("ara:required"), std::ptr::null()]);

static DESCRIPTOR: clap_plugin_descriptor = clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: PLUGIN_ID,
    name: cs!("Drum Spike ARA Spike"),
    vendor: cs!("Cherry Blossom Valley"),
    url: cs!("https://github.com/Cherry-Blossom-Valley/drumspike"),
    manual_url: cs!(""),
    support_url: cs!(""),
    version: cs!("0.0.1"),
    description: cs!("Spike: ARA 2 from Rust through CLAP"),
    features: &FEATURES.0 as *const [*const c_char; 4] as *const *const c_char,
};

#[repr(C)]
struct clap_ara_factory {
    get_factory_count: Option<unsafe extern "C" fn(*const clap_ara_factory) -> u32>,
    get_ara_factory: Option<unsafe extern "C" fn(*const clap_ara_factory, u32) -> *const ARAFactory>,
    get_plugin_id: Option<unsafe extern "C" fn(*const clap_ara_factory, u32) -> *const c_char>,
}
#[repr(C)]
struct clap_ara_plugin_extension {
    get_factory: Option<unsafe extern "C" fn(*const clap_plugin) -> *const ARAFactory>,
    bind_to_document_controller:
        Option<unsafe extern "C" fn(*const clap_plugin, ARADocumentControllerRef, ARAPlugInInstanceRoleFlags, ARAPlugInInstanceRoleFlags) -> *const ARAPlugInExtensionInstance>,
}

unsafe extern "C" fn araf_count(_f: *const clap_ara_factory) -> u32 { 1 }
unsafe extern "C" fn araf_get(_f: *const clap_ara_factory, i: u32) -> *const ARAFactory { if i == 0 { &FACTORY.0 } else { std::ptr::null() } }
unsafe extern "C" fn araf_id(_f: *const clap_ara_factory, i: u32) -> *const c_char { if i == 0 { PLUGIN_ID } else { std::ptr::null() } }
static ARA_FACTORY_EXT: clap_ara_factory = clap_ara_factory { get_factory_count: Some(araf_count), get_ara_factory: Some(araf_get), get_plugin_id: Some(araf_id) };

struct Plugin {
    clap: clap_plugin,
    ext: ARAPlugInExtensionInstance,
    sample_rate: Mutex<f64>,
    regions: Mutex<Vec<SyncPtr<*mut Region>>>,
}

unsafe extern "C" fn pext_factory(_p: *const clap_plugin) -> *const ARAFactory { &FACTORY.0 }
unsafe extern "C" fn pext_bind(
    p: *const clap_plugin, _dc: ARADocumentControllerRef, known: ARAPlugInInstanceRoleFlags, assigned: ARAPlugInInstanceRoleFlags,
) -> *const ARAPlugInExtensionInstance {
    let plugin = (*p).plugin_data as *mut Plugin;
    let me = plugin as *mut c_void;
    let has = |role: u32| assigned & role as i32 != 0;
    (*plugin).ext = ARAPlugInExtensionInstance {
        structSize: std::mem::size_of::<ARAPlugInExtensionInstance>(),
        plugInExtensionRef: std::ptr::null_mut(),
        plugInExtensionInterface: std::ptr::null(),
        playbackRendererRef: if has(kARAPlaybackRendererRole) { me as _ } else { std::ptr::null_mut() },
        playbackRendererInterface: if has(kARAPlaybackRendererRole) { &PR_INTERFACE.0 } else { std::ptr::null() },
        editorRendererRef: if has(kARAEditorRendererRole) { me as _ } else { std::ptr::null_mut() },
        editorRendererInterface: if has(kARAEditorRendererRole) { &ER_INTERFACE.0 } else { std::ptr::null() },
        editorViewRef: if has(kARAEditorViewRole) { me as _ } else { std::ptr::null_mut() },
        editorViewInterface: if has(kARAEditorViewRole) { &EV_INTERFACE.0 } else { std::ptr::null() },
    };
    log(&format!("{{\"event\":\"bind_to_document_controller\",\"known_roles\":{known},\"assigned_roles\":{assigned}}}"));
    &(*plugin).ext
}
static ARA_PLUGIN_EXT: clap_ara_plugin_extension = clap_ara_plugin_extension { get_factory: Some(pext_factory), bind_to_document_controller: Some(pext_bind) };

unsafe extern "C" fn ports_count(_p: *const clap_plugin, _is_input: bool) -> u32 { 1 }
unsafe extern "C" fn ports_get(_p: *const clap_plugin, index: u32, is_input: bool, info: *mut clap_audio_port_info) -> bool {
    if index != 0 { return false; }
    let mut name = [0 as c_char; 256];
    for (i, b) in (if is_input { "In" } else { "Out" }).bytes().enumerate() { name[i] = b as c_char; }
    *info = clap_audio_port_info { id: 0, name, flags: CLAP_AUDIO_PORT_IS_MAIN, channel_count: 1, port_type: CLAP_PORT_MONO.as_ptr(), in_place_pair: 0 };
    true
}
static AUDIO_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports { count: Some(ports_count), get: Some(ports_get) };

unsafe extern "C" fn p_init(_p: *const clap_plugin) -> bool { true }
unsafe extern "C" fn p_destroy(p: *const clap_plugin) { drop(Box::from_raw((*p).plugin_data as *mut Plugin)); }
unsafe extern "C" fn p_activate(p: *const clap_plugin, sr: f64, _min: u32, _max: u32) -> bool {
    let plugin: &Plugin = &*((*p).plugin_data as *mut Plugin);
    *plugin.sample_rate.lock().unwrap() = sr;
    true
}
unsafe extern "C" fn p_deactivate(_p: *const clap_plugin) {}
unsafe extern "C" fn p_start(_p: *const clap_plugin) -> bool { true }
unsafe extern "C" fn p_stop(_p: *const clap_plugin) {}
unsafe extern "C" fn p_reset(_p: *const clap_plugin) {}
/// Playback renderer: silence, plus one click (one sample at 0.9) at the first sample of each hit.
/// Hit position on the timeline = region start in playback time + (hit time in the source - region start in the source).
unsafe extern "C" fn p_process(p: *const clap_plugin, process: *const clap_process) -> clap_process_status {
    let plugin: &Plugin = &*((*p).plugin_data as *mut Plugin);
    let pr = &*process;
    let frames = pr.frames_count as usize;
    for o in 0..pr.audio_outputs_count as usize {
        let buf = &*pr.audio_outputs.add(o);
        for c in 0..buf.channel_count as usize {
            if !buf.data32.is_null() {
                std::ptr::write_bytes(*buf.data32.add(c), 0, frames);
            }
        }
    }
    if pr.transport.is_null() || pr.audio_outputs_count == 0 {
        return CLAP_PROCESS_CONTINUE;
    }
    let sr = *plugin.sample_rate.lock().unwrap();
    let pos_seconds = (*pr.transport).song_pos_seconds as f64 / clap_sys::fixedpoint::CLAP_SECTIME_FACTOR as f64;
    let block_start = (pos_seconds * sr).round() as i64;
    let out = &*pr.audio_outputs;
    let mut written: Vec<i64> = Vec::new();
    for region in plugin.regions.lock().unwrap().iter() {
        let region: &Region = &*region.0;
        let t = *region.times.lock().unwrap();
        let src: &Source = &*(*region.modification).source;
        for note in src.shared.notes.lock().unwrap().iter() {
            let t_mod = note.startPosition;
            if t_mod < t.start_mod || t_mod >= t.start_mod + t.dur_mod {
                continue;
            }
            let idx = ((t.start_play + (t_mod - t.start_mod)) * sr).round() as i64 - block_start;
            if idx >= 0 && (idx as usize) < frames {
                for c in 0..out.channel_count as usize {
                    *(*out.data32.add(c)).add(idx as usize) = 0.9;
                }
                written.push(block_start + idx);
            }
        }
    }
    if !written.is_empty() {
        let w: Vec<String> = written.iter().map(|x| x.to_string()).collect();
        log(&format!("{{\"event\":\"render\",\"sample_rate\":{sr},\"clicks\":[{}]}}", w.join(",")));
    }
    CLAP_PROCESS_CONTINUE
}
unsafe extern "C" fn p_get_extension(_p: *const clap_plugin, id: *const c_char) -> *const c_void {
    let id = CStr::from_ptr(id).to_bytes();
    if id == CLAP_EXT_AUDIO_PORTS.to_bytes() { return &AUDIO_PORTS as *const _ as *const c_void; }
    if id == EXT_ARA_PLUGIN || id == EXT_ARA_PLUGIN_DRAFT { return &ARA_PLUGIN_EXT as *const _ as *const c_void; }
    std::ptr::null()
}
unsafe extern "C" fn p_on_main_thread(_p: *const clap_plugin) {}

unsafe extern "C" fn f_count(_f: *const clap_plugin_factory) -> u32 { 1 }
unsafe extern "C" fn f_desc(_f: *const clap_plugin_factory, i: u32) -> *const clap_plugin_descriptor { if i == 0 { &DESCRIPTOR } else { std::ptr::null() } }
unsafe extern "C" fn f_create(_f: *const clap_plugin_factory, _host: *const clap_host, id: *const c_char) -> *const clap_plugin {
    if CStr::from_ptr(id) != CStr::from_ptr(PLUGIN_ID) { return std::ptr::null(); }
    let p = Box::into_raw(Box::new(Plugin {
        clap: clap_plugin {
            desc: &DESCRIPTOR, plugin_data: std::ptr::null_mut(), init: Some(p_init), destroy: Some(p_destroy), activate: Some(p_activate),
            deactivate: Some(p_deactivate), start_processing: Some(p_start), stop_processing: Some(p_stop), reset: Some(p_reset),
            process: Some(p_process), get_extension: Some(p_get_extension), on_main_thread: Some(p_on_main_thread),
        },
        ext: std::mem::zeroed(),
        sample_rate: Mutex::new(0.0),
        regions: Mutex::new(Vec::new()),
    }));
    (*p).clap.plugin_data = p as *mut c_void;
    log("{\"event\":\"create_plugin\"}");
    &(*p).clap
}
static PLUGIN_FACTORY: clap_plugin_factory = clap_plugin_factory { get_plugin_count: Some(f_count), get_plugin_descriptor: Some(f_desc), create_plugin: Some(f_create) };

unsafe extern "C" fn e_init(_path: *const c_char) -> bool { true }
unsafe extern "C" fn e_deinit() {}
unsafe extern "C" fn e_get_factory(id: *const c_char) -> *const c_void {
    let id = CStr::from_ptr(id).to_bytes();
    if id == CLAP_PLUGIN_FACTORY_ID.to_bytes() { return &PLUGIN_FACTORY as *const _ as *const c_void; }
    if id == EXT_ARA_FACTORY || id == EXT_ARA_FACTORY_DRAFT { return &ARA_FACTORY_EXT as *const _ as *const c_void; }
    std::ptr::null()
}

#[no_mangle]
#[used]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry { clap_version: CLAP_VERSION, init: Some(e_init), deinit: Some(e_deinit), get_factory: Some(e_get_factory) };
