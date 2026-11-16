# M1b2: the editor window

Ticket #538, umbrella #503 (the DAW plugin program), part of M1 (#510). Follows M1b1 (#537:
one service and one project for each ARA document) and spike S2 (#505: the page in a plugin
window). The plugin lives in `plugins/ara/`. Nothing in `static/` or `crates/` changed.

## What changed

Before: the plugin had no editor. Now: the plugin has the CLAP extension `clap.gui` (cocoa,
embedded). The editor is the Drum Spike page (`static/`) in a `wry` web view as a child of the
host's view. Every command of the page runs in the Drum Spike service of the ARA document, as in
the desktop app. The stub transport of S2 is gone.

## The design

### Where the page lives

- The build (`check.sh build`) copies `static/` to `Contents/Resources/page` of the CLAP. The
  VST3 carries the whole CLAP in `Contents/PlugIns` (the wrapper searches there first), so the
  page is in the VST3 too.
- `clap_entry.init` gets the path of the CLAP bundle. The CLAP host gives it, and so does
  clap-wrapper for the VST3 (it gives the path of the embedded CLAP: `fsutil.cpp` of version
  0.17.0 passes the bundle path to `init`). The plugin keeps that path and looks for
  `Contents/Resources/page/index.html` below it. If the path is no bundle with a page, the
  plugin takes the address of its own code (`dladdr`: `<bundle>/Contents/MacOS/<binary>`, three
  levels up). The report says which way found the page (`page_found_by`): both test bundles
  report `init`.
- A scheme handler for `dspage://localhost/` serves the files. It takes plain path components
  only (no `..`, no absolute path). The html carries a `Content-Security-Policy` that allows
  the `dspage:` scheme and `blob:` and `data:` only. A request to another host is refused by
  the web view and the test driver reports the violation. The navigation handler and the
  new-window handler refuse every other address.

### The shell script and `invoke`

- `src/editor/shell.js` runs before every page script (wry initialization script). It gives
  `window.__TAURI__` with `core.invoke`, `core.Channel`, `dialog.open`, `dialog.save`,
  `event.listen`, and sets `window.__DS_HOST_MODE__ = true`. A file dialog answers `null`
  (cancel): the host owns the files in host mode, and the page already says so.
- `invoke(cmd, args)` is `fetch` POST to `/__ds/invoke/<cmd>` with the JSON arguments. The
  answer is JSON or, for the four binary commands (`peaks_tile`, `samples_window`,
  `track_layer_wave`, `track_sampler_wave`), bytes (an `ArrayBuffer`). An error is a non-200
  answer with `{code, message}`; the shell rejects with that object, as the Tauri shell does.
  A body of 351 KB (16000 hits in `events_set`) came through whole (measured early, because
  WKWebView may hand a request body over as a stream).
- A `Channel` and the events of the service come by long poll (`/__ds/channel/<id>`,
  `/__ds/events`), the way of the test bridge of the app. No thread pushes into the web view.
  A channel argument goes to `call::run` as `{"__channel": id}`; the plugin keeps the messages
  in the `Channels` of the editor. The events of the service (`project-dirty`, `track-layer`)
  go to an event log that belongs to the document (`PageHandle`), so a new editor of the same
  document sees the log from its own start.

### Threads

- The web view is made, used and dropped on the host's main thread. The scheme handler of wry
  runs there. A file of the page is read and answered there (small, local).
- A command (`/__ds/` path) is handed to a new thread (`drumspike-ara-page`, at most 96 at
  once). That thread runs `drumspike_app::call::run` on the `Service` of the document and
  returns to the main thread through the main dispatch queue to answer the web view. A host
  with a window serves that queue. So a command that runs for minutes never blocks the host.
- The editor reaches the service through `PageHandle`: a clone of the `Service` (one `Arc`)
  with a lock of its own. It never takes the `DocService` lock, which the worker holds for the
  whole of a round.
