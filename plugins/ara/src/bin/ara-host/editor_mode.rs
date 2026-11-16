//! The `editor` mode of the test host (M1b2): the host analyses the files in an ARA document,
//! binds a plugin instance to it, opens the plugin's editor in a real AppKit window (the
//! CLAP GUI extension, cocoa, child of the content view) and runs one phase of the check. A
//! phase ends when the page's report arrives in the plugin's report file (DS_ARA_OUT). The
//! main thread pumps the run loop in slices: the page's timers, the compositor, the main
//! dispatch queue (the plugin answers the web view there) and the event delivery need it.
//!
//! ```text
//! ara-host <bundle> editor --files <wav>... --editor-mode ready|keys|focus|resize|measure|edit|shell|analyse|net
//!                          |edit-round|open-first|refuse|bigpost
//!                          [--cycles N] [--archive <file>] [--arg <text>] [--no-ara]
//!                          [--late-files <wav>...] [--wait-sync]
//! `--store-at-edit <file>`: the archive is stored the moment the page's edit report arrives
//! (modes `edit`, `edit-destroy`, `edit-region`); `edit-destroy` then closes the editor and the
//! document at once. `edit-region`: after the edit the host trims the region of the edited
//! source (`--region a,d,p`). `analyse-race --destroy-late <i> --archive-window <file>`: the
//! host destroys source `i` while the page's own analysis runs and stores an archive in that time.
//! `--late-files` are added to the document (and analysed) while the editor is open: in
//! `open-first` when the page has booted (the files then are `--late-files` only), in
//! `edit-round` and `refuse` after the page's report, in `analyse-race` when the page's own
//! analysis has started. `--wait-sync` (mode `edit`): after the edit the host waits for the row
//! `page_sync` of the plugin's worker (without the scenario `sync-delay`).
//! ```
//! Keys and clicks are real `NSEvent`s through `[NSApp sendEvent:]` (the S2 finding: the
//! window server path needs the accessibility trust).
use super::*;
use clap_sys::ext::gui::{clap_plugin_gui, clap_window, clap_window_handle};
use objc2::rc::Retained;
use objc2::MainThreadOnly;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEvent, NSEventMask, NSEventModifierFlags,
    NSEventType, NSResponder, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize};

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source_handled: bool) -> i32;
}

type Key = u16;
const K_SPACE: Key = 49;
const K_TAB: Key = 48;
const K_ESCAPE: Key = 53;
const K_RIGHT: Key = 124;
const K_Z: Key = 6;
const FLAG_COMMAND: u64 = 1 << 20;

#[derive(Clone, Copy)]
enum Action {
    Key(Key, u64),
    ClickOut,
    ClickIn,
}

fn schedule_of(mode: &str) -> Vec<(u64, Action)> {
    match mode {
        "keys" => vec![
            (2500, Action::Key(K_SPACE, 0)),
            (3000, Action::Key(K_RIGHT, 0)),
            (3500, Action::Key(K_Z, FLAG_COMMAND)),
            (4000, Action::Key(K_TAB, 0)),
            (4500, Action::Key(K_ESCAPE, 0)),
        ],
        "focus" => vec![
            (2500, Action::ClickOut),
            (3500, Action::Key(K_SPACE, 0)),
            (4500, Action::ClickIn),
            (5500, Action::Key(K_SPACE, 0)),
        ],
        _ => vec![],
    }
}

fn chars_of(code: Key) -> &'static str {
    match code {
        K_SPACE => " ",
        K_TAB => "\t",
        K_ESCAPE => "\u{1b}",
        K_RIGHT => "\u{f703}",
        _ => "z",
    }
}

fn post_key(code: Key, flags: u64, app: &NSApplication, window: &NSWindow) {
    let chars = objc2_foundation::NSString::from_str(chars_of(code));
    for ty in [NSEventType::KeyDown, NSEventType::KeyUp] {
        let ev = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            ty, NSPoint::new(0.0, 0.0), NSEventModifierFlags(flags as usize), 0.0, window.windowNumber(), None,
            &chars, &chars, false, code,
        );
        if let Some(ev) = ev {
            app.sendEvent(&ev);
        }
    }
}

