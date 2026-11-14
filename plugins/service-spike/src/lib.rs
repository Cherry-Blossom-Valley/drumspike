//! Drum Spike service spike (S3, #506): a CLAP plugin in Rust that runs the app service
//! (`drumspike-app`) as a normal crate, wrapped as VST3 by `clap-wrapper`. Throwaway code.
//!
//! One chain: host -> CLAP -> the service in the plugin process -> the event list of the
//! fixture project, byte for byte the one `service_tool` prints for the same fixture.
//!
//! The test switch is the environment (the test host sets it before it loads the plugin):
//!
//! - `DS_SERVICE_SPIKE_PROJECT`: the fixture project folder to analyse.
//! - `DS_SERVICE_SPIKE_OUT`: where the plugin writes the event list (one JSON object).
//! - `DS_SERVICE_SPIKE_META`: where the plugin writes `{"analysis_ms": ...}`.
//! - `DS_SERVICE_SPIKE_PANIC`: `init` or `process`: a forced panic in that entry point.
//!
//! No analysis runs on the audio thread: the job runs on its own thread; `process` only
//! passes audio through. Every CLAP entry point runs inside [`guard`], so a panic of the
//! service is caught at the boundary and the process lives.
#![allow(non_upper_case_globals, dead_code)]

use clap_sys::entry::clap_plugin_entry;
use clap_sys::ext::audio_ports::*;
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};
use clap_sys::version::CLAP_VERSION;
use drumspike_app::error::AppError;
use drumspike_app::reply::Reply;
use drumspike_app::{Service, ServiceConfig};
use std::ffi::{c_char, c_void, CStr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub(crate) struct SyncPtr<T>(pub T);
unsafe impl<T> Sync for SyncPtr<T> {}
unsafe impl<T> Send for SyncPtr<T> {}

macro_rules! cs {
    ($s:literal) => {
        concat!($s, "\0").as_ptr() as *const c_char
    };
}

/// The CLAP boundary: every entry point runs its body inside `catch_unwind`. A panic that
/// reached the C ABI would abort the host; caught here, the entry point answers its default
/// (false, null, `CLAP_PROCESS_ERROR`) and the process lives.
fn guard<T: Default>(where_: &'static str, body: impl FnOnce() -> T + std::panic::UnwindSafe) -> T {
    match std::panic::catch_unwind(body) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("service-spike: caught panic in {where_}");
            T::default()
        }
    }
}

/// The forced panic of the test switch: a panic in the named entry point, before any work.
fn forced_panic(where_: &str) {
    if std::env::var("DS_SERVICE_SPIKE_PANIC").as_deref() == Ok(where_) {
        panic!("service-spike: forced panic in {where_}");
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name).ok().filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// One call through the plugin's service: the app's own dispatch ([`drumspike_app::call::call`]),
/// inside the boundary guard. The tests of the crate answer it against the direct call.
///
/// # Errors
///
/// The error of the command; the caught-panic error when the boundary caught one.
pub fn service_call(svc: &Service, name: &str, args: &serde_json::Value) -> Result<Reply, AppError> {
    guard("service_call", move || Some(drumspike_app::call::call(svc, name, args, None)))
        .unwrap_or_else(|| Err(AppError::bad(format!("service-spike: caught panic in service_call ({name})"))))
}

static ANALYSIS_STARTED: AtomicBool = AtomicBool::new(false);

/// The service job: `drumspike_app::headless::analyze_projects` on the fixture project, on its
/// own thread. The event list (the `events` object of the answer, exactly the bytes
/// `service_tool` prints) is written to `$DS_SERVICE_SPIKE_OUT` under a temp name, then renamed,
/// so a reader never sees half a file. One job per process: later instances find the flag set.
fn start_service_job() {
    let (Some(project), Some(out)) = (env_path("DS_SERVICE_SPIKE_PROJECT"), env_path("DS_SERVICE_SPIKE_OUT")) else {
        return;
    };
    if ANALYSIS_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let meta = env_path("DS_SERVICE_SPIKE_META");
    let spawn = std::thread::Builder::new().name("service-spike-analysis".into()).spawn(move || {
        let started = std::time::Instant::now();
        // the service as a normal crate: the models dir is unused (the detector is DSP-only)
        let svc = Service::new(ServiceConfig::from_env(std::path::Path::new("/")));
        let dirs = vec![project.display().to_string()];
        let answer = drumspike_app::headless::analyze_projects(&svc, &dirs);
        let analysis_ms = started.elapsed().as_millis() as u64;
        let mut tmp = out.clone().into_os_string();
        tmp.push(".tmp");
        let error = match answer.ok().and_then(|one| one.into_iter().next()) {
            Some(result) => {
                match std::fs::write(&tmp, serde_json::to_string(&result["events"]).unwrap_or_default()) {
                    Ok(()) => None,
                    Err(_) => Some("the event list could not be written".to_string()),
                }
            }
            None => Some("the analysis of the fixture failed".to_string()),
        };
        // the meta first, the event list last: the rename is the completion signal, and a
        // reader that sees the event list must also see the finished meta
        if let Some(m) = meta {
            let body = match &error {
                Some(e) => format!("{{\"analysis_ms\":{analysis_ms},\"error\":{e:?}}}"),
                None => format!("{{\"analysis_ms\":{analysis_ms}}}"),
            };
            let _ = std::fs::write(&m, body);
        }
        if error.is_none() {
            let _ = std::fs::rename(&tmp, &out);
        }
    });
    if let Err(e) = spawn {
        eprintln!("service-spike: the analysis thread did not start: {e}");
    }
}

// ---------------------------------------------------------------- the CLAP plugin

const PLUGIN_ID: *const c_char = cs!("com.cherryblossomvalley.drumspike.servicespike");

static FEATURES: SyncPtr<[*const c_char; 2]> = SyncPtr([cs!("audio-effect"), std::ptr::null()]);

static DESCRIPTOR: clap_plugin_descriptor = clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: PLUGIN_ID,
    name: cs!("Drum Spike Service Spike"),
    vendor: cs!("Cherry Blossom Valley"),
    url: cs!("https://github.com/Cherry-Blossom-Valley/drumspike"),
    manual_url: cs!(""),
    support_url: cs!(""),
    version: cs!("0.0.1"),
    description: cs!("Spike: the app service inside a CLAP plugin"),
    features: &FEATURES.0 as *const [*const c_char; 2] as *const *const c_char,
};