- Every request thread runs inside `catch_unwind`; a panic gives the page an error answer
  (`500`, "the command panicked inside the plugin") and the plugin runs on. Every `clap.gui`
  entry point runs inside the plugin's panic guard.
- The plugin counts every use of the web view and every command, and notes the ones that run
  off the right thread (`off_main`). The check fails on a count above 0.

### Commands that `call::run` leaves to the shell

All 25 names of `SHELL_ONLY` (`crates/app/src/call.rs`) and the command of the drag plugin
have an answer in the plugin: `shell_sync` gives `0`; `shell_typing`, `startup_mark`,
`splash_*` give `null`; `app_ready` counts the ready of the page (the row `page_ready`);
`open_paths_take` gives no paths; `quit_app` is counted and does nothing (the host owns the
window; the plugin never ends the host); `reveal_path`, `open_link` (ids `website`, `source`,
`cbv`, `warranty`; the page never passes an address) and `open_url` (http or https only) call
`/usr/bin/open`; in a test run they are recorded only; `about_info` and `about_third_party`
answer from the build; the `selftest_*` commands answer 404 "is a test command of the desktop
app"; the file drag answers 501 "comes with part M1c". A name that is none of these goes to
`call::run`, which answers "unknown command" only for a real unknown name.

### An edit lasts

After a command that changes the project (`api::find(cmd).writes_project()`), and after a job
that ended (`analyze_status` or `job_status` with state `done`), the editor asks the worker
thread of the document (message `PageSync`) to read the events of every track again. If they
differ, the worker saves the project, sets the events of the document (the archive stores
them), sets the notes of the sources, and queues `notifyAudioSourceContentChanged` for the
sources whose notes changed. The host hears it in its next `notifyModelUpdates`, as it does
after a round.

### Focus, keys and size

- wry's `with_focused` does nothing on macOS. The plugin calls `makeFirstResponder` on the
  web view itself, at `show` and at `set_parent`.
- The web view fills the host's view; the autoresizing mask keeps it so. `set_size` also sets
  the bounds.

### One web view for each plugin instance

wry 0.57 leaks a web view on purpose when it drops it (`Drop for InnerWebView` retains the
WKWebView and its content manager once more). That cost 1.2 MB of the host's memory for every
opening of the editor (measured: 63 MB over 50 openings, linear over 120). So the plugin makes
ONE web view for the life of the plugin instance. `destroy` sends it to `about:blank` (the
page and its polls stop) and removes it from the host's view; `set_parent` puts it into the
new view and loads the page again, with a fresh state for the document. The one leak that is
left is the web view at the end of the plugin instance.

### The data store of the web view

wry 0.57 cannot place a persistent data store in the plugin's cache folder: a store with an
identifier lives below `~/Library/WebKit`, and the default store writes there and into
`~/Library/Caches` of the host. So the web view uses the non-persistent store: the plugin
writes nothing of the page outside its cache folder. Open point: the page keeps no setting of
its own (view settings in `localStorage`) between two openings of the editor.

## New crates

New crates only in `plugins/ara/Cargo.toml`: `wry` (pinned `=0.57.0`, the version of S2, no
default features), `objc2` 0.6, `objc2-app-kit` 0.3, `objc2-foundation` 0.3. They bring
the crates of the table in the note of spike S2 (`s2-page-in-plugin.md`) and a few
others. The licences, from `cargo metadata` for macOS: the plugin build has 149 crates outside
this repository. Strings: `MIT OR Apache-2.0` (76), `Unicode-3.0` (18),
`Zlib OR Apache-2.0 OR MIT` (17: the objc2 family), `MIT` (11), `Apache-2.0 OR MIT` (8),
`MIT/Apache-2.0` (4), `Unlicense OR MIT` (2), `Apache-2.0` (2), `MPL-2.0` (1:
`option-ext` 0.2.0, through `dirs-sys` of wry), `ISC` (1), `BSD-3-Clause` (1), and single
others (`0BSD OR Apache-2.0`, `Apache-2.0/MIT`, `Apache-2.0 AND MIT`,
`BSD-2-Clause OR MIT OR Apache-2.0`, `Unlicense`, `MIT OR Apache-2.0 OR Zlib`,
`(MIT OR Apache-2.0) AND Unicode-3.0`, `Zlib OR MIT OR Apache-2.0`). The one copyleft part is
`option-ext` (file-level MPL-2.0, used unchanged as a library).