fn post_click(app: &NSApplication, window: &NSWindow) {
    let f = window.frame();
    let at = NSPoint::new(f.size.width / 2.0, (f.size.height - 28.0) / 2.0);
    for (ty, pressure) in [(NSEventType::LeftMouseDown, 1.0), (NSEventType::LeftMouseUp, 0.0)] {
        let ev = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            ty, at, NSEventModifierFlags(0), 0.0, window.windowNumber(), None, 1, 1, pressure,
        );
        if let Some(ev) = ev {
            app.sendEvent(&ev);
        }
    }
}

fn pump(app: &NSApplication) {
    unsafe {
        let mode = objc2_foundation::NSString::from_str("kCFRunLoopDefaultMode");
        CFRunLoopRunInMode(
            (&*mode) as *const objc2_foundation::NSString as *const c_void,
            0.02,
            false,
        );
        let ev = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            NSEventMask::KeyDown
                | NSEventMask::KeyUp
                | NSEventMask::FlagsChanged
                | NSEventMask::LeftMouseDown
                | NSEventMask::LeftMouseUp
                | NSEventMask::MouseMoved,
            Some(&NSDate::now()),
            NSDefaultRunLoopMode,
            true,
        );
        if let Some(ev) = ev {
            app.sendEvent(&ev);
        }
        app.updateWindows();
    }
}

/// The rows of the plugin's report file (it is rewritten atomically).
fn rows() -> Vec<serde_json::Value> {
    let Some(path) = std::env::var("DS_ARA_OUT").ok().filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    std::fs::read_to_string(path)
        .map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
        .unwrap_or_default()
}

fn count_rows(event: &str, kind: Option<&str>) -> usize {
    rows()
        .iter()
        .filter(|r| r["event"] == event && kind.is_none_or(|k| r["kind"] == k))
        .count()
}

fn rss_mb() -> f64 {
    let out = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    out.trim().parse::<f64>().unwrap_or(0.0) / 1024.0
}

fn arg_flag(rest: &[String], name: &str) -> bool {
    rest.iter().any(|a| a == name)
}

