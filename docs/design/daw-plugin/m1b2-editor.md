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
the crates of the table in `plugins/webview-spike` note S2 (`s2-page-in-plugin.md`) and a few
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

RESULTS_PLACEHOLDER