`NOTICE` must list the parts of the bundle before any delivery (M1c): the web view crates and
their notices, `option-ext` with the MPL-2.0 text, the page with its help and brand files, and
the parts of the wrapper and the VST3 SDK module that are already named in the README.

## Test switches and the check

New switches (all in `switches.json`): `DS_ARA_EDITOR_MODE` and `DS_ARA_EDITOR_ARG` (the test
driver of the page; the test host sets them itself), `DS_ARA_EDITOR_FAULT` and
`DS_ARA_EDITOR_SCENARIO`. A real host sets none. The driver (`src/editor/driver.js`) is part
of the web view only when `DS_ARA_EDITOR_MODE` is set. The `editor` mode of `ara-host` analyses
the fixture session of M1a (four tracks) in an ARA document, binds a plugin instance to it, and
opens the editor in a real AppKit window. Keys and clicks are real `NSEvent`s through
`[NSApp sendEvent:]` (S2 finding: the window server path needs the accessibility trust).

## Result

The gate is `plugins/ara/check.sh` (stages `editor-*`, each with a fault run). Measured on the
build host (Apple silicon, macOS 27, run from a shell). Values are of the last full run.

| Gate line | Stage | Result |
| --- | --- | --- |
| 1 Window: `clap.gui`, web view child of the host view, 50 open/close cycles | `editor-ready`, `editor-ready-vst3`; fault `editor-fault-ready` | MET. Page ready 50 of 50 on the CLAP and 50 of 50 on the plugin inside the VST3. |
| 2 Page from the bundle, no network, CLAP and VST3 | `editor-bundle`, `editor-net` (+ `-vst3`); faults `editor-fault-bundle`, `editor-fault-net` | MET. The page came from `<bundle>/Contents/Resources/page` (CLAP) and from `DrumSpikeAra.vst3/Contents/PlugIns/DrumSpikeAra.clap/Contents/Resources/page`, found by the path of `clap_entry.init` in both. 0 requests to another scheme or host, 0 CSP violations in 100 page loads. |
| 3 Shell, `invoke` on the service, binary answers, error shape, channels | `editor-tauri`, `editor-binary`, `editor-error`, `editor-threads` (channel messages) | MET. Keys `core, dialog, event`; 4 binary commands gave an ArrayBuffer (6756, 8008, 8008, 8008 bytes); error `{code: 404, message}`; 17 channel messages of a page-started analysis. The layer wave needs a track without playback regions (a segmented track has no layer: the app answers 400), so that run uses `--no-regions`. |
| 4 Real data | `editor-data` (+ `-vst3`); fault `editor-fault-data` | MET. 4 tracks, 19 events; the events the page reads are byte for byte the reference (733 bytes) in all 100 page loads. |
| 5 An edit lasts | `editor-edit`, `editor-edit-notes` (+ `-vst3`); fault `editor-edit-fault` | MET. Hit 1 of the kick moved 0.75002 to 0.76002 s through `B.eventsEdit`; the archive restored in a new process has that list and the other 3 tracks unchanged; the ARA notes carry sample 36481 and no longer 36001. |
| 6 Shell commands | `editor-shell` (+ `-vst3`); fault `editor-shell-fault` | MET. 16 commands called (every name of the list that the page can call, `selftest_clock` and the drag): all have a defined answer, 0 "unknown command"; `quit_app` answered and the host lived. |
| 7 Threads | `editor-main-thread`, `editor-threads` (+ `-vst3`); faults `editor-fault-main`, `editor-threads-fault` | MET. 6800 web view calls checked, 0 off the main thread. With the editor open an analysis from the page ended in 204 ms; 2547 audio blocks ran meanwhile, 0 overruns, worst block 22 us of 10666 us. |
| 8 Keys, focus, size, frame time | `editor-keys`, `editor-focus`, `editor-resize` (+ `-vst3`), `editor-measure`; faults in `editor-gui-fault` | MET for keys (Space, Right, Cmd+Z, Tab, Esc reached the page and the page ran a command for each), focus (0 keys after a click in the host, 1 after a click in the editor) and resize (1000 -> 1400 -> 760 px). The plugin sets the first responder itself (fault `no-focus` fails both). **OPEN for the frame time**: see below. |
| 9 Faults | `editor-fault-ready`, `editor-panic`, `editor-panic-fault` | MET. A page that never reports ready fails the stage. A command that panics gave the page `{code: 500}`, the panic was caught, the host lived; without the guard the page gets no answer. The panic is a test scenario at the place where `call::run` runs, not a panic inside the app crate. |
| 10 Memory | `editor-memory` (+ `-vst3`); fault `editor-fault-memory` | MET. Resident memory of the test host, closed / open (first) / after 50 cycles: CLAP 73.8 / 116.8 / 120.7 MB; VST3 73.7 / 122.2 / 125.9 MB (limit: 50 MB above the first open). It counts the host process only; the WebContent processes of WebKit are not part of it. |
| 11 Licences | README, this note | MET. See "New crates". |
| 12 Standalone | `standalone-tree`, `standalone-metadata`, `standalone-diff`, `crate`, `workspace` | MET. Green. `static/` and `crates/` are not changed. |

