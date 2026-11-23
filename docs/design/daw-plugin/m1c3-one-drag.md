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

Round 2 (review of 5ef6d03), the rule that holds now: the worker writes ONLY inside `Exports/.prepared/`. It never creates, replaces or deletes a
file in `Exports/` itself, so a file of the user in `Exports/` (the export of the dialog, a file kept by hand) is never touched. A set in `.prepared/`
is written under a temporary name and renamed, so the old file keeps its inode and its bytes.

A take (the start of a drag, `plugin_deliver_take`) hard-links each prepared file into the fixed plugin-owned folder `Exports/Dragged/` under its final
name (the same folder tree, so a link works; there is no copy fallback: a failed link is an error with its reason and no drag starts). When the name
exists there, the same file (the same inode) is reused; another file of that name is never replaced: the new file is numbered (`Snare_spike 2.mid`).
The plugin never deletes anything in `Exports/Dragged/`. So a DAW that references a dropped file in place never sees it change, and a new drop
after an edit brings a new numbered file; the hint of the audit (item 8), delete the old clips before the new drop, still applies. The link is made
on the request thread of the take (one `link` call per file, no copy): nothing heavy runs at the press and nothing runs on the main thread. A take
checks that the prepared file exists; a set whose file is gone is not ready, it is stale and is made again. The drag command accepts files that
are directly in `Exports/Dragged/` (written by this editor), as before for `Exports/`.

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
- Disk, the true rule (round 2): one prepared file per (row, kind) in `.prepared/`. At every pass the worker removes the files that no ready
  set holds (an old set, a temporary file, the set of a row that is gone) inside `.prepared/` only. The folder itself stays until the user deletes it
  (there is no removal at the close of the last editor: the sets stay valid for the next open). `Exports/Dragged/` is never cleaned: what was
  delivered stays. Bound (ruling R11, S8): before a wav file is written the size is estimated (`ceil(song end x rate) x 4` bytes: float 32, one
  channel) and the job is refused with `error: not enough room for the prepared files (limit 2 GiB)` when the estimate and the bytes already in `.prepared/`
  (every file, temporary files too, minus the file it replaces) are over 2 GiB; the real size is checked again before the rename. Not bounded by the
  2 GiB: `Exports/Dragged/` (it holds links, so no extra bytes until the prepared file is replaced; then the old bytes live on in the delivered file),
  and the memory of the audio-spike sum of a row with several sources: it holds one float 32 buffer per source, refused above 1 GiB with `error: not enough
  memory for the audio spikes of this row`. `.prepared/` is a fixed child of the default folder written only by the plugin; no user path reaches it, and
  the exact-path rule for user destinations (#579) does not change. The name of a file is made by `file_stems` and `final_names` from the row name, so
  a hostile row name (`../x`, `/abs`, empty, 300 characters) stays one safe name; `make` also refuses, before any write, a path whose parent is not
  `.prepared/` (a unit test with `../`, `/abs` and `a/b`; the fault run `raw-name` shows the bad name reaching that check, which refuses it: the state is an error).
- Why the wav sets can take longer than the 3 s quiet time (the 9.4 s of round 1): the quiet time counts from the LAST wake, and the page's status poll
  (`analyze_status`, every 2 s, answer `done`) woke the worker at every answer through `changed_after` (the "ended" branch), so while any poll ran the
  quiet time restarted every 2 s. That was a defect. Now a command that writes the project wakes the worker, and a `done` answer wakes it ONCE for the job
  (`Prep::command_done`, unit test). The check keeps the trace switch `DS_ARA_PREPARE_TRACE` that shows each wake with its caller.
- A failed job: the handle shows the error; a press on it (mouse, Enter or Space), a take, and a back-off (5 s, doubled each time, at most 60 s) start it
  again. A new state of the hits (the `fp` changes) starts the watchdog again and the old failure no longer counts.
- A pass with no export folder (the service is open and gives none) marks every handle `error: no export folder`, not `preparing`.
- Memory of the worker and the thread: the closures of `Prep` hold the document weakly (the document owns the preparation: a strong clone was a cycle that
  nothing freed). The worker ends when the preparation is freed, and `prepare::stop_all` (called by `unload_worker` before the library can be unmapped)
  stops every worker thread and waits for them (at most 1.5 s). The fault `forever` waits for the stop sign or for the end of the document.
- Two rows with one note (the merged file): the hits of the rows merge on that pitch. Each hit keeps its own note-on and its own note-off, none is
  lost or doubled (a unit test with overlapping notes of two rows on one pitch); a player that does not count overlapping notes of one pitch ends the
  first note at the first note-off. The rows of a kit have their own notes, so this is the rare case.

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
more than 512 items in one drag`), no partial drag. "Drag all" (MIDI and wav in one drag) stays and is a hand check.

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
  tempo of this MIDI, choose the current tempo map."
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

Each new stage has a fault run that fails for its named reason. These are the names as built (round 2).

| Stage / test | What it proves | Fault run |
|---|---|---|
| stage `deliver`, run `base` (no edit) | `deliver-rows`: one MIDI file per DAW track with the hits of all clips at their song positions; `deliver-wav`: one spike and one audio-spike file per DAW track, each hit at its song sample; `deliver-size`: the spike files have the computed size (4 bytes x ceil(song end x rate) + a header under 2000); `deliver-merged`; `deliver-state`; `deliver-drag` (private pasteboard, 6 files in `Exports/Dragged/`); `deliver-prepared-folder`; `deliver-exports-clean` (the worker wrote no file in `Exports/`); `deliver-handles` | `per-source` (rows), `two-tracks` (merged) |
| stage `deliver`, run `edit` (three edits 400 ms apart, scenario `user-file`) | `deliver-stale` (a take at once is refused, the set becomes ready by itself, the new hit is in the MIDI file and the merged file); `deliver-spike-after` (the spike file of the edited row has a spike at every hit, the new ones too); `deliver-user-file` (a file of the user in `Exports/` has the same bytes after preparation, takes and edits); `deliver-delivered-keeps` (new numbered files, the first ones untouched); `deliver-cost` (four jobs, not a pass for each edit) | `stale-ok`, `link-exports` (the old behaviour changes the user's file), `all-rows`, `no-debounce` |
| stage `deliver`, run `names` | `deliver-file-names`: rows `Snare` and `Snare_audio` give six different file names without case, the later audio file is numbered | `no-file-numbering` |
| stage `deliver`, run `watchdog` / `hostile` | `deliver-watchdog` (a job that never ends: error, not preparing for ever); `deliver-hostile-name` (`../evil`, `/abs`: files stay in `.prepared/`) | `forever` with `no-watchdog`; `raw-name` |
| page tests `tests/e2e_app/test_host_deliver.py` | handles, states, collapse, the menu items as drag sources with no synthetic mouse event, no listener left on `document` after the menu closes, keyboard Enter and Space, `role="button"` and `aria-label`, Drag all prepares both spike kinds, the standalone page has nothing | `leak` (a page test that must fail the standalone check) |
| `export-dialog` (exists) | line 7, and (unchanged expectation) 4 MIDI and 4 WAV files in `Exports/` | `no-remember` (exists) |

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
6. Hard link into `Exports/Dragged/` (round 2): a volume that refuses links gives `error: could not link ...` and no drag; there is no copy fallback.
8. The tempo source of a real LUNA session is unknown (section 9). If it is `none`, the MIDI is right only at 120 BPM.
9. Rows that the page cannot show (`HT.dropped`: every source already the home of another row) get no header, so no handles; the
   toolbar still delivers them. The status line already counts them.
10. A DAW that references a dropped file in place sees the old link until the next drag replaces it. The hint says "delete the old clips,
    then drop again".

## 15. What phase 2 built, and where it differs from this note

- `plugins/ara/src/prepare.rs`: the producer and the worker (`Prep`). One file per DAW track and kind: `<DAW track>_spike.mid`,
  `<DAW track>_spike.wav`, `<DAW track>_audio_spike.wav`, and `Drums_merged.mid`. A set is `ready` only when its `fp` equals the `fp` of
  the hits now; a wake of the worker (an edit, a round, a change of rows or of the tempo) shows every set as `preparing` until the next pass has looked.
- Quiet time: MIDI 800 ms (at most 5 s from the first change), the wav kinds 3 s (at most 10 s). `DS_ARA_PREPARE_QUIET_MS` shortens it for the check.
- Followers (R4): the producer calls `tracks_get`, which runs the refresh of stale followers that the export runs (`follow::refresh_stale` ->
  `fresh_for_export`). `fresh_for_export` stays `pub(crate)` and `crates/app` is not changed: the plugin has no way to get the shared
  project, and the visibility change alone would not have been enough. What is not the same as the export: a follower that cannot be derived
  (its audio is offline) is read as stored, where the export refuses with a 409. In the plugin the sources are cached files, so the case needs
  an audio file that was removed by hand. The unit test `a_follower_row_carries_the_refreshed_hits_after_an_edit_of_its_leader` makes the stale
  follower (audio away during the edit of the leader, back afterwards) and shows the refreshed hits with the refresh and the stale ones without it.
- The only change outside `plugins/ara` and `static/` is `merged_midi_bytes` and `export_merged_midi` in `crates/core/src/midi.rs` (about 55
  lines with doc comments, two unit tests; nothing in `crates/app`).
- `plugin_deliver_state` starts the worker once; a poll of the page wakes nothing (the page asks again every 1.5 s while a handle is `preparing`).
- Superseded by round 2 (see section 5): the worker no longer links into `Exports/`; a take links into `Exports/Dragged/`.
- The drag limit is 512 files (`deliver::drag_count_ok`, unit test); `plugin_deliver_take` refuses more with `error: more than 512 items in one drag`.
- A dropped row (`HT.dropped`) is named in the hint line beside the toolbar handles (R10).
- Stage `deliver` (parts `deliver-ran`, `-rows`, `-wav`, `-size`, `-merged`, `-state`, `-drag`, `-prepared-folder`, `-handles`, `-stale`, `-cost`,
  `-watchdog`, `-hostile-name`) and `deliver-fault` (faults `per-source`, `two-tracks`, `stale-ok`, `all-rows`, `no-debounce`, `forever` with
  `no-watchdog`, `raw-name`). The page tests are `tests/e2e_app/test_host_deliver.py` (the fault run `leak` is a test there).
- The test host has no DAW and no mouse: the stages prove the files, the states, the handles in the real page and the drag command up to a private
  pasteboard. The real drag into a DAW is a hand check.

### Hand checks in a real DAW (owner)

1. Open a session with a drum track whose clips come from two audio files. Let the analysis finish. The MIDI handle of the row shows `ready 1`,
   the spike handles say `press to prepare`.
2. Drag the MIDI handle of the row onto an instrument track at the session start. The notes of both clips sit on their hits.
3. Press the Spike handle (no drag): it goes to `preparing`, then `ready 1` within a few seconds. Drag it onto an audio track at the session
   start. Check that the file starts at song position 0 and the spikes sit on the hits (zoom to samples on a loud hit).
4. Press the Audio spike handle, drag it onto an audio track: the snippets sit on their hits.
5. Edit a hit. The handles of that track show `preparing`, then `ready`. Drag again: the new hit is in the file. Delete the old clips first.
6. Drag "Drag all" (MIDI and wav files in one drag) and note whether the DAW takes mixed types and where each file lands.
7. Drag "Drag merged MIDI": one MIDI track with all drums on their notes. If the DAW asks about the tempo, choose the current tempo map and
   note what the report row `musical_context` says (source ara, transport or none).
8. Make the row header small (Track Height Small) and narrow the window under 520 px: the handles collapse to one handle and a menu; the menu
   lists the three kinds and a keyboard can reach it.
9. Close and open the session: the folder choice of the dialog is remembered (line 7, M1c2); the handles prepare again after the first round.
