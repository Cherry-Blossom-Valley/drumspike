# M1c3: one drag from the plugin to the DAW tracks

Ticket #576, umbrella #503 (the DAW plugin program), part of M1 (#510), third part of M1c (delivery).
This note is the design for phase 2. It follows `m1c1-delivery.md` (what #560 and #579 built) and `m1b8-song-time.md`
(rows, clips, `HT`). A fact that we did not measure says so. The code places are those of the base commit
(`0f8966b`, #555 merged).

Goal: from "the hits are right" to "MIDI and spikes are on DAW tracks" in ONE drag, with no dialog and no folder question.
A plugin cannot make DAW tracks, so the drag is the delivery. The files are made before the user asks.

## 1. Steps today and after

Today (`static/export.js`, `plugins/ara/src/deliver.rs`, the audit of 2026-11-22):

| Goal | Steps today |
|---|---|
| MIDI of all tracks | open File > Export (1), wait for the tile to leave "preparing" (2), drag the MIDI tile (3) |
| MIDI and spikes of all tracks | the same, then drag the Spikes tile (4) |
| One DAW track | open the dialog, choose "Selected track", wait, drag the MIDI tile, drag the Spikes tile (5) |
| Files for one DAW track with two audio files | not possible: the export makes one file per source, not per DAW track (section 3) |

After:

| Goal | Steps after |
|---|---|
| One DAW track, one kind | drag the handle in the header of the row (1) |
| All tracks, MIDI or spikes | drag "Drag MIDI" or "Drag spikes" in the toolbar (1) |
| All tracks, everything | drag "Drag all" in the toolbar (1) |
| One MIDI instrument with all drums | drag "Drag merged MIDI" in the toolbar (1) |

There is no wait: the files are made in the background after each round and each edit. A handle that is not ready says why
and does not start a drag. File > Export... stays as it is for the keyboard and for a chosen folder.

## 2. What places a file at the song position

Nothing in the plugin places a file. The DAW places a dropped file where the user drops it. What the plugin does is make
the file correct for ONE drop position: the start of the session (song position 0).

- MIDI: a standard MIDI file has no position field. Tick 0 is the drop position. Every note sits at its song time from tick 0
  (`crates/app/src/export_song.rs`, `to_song`). One file per DAW track holds the hits of ALL clips of the row, so one drop at
  the session start is right for every clip. Today a row with two clips of two audio files needs two drops.
- Spike WAV: the file starts at song position 0, is as long as the end of the last clip, and has a `bext` chunk with time
  reference 0 (`write_spikes_song`, `crates/core/src/spike.rs`). A DAW that places a Broadcast Wave file by its time reference
  puts it at 0 with or without the drop position. We did not measure which DAW reads the chunk on a drop (hand check of M1c1).
- What the role models do: the audit (`ux-audit-host-mode.md`, section 2) and `m1c1-delivery.md` section 7 say what we know.
  Melodyne and Auto-Align change the audio in place through ARA and need no delivery. A drum replacer with a "drag MIDI" handle
  (a public product feature; we did not run it) makes the files before the drag. This note follows that.

So the user must drop at the session start. The page says it in three places: the tooltip of every handle, one hint line
beside the toolbar handles ("Drop at the start of the session"), and the status line after a drag (as today, plus the text).
This is Gate line 5, first part.

## 3. One file per DAW track: the producer

Fact in the code: the export is per SOURCE. `HostTime::track(name)` (`plugins/ara/src/deliver.rs`) takes a project track
(one audio file) and gives the regions of that source; `build_jobs` (`crates/app/src/export.rs`) makes one job per track.
The file stems come from `display_names` (`names.rs`), which numbers per source. A DAW track with two clips from two audio
files (the hand check of M1b8) therefore gives two files, `X_spike.mid` and `X (2)_spike.mid`. #555 made the page rows per DAW
track (`rows.rs`, `Rows`), but the export still works per source. The brief of this ticket asks for ONE file per DAW track per
kind, so the files need a row-level producer.

Decision: a new module `plugins/ara/src/prepare.rs` (the producer) that works per ROW and calls public functions only:

- The row: `rows::Row` (`id`, `name`, `srcs`, `clips`), from `Backend.rows` (`rows_value` in `lib.rs`). A clip is
  `{src, a, b, s, k}`, the song time of a hit is `s + (t - a) * k` (the formula of `to_song`).
- The hits of a source: `Service::events_get(sid, name)`, then `export::uncut` (drop the loudness-cut and bleed-rule events, as
  the export does). A hit of source `src` is mapped through the clips of THIS row that have `src` and hold it, each clip once:
  the clips of the row are converted to `SongRegion` and `export_song::to_song` is called per source. A source that is in two
  rows (a member of both) delivers only the clips of the row it is made for. The dialog export gives it all its clips.
- The note of the row: the note of its home source (the first of `srcs`). The row owns the note (m1b8 section 4: a change
  reaches every source of the row), so all sources of a row have one note.
- MIDI: `drumspike_core::midi::export_midi_map` with the tempo map of section 9, `ppq` from `export_ppq`, the rate of the home
  source; the file is written to a temporary name and renamed (as the writers do).
- Synthetic spike: `export::write_spike_file_song(source, out, times, end_s)` with the times of all sources of the row, the first
  source for rate and sample format, `end_s` = end of the last clip of the row (`export_song::song_end`).
- Audio spike: `export::write_audio_spike_file_song(source, out, pairs, end_s)` takes ONE source file. A row with one source
  (the usual case) is that call with `export_song::song_pairs`. A row with several sources: one temporary file per source,
  then the producer adds them sample by sample into one file. The cached sources of the plugin are always float 32 bit
  (`wav.rs`), and the snippets of two sources can overlap only where two clips overlap in the song, so a plain sum is right.
  Sources with different rates in one row give the error `clips have different sample rates` (ruling R8, unit test).
- What the producer does not do: it does not call `export_drag_prepare` or `export_all` (they are per source, and
  `export_drag_prepare` clears the folder of the dialog tiles). It does not change `crates/app`.

Why not extend `ExportTime`/`build_jobs` in `crates/app` (a row as one job): the change is about 60 lines in the central export
path (events of several sources, pairs from several files), it touches code under the byte golden of the standalone export,
and the ticket scope is `plugins/ara/`, `static/`, docs, tests. The producer duplicates only the mapping and the file calls,
and every rule it needs is a public function with its own tests. Followers (ruling R4): a prepared set is never staler than File > Export...
would be. Before it reads the events of a source, the producer calls the same refresh as the export, `follow::fresh_for_export`
(`crates/app/src/follow.rs`); the only change in `crates/app` is its visibility, `pub(crate)` to `pub`. The refresh needs the
shared project (`registry::Shared`), which the producer gets through `Service` (a thin public wrapper is NOT added: if the
visibility change is not enough to call it, that point stops and is reported). The follower leader's edit is part of `fp`
because the follower's stored hits are in `fp` after the refresh.

Limits (ruling R1): the dialog and `File > Export...` keep the per-source export in this ticket. For a row with several sources the
dialog writes one file per source and the handles write one file per DAW track. A follow-up ticket moves the dialog to rows.
The dialog and `File > Export...` keep the per-source export. For a host where every DAW track has one source (the usual case)
the dialog and the handles write the same file names and the same bytes. For a row with several sources they differ, and the
dialog is the one that is "wrong" by the rule of this ticket. This is open question 1 (move the dialog to rows later).

## 4. File names

`<stem>_spike.mid`, `<stem>_spike.wav`, `<stem>_audio_spike.wav`: the suffixes of the dialog, so a single-source row gives the
same name from both ways and the later one overwrites the earlier (the contract of the dialog today).

The stem is the name of the DAW track: `Row.name`, which `rows::build_with` already makes unique between rows (a number
`(2)` goes to the later row, compared without case). The producer runs `deliver::file_stems` over `{row id -> row name}`:
`safe_stem` (control characters and `/ \ : * ? " < > |` become `-`, no leading dot, at most 80 bytes on a character boundary)
and the numbering by `export::file_name_key` for two names that end on one stem on the file system. Empty gives `track`.
This is the numbering of #560 applied to rows; the display-name numbering of `names.rs` is per source and is not used.

The merged MIDI file (ruling R7): `Drums_merged.mid`, a fixed stem with no project name lookup. When a row stem equals `Drums_merged` the
file is numbered by the same numbering as the other stems (`Drums_merged (2).mid`).

## 5. Where the files live; prepared set; staleness

Rule of the coordinator: prepared files live only in the default `Exports/` folder of the plugin project (the folder rule of
#579 stays; no new write path). The producer writes to `Exports/.prepared/` (hidden, below the default folder, made by the
producer with `create_dir_all`). The dialog tiles use `Exports/.drag/` and clear it on close; the two folders do not meet.

A drag does not move the prepared file (the dialog does: `export_drag_take` renames). The prepared set must stay valid for the
next drag. Ruling R5: the WORKER makes the final-name file in `Exports/` (a hard link, `fs::hard_link`; a copy when the link fails; both folders
are on one volume) as part of "ready". A copy that still runs means `preparing`. `plugin_deliver_take` makes no file: it only
checks `fp` and the files and answers their paths, so nothing heavy runs at the press and never on the main thread. The file in `Exports/` is what the DAW gets. A new preparation
writes a NEW file in `.prepared/` (temporary name, rename), and the worker replaces the link in `Exports/` when the new set is ready
(the kind is `preparing` meanwhile, so no drag takes the old link). This keeps the contract of the dialog: "Exports/ gets a file
only by a drag or an export", the file names are the export names, and a DAW that references the file in place does not see it
change under it. Asking the user to delete old clips before a new drop is the hint of the audit (item 8).

Generation. Each prepared set (row, kind) carries `fp`, a 64-bit hash of everything the file is made from:

- the clips of the row (`src, a, b, s, k`), the song end, the stem;
- for each source of the row: its uncut events (`t`, `v`), the note, the sample rate, the source file path;
- for MIDI: the tempo answer (`Answer`: source and map) and the ticks per quarter;
- for spikes: nothing more.

The "generation of the events" of the coordinator's rule is this hash plus a counter `n` that goes up when the hash of a
(row, kind) changes. A set is READY when `set.fp == current fp`. The plugin computes the current `fp` for every row at each
wake of the worker and at each `plugin_deliver_take`. A hash and not only a counter, because a clip moved in the DAW or a tempo
change changes the file with no command of the page (no `changed_after`), and an edit of the velocity of one hit must
not make every row stale (the cost of section 6).

Handle states, per (row, kind):

| State | Meaning | Draggable |
|---|---|---|
| `ready` (count) | a set with the current `fp` exists; count = files (1 per row and kind; the toolbar adds up) | yes |
| `preparing` | the `fp` differs from the set, or no set; a job is queued or runs | no |
| `error: reason` | the last job for the current `fp` failed, or ran for more than 30 s | no |
| `off` | a wav kind before its first use ("press to prepare"); the press starts the preparation | no, a press starts it |
| `none` | the row has no hit | no, text "no hits" |

An older set is never dragged: the state is computed from `fp` at the moment of the take, so a take that arrives after an edit
and before the debounce ends answers 409 `preparing`, and the page shows the state. "Never preparing for ever": a (row, kind)
in `preparing` for 30 s with no job running on it (a dead worker) or for more than 30 s of one job gets `error: no answer from
the preparation`; the existing `EX.waitMs` of the dialog (30 s, `exWait`) is the model. A later success flips it to `ready`
by itself.

## 6. Trigger, debounce, cost

Trigger. One worker thread per document, `drumspike-ara-prepare`, wakes on a condition variable. Wake sources, all existing
places:

- an edit: `changed_after` in `editor.rs` (`writes_project()` commands; it already calls `(sh.backend.changed)()`);
- the end of a round: the place in `lib.rs` where `doc.region_gen.fetch_add` runs after a round (and in `restore_project`);
- a change of rows (a clip moved, a rename): `rows_changed` in `lib.rs`;
- a change of the tempo snapshot (`musical.rs`) and of the document at restore.

Debounce: the worker waits for 800 ms with no wake, at most 5 s from the first wake, then compares `fp` and prepares what
differs. Order: MIDI of all rows, then spikes, then audio spikes (the wav kinds only after their first use, quiet time 3 s). One job at a time (the files are big). Never in an audio
callback: the audio thread has no part in it (`transport.rs` allocates nothing, and the stage `threads` keeps proving it); the
worker takes the service only through `events_get`. The preparation does not start while an analysis round of the document runs
(the hits are about to change): it waits for the round end.

Cost, with numbers from the code:

- MIDI: a few KB per row, milliseconds.
- Ruling R3: only MIDI is prepared eagerly. BOTH wav kinds (Spike and Audio spike) are `off` ("press to prepare") until their
  first use in the document session. A press on the handle (no drag needed) starts the kind for all rows; then `preparing`, then
  `ready`. From then on the kind is kept current for the changed rows only, with a quiet time of 3 s (MIDI: 800 ms). The toolbar
  "Drag spikes" and "Drag all" follow the same rule. The existing writers and their formats and lengths do not change.
- Synthetic spike (the numbers below are why R3 exists): `write_spikes_song` writes the FULL length of the song end in the sample format of the source. The plugin's cached
  sources are float 32 bit, so a spike file is `song_end * rate * 4 * channels` bytes: 10 minutes at 48 kHz mono is 115 MB per
  row, 8 rows about 920 MB. That is as expensive as the audio spike. The brief defers only the audio spike. Per-row `fp` keeps the
  cost in line: an edit in one row writes one file. Ruling R3 defers it.
- Audio spike: reads the source and writes the same length, plus the sum for a multi-source row. The flags "spike used" and "audio
  used" are in memory per document and start `off` after a reopen.
- Disk: one prepared file per (row, kind) in `.prepared/`, plus links in `Exports/`. The producer removes the files of a row
  that is gone, and the whole folder when the editor of the last instance of the document closes. A prepared file is removed only
  inside `.prepared/`. Rule of size (ruling R11): at each wake the worker removes every set whose row or kind no longer exists
  and every temporary file; the folder is bounded at 2 GiB: a job whose file would take the folder over that sum fails with `error:
  prepared files exceed 2 GB`. `.prepared/` is a fixed child of the default folder written only by the plugin; no user path reaches
  it, and the exact-path rule for user destinations (#579) does not change. A file name is made by `file_stems` from the row name, so
  a hostile row name (`../x`, `/abs`, empty, 300 characters) stays one safe name inside the folder; a fault run proves the stage can see a name that leaves it.

## 7. The handles in the page (host mode only)

Every page change is behind `hostMode()`. The standalone page gets no new element, no new focusable thing, no changed test value.

Header of a row. `buildTrackDOM` (`static/tracks.js`) builds `.thead` with the strip, `.tname`, `.tnote` and `.tctl`. In host
mode `.tctl` and `.meter` are hidden (group `audio` of `HOST_ABSENT`), so the header has room. A new strip `.tdel` is added
by a new function of a new file `static/deliver.js` (in `JS_FILES` for the coverage rule) after the row is built, only on
a DOM row that is the home of a row (`HT.homeOf`; the merged DOM rows have no header). It holds THREE handles, `MIDI`, `Spike`,
`Audio spike`, each a small `div` with `data-kind` and `data-state`, the text of the state (`ready 1`, `preparing…`,
`error: <reason>`, `press to prepare`) and a tooltip with the full reason and the hint "Drop at the start of the session".
The strip sits at the bottom left of the header, left of the instrument drawing (`.tart`, bottom right).

Collapse. When the header is too small for three handles, the strip becomes ONE handle ("Drag" with the state of the worst of
the three) and a small `…` button that opens a menu with the three kinds (each with its state). "Too small": the row height at
its smallest setting (`#lanesScroll[data-height="small"]`, where `.tart` is hidden too), a row dragged short (`.thead.tart-hidden`),
or a window under 520 px (the rule of `state.js`, where the Inspector steps back). The menu is a plain page menu (`openTrackMenu`
style), keyboard operable. The one handle drags the kind that is ready and first in the order MIDI, Spike (the same choice as the
menu's top item).

Toolbar. Four handles in `#topbar` next to `#exportAllBtn`, created by `static/deliver.js` only in host mode: "Drag MIDI",
"Drag spikes", "Drag all", "Drag merged MIDI" (section 10). Each shows its state: `ready N` (N files), `preparing…`, `error:
reason`. A row that is not ready makes the toolbar handle `preparing` (it drags nothing while a part of it is stale: all or
nothing, so a user never drops an incomplete set unknowingly). Under 520 px the four collapse to one handle with the same menu.
"Drag all" holds the files of the kinds MIDI and Spike of every row with hits, and the audio spike files when that kind is on.

Keyboard users keep File > Export... unchanged. A handle is a drag source (a mouse gesture); the menu of the collapsed form gives
a keyboard route to the same state text; the drag itself needs the pointer, as the dialog tiles do.

Drag gesture: the 4 px press-and-move of `setupExport` (`export.js`), calling `deliver.js` instead of `exDragStart`: it asks
`plugin_deliver_take` for the files and calls `B.dragFiles(files)` (`plugin:drag|start_drag`) with them. There is no
default-tempo question between press and drag (host mode already skips `askDefaultTempo` there; the 120 BPM text moves to the
tooltip of the MIDI handles).

New shell commands (plugin shell commands like `plugin_host_regions`, listed in `editor.rs` `SHELL_COMMANDS` and in the test host):

- `plugin_deliver_state` -> `{gen, rows: [{id, name, srcs, midi: S, spike: S, audio: S}], merged: S, hint}` with
  `S = {state, n, why}`. The same payload goes to the page as the event `plugin-deliver` (through the push gate of
  `plugin-regions`: at most one in 50 ms, the newest always arrives).
- `plugin_deliver_take {items: [{row, kind} | {merged: true}]}` -> `{files: [paths in Exports/]}`, or 409 with the state of the
  first item that is not ready. It adds the paths to `Shared.written`, so the guard of `plugin:drag|start_drag` (`drag_path_ok`:
  a regular file directly in `Exports/`, written by this editor) accepts them. A path outside `Exports/` is still 403.
- `plugin_deliver_want {kind: "audio"}` -> starts the first-use preparation of the audio spikes.

The limit of the drag command is 64 files (`start_drag_command`). "Drag all" of 22 or more rows with three kinds exceeds it, so
the limit goes to 512 (ruling R6, unit test of the bound). A request over the bound is an error state with the reason (`error:
more than 512 files`), no partial drag. "Drag all" (MIDI and wav in one drag) stays and is a hand check.

## 8. Faults and gate safety

No stage starts a system drag, posts a system event, shows a real panel or writes a system pasteboard. In a gate run the drag
goes to the private pasteboard (`deliver::drag_is_test`, `start_drag_system` refuses). The stage `drag-source` stays green: the
new code calls `deliver::start_drag` through the existing command only, so it adds no second system call; the stage reads the
source for exactly one such call.

## 9. Which tempo the MIDI file carries, and the LUNA question

The file carries the tempo map of the host in song seconds, as M1c1 built it (`HostTime::tempo`, `tempo_ara.rs`, `musical.rs`):
the ARA timeline (tempo entries and bar signatures) when the host offers it (source `ara`), else one tempo from the process call
(source `transport`, no steps), else the default map with 120 BPM (source `none`). The writer puts tempo events, the time signature
and a `Bar 1` marker into a tempo track, and anchors every note on its sample (ticks per quarter from `export_ppq`).

LUNA asks on a MIDI drop: "use the current tempo map" or "use the tempo map of the file". The plugin cannot answer it. Answer for
the user and for the page text: choose "current tempo map".

- The notes are written as ticks. A tick is a musical position through the file's map. If the DAW reads the ticks through ITS map,
  the notes land at the right song time exactly when the file's map equals the DAW's map. With source `ara` the file's map IS the
  DAW's map (we read it from the host), so "current" is right.
- "Tempo map of the file" replaces the session's tempo map with ours. With source `ara` that is the same map, so it does no harm,
  but it also writes into the user's session; with source `transport` or `none` it replaces the user's map with one tempo or
  with 120 BPM, which moves every audio clip that follows the grid. Never choose it.
- With source `ara` both answers give the same positions; with `transport` (a fixed tempo and the host tempo did not change) only
  "current" is right; with `none` (120 BPM from no host data) the positions are right only for a session at 120 BPM and the
  tooltip of the MIDI handle says so (the warning that the dialog shows today moves there).
- A MIDI file with NO tempo track (all notes written as quarters through the host's map, the standard tick grid of 32767 per quarter)
  would end the question only for source `ara`: the note positions then follow whatever map the DAW uses. For `transport` and `none`
  there is no map to write the quarters with. It also changes the sample-exact tick rule of #441 into a grid that depends on the
  host's tempo. The risk is higher than the gain. PROPOSAL, not built: keep the tempo track. Do not build the file without the
  coordinator's word.
- The page text for the handles (a hint under the toolbar handles, shown while the MIDI kind is on): "If your DAW asks about the
  tempo of the MIDI file, choose the current tempo map."
- What is not known: which of the three sources a real LUNA session gives (the row `musical_context` of the report says); and
  whether LUNA reads a tempo track of a dropped file at all in the way it asks (hand check, M1c1 step 5).

## 10. Gate line 6: the merged MIDI file

One MIDI file with all drums on one track, offered beside one file per drum.

What exists: `export_multi_midi` / `multi_midi_bytes` (`crates/core/src/midi.rs`) write a type 1 file with a tempo track and ONE
MIDI track PER drum (`MidiTrack {name, note, rate, events}`): the "single" mode of the dialog. A DAW makes one MIDI track
per drum from it. The per-drum file (`midi_bytes_map`, `track1_messages`) has one note track. There is no writer that puts the
notes of several drums into ONE note track.

Decision: add one pure function `merged_midi_bytes` (and a thin `export_merged_midi`) to `crates/core/src/midi.rs`, built only
from existing private parts: `note_events_tm` per `MidiTrack` (each at its own rate, as `multi_midi_bytes` does), the notes
concatenated, `track1_messages` (sorts by tick and note), the tempo track of `multi_midi_bytes`, one note track. The size:
about 45 lines of code and its tests, additive, with no caller except the plugin producer. Nothing changes in `crates/app`: no new
command, no change of `MidiMode`, `ExportOpts`, `REGISTRY`, `dispatch_rows.txt` or `docs/agent-api.md`. A byte test proves that a
call with ONE track gives exactly the bytes of `midi_bytes_map` for the same events (the merged writer is the per-drum writer
for one drum); a second test has two notes and checks the tick order and the single track.

So the change outside `plugins/ara` and `static/` is small, additive, and not on the standalone path. Line 6 goes ahead; it is
not stopped. If the coordinator reads "behind the song-time path" strictly (the function has no song-time guard, because a writer
has none), this is the one point to rule on.

The page: the handle "Drag merged MIDI" in the toolbar only (a merged file spans rows, so it has no row header). Its state is
the state of the MIDI kind for all rows (a stale row makes it `preparing`). The note of each hit is the note of its row; two rows
with one note give the same note twice, as two files would. The track name in the file is `spikes`, like the per-drum files.
The preparation of the merged file is a job like the others, after the per-row MIDI files.

## 11. Gate line 7: the remembered folder

Line 7 is already done by #579 (M1c2, `m1c1-delivery.md`, "The rule now"): the folder the user chooses is kept for the document
in `export-folders.json` (`deliver::FOLDERS_FILE`, `load_folders`, `save_folders`, `remember_folder`), over a reopen of the
editor, and the page falls back to `Exports/` with one short text when the folder is gone. Proof: the stage `export-dialog`, parts
`export-dialog-remembered` (a new editor of the document opens with the chosen folder) and `export-dialog-list`, with the fault
run `no-remember` that fails the first of them; the unit tests `a_remembered_folder_is_kept_in_the_file_and_loaded_again` and
`hostile_content_of_the_folder_list_gives_no_folder`. M1c3 changes nothing there: its files go only to the default folder.
What this note adds for line 7 is a check that the new stages do not break it: the stage `export-dialog` runs unchanged in the
last full run.

## 12. Stages and fault runs

Each new stage has a fault run that fails for its named reason. Names are proposals.

| Stage | What it proves | Fault run |
|---|---|---|
| `prepare-rows` (unit + CLAP) | one file per DAW track and kind with the hits of all clips of the row at song time (two sources in one row, a source in two rows, a clip moved in the DAW); names = row names plus suffix, equal names numbered; the file bytes read back (ticks within 1 sample, spike samples) | `DS_ARA_PREPARE_FAULT=per-source`: one file per source, the stage sees two files for one row |
| `prepare-state` | after a round and after an edit the sets become `ready` by themselves; a take during `preparing` answers 409; an old set is never taken after an edit; a state never stays `preparing` (30 s watchdog, shortened by `DS_ARA_PREPARE_WAIT_MS`); both wav kinds are `off` until their first press | `stale-ok`: the take ignores `fp` (an old set is taken); `forever`: no job ends (the watchdog must fire) |
| `prepare-follower` | a follower row prepared after an edit of its leader carries the refreshed hits (R4) | `no-refresh`: the producer reads the stored hits, the stage sees the old ones |
| `prepare-names` | a hostile row name (`../x`, `/abs`, empty, 300 chars) gives a safe name inside `.prepared/`; orphaned sets are removed; the 2 GiB bound (R11) | `raw-name`: the row name is used as given, the stage sees a file outside the folder |
| `prepare-cost` | an edit in one row prepares only that row (count of jobs); no work in an audio callback (`OFF_AUDIO` stays 0, the allocation test of `threads`) | `all-rows`: every row prepares on each edit |
| `deliver-handles` | the REAL page in the REAL editor in host mode: three handles in a row header, four in the toolbar, states from the plugin event, the collapse to one handle at the small height and under 520 px; a press and a move on a handle runs `plugin_deliver_take` and the drag command up to the private pasteboard (files in `Exports/`, `session: test`); the hint text | `no-collapse`: the collapse rule is off, the stage sees three handles at the small size |
| `deliver-merged` | the merged file has one note track, the notes of all drums, bytes of one track equal the per-drum file | `two-tracks`: the writer makes one track per drum |
| `deliver-standalone` (page tests in `tests/`) | with `hostMode()` false no handle, no element, no listener; every existing page test keeps its expected value | `leak`: the handles show without the mode |
| `export-dialog` (exists) | line 7 | `no-remember` (exists) |

Rust unit tests: `prepare.rs` (the mapping per row, names, `fp` changes for each input, the state machine), `merged_midi_bytes`.
JS: `static/deliver.js` in `JS_FILES` with 70 percent line coverage by `tests/e2e_app` (a page test with the test bridge).

## 13. Work plan for phase 2

One commit per line or pair, risky parts first. This note is commit 1 (Gate line 1).

1. (line 2a) `prepare.rs`: rows to files (MIDI, spike, audio spike), names, `fp`; unit tests. Stages `prepare-rows`.
2. (line 2b) The worker: triggers, debounce, states, watchdog, `plugin_deliver_state` and `plugin-deliver`. Stage `prepare-state`, `prepare-cost`.
3. (lines 3, 5) `plugin_deliver_take`, `static/deliver.js`: the header handles with the collapse, the toolbar handles, the hints. Stage `deliver-handles`, `deliver-standalone`.
4. (line 4) names: done in step 1 (stems of rows); the stage checks equal names.
5. (line 6) `merged_midi_bytes` and the handle. Stage `deliver-merged`.
6. (line 7) nothing to build; run `export-dialog` in the last gate.
7. README of `plugins/ara`, `switches.json` rows for the new switches, the hand check sheet (outside the repository).

## 14. Open questions and risks

1. The dialog exports per source, the handles per row. For a row with several sources the two ways give different files. Move the
   dialog to rows later (a ticket), or keep both. This ticket leaves the dialog as it is.
2. (Settled by R3: only MIDI is eager.)
3. A row whose sources have different sample rates: the audio sum refuses with `clips have different sample rates` (R8); the spike
   takes the rate of the first source.
4. (Settled by R4: the same refresh as the export.)
5. Whether a DAW takes MIDI and WAV in one drag ("Drag all"): hand check (R6).
6. Hard link in `Exports/`: if the volume refuses links, the worker copies; the kind is `preparing` until the copy ends (R5).
8. The tempo source of a real LUNA session is unknown (section 9). If it is `none`, the MIDI is right only at 120 BPM.
9. Rows that the page cannot show (`HT.dropped`: every source already the home of another row) get no header, so no handles; the
   toolbar still delivers them. The status line already counts them.
10. A DAW that references a dropped file in place sees the old link until the next drag replaces it. The hint says "delete the old clips,
    then drop again".
