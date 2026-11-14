//! The test host of the service spike: loads the CLAP bundle with dlopen, switches the
//! plugin's service job on through the environment, and processes audio blocks while the
//! analysis runs. Each block is measured against its own duration
//! (`frames / sample_rate`): one block over its budget is an overrun and fails the run
//! (no analysis may run on the audio thread). With `DS_SERVICE_SPIKE_PANIC=process` the
//! plugin panics in every `process` call; the host proves the process lives by finishing
//! its blocks and exiting 0. The report is the last line on stdout (JSON).
//!
//! usage: service-spike-host <path-to-.clap-bundle-or-its-binary> [--frames N] [--blocks N]

use clap_sys::entry::clap_plugin_entry;
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::process::{clap_process, clap_process_status};
use clap_sys::version::CLAP_VERSION;
use libloading::os::unix::{Library, Symbol};
use std::ffi::{c_char, c_void, CString};
use std::time::{Duration, Instant};

const PLUGIN_ID: &[u8] = b"com.cherryblossomvalley.drumspike.servicespike\0";
const FRAMES: usize = 512;
const SAMPLE_RATE: f64 = 48000.0;
/// About 10 s of audio at 512 frames: more than enough time for a small analysis.
const MAX_BLOCKS: usize = 200_000;

macro_rules! cs {
    ($s:literal) => {
        concat!($s, "\0").as_ptr() as *const c_char
    };
}

static HOST: clap_host = clap_host {
    clap_version: CLAP_VERSION,
    host_data: std::ptr::null_mut(),
    name: cs!("service-spike-host"),
    vendor: cs!("Cherry Blossom Valley"),
    url: cs!("https://github.com/Cherry-Blossom-Valley/drumspike"),
    version: cs!("0.0.1"),
    get_extension: None,
    request_restart: None,
    request_process: None,
    request_callback: None,
};

unsafe extern "C" fn in_events_size(_list: *const clap_sys::events::clap_input_events) -> u32 {
    0
}
unsafe extern "C" fn in_events_get(_list: *const clap_sys::events::clap_input_events, _index: u32) -> *const clap_sys::events::clap_event_header {
    std::ptr::null()
}
unsafe extern "C" fn out_events_try_push(_list: *const clap_sys::events::clap_output_events, _event: *const clap_sys::events::clap_event_header) -> bool {
    false
}

/// The binary inside the bundle: a directory path becomes `<bundle>/Contents/MacOS/<name>`.
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
    p.ancestors().nth(2).map(|a| a.display().to_string()).unwrap_or_else(|| p.display().to_string())
}

fn arg_value(args: &[String], name: &str, default: usize) -> usize {
    let mut out = default;
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == name {
            out = args[i + 1].parse().unwrap_or(default);
        }
        i += 1;
    }
    out
}

/// The init of the CLAP entry, as the CLAP ABI declares it (`plugin_path`, `factory_id`).
/// clap-sys 0.5.0 names one argument; the entry reads neither, but hosts pass two.
type EntryInit = unsafe extern "C" fn(*const c_char, *const c_char) -> bool;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(plugin_path) = args.first() else {
        eprintln!("usage: service-spike-host <path-to-.clap-bundle-or-its-binary> [--frames N] [--blocks N]");
        std::process::exit(64);
    };
    let frames = arg_value(&args, "--frames", FRAMES);
    let max_blocks = arg_value(&args, "--blocks", MAX_BLOCKS);
    let wait_file = std::env::var("DS_SERVICE_SPIKE_OUT").ok().filter(|s| !s.is_empty());
    let panic_mode = std::env::var("DS_SERVICE_SPIKE_PANIC").as_deref() == Ok("process");

    unsafe {
        let bin = binary_of(plugin_path);
        let bundle = bundle_of(plugin_path);
        let lib = match Library::new(&bin) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("service-spike-host: dlopen {bin:?} failed: {e}");
                std::process::exit(2);
            }
        };
        let entry: Symbol<*mut c_void> = match lib.get(b"clap_entry") {
            Ok(s) => s,
            Err(e) => {
                eprintln!("service-spike-host: no clap_entry in {bin:?}: {e}");
                std::process::exit(2);
            }
        };
        // the dlsym result is the address of the static clap_entry itself
        let entry = entry.into_raw() as *const clap_plugin_entry;
        let plugin_path_c = CString::new(bundle).unwrap_or_default();
        let factory_id = CString::new(CLAP_PLUGIN_FACTORY_ID.to_bytes()).unwrap_or_default();
        let init: EntryInit = std::mem::transmute(((*entry).init).expect("clap_entry.init"));
        if !init(plugin_path_c.as_ptr(), factory_id.as_ptr()) {
            eprintln!("service-spike-host: clap_entry.init refused");
            std::process::exit(2);
        }
        let factory = ((*entry).get_factory).expect("clap_entry.get_factory")(factory_id.as_ptr()) as *const clap_plugin_factory;
        if factory.is_null() {
            eprintln!("service-spike-host: no plugin factory");
            std::process::exit(2);
        }
        let id = CString::new(&PLUGIN_ID[..PLUGIN_ID.len() - 1]).unwrap_or_default();
        let plugin = ((*factory).create_plugin).expect("create_plugin")(factory, &HOST, id.as_ptr());
        if plugin.is_null() {
            eprintln!("service-spike-host: create_plugin returned null");
            std::process::exit(2);
        }
        if !((*plugin).init.expect("init"))(plugin) {
            eprintln!("service-spike-host: plugin init refused");
            std::process::exit(2);
        }
        if !((*plugin).activate.expect("activate"))(plugin, SAMPLE_RATE, 64, 4096) {
            eprintln!("service-spike-host: activate refused");
            std::process::exit(2);
        }
        if !((*plugin).start_processing.expect("start_processing"))(plugin) {
            eprintln!("service-spike-host: start_processing refused");
            std::process::exit(2);
        }

        // one mono input pair, one mono output pair, no events
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
        let mut statuses: std::collections::HashMap<clap_process_status, usize> = std::collections::HashMap::new();
        let mut saw_event_file = wait_file.as_deref().map(std::path::Path::new).map(std::path::Path::exists).unwrap_or(true);
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
            let status = ((*plugin).process.expect("process"))(plugin, &process);
            let took = t0.elapsed();
            *statuses.entry(status).or_default() += 1;
            if took > budget {
                overruns += 1;
                if took > worst {
                    worst = took;
                }
            }
            blocks += 1;
            if panic_mode && blocks >= 128 {
                break; // the panic round: enough blocks to prove the boundary holds
            }
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
        ((*entry).deinit).expect("deinit")();
        drop(lib);

        let budget_us = (budget.as_secs_f64() * 1e6).round() as u64;
        let worst_us = (worst.as_secs_f64() * 1e6).round() as u64;
        let mut st: Vec<String> = statuses.into_iter().map(|(s, n)| format!("{s}:{n}")).collect();
        st.sort();
        let report = format!(
            "{{\"blocks\":{blocks},\"frames\":{frames},\"sample_rate\":{SAMPLE_RATE},\"overruns\":{overruns},\"worst_block_us\":{worst_us},\"budget_us\":{budget_us},\"event_file\":{saw_event_file},\"panic_mode\":{panic_mode},\"statuses\":\"{}\"}}",
            st.join(",")
        );
        println!("{report}");
        let ok = overruns == 0 && saw_event_file;
        std::process::exit(if ok { 0 } else { 2 });
    }
}