pub fn run(bundle: &str, files: &[String], rest: &[String]) -> i32 {
    let mode = args_value(rest, "--editor-mode").unwrap_or_else(|| "ready".into());
    let cycles: usize = args_value(rest, "--cycles").and_then(|v| v.parse().ok()).unwrap_or(1);
    let archive = args_value(rest, "--archive");
    let no_ara = arg_flag(rest, "--no-ara");
    let late_files = args_list(rest, "--late-files");
    let wait_sync = arg_flag(rest, "--wait-sync");
    let store_at_edit = args_value(rest, "--store-at-edit");
    let region_spec = args_value(rest, "--region").and_then(|v| parse_region(&v).ok());
    let destroy_late: Option<usize> = args_value(rest, "--destroy-late").and_then(|v| v.parse().ok());
    let archive_window = args_value(rest, "--archive-window");
    std::env::set_var("DS_ARA_EDITOR_MODE", &mode);
    if let Some(a) = args_value(rest, "--arg") {
        std::env::set_var("DS_ARA_EDITOR_ARG", a);
    }
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        eprintln!("editor: the host must run on the main thread");
        return 2;
    };
    unsafe {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(120.0, 120.0), NSSize::new(1000.0, 640.0)),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setReleasedWhenClosed(false);
        window.makeKeyAndOrderFront(None);
        window.orderFrontRegardless();
        window.setLevel(3);
        app.finishLaunching();
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);

        let mut shown = false;
        for _ in 0..25 {
            pump(&app);
        }
        // the page is "visible" to WebKit only while its window is not occluded
        eprintln!(
            "editor: window visible={} occlusion={:?} key={} app_active={}",
            window.isVisible(),
            window.occlusionState(),
            window.isKeyWindow(),
            app.isActive()
        );
        shown |= window.isVisible();
        let _ = shown;
        let rss_closed = rss_mb();
        let lib = match load(bundle) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        let mut doc = match HostDoc::new(&lib) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        if !files.is_empty() && mode != "open-first" {
            if let Err(e) = doc.add_and_analyse(files, "editor") {
                eprintln!("{e}");
                return 2;
            }
        }
        let plugin = match clap_plugin_of(&lib) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        if !no_ara {
            let ext_id = CString::new(&ARA_PLUGIN_EXT_ID[..ARA_PLUGIN_EXT_ID.len() - 1]).unwrap();
            let ext = ((*plugin).get_extension.expect("get_extension"))(plugin, ext_id.as_ptr());
            if ext.is_null() {
                eprintln!("no ARA plugin extension");
                return 2;
            }
            let aext = ext as *const drumspike_ara::clap_ara_plugin_extension;
            let roles: ARAPlugInInstanceRoleFlags =
                (kARAPlaybackRendererRole | kARAEditorRendererRole | kARAEditorViewRole) as ARAPlugInInstanceRoleFlags;
            let pext = ((*aext).bind_to_document_controller.expect("bind"))(plugin, dc_ref_of(doc.inst), roles, roles);
            if pext.is_null() {
                eprintln!("bind_to_document_controller returned null");
                return 2;
            }
        }
        let gui_id = CString::new("clap.gui").unwrap();
        let gui = ((*plugin).get_extension.expect("get_extension"))(plugin, gui_id.as_ptr()) as *const clap_plugin_gui;
        if gui.is_null() {
            eprintln!("the plugin has no clap.gui extension");
            return 2;
        }
        let cocoa = CString::new("cocoa").unwrap();

        // the audio thread of the `analyse` mode: blocks while the page's analysis runs
        let audio_stop = std::sync::Arc::new(AtomicBool::new(false));
        let audio_stats = std::sync::Arc::new(Mutex::new((0usize, 0usize, 0u128, 0u128)));
        let mut audio_thread = None;
        if mode == "analyse" || mode == "analyse-race" {
            ((*plugin).activate.expect("activate"))(plugin, SAMPLE_RATE, 64, 4096);
            ((*plugin).start_processing.expect("start_processing"))(plugin);
            let p = plugin as usize;
            let (stop, stats) = (audio_stop.clone(), audio_stats.clone());
            audio_thread = Some(std::thread::spawn(move || {
                let plugin = p as *const clap_plugin;
                let mut input = vec![0.0f32; FRAMES];
                let mut output = vec![0.0f32; FRAMES];
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
                let in_events = clap_sys::events::clap_input_events {
                    ctx: std::ptr::null_mut(),
                    size: Some(in_events_size),
                    get: Some(in_events_get),
                };
                let out_events = clap_sys::events::clap_output_events {
                    ctx: std::ptr::null_mut(),
                    try_push: Some(out_events_try_push),
                };
                let budget = Duration::from_secs_f64(FRAMES as f64 / SAMPLE_RATE);
                let mut n = 0i64;
                while !stop.load(Ordering::SeqCst) {
                    let process = clap_process {
                        steady_time: n,
                        frames_count: FRAMES as u32,
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
                    if let Ok(mut s) = stats.lock() {
                        s.0 += 1;
                        if took > budget {
                            s.1 += 1;
                        }
                        s.2 = s.2.max(took.as_micros());
                        s.3 = budget.as_micros();
                    }
                    n += 1;
                    // a real host paces its blocks; this one runs at a tenth of real time
                    std::thread::sleep(budget / 10);
                }
            }));
        }

        let mut opened = 0usize;
        let mut failed: Option<String> = None;
        let mut rss_open = 0.0;
        let mut window_visible = false;
        let ready_limit = Duration::from_secs(25);
        let phase_limit = Duration::from_secs(60);
        for round in 1..=cycles {
            if !((*gui).create.expect("create"))(plugin, cocoa.as_ptr(), false) {
                failed = Some("gui create refused".into());
                break;
            }
            let parent_view = window
                .contentView()
                .map(|v| (&*v) as *const NSView as *mut c_void)
                .unwrap_or(std::ptr::null_mut());
            let parent = clap_window {
                api: cocoa.as_ptr(),
                specific: clap_window_handle { cocoa: parent_view },
            };
            if parent_view.is_null() || !((*gui).set_parent.expect("set_parent"))(plugin, &parent) {
                failed = Some("gui set_parent refused (the web view did not build)".into());
                break;
            }
            if !((*gui).show.expect("show"))(plugin) {
                failed = Some("gui show refused".into());
                break;
            }
            let ready_before = count_rows("page_ready", None);
            let want_kind: Option<&str> = match mode.as_str() {
                "keys" | "focus" | "resize" | "measure" | "shell" | "analyse" | "bigpost" | "refuse" => {
                    Some(mode.as_str())
                }
                // the edit mode ends with the event report that follows the edit report
                "edit" => Some("edit-events"),
                "analyse-race" => Some("analyse"),
                "edit-round" | "open-first" | "edit-region" => Some("follow"),
                "edit-destroy" => Some("edit"),
                "net" => Some("now"),
                _ => None,
            };
            let reports_before = want_kind.map_or(0, |k| count_rows("page_report", Some(k)));
            let boots_before = count_rows("page_report", Some("boot"));
            let edits_before = count_rows("page_report", Some("edit"));
            let refuse_before = count_rows("page_report", Some("refuse"));
            let started_before = count_rows("page_report", Some("analyse-started"));
            let mut late_done = false;
            let mut edit_handled = false;
            let mut destroyed_jobs: Option<usize> = None;
            if round == 1 {
                // WebKit draws a page only while its window is not occluded: a locked screen
                // hides every window, and the page is then suspended
                for _ in 0..20 {
                    pump(&app);
                }
                window_visible = window.occlusionState().0 & 2 != 0;
            }
            let t0 = Instant::now();
            let limit = if mode == "measure" && !window_visible {
                Duration::from_secs(20)
            } else if want_kind.is_some() {
                phase_limit
            } else {
                ready_limit
            };
            let mut schedule = schedule_of(&mode);
            let mut resized = 0usize;
            let mut saved_resp: Option<Retained<NSResponder>> = None;
            let mut got = false;
            while t0.elapsed() < limit {
                let elapsed = t0.elapsed().as_millis() as u64;
                let due: Vec<Action> = schedule
                    .iter()
                    .filter(|(at, _)| *at <= elapsed)
                    .map(|(_, a)| *a)
                    .collect();
                schedule.retain(|(at, _)| *at > elapsed);
                for a in due {
                    match a {
                        Action::Key(code, flags) => post_key(code, flags, &app, &window),
                        Action::ClickOut => {
                            saved_resp = window.firstResponder();
                            if let Some(cv) = window.contentView() {
                                window.makeFirstResponder(Some(&*cv));
                            }
                        }
                        Action::ClickIn => {
                            post_click(&app, &window);
                            if let Some(r) = &saved_resp {
                                window.makeFirstResponder(Some(&**r));
                            }
                        }
                    }
                }
                if mode == "resize" {
                    if resized == 0 && elapsed >= 2000 {
                        window.setFrame_display(
                            NSRect::new(NSPoint::new(120.0, 120.0), NSSize::new(1400.0, 850.0)),
                            true,
                        );
                        resized = 1;
                    } else if resized == 1 && elapsed >= 4000 {
                        window
                            .setFrame_display(NSRect::new(NSPoint::new(120.0, 120.0), NSSize::new(760.0, 480.0)), true);
                        resized = 2;
                    }
                }
                if !edit_handled && count_rows("page_report", Some("edit")) > edits_before && mode.starts_with("edit") {
                    edit_handled = true;
                    if let Some(path) = &store_at_edit {
                        // the archive is stored the moment the page's edit is answered
                        match doc.store() {
                            Ok(blob) => {
                                let _ = std::fs::write(path, &blob);
                                println!("{{\"event\":\"stored_at_edit\",\"bytes\":{}}}", blob.len());
                            }
                            Err(e) => {
                                failed = Some(e);
                                break;
                            }
                        }
                    }
                    if mode == "edit-destroy" {
                        got = true;
                        break; // the editor and the document close at once
                    }
                    if mode == "edit-region" {
                        let name = rows()
                            .iter()
                            .rev()
                            .find(|r| r["event"] == "page_report" && r["kind"] == "edit")
                            .and_then(|r| r["report"]["name"].as_str().map(str::to_string))
                            .unwrap_or_default();
                        let at = files.iter().position(|f| stem_safe(f) == name);
                        if let (Some(i), Some((a, d, p))) = (at, region_spec) {
                            let jobs_before = count_rows("job", None);
                            update_region(&doc, i, a, d, p);
                            let t = Instant::now();
                            while t.elapsed() < Duration::from_secs(30) && count_rows("job", None) <= jobs_before {
                                pump(&app);
                                doc_pump(&doc);
                            }
                        } else {
                            failed = Some(format!("no source for the edited track {name:?} or no --region"));
                            break;
                        }
                    }
                }
                if mode == "analyse-race" && destroyed_jobs.is_none() {
                    if let Some(i) = destroy_late {
                        if count_rows("page_report", Some("analyse-started")) > started_before {
                            destroyed_jobs = Some(count_rows("job", None));
                            if let Err(e) = doc.destroy_source(i) {
                                failed = Some(e);
                                break;
                            }
                            if let Some(path) = &archive_window {
                                // a store while the page's job still runs and the track is not gone yet
                                match doc.store() {
                                    Ok(blob) => {
                                        let _ = std::fs::write(path, &blob);
                                        println!("{{\"event\":\"stored_in_window\",\"bytes\":{}}}", blob.len());
                                    }
                                    Err(e) => {
                                        failed = Some(e);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                if !late_done && !late_files.is_empty() {
                    let trigger = match mode.as_str() {
                        "open-first" => count_rows("page_report", Some("boot")) > boots_before,
                        "edit-round" => count_rows("page_report", Some("edit")) > edits_before,
                        "refuse" => count_rows("page_report", Some("refuse")) > refuse_before,
                        "analyse-race" => count_rows("page_report", Some("analyse-started")) > started_before,
                        _ => false,
                    };
                    if trigger {
                        late_done = true;
                        // the editor stays alive while the host works (its page waits)
                        let jobs_before = count_rows("job", None);
                        if let Err(e) = doc.add_and_analyse(&late_files, "late") {
                            failed = Some(e);
                            break;
                        }
                        // the source is complete a moment before the plugin writes the row of its
                        // round: wait for the row, so the report is whole when the run ends
                        let t = Instant::now();
                        while t.elapsed() < Duration::from_secs(20) && count_rows("job", None) <= jobs_before {
                            pump(&app);
                        }
                        if count_rows("job", None) <= jobs_before {
                            failed = Some("the plugin wrote no row for the round of the late files in 20 s".into());
                            break;
                        }
                    }
                }
                pump(&app);
                let ready_now = count_rows("page_ready", None) > ready_before;
                let done = match want_kind {
                    Some("analyse") if destroy_late.is_some() => {
                        count_rows("page_report", Some("analyse")) > reports_before
                            && destroyed_jobs.is_some_and(|j| count_rows("job", None) > j)
                    }
                    Some(k) => count_rows("page_report", Some(k)) > reports_before,
                    // the ready mode waits for the page's boot report too (the driver sends it
                    // right after the ready call)
                    None => ready_now && count_rows("page_report", Some("boot")) > boots_before,
                };
                if done {
                    got = true;
                    break;
                }
            }
            // the report of the edit may arrive inside the same pump that ends the wait (the
            // edit-destroy mode): the archive of that moment is stored here then
            if got && !edit_handled && mode == "edit-destroy" {
                if let Some(path) = &store_at_edit {
                    match doc.store() {
                        Ok(blob) => {
                            let _ = std::fs::write(path, &blob);
                            println!("{{\"event\":\"stored_at_edit\",\"bytes\":{}}}", blob.len());
                        }
                        Err(e) => failed = Some(e),
                    }
                }
            }
            if round == 1 && got {
                // let the page settle, then the first memory sample with the editor open
                let t = Instant::now();
                while t.elapsed() < Duration::from_millis(1500) {
                    pump(&app);
                }
                rss_open = rss_mb();
            }
            if got && mode == "edit" && wait_sync {
                // the plugin reads the events again on its worker; wait for its row
                let t = Instant::now();
                while t.elapsed() < Duration::from_secs(15) && count_rows("page_sync", None) == 0 {
                    pump(&app);
                    doc_pump(&doc);
                }
                // the host asks for the plugin's model updates a few times (the notification of
                // a source whose notes changed comes in `notifyModelUpdates`)
                let t = Instant::now();
                while t.elapsed() < Duration::from_millis(500) {
                    pump(&app);
                    doc_pump(&doc);
                }
            }
            ((*gui).destroy.expect("gui.destroy"))(plugin);
            if !got {
                failed = Some(format!(
                    "no page report after {} s (phase {mode}, round {round})",
                    limit.as_secs()
                ));
                break;
            }
            opened += 1;
            let t = Instant::now();
            while t.elapsed() < Duration::from_millis(100) {
                pump(&app);
            }
        }
        // the editor is closed: let queued main-thread jobs run, then the last sample
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(1500) {
            pump(&app);
        }
        let rss_after = rss_mb();
        if let Some(h) = args_value(rest, "--hold").and_then(|v| v.parse::<u64>().ok()) {
            // a debugging aid: stay alive so `leaks <pid>` or `vmmap <pid>` can look
            eprintln!("editor: holding pid {}", std::process::id());
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(h) {
                pump(&app);
            }
        }
        audio_stop.store(true, Ordering::SeqCst);
        if let Some(h) = audio_thread {
            let _ = h.join();
            ((*plugin).stop_processing.expect("stop_processing"))(plugin);
            ((*plugin).deactivate.expect("deactivate"))(plugin);
        }
        if failed.is_none() {
            if let Some(path) = &archive {
                doc_pump(&doc);
                match doc.store() {
                    Ok(blob) => {
                        let _ = std::fs::write(path, &blob);
                        println!("{{\"event\":\"stored\",\"bytes\":{}}}", blob.len());
                    }
                    Err(e) => failed = Some(e),
                }
            }
        }
        ((*plugin).destroy.expect("destroy"))(plugin);
        doc.close();
        ((*lib.entry).deinit.expect("deinit"))();
        let audio = audio_stats.lock().map(|s| *s).unwrap_or_default();
        println!(
            "{{\"event\":\"editor_run\",\"mode\":{mode:?},\"cycles\":{cycles},\"opened\":{opened},\"ok\":{},\"why\":{:?},\"window_visible\":{window_visible},\"rss_closed_mb\":{rss_closed:.1},\"rss_open_mb\":{rss_open:.1},\"rss_after_mb\":{rss_after:.1},\"audio_blocks\":{},\"audio_overruns\":{},\"audio_worst_us\":{},\"audio_budget_us\":{}}}",
            failed.is_none(),
            failed.clone().unwrap_or_default(),
            audio.0,
            audio.1,
            audio.2,
            audio.3
        );
        if failed.is_none() {
            0
        } else {
            2
        }
    }
}

fn doc_pump(doc: &HostDoc) {
    unsafe { super::pump(doc.inst) }
}

fn stem_safe(file: &str) -> String {
    // the track name of a file: the app's `safe_name` of its stem
    let stem = std::path::Path::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = String::new();
    let mut gap = false;
    for c in stem.chars() {
        if c.is_ascii() && (c.is_ascii_alphanumeric() || c == '-') {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

fn update_region(doc: &HostDoc, i: usize, a: f64, d: f64, p: f64) {
    unsafe {
        let dc = iface_of(doc.inst);
        let state = &*doc.state;
        if let Some(r) = state.regions.get(i) {
            let props = playback_props(a, d, p);
            (dc.updatePlaybackRegionProperties
                .expect("updatePlaybackRegionProperties"))(dc_ref_of(doc.inst), *r, &props);
        }
    }
}