### Open: the frame time (gate line 8)

The stage `editor-measure` fails on this host and says why. The screen of the build host was
locked during all runs (`CGSSessionScreenIsLocked` = 1). A locked screen occludes every window
(occlusion state without the visible bit); WebKit then reports `visibilityState = hidden`,
runs no `requestAnimationFrame` and suspends the page after a few seconds. The page draws no
frame, so the 95th percentile over 8 s of scroll and zoom cannot be measured; 0 frames were
seen. This is not a result for the plugin. The stage is not bent: it fails with the reason
whenever the window is occluded and measures the frame time (limit 33 ms, 8 s, scroll 10 times
and zoom 2 times a second on the lanes) whenever it is visible. It has to run once on a host
with an unlocked screen. Its fault run (`slow-frames`) fails here too, but for the same reason,
so on this host that fault run proves nothing about the limit.

## Findings

- Gate line 8 of the ticket and the S2 note say the keys do not reach the page without a
  first responder: confirmed again (fault `no-focus`: 0 of 5 keys).
- wry 0.57 drops a web view with a deliberate extra retain (see "One web view for each plugin
  instance"): 1.2 MB of host memory for each editor, in `Malloc Small` and Foundation objects
  (`heap` showed 60 live `WebPageProxy` after 60 closed editors). The plugin keeps one view.
- WKWebView delivered a 351 KB request body to the scheme handler whole.
- The check runs the editor stages on the CLAP and on the CLAP inside the VST3 bundle through
  the same test host. It does not load the VST3 through the wrapper's `IPlugView`: pluginval at
  strictness 5 does that for the wrapped VST3 (stage `pluginval`, green; the plugin has no ARA
  binding there, so the editor uses a service of its own with no project).

## Open points

- The frame time (above).
- The web view uses a non-persistent data store: the page keeps no `localStorage` setting
  (view settings, zoom presets) between two openings of the editor. wry 0.57 cannot place a
  persistent store in the cache folder.
- The one web view of a plugin instance is leaked by wry at the end of the instance (about
  1.2 MB each).
- The memory figure counts the host process only.
- `quit_app`, the file dialogs and the file drag are no-ops or errors (the host owns the
  window and the files; the drag is part M1c).
- The host transport (position, play, stop, locate) is a later part: the page runs with
  `__DS_HOST_MODE__` set and no `B.hostTransport`.
