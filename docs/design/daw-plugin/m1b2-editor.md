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
| 1 Window: `clap.gui`, web view child of the host view, 50 open/close cycles | `editor-ready`, `editor-ready-vst3`; fault `editor-fault-ready` | MET. Page ready 50 of 50 on the CLAP and 50 of 50 on the CLAP inside the VST3 bundle (loaded directly; the wrapper view is `editor-vst3-view`). |
| 2 Page from the bundle, no network, CLAP and the CLAP inside the VST3 bundle (not through the wrapper's view: see `editor-vst3-view` below) | `editor-bundle`, `editor-net` (+ `-vst3`); faults `editor-fault-bundle`, `editor-fault-net` | MET. The page came from `<bundle>/Contents/Resources/page` (CLAP) and from `DrumSpikeAra.vst3/Contents/PlugIns/DrumSpikeAra.clap/Contents/Resources/page`, found by the path of `clap_entry.init` in both. 0 requests to another scheme or host, 0 CSP violations in 100 page loads. |
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

## Round 2 (review of the coordinator)

### The rules between the page and the plugin

- **A round keeps the hand edits (F1).** A round of the plugin (a new source, a changed region)
  analyses all tracks together as before, but with the app's own "keep my edits"
  (`Service::analyze_start_keep` with `keep_edited = true`): the hits the user added or moved in
  the page stay in the new events and win within 20 ms over the detected ones. After each round
  and after each restore the plugin sends the event `plugin-reload` into the event log of the
  document; the shell script of the page answers it with the page's own load path
  (`flushEvents`, `initSessions`, `loadProject`), so the page holds the new track list and
  events without a reload by hand. A burst of events is one reload. The event is not sent after
  a command of the page itself (the page already holds that state).
- **The editor before the first round (F8).** An editor that opens before any analysis shows the
  page's own empty state ("no project is open: the host opens the project": the service has no
  project yet). When the first round ends the `plugin-reload` event makes the page load the
  project. Stage `editor-open-first`.
- **An edit reaches the archive before the page gets its answer (F2).** After a command that
  changes the project, the request thread reads the events of every track of the document and sets
  the document's events and the notes of the sources, using only the locks of the document's own
  state (never the `DocService` lock that the worker holds for a whole round). The worker saves
  the project later. A store of the host that comes at once writes the edit; an edit just before
  the destroy of the controller is not dropped.
