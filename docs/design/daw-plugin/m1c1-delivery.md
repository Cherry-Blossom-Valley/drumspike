# M1c1: MIDI and spikes leave the plugin, in song time

Ticket #560, umbrella #503 (the DAW plugin program), part of M1 (#510), first part of M1c (delivery).
The plugin is in `plugins/ara/`. This note is the design that phase 2 builds. It decides each point
with the reason and the code place. A fact that we did not measure says so.

The first complete run in a real host showed the hits in the plugin page. Nothing could leave it:
export is absent in host mode (#539) and the file drag answers 501. This part gives the user two ways:
a file drag out of the plugin window onto a DAW track, and an export into a folder.

## What exists today

- The app exports in the time of the source: file time 0 is sample 0 of the audio file (`crates/app/src/export.rs`,
  `export_all_opts`; the hit time `t` of an event is the time in its source).
- The page of the app has the Export dialog (`static/export.js`). Its drag tiles use two commands:
  `export_drag_prepare` renders the files into the hidden folder `Exports/.drag/` of the project, and
  `export_drag_take` moves the dragged files into `Exports/`. Then `B.dragFiles` calls `plugin:drag|start_drag`
  (`static/backend.js`, plugin `tauri-plugin-drag` 2.1.1, which uses the crate `drag` 2.1.1).
- The plugin runs the page in a `wry` web view. Its commands go to `plugins/ara/src/editor.rs`. The class of a
  command decides: `ALLOWED` (runs on the service of the document), `SHELL_COMMANDS` (answered by the plugin),
  or refused. `writes_outside` refuses `export_all`, `export_existing`, `export_dest_set`, `export_drag_prepare`,
  `export_midi`, `export_spikes`, `export_layer` and `sampler_calibration_midi` when the call has a non-empty `dest`
  or `path` (rule of #532: a command writes only below the cache folder of the document).
  `plugin:drag|start_drag` answers 501. The shell script (`plugins/ara/src/editor/shell.js`) answers
  `dialog.open` and `dialog.save` with cancel (null).
- `HOST_ABSENT` group "files" in `static/state.js` hides the export menu items, the export commands and the
  controls `#exportAllBtn`, `#exportModal`, `.lyRender`, `.smCal`, `#insFileRow`, `#insStateRow`.
- The plugin knows, per source, the playback regions of the last round (`round_regions` in `lib.rs`; the
  `RegionRec` of `service.rs` has `start_mod`, `dur_mod`, `start_play`). `Backend.regions` and
  `Backend.track_info` give them to the editor. `transport.rs` has `page_time` and `song_time` (m1b5, m1b7).
  The notes for the host (#532) use `notes_for` and `source_sample` with those regions.
- The musical context of ARA is a stub: `dc_createMusicalContext` keeps a counter and reads nothing.
  The plugin does not use the content access controller of the host. The process call of the host gives
  a transport with seconds and a tempo, but no beats (fact from the real run).

## 1. Time base

Decision: a file of a track starts at song position 0 of the DAW and holds each hit at its song position.

Mapping of one hit with source time `t` (the time in the source, as stored in the event):

- For each region `r` of the source with `r.start_mod <= t < r.start_mod + r.dur_mod`: song time
  `s = r.start_play + (t - r.start_mod)`. This is the inverse of `page_time`; it uses the same regions as the
  notes of #532 (`round_regions`, else the live ones, `regions_for_notes`). The events belong to that round.
- Several regions per source (a clip that the user copied): the hit comes once per region that holds it, at
  each `s`. Two regions of one source that overlap in the song give two notes at different times; that is
  what the user hears in the DAW, so it is right.
- A hit in no region (a gap, a trimmed-away part): not in the file. A hit whose `s` is below 0 (a region that
  starts before song 0): not in the file, counted apart. Today the MIDI export moves a hit before the start
  to tick 0; in song time that note would be at the wrong place, so it is dropped. Both counts go into the answer
  (`outside`, new key `outside_song`) and the page says them: "N hits outside every region, M before the song
  start: not in the files".
- A source that never had a playback region: identity (`s = t`); the file length is the source length. A source whose clips were all deleted in the DAW is not that case: see round 3 and round 4 (every hit left out). This is the rule
  of `song_time` and `source_sample` for no region.
- A region with a time stretch: a hit follows the stretch as one linear factor (`dur_play / dur_mod`); warp markers inside a clip are not followed. The page says so. (m1b5 still maps the playhead without the stretch.)
  The note names this limit; the page does not hide it (open question 5).
- Several sources at one offset (the usual drum session) give the same offsets; nothing special.

Spike WAV file (synthetic spikes):

- The sample rate and the sample format are those of the source (as today). The file length is the end of
  the last region of the source in song time, in samples (`ceil((start_play + dur_mod) * rate)`, maximum over
  the regions). No region: the source length. Each hit is at sample `round(s * rate)`, the same rounding as the
  note anchor of the MIDI export (#441), so the spike and the note meet.
- Broadcast Wave: the file has a `bext` chunk with `time_reference` 0, description "Drum Spike spikes". Today the spike
  copies the `bext` of the source and the plugin's cached WAV has none, so there is no chunk. In song time the
  chunk is written always: the file starts at song position 0 and says so. A DAW that places a Broadcast Wave file
  by its time reference then puts it at 0 too, with or without the user's drop position. (We did not measure which
  host reads the time reference on a drag; the hand check sheet asks.)
- The audio-spike format (`write_audio_spike_file`) is not in M1c1: it cuts snippets of the source at the
  source time and would have to place them at the song time. The rows stay hidden in host mode (open question 2).

What the existing export code needs (the only change in `crates/`):

- A new trait `ExportTime` in `crates/app/src/export.rs` and a setter on the service,
  `Service::set_export_time(Option<Arc<dyn ExportTime>>)` (a field in the inner state). The trait gives, per track name:
  the regions `(start_mod, dur_mod, start_play)` or none, the output file stem, and (once) the tempo map in song seconds.
  `build_jobs` asks it. With no object set (the standalone app, the CLI, the MCP server, every test that exists
  today) the code path is the old one, line for line. The page cannot set it: the plugin sets it at the creation
  of the service of a document (`plugins/ara/src/service.rs`, near `Service::new`).
- Why not a field of `ExportOpts`: `ExportOpts::from_args` parses the page's JSON, so the page could pass regions.
  Why not the plugin mapping the events itself: `export_spikes` takes events, but the spike length and the
  `bext` come from the source file in `write_spike_file`, and the drag and export commands would have to be
  rebuilt in the plugin (argument parsing, the follower refresh, the owner check, the single file mode).
- `Job` gets `song: Option<SongTrack>` (regions, length). `export_track` maps `events` to song events once
  (new pure function `to_song(events, regions) -> (Vec<Value>, Counts)` in a new file
  `crates/app/src/export_song.rs`) and writes the spike with a new core function `write_spikes_song(out, rate,
  n_frames, times, format)` in `crates/core/src/spike.rs`, which writes the `bext` with time reference 0.
  `write_spikes` is not touched.
- The identity proof: the unit tests of the old path stay unchanged; a new test runs `export_all_opts` with no
  object and compares the bytes with a golden made on the base commit (`tests/test_exports.py`,
  `tests/golden/*` are not touched, they are read). `tools/identity.py snapshot` on the base commit and on the head,
  then `compare`, shows 0 difference (the refactoring rule of AGENTS.md).

Sampler offset ("Compensate"): it works in source time, so it is computed first (`precompute_midi`) and the mapping
comes after; the region of a note is the region of its unshifted hit. In the plugin the calibration controls
stay hidden (`.smCal`, `.lyRender`), so no track has one; the option row is hidden in host mode and the plugin
passes `compensate: false`. Markers in MIDI: no marker is written in song time (the markers of the page are source times
and a source with two regions has no one song time for them); the row is hidden.

File names: the files are named after the DAW track, not after the key of the track. The key is a hash of
32 hex characters in the first host; a user who drags `3fa9..._spike.mid` onto a track cannot tell what it is.
The trait gives the stem: the display name of m1b10 (`display_names`), made safe by the rule of `stem_of`
(`service.rs`), with the key as fallback. The names are unique (m1b10 numbers a double name), so the owner check
(`check_file_owner`) stays valid. Result: `<DAW track name>_spike.mid`, `<DAW track name>_spike.wav`.

## 2. Tempo in the MIDI file

What the file carries: the existing writer (`export_midi_map`, `export_multi_midi`) already writes tempo events,
time signature events and a `Bar 1` marker from a `TempoMap` (seconds of the file). In song time the seconds of
the file are song seconds, so the plugin gives the writer a `TempoMap` in song seconds. It is not written into the
document: the document's `tempo_map` is in source time (the page draws it on the lanes), the host's map is in song
time, and two regions of one source put the same source time at two song times. The map comes through the trait.

How the plugin reads it (ARA 2, `ARAInterface.h`, "Timeline"):

- `isMusicalContextContentAvailable` and `getMusicalContextContentGrade` of the content access controller of the
  host (`ARADocumentControllerHostInstance.contentAccessControllerInterface`, optional: check for NULL and for
  `structSize`), for `kARAContentTypeTempoEntries` (20) and `kARAContentTypeBarSignatures` (21).
  Then `createMusicalContextContentReader` with a NULL range, `getContentReaderEventCount`,
  `getContentReaderDataForEvent` per event, `destroyContentReader`.
- Tempo entries `(timePosition s, quarterPosition)` are sync points: between two points the tempo is constant,
  `bpm = 60 * dq / dt`. Before the first point the tempo is that of the first section, after the last that of the
  last section. A valid timeline has at least 2 points and one at quarter 0.
- Bar signatures `(numerator, denominator, position in quarters)`. The first one is bar 1.

Conversion to `TempoMap` (a pure function `tempo_from_ara(entries, bars) -> Result<TempoMap, Reason>` in a new
file `plugins/ara/src/tempo_ara.rs`, unit tested):

- `bpm` = tempo of the section that holds song time 0. One `TempoStep {t, bpm}` per later sync point where the
  tempo changes by more than 1e-9 relative (equal sections merge). A step closer than 1 ms to the one before
  (`MIN_STEP_GAP_S`) merges into it (the later tempo wins; counted in the report). Sync points at `t <= 0` only
  set the start tempo.
- `meter` = the first bar signature, `changes` = the later ones as `(bar, num, den)` with the bar number computed from
  the quarter positions (a position that is not a whole number of bars of the earlier signature: that signature
  and the later ones are dropped, the reason is `bars-not-whole`).
- `bar1_s` = the time of the first bar signature's quarter position through the sync points. When it is below 0 the
  map keeps `bar1_s = 0` (`TempoMap` does not support bar 1 before 0, `validate`); the file's bar lines then start at
  song 0. The reason `bar1-before-start` goes to the page.
- A tempo outside 4 to 1000 BPM or more than 100,000 steps fails the conversion: reason `tempo-range`.

The arithmetic that puts a note within 1 sample (existing, #441/#466, `crates/core/src/tempo.rs`, `midi.rs`):

- `export_ppq(rate, map, ...)` takes ticks per quarter so that one tick is one sample at the rate when this fits 15
  bits, else 32767 (map with several tempos: 32767).
- `TickMap::new(ppq, map)`: a step becomes a `set_tempo` at the tick nearest to its time; ticks come from the exact tempo
  (`exact_us = 6e7 / bpm`), and the `set_tempo` value is rounded to whole microseconds.
- Each note-on is anchored at its sample: tick = nearest tick to `round(s * rate) / rate` through the file's map.
  `worst_error_samples` measures the distance of the grid; the page says it when it is above 0.5 sample (existing text
  in `exSummary`).
- Bound, stated and tested: the note reads back (`TickMap::time(tick)`) within 1 sample of `s` at the session rate
  when one tick is at most 2 samples, that is `bpm >= 30 * rate / 32767` (about 44 BPM at 48 kHz, 88 BPM at
  96 kHz). Below that, the answer carries the measured worst error. A DAW that applies the file's whole-microsecond
  `set_tempo` instead of its own tempo sees an extra relative difference up to 1.3e-7 for a tempo whose quarter is not a
  whole number of microseconds (about 2 samples over 5 minutes at 48 kHz); no standard MIDI file avoids it. The unit
  test uses two tempos with whole-microsecond quarters (120 and 100 BPM) at 44.1 and 48 kHz for the 1 sample proof,
  and a second test with 97.3 BPM that asserts the reported bound equals the measured one.
- The single file mode (`<project>_drums.mid`) uses the highest rate of the staged tracks, as today.

With no tempo from the host, in this order:

1. The ARA timeline, when the host offers the content and the conversion works. Source `ara`.
2. The tempo of the process call (CLAP `transport.tempo` with `CLAP_TRANSPORT_HAS_TEMPO`), stored by the audio thread
   in the same atomic hand-over as the position (`transport.rs`; no allocation). One fixed tempo, no steps, meter 4/4.
   The plugin notes whether it saw more than one value (`tempo_changed`): then the source is `transport-varies` and the
   page says the host's tempo changes and the file does not follow. Source `transport`.
3. Nothing: the default map, 120 BPM. This is today's behaviour, including the warning `No tempo set` of
   `default_map_warning` and the ask `askDefaultTempo` of the page (#151). Source `none`.

The page shows which source it has: "tempo from the host's tempo map" / "one tempo from the host (97.3 BPM), no map" /
"no tempo from the host: 120 BPM". A command `plugin_host_tempo` (shell command, read only) answers
`{source, bpm, steps, reason}`; the Export dialog calls it when it opens.

When the plugin reads the content: at the end of an edit cycle of the host (`dc_endEditing` and
`dc_updateMusicalContextContent`, the thread of the document controller, the main thread), and once at the
creation of the editor. The result is a snapshot in a mutex (`musical: Mutex<Option<Musical>>`), so an export on a
request thread never calls the host. We did not verify that every host allows content access from another thread, so
the plugin makes the calls on the thread the host calls us on. `dc_createMusicalContext` must keep the host ref
(it gets `ARAMusicalContextHostRef` and drops it today): a record `{host_ref, id}` instead of the counter. A document
with several musical contexts uses the one of the region sequence with the lowest `orderIndex`.

What we do not know: whether the first host offers the content access controller and the two content types. The
report of the plugin gets a row `musical_context` with `{controller, tempo_available, tempo_grade, entries,
bars_available, bars, reason}` so the first run in the host tells it (like `playback_offer` of m1b7). Until it does,
source 2 and 3 are real paths, not corner cases.

Fit with the app's options: "Compensate" is hidden (above). "Markers in MIDI" is hidden. The default-tempo ask
(#151) fires only for source `none`. The project's own `tempo_map` and the page's tempo commands are not used by
the song-time export; the standalone export is unchanged.

## 3. The native file drag out of a web view in a plugin

How the app does it: the page calls `plugin:drag|start_drag {item: [paths], image: png data URL, onEvent}`. The plugin
`tauri-plugin-drag` decodes the image and calls `app.run_on_main_thread`, which calls
`drag::start_drag(&window, DragItem::Files, image, callback, Options)`. In `drag` 2.1.1, `platform_impl/macos`:

1. takes the `NSView` of the window handle, its `window`, and the window's `contentView`;
2. makes an `NSImage` from the PNG and one `NSDraggingItem` per file with `NSURL fileURLWithPath` as pasteboard
   writer (so the drag pasteboard holds `public.file-url` items, one per file; the files must exist before);
3. builds a synthetic `NSEvent` of type left mouse dragged at `window.mouseLocationOutsideOfEventStream`;
4. calls `contentView.beginDraggingSessionWithItems:event:source:` with its own `NSDraggingSource` object
   (class `DragRsSource`), whose `draggingSession:endedAtPoint:operation:` calls the callback (Dropped or Cancel).

What we reuse: the crate `drag` 2.1.1 itself (Apache-2.0 OR MIT, already in `NOTICE` for the app) as a dependency of
`plugins/ara`, with `default-features = false` and a small wrapper type that implements `HasWindowHandle` for the
AppKit view of our web view (the `raw-window-handle` AppKit handle from the `NSView` pointer that `editor.rs` already
has). The commented decision: try the crate first, because it is the code that works in the app. If the host test
shows a problem (below) the plugin gets its own module `plugins/ara/src/drag.rs` with the same steps and these
changes: the source view is our web view (not the host's content view), a check of the pressed mouse button, no
`expect`. The own module needs a unique Objective-C class name (`DSAraDragSource`), because two plugin builds in one
host process would register one name twice.

The shell command: `plugin:drag|start_drag` moves from the 501 answer to a real answer in `shell_command` of `editor.rs`:

- Check first, on the request thread: every `item` path is absolute, canonical, a regular file, and its parent folder is
  the `Exports/` folder of this document or a folder chosen in this editor session (section 4). Anything else: 403.
  The page is our own code, but a drag hands a file to another application, so the plugin does not drag a path the
  page names freely. At most 64 files. `image` must be a PNG data URL of at most 256 KB.
- Then hand the job to the host's main thread with `on_main` (as `playback` does, same queue style, `main_check`
  counted), wait for the answer up to `MAIN_WAIT` (2 s). The drag call itself returns when the session starts (it does not
  wait for the drop); the callback result (`Dropped` or `Cancel`) goes to the report as a row `drag_result` and to the page by the channel
  `onEvent`, which the shell script already can deliver.
- On the main thread, before the call: `NSEvent.pressedMouseButtons & 1` must be set. If the user released the
  button while the request travelled (fetch, request thread, main thread), the answer is `{ok: false, reason: "the
  mouse button was released"}` and no session starts. A session that starts with no button down would end at once or
  hang; we did not measure which, so the check stands before the call.
- The window must exist: the view's `window` is not nil. If it is nil: 409 "the editor has no window".

Risk list:

- R1 A drag session that starts from a mouse event the web view owns. The mouse down went to the `WKWebView`; the
  page decided on the move (4 px, `export.js` line of `mousemove`), awaited an IPC call and then asked for the
  session. In the app this works with the same order. Here the path is longer: `fetch` to the page's scheme, a
  request thread, a hop to the main thread. We measure the latency in the stage (drag start to session). Mitigation: the
  files are made when the tiles are ready (as in the app), so `exDragStart` awaits only `export_drag_take` (a rename)
  before the call. In host mode the default-tempo ask (a modal in the page) must not come between press and drag:
  the ask moves to the moment the tiles are rendered (open question 4).
- R2 The host window. The session starts on a view of the host's window, with a synthetic event at the mouse
  location in that window. A host with its own drag handling on that view could take the session or refuse
  the drop. We cannot know without the hand check. The own module (source view = our web view) lowers the risk.
- R3 Threads. `beginDraggingSession` is on the main thread only. A call off the main thread is refused by the
  plugin (`main_check`, `OFF_MAIN` stays 0 in the stage); a panic inside `drag` is caught by `run_boxed`.
- R4 The key shield of #561 handles keyboard events only; it does not touch mouse events. The drag must not break
  it either: while a drag session runs, the web view gets no key events, and when it ends the shield state is as before.
  The stage checks the shield counters before and after.
- R5 The pasteboard. File URLs are written when the session starts, from files that exist. If a file vanishes (the user
  deletes `Exports/` during the drag) the drop target gets a dead URL. The plugin does not lock the files.
- R6 Sandbox of the host. A host that sandboxes the plugin may deny file URLs outside its container to the drop
  target. We did not measure this; the files are in the cache folder of the document and in `Exports/` of the project.
- R7 The editor closes during the session. `destroy` ends our view; the session object holds only the callback. The
  callback checks the `closed` flag before it writes anything.

Automatic proof, and its limit: a real drop needs a real mouse. The stage in the test host starts the real
session with the real code, reads `session.draggingPasteboard` at once (the file URLs) and then ends the session
with the host's own cancel (an `NSEvent` mouse up posted to its window by the test host, no system-wide events). The drop target of the
test host is a view that registers for `public.file-url`; its `performDragOperation:` is called with a stub
`NSDraggingInfo` that returns the pasteboard of the real session, so the code that reads the paths is the code that a
DAW runs. The part that no stage covers: the user's mouse over the real DAW. That is the hand check (sheet, section 8).
Open question 1 asks whether a stage may post CGEvents to drag for real (needs the accessibility permission and takes
the mouse for a few seconds).

## 4. Export All in host mode: the folder, and the one exception to #532

Folder choice: the page's `pickFolder` calls `B.pickDir`, which calls `dialog.open({directory: true})`. The shell script
answers null today. New: `dialog.open` with `directory: true` calls a new shell command `plugin_pick_folder` (title,
default path). `pickFile` and `pickSave` stay cancel (the file dialogs are of the absent groups).

Panel type: the ticket says "save dialog". A save panel chooses a file name, not a folder, and the app's own folder choice is
the folder (open) panel with "Choose" and "New Folder". The plugin uses `NSOpenPanel` with `canChooseDirectories`,
`canChooseFiles = false`, `canCreateDirectories = true`, `allowsMultipleSelection = false`. For the user it is the same act as
in the app. (Open question 3: the coordinator confirms this wording.)

Thread and sheet: AppKit panels run on the main thread only. The request thread hops to the main thread with `on_main` and the panel
is shown as a sheet: `beginSheetModalForWindow:completionHandler:` on the window of our web view. Reasons: a sheet blocks
only that window; `runModal` would run a nested modal loop on the host's main thread, and the host's timers and
ARA calls (all on the main thread) would not run in the default mode while the dialog is open. The completion handler
(main thread) sends the result through a channel to the waiting request thread. The request thread waits in slices of
250 ms and watches `closed` (as `poll_channel` does); the 2 s of `MAIN_WAIT` apply to the hop only, not to the user. At most one
panel per editor; a second request answers 409 "the folder dialog is open". `destroy` of the editor ends the sheet
(`endSheet:`), and the answer is cancel. A window that is nil: 409. There is no `runModal` fallback.

The one exception: a list `chosen: Mutex<Vec<PathBuf>>` in `Shared` of `editor.rs` (one editor session: it is made at
`set_parent` and dropped at `destroy`). Only the result of `plugin_pick_folder` adds to it, after the plugin made the path canonical
(`std::fs::canonicalize`) and checked that it is a directory. No other command adds. Nothing is stored in the document or the state blob,
so a later editor session starts with an empty list.

The check, exact, in `writes_outside` (becomes `refuse_write(sh, cmd, args) -> Option<String>`):

1. Look at the keys `dest` and `path` as today. If both are absent or empty: allowed (as today).
2. `path` non-empty: refuse (as today; no exception).
3. `dest` non-empty: allowed only when `cmd` is one of `export_all`, `export_existing`, `export_dest_set`, AND `dest` is an
   absolute path, AND `canonicalize(dest)` succeeds, AND the result equals (path equality, no prefix, no descendant) one entry of `chosen`,
   AND the entry was canonical at the time of the choice (so a folder that was replaced by a symlink after the choice no longer
   equals). Otherwise refuse with the text of today.
4. Every other command in the old list (`export_drag_prepare`, `export_midi`, `export_spikes`, `export_layer`,
   `sampler_calibration_midi`) is refused with a `dest` as today. `export_drag_prepare` needs none: the page strips `dest` for it
   (`backend.js`) and the service writes to `Exports/.drag/`.
5. The service still runs `check_dest` (absolute, a folder, writable) and `check_export_plan` (the plan never writes a protected file:
   the audio of a source, the document's files). The exception adds a place, not a power.

`export_last_set` has a `dest` too, but it only stores text in the document. It stays as it is. A kept `export_dest` from an
earlier session is not in `chosen`, so `exportExisting` refuses it and the page falls back to `Exports/` with its own note
(`export.js`, "The folder kept for this project cannot be used"). That is the wanted fail-safe.

Faults: `DS_ARA_EDITOR_FAULT=any-dest` lets any `dest` through (the stage must then see a write outside and fail);
`DS_ARA_EDITOR_FAULT=prefix-dest` accepts a subfolder of a chosen folder (the stage writes into a subfolder and fails).
The test switch `DS_ARA_PICK_DIR=<folder>|cancel` answers `plugin_pick_folder` without a panel (build with `test-switches` only).

## 5. The page

What comes back in host mode (the entry `files` of `HOST_ABSENT` is split):

- Menu: `export` (File > Export...) and `export_selected` (Track > Export Selected Tracks...). Controls: `#exportAllBtn`, `#exportModal`.
- Inside the dialog: scope (all or selected track), MIDI, mode (per track or one multi-track file), Spikes with the synthetic type, the
  destination row with Choose... and Reset, the replace count, the Export button, the result line with Show in Finder, the two drag tiles
  and the tempo source line.
- Commands that leave the absent list: `export_all`, `export_existing`, `export_dest_set`, `export_last_get`, `export_last_set`,
  `reveal_path` (the plugin already implements it with `open -R`), and the new `plugin_host_tempo` is not a page group command.

What stays absent (list `files`, shorter): the menu items `export_again` (it repeats a folder that is not in `chosen` after a
restart) and `rename`; the commands `export_markers_set`, `export_compensate_set`, `export_midi`, `export_spikes`, `export_layer`,
`sampler_calibration_midi`, `project_rename`; the controls `.lyRender`, `.smCal`, `#insFileRow`, `#insStateRow`, and the dialog rows
Markers in MIDI, Compensate sampler offset, and the spike types Audio and Both. Those rows have no id on the row today; phase 2 gives
the rows a class `exHostOff` in `static/index.html` (no change of any value or behaviour with the mode off).

Page code changes (host mode only): `openExport` loses its `hostAbsent("files")` line; `exportAgain` keeps it. `exDragStart` in host
mode skips `askDefaultTempo` (the ask moves, R1) and `B.dragFiles` calls `plugin:drag|start_drag`, which now answers. The summary
text names the three counts of section 1 and the tempo source of section 2. `exOpts` passes `compensate: false` in host mode.

Proof that the standalone app is unchanged:

- With `hostMode()` false nothing in the new code runs: `HOST_ABSENT` is not read, the new rows have a class that no CSS
  rule hides without `body.host`, the commands are the old ones.
- `tests/e2e_app` run with no change of an expected value; the list tests `test_host_page.py` and `test_host_menu.py` change only for
  the entries that move out of `files` (their counts are read from the list, so the walk covers them). JS coverage stays at 70 percent per file
  (`tests/jscov.py`); `static/export.js` is already in `JS_FILES`.
- Export bytes: the identity snapshot before and after on the corpus, `compare` shows 0 difference; and a Rust test that runs
  `export_all_opts` with no `ExportTime` object and compares every file with the files made by the base commit.
- `crates/app/tests/dispatch_rows.txt` and the registry test do not change (no new command in `REGISTRY`; `plugin_host_tempo` and
  `plugin_pick_folder` are plugin shell commands like `plugin_host_play`).

## 6. The test host and the stages

The test host (`plugins/ara/src/bin/ara-host.rs`) gets a window with a drop view that registers for `public.file-url` and records
the dropped paths (path and byte size) to the report, as in section 3. Stage list; each stage has a fault run that fails for the named
reason.

| Stage | What it proves | Fault run |
|---|---|---|
| `delivery_time` (unit, `cargo test`) | `to_song`: regions, several regions per source, gaps, before song 0, no region; counts | `DS_ARA_DELIVERY_FAULT=source-time`: the file keeps source time, the stage sees the offset |
| `delivery_tempo` (unit) | `tempo_from_ara` for two tempos, equal-section merge, bars; read back within 1 sample at 44.1 and 48 kHz | `DS_ARA_DELIVERY_FAULT=one-tempo`: the file has the first tempo only, a note is off by more than 1 sample |
| `delivery_files` (CLAP) | one MIDI file and one WAV per track from `export_all`; note ticks read back and spike sample positions equal the song positions (0 samples for the spike, 1 for the note) | `DS_ARA_DELIVERY_FAULT=no-bext`: no `bext`, the time reference check fails |
| `delivery_files_vst3` | the same through the plugin inside the VST3 | the same fault |
| `delivery_moved` | a region cut and moved (changes `start_play` and `start_mod`): the positions follow; a source with two regions gives each hit twice | `DS_ARA_DELIVERY_FAULT=stale-regions`: the live regions, not the round's, are used; the stage sees the old song time |
| `delivery_rate` | a second run with another session and source rate (44.1 and 96 kHz): positions equal, ticks per quarter as the rule says, the reported bound equals the measured one | `DS_ARA_DELIVERY_FAULT=fixed-rate`: the rate is taken as 48 kHz |
| `delivery_notempo` | no musical context content: source `transport`, then `none`; the page text names the source | `DS_ARA_DELIVERY_FAULT=invent-map`: the plugin invents a map, the stage sees steps with no host source |
| `delivery_drag` | the real session starts; the drag pasteboard holds the file URLs of the one MIDI and the WAV per track; the drop view records the same paths | `DS_ARA_DRAG_FAULT=no-write`: the URLs are written after the session starts, the pasteboard is empty |
| `delivery_drag_guard` | a path outside `Exports/` and outside a chosen folder: 403, no session | `DS_ARA_DRAG_FAULT=any-path` |
| `delivery_drag_release` | no mouse button down: `ok: false`, no session | `DS_ARA_DRAG_FAULT=no-button-check` |
| `delivery_drag_thread` | `OFF_MAIN` stays 0; a call from a request thread is the hop | `DS_ARA_EDITOR_FAULT=inline` (exists) |
| `delivery_folder` | `plugin_pick_folder` with `DS_ARA_PICK_DIR`: the export writes into the chosen folder; a second folder is refused; a subfolder is refused; a symlink swapped in is refused | `any-dest` and `prefix-dest` (section 4) |
| `delivery_folder_cancel` | cancel: the answer is null, nothing is chosen, the export into any `dest` is refused | none needed (the stage proves the refusal) |
| `delivery_page` | the page in host mode: the controls of the list are visible, the others absent; the standalone page test list is unchanged | `DS_ARA_EDITOR_FAULT=missing-function` style: the list entry is removed, the test names it |

The real sheet and the real drop are not in a stage. The checks that use the panel and the mouse are on the hand check sheet.

## 7. What the role models do

What we know, and where it comes from:

- The ticket names Melodyne as the model for the MIDI file export and the drag. In the repository, `m1b4-analyse-without-request.md` says it
  declares several analyzable content types, and `m1b10-track-names.md` follows the ticket for the display of DAW track names; that part did
  not run the product.
- One web search in this phase (vendor help, DAW forums, trade magazine articles; only the search result texts were read, not the pages). The
  results say: Melodyne exports audio, and a tempo map as a MIDI file; "Note to MIDI" export needs the higher editions; with ARA the user
  drags a processed audio clip onto an instrument track of the DAW and notes appear; a Melodyne tempo map goes to the DAW either through ARA
  (drag of the clip onto the tempo track) or as a MIDI file that the user imports. Reports about hosts that ignore the tempo of an imported
  MIDI file exist. All of this is anecdotal; we did not confirm it with the product or with the ARA documentation.
- What we could not look up: the real behaviour of the product's drag (which time base, whether it writes the time reference of a WAV, which
  file names), a vendor description of how it reads the ARA tempo entries, and the behaviour of any other editor with ARA. Nothing in this
  note depends on them. The design follows the ARA header and the code of the app.

Later parts, not in M1c1: spikes as the audio output of the plugin with a bounce, notes that the host reads by ARA (the content reader
for notes of the audio modification exists as a stub), the audio-spike format in song time, the tempo map of the plugin as a write into the
host, Export Again with a kept folder, time-stretched regions.

## 8. Work plan for phase 2

Small commits, risky parts first. Each commit has its own tests; the heavy gates run once at the end.

1. Drag spike (the risk R1, R2): `plugins/ara/Cargo.toml` takes `drag = "=2.1.1"` (no default features) and `raw-window-handle`;
   `drag.rs` with the window handle wrapper and the main-thread hop; `plugin:drag|start_drag` answers for a path in `Exports/`; the test host gets a window
   with a drop view and the stage `delivery_drag` (pasteboard). Decide here: crate or own module. NOTICE and the licence table of the README.
2. Release check, path guard and the faults of `delivery_drag_*`.
3. The sheet for the folder: `plugin_pick_folder`, the `chosen` list, `refuse_write`, the shell script `dialog.open`, the faults, `delivery_folder*`.
4. Musical context: keep the host ref, the snapshot, the content reader on the main thread, the report row `musical_context`, `tempo_from_ara` with
   its unit tests, the transport tempo in `transport.rs` (audio thread, no allocation: the existing allocation test names it).
5. `crates/`: `ExportTime` trait, `set_export_time`, `build_jobs`, `to_song`, `write_spikes_song`; the identity proof first (a test with no object, then the
   snapshot), then the song tests (`delivery_time`).
6. The plugin implements the trait (regions of the round, stems of the display names, the tempo map) and sets it at the service creation; the stages
   `delivery_files`, `delivery_files_vst3`, `delivery_moved`, `delivery_rate`, `delivery_notempo` with their faults.
7. The page: `HOST_ABSENT` split, `exHostOff` rows, `exDragStart` and `exOpts` in host mode, the summary text, the e2e tests, the coverage.
8. `plugins/ara/README.md`, `docs/agent-api.md` only if a command changes (none planned), `switches.json` rows for the new switches, the hand check sheet
   (private, outside the repository).
9. The gate: `plugins/ara/check.sh all` and the complete gate of the app, in the background with one log per step; `tools/check_private.py --diff origin/main`.

## Decisions of the coordinator (answers to the open questions)

1. No stage posts a global event (no CGEvent, no accessibility permission, no take-over of the mouse). The stages use in-process events
   posted to the window of the test host, the pasteboard of the real drag session, and the stub dragging info for the drop code. This does
   not prove a real drop by a DAW or the real mouse path. The hand check sheet has one line for it: drag the MIDI and the spikes onto a DAW
   track with the real mouse and note where each lands.
2. The synthetic spike WAV and the MIDI are in M1c1. The audio-spike types ("Audio" and "Both") stay absent in host mode in this part; the
   dialog says so with the short text of the group, and they are a later part.
3. The folder choice is an open panel in directory mode, shown as a sheet on the window of the plugin view.
4. The default-tempo ask moves out of the press-and-drag moment in host mode.
5. The export dialog shows one short line when a region with a time stretch is among the exported tracks (it names how many tracks). The
   files are still written.
6. File names come from the DAW track name. A rename leaves the old files. Two tracks with one name use the numbered display name of #558. The name
   is made safe for a file name with the rule the app already has.
7. A hit before song 0 or in no region is dropped and counted; the dialog shows the counts.
8. One fixed tempo from the process call is enough for the first hand check. The page names the tempo source and the report has the row
   `musical_context`.

Additions for phase 2: rebase onto main after the key shield of #561 (`plugins/ara/src/keys.rs`: a shield view above the web view, a local key
monitor and a local mouse monitor that only change the first responder). A stage checks that a drag that starts in the page still works with
that mouse monitor, and that the folder sheet gets its keys (a sheet is another window, so the monitors pass its events). `crates/` changes need
the whole pytest suite and the Rust workspace tests. A byte golden for the standalone export is a must, and one for the multi-track MIDI mode
too. Names in tests are synthetic.

## What phase 2 built, and where it differs from this note

- `crates/app/src/export_song.rs` (trait `ExportTime`, `to_song`, `Service::set_export_time`) and `write_spikes_song` in
  `crates/core/src/spike.rs`. The byte golden for the standalone export (per-track files and the multi-track file, on the default map
  and on a map with a step) is `crates/app/tests/export_song.rs`; its hashes were made on the base commit.
- The musical context in use is the first one that the host created (the note said: the one of the region sequence with the lowest
  `orderIndex`). The plugin reads the content on the host's thread at the creation and update of a context and at the end of an edit cycle.
- The stretch list: a region counts as stretched when its transformation flags say so or its length in the song differs from its length
  in the source. The answer of `plugin_host_tempo` carries the track names; the dialog line counts the exported ones.
- Faults that exist: `DS_ARA_DELIVERY_FAULT` = `source-time`, `one-tempo`, `invent-map`; `DS_ARA_EDITOR_FAULT` = `any-dest`, `prefix-dest`,
  `any-path`, `no-button-check`. The faults `no-bext` and `stale-regions` of the table in section 6 were not built.
- The test host has no content access controller and sends no tempo, so a stage can reach tempo source `none` only. The ARA reading
  (`musical.rs`) is covered by the unit tests of the conversion (`tempo_ara.rs`) and by nothing that runs the host side; the first run
  in a real host shows it in the row `musical_context`.
- The drag evidence of the stages is the drag pasteboard of the real session (rows `drag_started`); there is no drop view and no stub
  dragging info in the test host. A real drop, and the check that the mouse monitor of #561 leaves a drag from the page alone and that the
  folder sheet gets its keys, are for the hand check sheet.

## Round 2 (review of the first build)

What changed after the review:

- A clip that the user moves in the DAW exports at its new song position. A move in playback time alone starts no analysis round, so the
  song position is read live; the slice of the source and its length come from the round (the hits belong to it). A round region with no live
  region of the same slice is gone and is left out; a live region with no round entry uses its live values (`deliver::merge_regions`).
  Stage `delivery` (part "moved") with the fault `stale-regions`.
- A region that the host stretches in time maps a hit by `start_play + (t - start_mod) * dur_play / dur_mod`; the song end uses the play length.
- The tempo of the process call is kept per sending instance and cleared when a document is created.
- The context with the lowest `orderIndex` is read. The event count of the reader is capped at 100000 (the row says when it was hit). A call
  from another thread does nothing.
- Bar numbers: with bar 1 before song 0 the meter changes are counted from song 0; when song 0 lies inside a bar they are dropped with a reason.
  A section that lies completely before song 0 does not refuse the map. A single sync point carries no tempo (a tempo needs two points), so
  it still falls back; the note stays as it was.
- File stems are numbered with the app's `file_name_key` and cut at 80 bytes on a character boundary.
- A drag takes only files that this editor's exports or drag takes wrote. A request that timed out leaves an abandoned flag, so the queued job
  starts no drag later. The folder sheet is refused at once when the window is not visible and ends after 10 minutes.
- A track with no source of the host keeps the host name; every hit is left out and counted. A hit on the sample after the last region is kept
  (the file gets one frame more). A region that ends before song 0 makes no spike file; the answer says so.
- The dialog shows a clear warning line when the host's tempo changed (the bar positions of the MIDI file can be wrong; the times are right).

Not proved by a stage (a hand check or a later ticket):

- The real folder sheet, and the keys that the sheet gets.
- The real mouse drag, with the key shield of #561 on; a drop view in the test host.
- The ARA reading of the tempo (`musical.rs`): the test host has no content access controller, so only the conversion is tested. Whether the
  reader is destroyed on every path was read in the code, not counted by the test host.
- The faults `no-bext` and `stale-regions` of the table in section 6: `stale-regions` exists now; `no-bext` does not.
- The `any-path` fault run ends with a 504 because the queued job never answered in time; the stage only needs "not 403".

Hand check steps (for the owner, in a DAW):

1. Open a session with three clips of different lengths that start at different song positions. Open the plugin on the clips and let the analysis finish.
2. Open Export. Check the note under the options: it names where the tempo of the MIDI file comes from, and the line that files start at song position 0.
3. Press Choose, pick an empty folder in the sheet. Check that the sheet takes the keyboard (arrow keys, Return, Escape) and that the DAW does not react.
4. Press Export. Check the folder: one MIDI file and one WAV per track, named after the DAW tracks; Show in Finder opens the folder.
5. Drag the MIDI tile onto an instrument track, at the session start. Check that each note sits on the hit it belongs to (zoom to samples on a loud hit).
6. Drag the spike tile onto an audio track, at the session start. Check the same, and that the spike file does not start later than song position 0.
7. Move a clip in the DAW, do not analyse again, export again: the notes and spikes follow the clip.
8. Bounce the drag-in track, measure the offset against the clip in samples, and write it down.

## Round 3

- The export uses the LIVE regions of a source alone: the hits are in source time, so the live slice, the live song position and the live
  length give the whole mapping. A region that the user trimmed or moved in the DAW needs no new analysis round; a hit outside the live slice is
  left out and counted. The round's regions are used only under the fault `stale-regions`. A source with no live region but regions at the last round (every clip deleted in the DAW) is gone: every hit is left out and counted in `outside_song`, and no file goes out in source time. A source that never had a region keeps the identity time base. There is no reason to match by region id: a hit
  that lies outside the live slice is not in the region, and a live region that lies where no hit was found simply holds none. A trim in the DAW starts a new analysis
  round in this plugin (the rule of M1b4), so after the round the live and the round regions agree and the hits are those of the live slice. The
  fault `stale-regions` therefore shows only for a move in playback time, which starts no round (stage part "moved"). The live-only rule for a
  trim is covered by the unit test `a_moved_or_trimmed_region_exports_through_its_live_values`; the stage part "trimmed" proves that every
  note of the files lies in the live slice at its song position and that the trim left fewer hits. There is no fault run for the trim part.
- The test host has a content access controller (`--musical tempo`: 120 BPM for 10 s, then 100 BPM, and signatures 4/4 and 3/4 from bar 5;
  `--musical no-tempo`: a controller that says it has no tempo or bar content). Counters in the summary line of the host say how many
  readers the plugin made and gave back and on which thread. Stage `delivery` (parts "ara"): the row `musical_context` has the entries, the page is told
  source `ara`, readers created equal readers destroyed, and the exported MIDI files carry the tempo events 500000 and 600000 and the signatures.
  The second part (no tempo content) shows the fallback with no reader made. Fault `keep-reader`.
- The `any-path` fault run of the drag ends in a 504. The queued job did not answer in time. We do not know why; it was not investigated.
  The stage only needs the answer not to be the refusal 403.

## Round 4

- A source whose clips are all deleted in the DAW is gone: every hit is left out and counted (`outside_song`), never exported at the old positions
  and never in source time. The round's regions are used only under the fault `stale-regions`. Unit tests in `deliver.rs`; the crate
  integration test covers the hide-all result. No stage part runs this case in the test host (no host flag deletes a region yet).
- The stretch line of the dialog says that the hits follow the stretch as one linear factor and that warp markers inside a clip are not followed.
- The process-call tempo keeps `varies` when instances alternate with different tempos; a new document clears it.
- A list that the event cap cut is no tempo map: the answer is no tempo with the reason `capped`, so the fallback runs.
- A drag may take the files that `export_existing` names too (relative to its `dest`).

## M1c2 (#579): the export dialog and the drag with the default folder

The owner opened the export dialog in a real DAW for the first time (2026-11-22). Two things were wrong: the dialog said "The folder cannot
be used" at once, and the two drag tiles never left "preparing". The gate of M1c1 did not see either.

### Cause

- The page does not ask for the default folder with no `dest`. At open it reads `default_dest` from `export_existing` and keeps it as the
  destination (`EX.dest = EX.def`); from then on `exInfo` calls `export_existing` with `dest` = that folder. In the app this is the normal
  way; the folder is an explicit, absolute path.
- The rule of section 4 allowed an explicit `dest` only when it equalled a folder that the user chose in the sheet of THIS editor session
  (`chosen`, empty at the start). The default folder of the document (`Exports/` below its project folder) was not in it, so
  `export_existing` answered 403, the page set `destOk = false` and wrote "The folder cannot be used", and Export stayed off. A
  second fault stood behind the first: the service checks that a `dest` is an existing folder (`check_dest`), and `Exports/` does not exist
  until the first export, so even an allowed default folder gave a 404.
- The tiles had two causes in the page. The render timer of the dialog ran after a refused `dest`, and `exDragSchedule` returned early at
  `!EX.destOk` with no state, so a tile that did not prepare had no way to say so (`exRender` shows "preparing" when it has neither files nor
  an error).
- Why the stages did not see it: stage `delivery` calls the commands of the service directly through `window.__TAURI__.core.invoke`, and it
  chooses the folder first (`DS_ARA_PICK_DIR`), so the explicit `dest` it uses is always the one in `chosen`. It never opened the dialog, so
  the page code that names the default folder (`openExport`, `exInfo`, `exDragSchedule`) never ran in the real page. The page tests of
  `tests/e2e_app` run on the test bridge, which has no destination rule at all. Nothing ran the page and the rule together.

### The rule now

A `dest` (commands `export_all`, `export_existing`, `export_dest_set`; `export_drag_prepare` still takes none) is allowed when it is

1. the default export folder of the document, `Exports/` of its project (`deliver::is_default_dest`): the same place by `resolve_lenient`,
   which makes the existing part of both paths canonical and keeps the missing names behind it, so it holds also while the folder does
   not exist; a missing part with a `..` is refused. The plugin makes this one folder (`create_dir_all`, and nothing else) before the service
   checks it. Or
2. equal (exact canonical path, no prefix, no subfolder) to a folder that the user chose in the sheet: in this editor session, or in the list
   of the document (below).

Nothing else changes: a subfolder of the default folder, another folder below the cache folder and a `path` key are still refused
(unit test `the_default_folder_is_a_dest_also_before_it_exists_and_the_rule_stays_exact`; the fault runs `any-dest`, `prefix-dest` and
`any-path` of stage `delivery` still fail for their reasons).

The list of folders: `export-folders.json` in the folder of the document (next to `project/`, not inside it). Only the plugin writes it, from
`plugin_pick_folder` after the sheet; no command of the page reaches it. It is read at every check, and every entry is checked again: it must be a string, an absolute path that is equal to its own
`canonicalize` (a link that moved to another folder is no longer equal) and a folder now (a path that is a file now, or gone, drops out);
the file must be at most 64 KB and the list keeps the newest 32. A wrong type, a relative path or a broken file gives no folder, never an error
(unit tests `hostile_content_of_the_folder_list_gives_no_folder` and `a_remembered_folder_is_kept_in_the_file_and_loaded_again`). A drag may
also take files from a remembered folder, as from a chosen one.

The page (host mode only): a remembered folder that is not allowed or not there any more gives the default folder and one short text, "The
folder chosen before is not available: Exports/ is used." (no error: Export and the tiles work), and the project forgets the folder, so the
text comes once. A tile that cannot prepare, because the destination is refused or the render failed, shows `error: <reason>`; it never stays
at "preparing". In the app the page is as before.

### The stage that closes the hole

Stage `export-dialog` (mode `exportdialog` of the test host and of `driver.js`) drives the REAL dialog in the REAL page of the plugin, from the
menu: File > Export... by DOM clicks, in two editors of one document. It reports what the dialog shows at each step: no folder text, the default
folder, both tiles "ready" with 4 files, a press and a move on the Drag MIDI tile starts the real drag session (the pasteboard holds the 4 files
in `Exports/`), Export writes 4 MIDI and 4 WAV files, a second open after a close does the same, Choose... (answered by `DS_ARA_PICK_DIR`) shows
the chosen folder and Export writes there, and a new editor of the document opens with that folder. Fault runs: `old-dest` (the rule of M1c1) gives
the folder error and the tiles do not reach "ready"; `no-remember` (the choice is not kept) opens the new editor with the default folder.

### Not proved by a stage

- A DAW session that is closed and opened again (the document id comes back from the archive, so the folder of the document and its list are the
  same): the stage runs two editors of one document, not a restored archive. The unit tests cover the file.
- A remembered folder that is deleted between two runs: the test host cannot delete it. Covered by the unit tests of the list and by the page
  test `test_a_remembered_folder_that_is_gone_falls_back_to_the_default_with_one_short_text_in_host_mode`.
- The real mouse drag into a DAW track, and the real folder sheet: hand check.

### Hand check steps (for the owner, in a DAW)

1. Open the plugin on a drum track and let the analysis finish. Open File > Export... from the plugin menu.
2. The dialog shows the folder `Exports/` of the document and no red text. "Drag MIDI" and "Drag Spikes" change from "preparing..." to a file count
   within a few seconds.
3. Press Export. The result line names the files; Show in Finder opens `Exports/` and holds one MIDI and one WAV file per track.
4. Close the dialog and open it again: the same, with no text.
5. Drag the "Drag MIDI" tile onto an instrument track and the "Drag Spikes" tile onto an audio track, at the session start.
6. Press Choose..., pick a folder, press Export: the files are in that folder. Close the plug-in window and open it again: the dialog opens with that
   folder. Delete the folder in the Finder, close and open the window again: the dialog opens with `Exports/` and one short line, and Export works.