struct Plugin {
    clap: clap_plugin,
    sample_rate: Mutex<f64>,
}

unsafe extern "C" fn ports_count(_p: *const clap_plugin, _is_input: bool) -> u32 {
    guard("ports_count", move || 1u32)
}

unsafe extern "C" fn ports_get(_p: *const clap_plugin, index: u32, is_input: bool, info: *mut clap_audio_port_info) -> bool {
    guard("ports_get", move || {
        if index != 0 {
            return false;
        }
        let (id, name, pair) = if is_input { (0u32, "Input", 1u32) } else { (1u32, "Output", 0u32) };
        let mut nm = [0 as c_char; 256];
        for (i, b) in name.bytes().enumerate() {
            nm[i] = b as c_char;
        }
        *info = clap_audio_port_info {
            id,
            name: nm,
            flags: CLAP_AUDIO_PORT_IS_MAIN,
            channel_count: 1,
            port_type: CLAP_PORT_MONO.as_ptr(),
            in_place_pair: pair,
        };
        true
    })
}

static AUDIO_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports { count: Some(ports_count), get: Some(ports_get) };

unsafe extern "C" fn p_init(_p: *const clap_plugin) -> bool {
    guard("init", move || {
        forced_panic("init");
        start_service_job();
        true
    })
}

unsafe extern "C" fn p_destroy(p: *const clap_plugin) {
    drop(Box::from_raw((*p).plugin_data as *mut Plugin));
}

unsafe extern "C" fn p_activate(p: *const clap_plugin, sample_rate: f64, _min: u32, _max: u32) -> bool {
    guard("activate", move || {
        let plugin: &Plugin = &*((*p).plugin_data as *mut Plugin);
        *plugin.sample_rate.lock().unwrap_or_else(|e| e.into_inner()) = sample_rate;
        true
    })
}

unsafe extern "C" fn p_deactivate(_p: *const clap_plugin) {}
unsafe extern "C" fn p_start(_p: *const clap_plugin) -> bool {
    guard("start_processing", move || true)
}
unsafe extern "C" fn p_stop(_p: *const clap_plugin) {}
unsafe extern "C" fn p_reset(_p: *const clap_plugin) {}

/// The audio thread: pass the first mono pair through (silence without an input), nothing else.
/// The service runs on its own thread and is never touched here.
unsafe extern "C" fn p_process(_p: *const clap_plugin, process: *const clap_process) -> clap_process_status {
    guard("process", move || {
        forced_panic("process");
        let pr = &*process;
        let frames = pr.frames_count as usize;
        if pr.audio_outputs_count > 0 {
            let out = &*pr.audio_outputs;
            let input = if pr.audio_inputs_count > 0 { &*pr.audio_inputs } else { std::ptr::null() };
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
    guard("get_extension", move || {
        let id = CStr::from_ptr(id).to_bytes();
        if id == CLAP_EXT_AUDIO_PORTS.to_bytes() {
            return &AUDIO_PORTS as *const _ as *const c_void;
        }
        std::ptr::null()
    })
}

unsafe extern "C" fn p_on_main_thread(_p: *const clap_plugin) {}

unsafe extern "C" fn f_count(_f: *const clap_plugin_factory) -> u32 {
    1
}
unsafe extern "C" fn f_desc(_f: *const clap_plugin_factory, i: u32) -> *const clap_plugin_descriptor {
    if i == 0 { &DESCRIPTOR } else { std::ptr::null() }
}
unsafe extern "C" fn f_create(_f: *const clap_plugin_factory, _host: *const clap_host, id: *const c_char) -> *const clap_plugin {
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
        sample_rate: Mutex::new(0.0),
    }));
    (*p).clap.plugin_data = p as *mut std::ffi::c_void;
    &(*p).clap
}

static PLUGIN_FACTORY: clap_plugin_factory =
    clap_plugin_factory { get_plugin_count: Some(f_count), get_plugin_descriptor: Some(f_desc), create_plugin: Some(f_create) };

unsafe extern "C" fn e_init(_path: *const c_char) -> bool {
    guard("entry init", move || true)
}
unsafe extern "C" fn e_deinit() {}
unsafe extern "C" fn e_get_factory(id: *const c_char) -> *const std::ffi::c_void {
    let id = CStr::from_ptr(id).to_bytes();
    if id == CLAP_PLUGIN_FACTORY_ID.to_bytes() {
        return &PLUGIN_FACTORY as *const _ as *const std::ffi::c_void;
    }
    std::ptr::null()
}

/// The CLAP entry the loader looks for (the bundle carries it in its binary).
#[no_mangle]
#[used]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry { clap_version: CLAP_VERSION, init: Some(e_init), deinit: Some(e_deinit), get_factory: Some(e_get_factory) };