- **The host owns the track set and the project files (F3, F5).** The editor answers 403 for the
  commands that add, remove, restore, reorder, rename or scan tracks, and for open, new, close,
  import, save as, rename, consolidate, relink and legacy-import of a project. It answers 403 for
  what is not in a plugin: the agent skill, the feedback post (a network request from the plugin
  process), the app's own audio transport and output devices (the host plays the audio). The page
  side (hiding these controls) is ticket #539. `window_call_next` answers `{enabled: false}`
  (the page's agent relay then ends) and `window_call_result` 403. The sync of the worker reports
  an error of `events_get` instead of keeping ghost events.
- **Every command of the app has one class (F5).** `ALLOWED`, three refused lists and the shell
  list in `editor.rs`; a unit test walks `drumspike_app::api::REGISTRY` and fails for a command in
  no class or in two. A new command of the app fails that test until it is classed. (The two
  routes without a registry row, `plugins_list` and `plugin_accept`, are in `ALLOWED` by name;
  the public surface has no list of them.)
- **Writes outside the cache folder.** Every allowed command writes below the cache folder (the
  document, the project folder with `Exports/`, `Analysis/` and the peak cache, the home of the
  document's service with settings, recent list and kit presets) or reads. `export_all`,
  `export_layer`, `export_existing`, `export_dest_set`, `export_drag_prepare`, `export_midi`,
  `export_spikes` and `sampler_calibration_midi` answer 403 when they carry a `dest` or `path` of
  their own: the page's file dialogs answer cancel, so it has none. `reveal_path`, `open_link` and
  `open_url` start `/usr/bin/open` (the plugin waits for the child, no zombie) and write nothing.
  The file drag answers 501.
- **A round and a job of the page (F4).** The app allows one job per project. A round of the
  plugin waits (polling every 20 ms, at most 600 s, cancel-aware) while a job of the page is
  queued or running, then starts. A `analyze_start` of the page while a round runs gets the app's
  409 ("an analysis is already queued or running for this project: cancel it or wait for it").
  Stage `editor-race`: a round (a new source) comes while the page's own analysis of a long source
  runs; no round fails.
- **Bursts.** More than 96 requests at once get 503 "the editor is busy" at once; the page shows
  the error of that call. The page starts few requests at a time (peak seen in the checks: 3 to
  about 12), so this is a safety bound.

### Thread enforcement (F7)

`set_parent`, `destroy`, `show` and `set_size` do nothing and answer false (or return) when they
are not on the host's main thread; the counter `off_main` notes it. Every closure that wry calls
(scheme handler, navigation, new window, ipc) runs inside the panic guard. `set_parent` does not
hold the borrow of the editor slot while the web view is made or moved.

### What the stages prove now

| Finding | Stage and fault run |
| --- | --- |
| F1 round keeps hand edits, page follows | `editor-keep-edits`, `editor-follow` (+ `-vst3`); faults `editor-keep-edits-fault` (`drop-edits`), `editor-follow-fault` (`no-reload`) |
| F2 edit reaches the archive at once | `editor-edit` (the archive is stored at once, no wait for the worker); fault `editor-edit-fault-no-record` (with the scenario `sync-delay`) |
| F3 track set owned by the host | `editor-refuse` (14 commands answer 403, tracks unchanged, a later round runs); fault `editor-refuse-fault` |
| F4 round and page job | `editor-race`; fault `editor-race-fault` (`no-page-wait`) |
| F5 command classes | unit test `every_command_of_the_app_is_in_exactly_one_class` (in `crate-test`) |
| F6 notes evidence | `editor-edit-notes` always present, rate from the host's rows; fault `editor-edit-fault-no-notes` |
| F8 editor before the first round | `editor-open-first`; fault `editor-open-first-fault` |
| F9 page copy equals `static/` | `editor-page-copy` (81 files, hashes); fault `editor-page-copy-fault` |
| F10 the wrapper's view | `editor-vst3-view` (pluginval opens the editor of the wrapped VST3); fault `editor-vst3-view-fault` |
| F11 event reaches the page; analysis thread | `editor-events`; `editor-threads-analysis` (faults `editor-edit-fault-no-events`, `editor-threads-analysis-fault`) |

### The VST3 stages, exactly (F10)

The `-vst3` stages load the CLAP that sits inside the VST3 bundle directly with the test host
(`DrumSpikeAra.vst3/Contents/PlugIns/DrumSpikeAra.clap`). They do NOT go through the wrapper's
view. The stage `editor-vst3-view` does: pluginval at strictness 5 opens the editor of the
wrapped VST3 through the wrapper's `IPlugView` (several times), and the plugin's report shows
the page found in the VST3 bundle and ready. There the plugin has no ARA document, so the editor
uses a service of its own with no project. A run in a real DAW with an ARA document is the hand
check that remains.

### Frame time: how to run it alone

`plugins/ara/check.sh check` runs every stage. The two measure stages alone, on a host with an
unlocked screen (after `check.sh build` and the stages `fixture` and `reference` have made
`out/`):

    cd plugins/ara && /usr/bin/python3 -I check.py fixture reference editor-gui

`editor-gui` runs keys, focus, resize and measure on the CLAP and on the CLAP in the VST3. The
stage `editor-measure` fails with the reason while the screen is locked.

## Round 3 (second review of the coordinator)

### What a region change keeps (M1)

When the host changes the regions or the properties of a source, the plugin makes its track
record again with the new segments. The removal answers the whole old record
(`tracks_remove` with `snapshot`), and the plugin carries EVERYTHING the user set on the track
into the new record: hits (also the edited ones), role, note, mix mode, volume and pan, params,
Learn data, layer and sampler, follow, row data, and every other key. Only `segments` and `source`
are replaced by the new ones; `sr`, `n_samples` and `duration` are dropped, because the analysis
of the same round writes them again. Hits that now lie outside the new regions stay in the event
list as the app keeps them; the notes leave them out. Then the round runs with "keep my edits".
Stage `editor-regions-keep` (fault `drop-track-data`).

### A job of the page and the worker (M3, S1, S2)

- The worker never waits for a job of the page. A round or a removal that meets a running job
  goes back to the queue (`Msg::Defer`, `Msg::Remove` with a counter) and comes again after 100 ms,
  for at most about 10 minutes; the worker serves the other documents meanwhile. A destroy of
  another document does not wait for it.
- A removal that fails keeps the events of the document (it never takes an error for "no track is
  left") and is tried again. The ghost track leaves the project when the job has ended. Stage
  `editor-race-destroy` (fault: `no-page-wait`).
- A job of the page may start between the check and the call: the calls that the job blocks
  (`tracks_remove`, `analyze_start_keep`) are tried again for at most 1 s on a 409 (S1, R4); a 409 after
  that goes back to the queue like a job of the page.

### The edit stage (M2)

The scenario `sync-delay` delays only the worker's sync (12 s); the request thread's own record
is not delayed. `editor-edit` runs with it and stores the archive the moment the edit is
answered; the fault `no-record` takes the record away and the stage fails. Stage
`editor-edit-destroy` stores the archive at the edit, closes the editor and the document at once,
and checks the archive in a new process and the project on disk.

### The reload and the view (S3, S4, S7)

The reload of the page after `plugin-reload` DROPS the page's pending writes (`cancelEvents`)
instead of flushing them: they hold the page's older list and would be written over the data of
the round. The zoom and the scroll of the lanes are kept across the reload (the script sets
`S.view` and the scroll position; this uses two internals of the page, no change in `static/`).
The undo and redo history is lost (`loadProject` resets it): open point for #539. The shell
script uses five functions of the page (`PAGE_FUNCTIONS`); a missing one is reported (console
error and the shell command `plugin_report`, a row `page_shell_report`), and the stage
`editor-shell-functions` fails when one is missing in the bundled page. The first poll of events
starts at the head of the log when the editor opened, so a `plugin-reload` announced before the
first poll is not lost. A command that is in no list at all is refused (403). `get_size` reads an
atomic; a `destroy` off the main thread is refused and leaves a row.

### Same source for the notes (S5)

The notes are mapped with the regions and the rate each source had in the last round (or restore),
by the worker and by the request thread alike (`round_regions`), not with the live regions that a
host change may have moved since. A test (`notes_map_with_the_regions_of_the_round_not_the_live_ones`)
shows the difference.

### Which hand edits survive a round (S6)

Survive a DAW round: hits the user added, moved or edited in the page; the role, note, mix,
params and every other setting of a track, also across a region change. Do NOT survive: a hit that
the user DELETED. `keep_edited` of the app keeps added, moved and edited hits only, so the next
round picks a deleted hit again, and a round of the plugin is silent. The plugin builds no
workaround. Open points for the owner, two options: (a) the app writes a delete as a
`deactivated` mark that the next analysis keeps (a change in `crates/`), or (b) the plugin
re-applies the deletes after a round (it would have to remember them per track).

## Round 4 (third review of the coordinator)

- **Follow links survive a region change (R1).** The plugin passes the app's whole snapshot row
  (`name`, `index`, `order_index`, `track`, `followers`) to `tracks_restore` and replaces only the
  `track`, so the followers of the removed track are linked again and a new field of the app's
  snapshot is carried too. Stage `editor-regions-keep` now sets a follow link in the page (fault
  `drop-track-data` loses it).
- **A refused restore (R2).** When the restore of the new record is refused, the OLD record is put
  back and the round reports it; when that fails too, the track leaves the plugin's list (the
  source is a new track at the next round), so later rounds do not fail with a 404. A removal of a
  track the project does not have (the app answers 400 "unknown track") counts as removed: no
  retries for a ghost. Unit tests with a test hook (`fail_restores`).
- **The worker's sync (R3).** `editor-edit-sync` runs the edit WITHOUT the delay scenario and waits for
  the row `page_sync`: the worker saved the project (`saved`), the moved hit is on disk, and the host
  was told that the notes of the source changed (`host_content_changed`). The row is now written for
  every sync (the request thread has usually recorded the events already, so `changed_notes` is
  empty then; the notification of the host comes from the record).
- **What the worker waits for (R4).** Nothing for longer than 1 s. A call that a page job blocks
  (`tracks_remove`, `analyze_start_keep`) is tried again for at most 1 s under the document lock;
  a 409 after that sends the round (`round_deferred`) or the removal back through the queue, like
  a job of the page: it comes again after 100 ms, for about 10 minutes at most. Other documents are
  served meanwhile; a destroy of another document waits at most for that 1 s retry of the call in
  flight.
- **The reload drops writes (R5).** `cancelEvents` drops page writes that were not sent yet: the
  400 ms debounce of an edit, and an edit that got a 409 during a round. The user sees such an
  edit vanish at the reload (open point for #539: a note in the page). The script no longer waits
  for the page's edit queue (`edIdleHook`): a queued operation is index-based and would be applied
  to the new list. The page parts the script uses (four functions, the globals `S` and `F`, `S.view`
  and the element `lanesScroll`) are checked in the bundled page by `editor-shell-functions`; the
  stage `editor-shell-report` runs the report path end to end (a missing function gives the row
  `page_shell_report`).
- **Regions of a round (R6).** The notes of a page edit use the regions of the snapshot the round
  kept (the service returns them with the result), so `record_page_edit` and `notes_of` agree also
  for a changed source without audio; the entry of a destroyed source is removed.

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
- The host transport (position, play, stop, locate) came with M1b5 (#546,
  `m1b5-host-transport.md`). Until then the page ran with `__DS_HOST_MODE__` set and no
  `B.hostTransport`. Play and locate from the page to the host are still a later part.
