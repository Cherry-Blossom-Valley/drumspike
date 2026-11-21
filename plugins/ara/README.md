# Drum Spike ARA plugin (M1a, #525; M1b1, #537; M1b2, #538; M1b5, #546; M1d1, #552)

The Drum Spike plugin for DAWs: one CLAP plugin in Rust with ARA 2 and, since M1b2, an editor window,
wrapped as one self-contained VST3 (the CLAP inside the bundle). The plugin takes the
audio sources and playback regions of the host, analyses them with the Drum Spike service
(`drumspike-app`, the same detection as the desktop app), gives the hits back to the host
as ARA notes, and keeps its state in the ARA archive of the session.

Since M1b1 the plugin holds one Drum Spike service and one project for each ARA document,
for the life of the document controller (see "The session" below).

The crate is outside the Cargo workspace members (the workspace builds without the ARA
SDK); it uses `drumspike-app` as a path dependency. The ARA 2 C API is generated with
bindgen from `$ARA_API_DIR/ARAInterface.h` (the ARA SDK 2.3.0; `check.sh deps` fetches it
into `.deps`).

## Build

One command builds the CLAP (universal: arm64 + x86_64, joined with lipo; the build fails
when `lipo -info` does not name both) and one self-contained VST3 bundle with the CLAP
inside (`.deps/build-wrapper/DrumSpikeAra.vst3/Contents/PlugIns/DrumSpikeAra.clap`; the
wrapper searches its own bundle's `PlugIns` first, so no `CLAP_PATH` is needed):

    ./check.sh all        # deps, build, check
    ./check.sh build      # the bundles, both test hosts, service_tool
    ./check.sh check      # the gate: check.py all

The build makes two variants of the bundle. The gate variant (`.deps/DrumSpikeAra.clap` and
`.deps/build-wrapper/DrumSpikeAra.vst3`) is built with the cargo feature `test-switches`; the
gate needs it for its fault runs. The delivery variant (`.deps/delivery/DrumSpikeAra.clap` and
`.deps/delivery/DrumSpikeAra.vst3`) is built without it. It is the same wrapper with the CLAP
that reads no test switch (see "Test switches of the check"). Only the plugin crate is built
twice; the dependencies are built once.
The delivery variant is built last, so the loose dylibs in `target/<triple>/release` are the
delivery variant too. A later installer takes the bundles from `.deps/delivery`, never from
`.deps` or `.deps/build-wrapper`.

The version of every bundle is the `version` of `Cargo.toml`. The ARA factory and the CLAP
descriptor take it at compile time. `check.sh` sets `CFBundleVersion` and
`CFBundleShortVersionString` of the CLAP, and gives it to the wrapper
(`CLAP_WRAPPER_BUNDLE_VERSION`) for the VST3.

The signature is the last step of the build. Every bundle is signed ad hoc with
`codesign --force -s -`: first the CLAP inside the VST3, then the VST3 (a later change of a
bundle file breaks the seal). `codesign --verify --deep --strict` passes on the bundle, and the
seal covers the page. A real identity and the notarization are part of M2.

`./check.sh deps` fetches the third-party parts into `.deps` (or `$DS_ARA_DEPS`): the ARA
SDK 2.3.0 API headers, the ARA examples (with `ARA_Library` and the CLAP 1.1.9 and
VST3 SDK v3.8.0_build_66 companions, patched like S1, for the independent conformance
host), clap-wrapper 0.17.0 with the patch of `patches/`, pluginval 1.0.4 (pinned SHA-256)
and clap-validator 0.4.1, and both macOS Rust targets. Heavy: run it under the host lock.

## The editor (M1b2)

The plugin has the CLAP extension `clap.gui` (cocoa, embedded). The editor is the Drum Spike
page (`static/`) in a `wry` web view (WKWebView) as a child of the host's view, on the real
service of the ARA document. The design, the threads and the measured results are in
`docs/design/daw-plugin/m1b2-editor.md`. In short:

- The build copies `static/` to `Contents/Resources/page` of the CLAP (the VST3 carries the
  whole CLAP in `Contents/PlugIns`, page included). The plugin finds the page from the path
  that the host gives to `clap_entry.init` (the wrapper gives the path of the embedded CLAP),
  or from the address of its own code. A scheme handler for `dspage://localhost/` serves the
  files; a Content-Security-Policy keeps every request on that scheme. There is no network.
- A shell script (`src/editor/shell.js`) gives the page `window.__TAURI__` and sets
  `window.__DS_HOST_MODE__`. `invoke` runs `drumspike_app::call::run` on the service of the
  document, on a thread of its own, never on the host's main thread.
- The web view is made, used and dropped on the host's main thread only. One web view lives
  as long as the plugin instance (wry 0.57 leaks a dropped web view, see the design note).
- Which hand edits survive a round: hits the user added, moved or edited, and the settings of a track
  (role, mix, params), also across a region change. A hit the user DELETED comes back at the next
  round (the app's "keep my edits" keeps added, moved and edited hits only; the note names the two
  options for the owner).
- A round of the plugin keeps the hits the user placed in the page (the app's "keep my edits"),
  and after a round the page loads its project again (the event `plugin-reload`). The host owns
  the track set and the project files: the editor answers 403 for the commands that change them,
  for the agent skill, the feedback post and the app's own audio transport. Every command of the
  app is in exactly one class (a unit test walks the app's registry).
- The web view uses a non-persistent data store: the page keeps no settings between two
  openings of the editor (open point: wry 0.57 cannot place a persistent store in the cache folder).

## Cache folder

The plugin writes only below one cache folder:

    ~/Library/Caches/DrumSpike/ARA

(`DS_ARA_CACHE` overrides it in the check.) Inside it, one folder for each ARA document:

    documents/<id>/audio/<n>-<stem>.wav   the source audio the host delivers through the
                                          ARA audio access controller, one file per ARA
                                          audio source of this document
    documents/<id>/project/               the Drum Spike project of the document (one track
                                          per audio source; the host's playback regions
                                          are the segments of the track)
    documents/<id>/service/               the home folder of the service (its recent list)

The plugin never writes into an audio file of the session, and nothing below the user's
home outside this folder.

The id registry (which keeps two live documents apart) holds for ONE process. Two processes
that open the same session file share `documents/<id>`: the plugin does not lock the folder.
A host that runs the plugin in more than one process (a plugin sandbox, two instances of the
DAW) must not open one session in both at the same time.

The plugin never removes a document folder. It cannot know whether an archive of the host
still names it. The size of a folder is about the size of the audio of its sources as 32-bit
float: 4 bytes for each sample and channel (a mono source at 48 kHz: about 11.5 MB for each
minute), plus the project (small). A user can delete a folder of a session that is gone; the
next restore of an archive that names it then starts from the stored events and the host's
audio again. An automatic clean-up rule is an open point (see the session note).

## The session

One document controller owns one `drumspike_app::Service` (made with the controller,
dropped with it) and one project folder. The id of the folder is made by the plugin once per
document and stored in the ARA archive, so a restore in a new process finds the same folder.

- A request to analyse sources adds them as tracks to the open project. The rule: whenever
  the set of analysed tracks of a document changes (a request adds sources, a source is
  destroyed, the regions or the properties of a source change), the result of ALL tracks is
  the result of one analysis of all those tracks together, as the service gives it (the
  session steps cross bleed, ghost gate, tom bleed and flam run over the tracks of one job,
  so a job over the new tracks alone would never let a track see the other microphones).
  Every round takes a fresh snapshot of every source. Requests that arrive within 250 ms of
  each other (at most 2 s in all) are one round; the report names the number of rounds. The
  project is saved after every round. A request for a source that already has its result
  changes nothing and starts no round.
- A source is one track for its life (the key is its creation index). A round that fails or
  is cancelled leaves the tracks; the sources of that round are complete for the host (it
  must not wait for ever) and have no notes, and the next request for them runs a round
  again. The service answers a result for every track: a track it could not analyse (a WAV
  that cannot be read) is named in the `errors` of the `job` row and the round is marked
  `partial`. If that track never had events, it has none and no notes. If it had events from
  an earlier round, they STAY in the document, in the archive and as notes, and the `job` row
  names the track in `stale`: they are the events of an earlier round, never the result of
  this one. The archive stores the mark per track (an archive without it loads as not stale); a
  restored stale track needs analysis, so the next round with audio analyses it again. Its source ends without a new result (complete for the host, retried at the next
  request). A track whose audio is not here (a track restored from
  the archive, or a cached file that is not the one the reader wrote: the plugin checks the
  size against the samples it wrote) is named in `skipped`, keeps its stored events, and
  marks the round `partial` too. A source whose audio does not come is waited for 30 s after
  its request; then the round goes on without it, the `job` row names it in `late`, a
  `source_late` row is written and the round is `partial`. When the audio of that source comes
  later, a round takes it at once (the host need not ask again); a later request does the same. The `job` row lists the tracks that took part. A move of a region in playback time
  alone (not its modification range) is no change and starts no round.
- The notes of an earlier source can change in a later round (a bleed hit leaves the output).
  The plugin then tells the host: in `notifyModelUpdates` it calls the host's
  `notifyAudioSourceContentChanged` for exactly those sources, and serves the new notes.
- `destroyAudioSource` removes the source from the controller's list at once; its track
  leaves the project (and its cached audio the cache) on the worker thread, and the tracks
  that are left are analysed together again.
- A restore from the archive sets the events of the document and the notes of the sources
  on the calling thread, then sends the project part to the worker: the worker may be busy
  for minutes with another document, and an archive stored in that time must carry the
  events. The worker writes the project document with the stored events and opens it. It
  decodes no audio file. The archive comes from a file the user may have edited: the stored
  document id is taken only in the form the plugin makes (16 lowercase hex digits), a track
  name only without a path separator and without `..`; anything else is replaced (a fresh id,
  a name from the source name) and the report says so. An id that another live document of
  the process holds (a session file that was copied, or opened twice) is not taken either:
  the second document keeps its own fresh id and its folder starts from the archive data. A restored track's source points at the cache path where the
  reader would write the audio; that file does not exist until the host gives the audio
  again. `project_open` and `events_get` work without it; a call that needs the audio gets
  the app's own offline answer (a crate test), and a later round skips such a track and keeps
  its stored events. An archive of M1a (version 1, no document id) restores into a fresh id;
  the archive version stays 1 (the two new fields are optional).
- Two document controllers in one process have their own service, folder, source numbering
  and file names.
- All of this runs on the plugin's worker thread. Nothing runs on the audio thread, and the
  host's main thread only reads and writes the state it needs for the ARA calls.
  `destroyDocumentController` stops the readers (each ends at the next block of 64k samples,
  so the join lasts as long as one host read call), cancels a running analysis, and then waits
  for the worker for the rest of 500 ms, counted from the start of the call. The plugin bounds
  its own work to 500 ms; the join of a reader is as long as the host's read call inside it,
  which the plugin cannot bound (a cancel that arrives before the analysis starts ends the
  round without running it); the worker owns the document through shared ownership and finishes the save on its
  own. When the host unloads the plugin library (`clap_entry.deinit`), the plugin cancels
  every running round, tells the worker to end and waits for it at most 1.5 s; the cancel
  takes effect between two tracks. A worker that is still inside one track after that stays
  behind (a thread cannot be stopped from outside), and the report says `ended: false`. A
  host that loads the library again in the same process gets a new worker.
- The reader threads and the worker thread run inside the panic guard. A panic in a reader
  ends that read as a failure and tells the worker; a panic in the worker ends the round it
  happened in (its sources end without a result) and the worker starts again. No function
  pointer of the host is unwrapped in the plugin code (the host tables are optional input).

## Analysis without a request (M1b4)

The plugin analyses a source as soon as its audio is here. The host need not call
`requestAudioSourceContentAnalysis`: LUNA does not (the finding of the first run in a real DAW).
A request still works and gives the same result, with no second round for the same audio. A
source restored with its result is not analysed again because its audio is read. The rule and
the finding are in `docs/design/daw-plugin/m1b4-analyse-without-request.md`.

The plugin also reads new samples when the host sends `updateAudioSourceContent` (signal changed)
and analyses them, and it tries a failed round again by itself (three times, then a row
`retry_gave_up`). Stages `no-request-update`, `no-request-retry`, `no-request-retry-limit`,
`no-request-cancel` and `no-request-spawn`, each with a fault run; the rules are in the design note.
Test host options: `grow --update-content <k>:<wav>`, `--linger-ms <n>`, `--cancel-read <phase>`.

The test host has a mode for it: `ara-host <bundle> grow --files <wav>... --no-request` (the
host enables the samples access and never requests). `--request-order read-first|request-first`
and `--slow-read <k>:<ms>` make the other orders of the stages `request-order` and
`no-request-group`. The switch `DS_ARA_ROUND_FAULT=request-only` brings back the old way (a round
only on a request); the stage `no-request` then fails with "no round". The stages are
`no-request`, `request-order`, `no-request-group`, `no-request-restored` and
`editor-no-request`, each with a fault run.

The plugin has the CLAP extension `clap.state` (LUNA asks for the state of every instance and
logged "loadPreset failed" without it). The state is a small versioned blob with no project
data; load accepts an empty stream and a newer version. Stage `state`, with the fault switch
`DS_ARA_STATE_FAULT=strict`. The VST3 wrapper maps `getState` and `setState` to these two
functions; no tool in `.deps` calls them on the VST3 alone, so they also have unit tests.
The report has evidence rows of what the host sends (`view_selection`, `view_hide_sequences`,
`process_first`, `transport_seen`); the audio thread writes none of them.

## The host transport (M1b5)

The page in the editor follows the transport of the host: the playhead moves with the host, stops
with it and jumps at a locate. The plugin sets `B.hostTransport` in the shell script of the page
and feeds it. The page plays no audio. Play, stop and locate of the page go to the host (next
section).

- **Source:** the transport of the CLAP `process` call (song position in seconds, play flag). ARA
  gives a plugin no transport. In the VST3 the wrapper fills the same fields from the host.
- **Hand-over:** the audio thread stores atomics in a sequence lock (no mutex, no allocation, no
  row). The long poll `/__ds/transport` of the editor reads it.
- **Rate:** one message at a change of the state or a jump, 25 a second while the host plays, none
  while it is stopped and unchanged.
- **Mapping:** the playback regions of the last round (the ones the notes use) turn the song
  position into the source time that the page shows. Outside every region the playhead runs on
  with the offset of the nearest earlier region.
- **No transport** (no process call, or a call without one): the playhead stays, no message, no
  error row.
- **Report rows:** `transport_seen` (first process call with a transport, with its `source`),
  `transport_page_first` (first message to a page) and `transport_stale` (a playing host stopped
  its process calls).

The test host plays a transport script: `ara-host <bundle> editor --editor-mode transport
--transport script|loop|null|none [--regions-all a,d,p]`. Stages `transport-handover`,
`editor-transport`, `editor-transport-measure` and `editor-transport-none`, each with a fault run
(switch `DS_ARA_TRANSPORT_FAULT`). The design is in `docs/design/daw-plugin/m1b5-host-transport.md`.

## The page moves the host (M1b7)

Play, stop and locate in the page move the transport of the host, as the ARA editors of other
plugins do. The page time is a time of the source; the plugin turns it into a song position with
the playback regions of the last round (the inverse of the mapping of M1b5) and calls the ARA
playback controller of the host (`requestStartPlayback`, `requestStopPlayback`,
`requestSetPlaybackPosition`). The position that the host lands on comes back through the
transport follow of M1b5. The page does not move its own playhead for a play or a locate that the
host did not do.

- **Shell commands:** `plugin_host_play`, `plugin_host_stop`, `plugin_host_locate` and
  `plugin_host_offer` (not commands of the app). The shell script of the page calls them from
  `B.hostTransport.play/stop/locate`.
- **Thread:** the calls to the host go to the main thread of the host, the thread of the document
  controller. The request thread never calls the host.
- **Several regions:** a region that holds the page time and the host position, else the first
  that holds the page time, else the one that holds the host position, else the first region.
- **No region** (no round yet, a source without a region): nothing is sent and the page says why.
- **No playback controller, or a call that is not offered:** the Play button and the ruler show
  "not available" with a text that names the host. No error row is written.
- **Order:** one queue of host requests per editor; the newest locate wins, at most 8 wait, they do
  not use the budget of the app commands, and a closed editor makes no call.
- **Selected track:** the follow and the locate use the regions of the track the page has selected
  (the shell tells the plugin), and the time of a locate is bounded.
- **A host that does nothing:** the shell shows the last host position again after 1 s.
- **Report rows:** `playback_offer` (what the host gives, at the creation of the document
  controller), `playback_request` (the first request of each kind) and `playback_unavailable` (the
  first refused request of each kind).

The test host has a playback controller: `ara-host <bundle> editor --editor-mode playback
--transport follow --playback full|ignore|partial [--snap <s>]` (without `--playback` it gives
none). Stages `playback-map`, `editor-playback`, `editor-playback-none`, `editor-playback-partial`
and `editor-playback-refused`, each with a fault run (switch `DS_ARA_PLAYBACK_FAULT`). The design
is in `docs/design/daw-plugin/m1b7-page-moves-host.md`.

## The check

`plugins/ara/check.sh check` (-> `check.py all`) runs every gate line of the ticket: the
fixture session (four tracks with bleed, roles from the names), the byte-for-byte compare
against `service_tool analyze` on the CLAP and on the plugin inside the VST3, the regions
fixture (two regions, one trimmed; the track is made from segments), the archive store /
restore with the decode counter at 0 and the committed fixture `fixtures/archive-v1.ara`,
the audio-thread check with its fault run, the file probe, pluginval (strictness 10, on both variants of the VST3) and
clap-validator, the standalone rule, and the crate and workspace trios. The M1b1 stages
(`doc-*`, each with a fault run) drive the host modes `grow` (sources arrive in batches, one
can be destroyed) and `docs` (two document controllers in one process): one service per
document, all tracks analysed together (a fixture whose split result differs), one round for
requests that arrive close together, the host told about changed notes, two documents apart, the document id in the archive, a destroyed
source leaves the project, the destroy of a controller during an analysis returns in under
1 s, and the home of the service (a run with `HOME` below `out/`
lists the files below it outside the cache folder: 0). The stages of the review round
(`doc-id-hostile`, `doc-same-id`, `doc-restore-busy`, `doc-retry`, `doc-rerequest`,
`doc-errors`, `doc-regions-change`, `doc-close`, `doc-panic`, `switches`, `no-unwrap`) are in
`docs/design/daw-plugin/m1b1-session.md`, each with its fault run. The ticket's "in
the ARA test host" is two hosts: the plugin's own `ara-host` drives the stages above, and
the `ARATestHost` of the ARA SDK examples - built from the S1 pattern with the S1 patch -
runs its own tests (ContentReading, PropertyUpdates, Archiving, PlaybackRendering,
EditorView) on the CLAP and on the wrapped VST3, each must exit 0. The plugin has no
editor GUI, but it implements the EditorView role interface and its playback renderer
passes audio through, so no test is skipped.

## Test switches of the check

A real host sets none of these. The list is `switches.json`: the table below is made from
it (`check.py write-readme`), the stage `switches` fails when the source uses a variable that
is not listed, when a listed variable is not used, or when this table was edited by hand, and
`check.sh` fails at the start when a fault or scenario switch is set in the caller's
environment. A normal run of the check clears every variable of the list for the programs it
starts.

The rule is a cargo feature. The fault and scenario switches are read only by a build with
the feature `test-switches`; the gate builds the plugin so. A delivery build has no such
feature and never reads them: a variable in the environment of a host cannot change the plugin.
That includes the two that weaken safety on purpose, `DS_ARA_ID_FAULT=trust` and
`DS_ARA_HOME_FAULT=1`. The code reads them only through one function (`test_switch` in
`src/lib.rs`). `switches.json` stays the one list. `check.sh build` makes the delivery variant. The stage
`delivery-switches` runs the delivery CLAP (inside the delivery VST3) with `DS_ARA_ROUND_FAULT=request-only`
and shows that it still gives the round, while the gate CLAP obeys the variable. The fault run
of that stage refuses the gate CLAP.

The report variables `DS_ARA_OUT` and `DS_ARA_CACHE` are not switches. They work in both
variants, because the hand checks in a DAW need them.

<!-- switches:begin (made from switches.json by `check.py write-readme`) -->

| Variable | Kind | Values | What it does |
| --- | --- | --- | --- |
| `DS_ARA_CACHE` | config | a folder | the cache folder the plugin writes below (default ~/Library/Caches/DrumSpike/ARA) |
| `DS_ARA_OUT` | config | a file | where the plugin writes its report, one JSON object per line |
| `DS_ARA_DEPS` | check | a folder | check.sh only: the folder of the third-party parts (default .deps) |
| `DS_ARA_TESTHOST` | check | a file | check.sh only: the ARATestHost binary of the conformance stage |
| `DS_ARA_FAULT` | fault | in-process | the analysis runs inside process on purpose, so the overrun check can fail |
| `DS_ARA_TABLES_FAULT` | fault | 1 | the plugin serves a document controller table with one emptied slot |
| `DS_ARA_RESTORE_FAULT` | fault | decode, newid, empty, late-events | the restore decodes one source, ignores the stored id, drops the stored events, or sets the events of the document on the worker thread |
| `DS_ARA_SERVICE_FAULT` | fault | per-job | a new service for every job (the M1a way) |
| `DS_ARA_TRACKS_FAULT` | fault | forget | a later request forgets the earlier tracks |
| `DS_ARA_FOLDER_FAULT` | fault | shared | every document uses one folder |
| `DS_ARA_DESTROY_FAULT` | fault | keep | a destroyed source stays in the list and the project |
| `DS_ARA_HOME_FAULT` | fault | 1 | the service home is below the user's home, outside the cache folder |
| `DS_ARA_ROUND_FAULT` | fault | new-only, always, no-deadline, no-late-round, no-page-wait, request-only, ready-only, read-again, ignore-update, no-retry, retry-forever, force-failed, no-spawn-failed, clear-stale, no-read-retry, keep-wavs, clear-failed, cancel-fails | a round analyses only the new tracks, a request for a complete source starts a full round, a request waits for ever for audio that does not come, or audio that comes after the deadline starts no round; request-only: a round starts only on a request of the host (the plugin before M1b4: LUNA never asks, so it got no round); ready-only: a source is queued only when its audio is here, so sources whose audio comes one after the other get a round each; read-again: the read of a source that has its result (one restored from an archive) starts a round; ignore-update: updateAudioSourceContent does nothing (new samples of a source are never read or analysed); no-retry: a failed round is not tried again by the plugin; retry-forever: it never gives up; force-failed: a source whose read was cancelled forces a round of the complete tracks; no-spawn-failed: a reader thread that cannot start leaves its source waiting for the audio deadline; clear-stale: a round on the old audio marks the new audio of a content update as analysed; no-read-retry: a failed read of new audio is not tried again; keep-wavs: the WAV files of a destroyed source stay; clear-failed: a round on the old audio clears the failed mark of a source whose new read failed for good (it stays incomplete for ever); cancel-fails: a read that a content update cancelled counts as a failed read; no-page-wait: a round or a removal does not wait for a job of the page: the round fails with its 409 and a failed removal wipes the events of the document |
| `DS_ARA_SETTLE_FAULT` | fault | none | no settle time: every request is a round |
| `DS_ARA_CLOSE_FAULT` | fault | wait, nocancel | the destroy of a controller waits for the round, or does not cancel it |
| `DS_ARA_ID_FAULT` | fault | trust | the plugin trusts the document id and the track names of an archive, and the names of the host sources |
| `DS_ARA_REGISTRY_FAULT` | fault | off | no registry of the document ids in use: two live documents may share one |
| `DS_ARA_ERRORS_FAULT` | fault | swallow | the error of a single track is not read |
| `DS_ARA_RETRY_FAULT` | fault | duplicate | a retried source makes a second track |
| `DS_ARA_TOUCH_FAULT` | fault | ignore, playback-time | a change of the regions or the properties of a source is not seen, or a move in playback time alone starts a round |
| `DS_ARA_GUARD_FAULT` | fault | off | the reader thread and the worker thread run without the panic guard |
| `DS_ARA_UNLOAD_FAULT` | fault | skip | the unload of the library does not stop the worker |
| `DS_ARA_PANIC_TEST` | scenario | reader, worker | the reader thread or the worker thread panics once |
| `DS_ARA_SCENARIO` | scenario | corrupt-wav:<n>, fail-first-round, slow-setup, audio-deadline-ms:<n>, fail-rounds:<n>, no-reader-thread | the WAV of source n is damaged after the read, the first round fails once, every round waits 1.5 s before its analysis starts, or a request waits n ms for audio (default 30000); fail-rounds:<n>: the first n rounds fail; no-reader-thread: the reader thread of a source cannot start |
| `DS_ARA_RETRY_MS` | scenario | milliseconds | the first wait of the automatic retry of a failed round (default 2000, then twice as long each time; kept between 10 and 60000) |
| `DS_ARA_STATE_FAULT` | fault | strict | the load of the plugin state refuses an empty stream, a newer version and anything else than its own blob (the host's instance creation then fails, as in LUNA) |
| `DS_ARA_EDITOR_MODE` | scenario | ready, keys, focus, resize, measure, edit, shell, analyse, net, bigpost, transport, playback | the editor page runs a test driver of that name and reports to the plugin (the test host sets it itself); transport: the page samples its drawn position while the host plays a transport script; playback: the page does play, stop and locate like a user and the host records what it is asked |
| `DS_ARA_EDITOR_ARG` | scenario | a text | the argument of the test driver of the editor page (a WAV path of the shell mode) |
| `DS_ARA_EDITOR_FAULT` | fault | no-ready, no-bundle, no-shell, no-sync, no-record, no-notes, no-reload, no-events, drop-edits, drop-track-data, missing-function, allow-track-set, no-focus, no-resize, inline, leak, slow-frames, page-transport | the editor: the page's ready call is not counted, the plugin finds no page in its bundle, the shell commands are left to the service, a command of the page does not reach the archive or the request thread does not record it before the page's answer (no-record), the notes do not follow an edit, the page is not told that a round ended or gets no event of the service, a round of the plugin drops the hand edits of the page, the shell script of the page finds a function missing (it reports it), a region change drops the hand data of the track (hits, role, mix), the page may change the track set, the web view is not the first responder or does not follow a resize, a command runs on the main thread, every editor leaks 4 MB, the page draws slow frames, or (page-transport) the page runs a command of the app's own transport |
| `DS_ARA_TRANSPORT_FAULT` | fault | off, identity, flood, invent, alloc, keep-playing | the host transport of the page (M1b5): off: the audio thread stores no transport, so the page never follows; identity: the song position is the page time, the regions are not used; flood: a message to the page every 8 ms instead of every 40 ms; invent: with no transport seen, the page is told the host plays from 0; alloc: the audio thread allocates in its call (the unit test that names the hand-over fails); keep-playing: a cleared slot or a host with no position does not stop the page (the way before review round 3) |
| `DS_ARA_PLAYBACK_FAULT` | fault | direct, identity, skip, anonymous, error-row, no-fallback, no-supersede, no-drag-guard, keep-error, first-track | the page moves the host (M1b7): direct: the request thread calls the ARA playback controller itself, not the main thread; identity: a locate goes to the host with the page time as song position (no regions); skip: the plugin makes no call to the host and says it did; anonymous: the text of a missing control does not name the host; error-row: a refused request of the page is an error (an error row is written); no-fallback: the shell never shows the host position again after a request the host did not follow, and Play ends at once; no-supersede: every locate of the queue of host requests runs, none is dropped for a newer one; no-drag-guard: the shell shows the host position again under the pointer of a drag; keep-error: the text of a failed play stays when the host starts later; first-track: the follow of the host transport maps with the first source, not the selected track |
| `DS_ARA_EDITOR_SCENARIO` | scenario | panic:<command>, sync-delay | the named command of the page panics inside the plugin where the service would run it, or the worker hears 12 s late about a change of the page (the request thread's own record is not delayed) |

Kinds: **config**: The plugin reads it in every build, the delivery build too (the check sets it for every run; the hand checks in a DAW need it). **check**: Read by the check only. **fault**: Fault switch: breaks one line of the gate on purpose, so the check of that line can fail. Read only by a build with the cargo feature `test-switches` (the gate build); a delivery build ignores it. **scenario**: Scenario switch: sets up a case for a stage (not a fault). Read only by a build with the cargo feature `test-switches`; a delivery build ignores it.

<!-- switches:end -->

## Licences

The third-party parts of the plugin and of the check, one row each (the versions are the
ones `.deps` fetches):

| Part | Version | Licence | Where it is used |
| --- | --- | --- | --- |
| ARA SDK (ARA_API, ARA_Library, ARA_Examples, Celemony) | 2.3.0 | Apache-2.0 (its bundled examples: cpp-base64 zlib, pugixml MIT, ICST AudioFile BSD-2-Clause) | `ARA_API` headers feed the bindgen bindings; the examples build the independent `ARATestHost` of the conformance run. Nothing of it is linked into the plugin. |
| CLAP | 1.1.9 | MIT | The entry/factory ABI the plugin implements (headers only). |
| clap-wrapper | 0.17.0 | MIT | Compiled into the wrapped VST3 bundle. |
| VST3 SDK | 3.8.0_build_66 | MIT | Compiled into the wrapped VST3 bundle (the wrapper's module and `pluginterfaces`). |
| pluginval | 1.0.4 | GPL-3.0 (Tracktion) | Check-time tool only; never part of a bundle. |
| clap-validator | 0.4.1 | MIT (Robbert van der Helm) | Check-time tool only; never part of a bundle. |
| wry | 0.57.0 | Apache-2.0 OR MIT | M1b2: the web view of the editor (macOS: WKWebView). No default features. |
| objc2, objc2-foundation, objc2-app-kit (and the objc2-* crates they bring), block2, raw-window-handle, dpi, url and its icu crates, http, cookie and the small crates of wry (see the design note for the full list) | objc2 0.6.5, app-kit and foundation 0.3.2 | MIT, Zlib OR Apache-2.0 OR MIT, Unicode-3.0, ISC (libloading) | M1b2: compiled into the plugin binaries. `dirs-sys` brings `option-ext` 0.2.0, MPL-2.0 (a file-level copyleft; used unchanged as a library). |
| The page (`static/` of this repository) | this commit | Apache-2.0 (repository code) | M1b2: copied into `Contents/Resources/page` of the CLAP. Its own third-party parts (help, brand files) go on the list of `NOTICE` before any delivery. |
| Rust crates of the plugin | `Cargo.lock` | 149 crates outside this repository (`cargo metadata`, macOS): MIT, Apache-2.0, BSD-2/3-Clause, Zlib, ISC, Unlicense, Unicode-3.0, 0BSD, and one with a file-level copyleft: `option-ext` 0.2.0, MPL-2.0 (through `dirs-sys` of wry; used unchanged, so its notice and the MPL text go on the `NOTICE` list before delivery) | Compiled into the plugin binaries. `drumspike-app` and `drumspike-core` are this repository's own crates (Apache-2.0). |

The crate itself is repository code under the repo licence (Apache-2.0, see `NOTICE` at
the repository root). Before any delivery of a bundle (M1c), `NOTICE` must list the parts
of the bundle (the web view crates, the page and its help and brand files, `option-ext` with the MPL-2.0 text) and the parts
that end inside it (the wrapper and the VST3 SDK module, the plugin's crates and their
notices) the way the app's `THIRD_PARTY.md` does.
