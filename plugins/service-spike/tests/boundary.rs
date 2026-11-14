//! The tests of the gate (#506, item 2): a call of a known command through the plugin's
//! service answers the same JSON as `drumspike_app::call`, and a forced panic inside a
//! plugin entry point is caught at the CLAP boundary (the process lives). The two phases
//! that touch the test switch share one test: they set the same process environment.

use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, CLAP_PROCESS_CONTINUE, CLAP_PROCESS_ERROR};
use clap_sys::version::CLAP_VERSION;
use drumspike_app::{Service, ServiceConfig};
use ds_service_spike::clap_entry;
use serde_json::json;
use std::ffi::{c_char, CString};

macro_rules! cs {
    ($s:literal) => {
        concat!($s, "\0").as_ptr() as *const c_char
    };
}

static TEST_HOST: clap_host = clap_host {
    clap_version: CLAP_VERSION,
    host_data: std::ptr::null_mut(),
    name: cs!("service-spike-test"),
    vendor: cs!("Cherry Blossom Valley"),
    url: cs!("https://github.com/Cherry-Blossom-Valley/drumspike"),
    version: cs!("0.0.1"),
    get_extension: None,
    request_restart: None,
    request_process: None,
    request_callback: None,
};

/// A plugin instance through the exported CLAP entry, like a host builds it.
unsafe fn create_plugin() -> *const clap_plugin {
    let factory_id = CString::new(CLAP_PLUGIN_FACTORY_ID.to_bytes()).unwrap_or_default();
    let factory = (clap_entry.get_factory.expect("get_factory")(factory_id.as_ptr())) as *const clap_plugin_factory;
    assert!(!factory.is_null(), "no plugin factory");
    // clap-sys 0.5.0 names one argument of init; the CLAP ABI has two (path, factory id)
    let init: unsafe extern "C" fn(*const c_char, *const c_char) -> bool =
        std::mem::transmute(clap_entry.init.expect("init"));
    let bundle = CString::new("service-spike-test-bundle").unwrap_or_default();
    assert!(init(bundle.as_ptr(), factory_id.as_ptr()), "entry init refused");
    let id = CString::new("com.cherryblossomvalley.drumspike.servicespike").unwrap_or_default();
    let plugin = ((*factory).create_plugin).expect("create_plugin")(factory, &TEST_HOST, id.as_ptr());
    assert!(!plugin.is_null(), "create_plugin returned null");
    plugin
}

unsafe fn one_block_process(
    plugin: *const clap_plugin,
    frames: usize,
) -> (clap_sys::process::clap_process_status, Vec<f32>, Vec<f32>) {
    let mut input = vec![0.0f32; frames];
    for (i, s) in input.iter_mut().enumerate() {
        *s = ((i % 97) as f32 / 97.0) - 0.5;
    }
    let mut output = vec![-1.0f32; frames];
    let in_buf = clap_sys::audio_buffer::clap_audio_buffer {
        data32: &mut input.as_mut_ptr(),
        data64: std::ptr::null_mut(),
        channel_count: 1,
        latency: 0,
        constant_mask: 0,
    };
    let mut out_buf = clap_sys::audio_buffer::clap_audio_buffer {
        data32: &mut output.as_mut_ptr(),
        data64: std::ptr::null_mut(),
        channel_count: 1,
        latency: 0,
        constant_mask: 0,
    };
    let process = clap_process {
        steady_time: 0,
        frames_count: frames as u32,
        transport: std::ptr::null(),
        audio_inputs: &in_buf,
        audio_outputs: &mut out_buf,
        audio_inputs_count: 1,
        audio_outputs_count: 1,
        in_events: std::ptr::null(),
        out_events: std::ptr::null(),
    };
    let status = ((*plugin).process.expect("process"))(plugin, &process);
    (status, input, output)
}

#[test]
fn service_call_through_the_plugin_answers_the_same_json_as_the_direct_call() {
    let svc = Service::new(ServiceConfig::default());
    let cases = [
        ("app_info", json!({})),
        ("project_list", json!({})),
        ("nope", json!({})),
    ];
    for (name, args) in cases {
        let through = ds_service_spike::service_call(&svc, name, &args);
        let direct = drumspike_app::call::call(&svc, name, &args, None);
        assert_eq!(
            through, direct,
            "{name}: the plugin's service answers another JSON than the app"
        );
    }
}

#[test]
fn the_entry_points_survive_a_forced_panic_and_a_block_passes_through() {
    unsafe {
        // 1. the panic phases: the guarded entry points answer their default, the process lives
        std::env::set_var("DS_SERVICE_SPIKE_PANIC", "init");
        let plugin = create_plugin();
        assert!(
            !((*plugin).init.expect("init"))(plugin),
            "init survived the panic as false"
        );

        std::env::set_var("DS_SERVICE_SPIKE_PANIC", "process");
        assert!(
            ((*plugin).activate.expect("activate"))(plugin, 48000.0, 64, 4096),
            "activate survived"
        );
        let (status, _, _) = one_block_process(plugin, 64);
        assert_eq!(
            status, CLAP_PROCESS_ERROR,
            "the caught panic answers the process error status"
        );
        // the process lives: the test goes on (an uncaught panic here would abort the runner)
        assert!(
            ((*plugin).start_processing.expect("start_processing"))(plugin),
            "start_processing survived"
        );
        ((*plugin).destroy.expect("destroy"))(plugin);

        // 2. without the switch: the block passes through unchanged
        std::env::remove_var("DS_SERVICE_SPIKE_PANIC");
        let plugin = create_plugin();
        assert!(((*plugin).init.expect("init"))(plugin), "init");
        assert!(
            ((*plugin).activate.expect("activate"))(plugin, 48000.0, 64, 4096),
            "activate"
        );
        let (status, input, output) = one_block_process(plugin, 512);
        assert_eq!(status, CLAP_PROCESS_CONTINUE, "the block processes");
        assert_eq!(output, input, "the block passed through unchanged");
        ((*plugin).destroy.expect("destroy"))(plugin);
    }
}
