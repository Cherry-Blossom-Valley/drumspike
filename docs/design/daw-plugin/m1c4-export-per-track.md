# M1c4: the export dialog makes one file per DAW track

Ticket #589, umbrella #503 (the DAW plugin program), part of M1 (#510), fourth part of M1c (delivery).
This note is the design for phase 2. It follows `m1c1-delivery.md` (the dialog, the folder rule of #579) and
`m1c3-one-drag.md` (the producer `plugins/ara/src/prepare.rs`, names, the stale rule). The code places are those of the
base commit `a4f70f2`. A fact that we did not measure says so.

Goal: in a DAW, File > Export... gives exactly what the drag handles give: one file per DAW track per kind, the same
names, the same bytes, made by the same producer. The standalone app does not change.

## 1. What the dialog makes today, and after

Today, in a DAW (`static/export.js`):

- The dialog counts and lists SOURCES (`S.order`, `exTrackSets`): a DAW track with clips of two audio files is two tracks.
- Export calls `export_all` (`exRun`). `build_jobs` makes one job per source, so a DAW track with two audio files gives
  two MIDI files and two spike files. The stems come from `display_names` (per source, `names.rs`).
- The replace count (`exInfo`) is `export_existing`, per source.
- The two tiles use `export_drag_prepare` and `export_drag_take`: the same per-source files, in `Exports/.drag/`.
- The handles (M1c3) make one file per row in `Exports/.prepared/`. For a single-source row the names and bytes are the
  same as the dialog (m1c3 section 3). For a row with two sources they differ.

After, in a DAW only (`hostMode()`):

- The dialog lists DAW tracks: the rows of `plugin_deliver_state` (`DLV.st.rows`), "All tracks (N)" counts rows.
  "Selected track" is the row of the selected lane (`DLV.rowFor(S.selTrack)`).
- Export writes, for each chosen row and kind, the file that the handle gives: `<DAW track>_spike.mid`,
  `<DAW track>_spike.wav`, `<DAW track>_audio_spike.wav`, and for the single file mode `Drums_merged.mid`.
- The replace count counts those names in the destination folder.
- The two tiles hold the prepared sets of the chosen rows and drag them with `plugin_deliver_take`, like the handles
  (ruling R6, section 7).
- The standalone page and the app commands (`export_all`, `export_existing`, `export_drag_*`) are not touched.

## 2. One producer: the dialog copies the prepared set

The dialog does not render. A new shell command `plugin_deliver_export` takes the files that the worker made
(`Prep`, in `Exports/.prepared/`) and puts a copy of each in the destination folder. Reasons:

- Byte-equal to the handles by construction: the bytes are those of the prepared file. There is no second render and
  no second path that could differ in a rounding, a name or a length. Copying a file is not logic.
- No export option is missing from the producer: every option of the dialog that has a meaning in a DAW is a (row, kind)
  of the producer (section 3). Nothing is added to `prepare.rs` for the files. One small additive function is
  added for the counts of left-out hits (section 6), and it calls `to_song`, the same function the files use.
- Cost: no second render of a 115 MB spike file (m1c3 section 6). The cost of the dialog is the copy. We did not
  measure it; the ticket measure for the 10 minute song includes the copy time of the 12 spike files.

The command sits in `editor.rs` beside `plugin_deliver_take` and uses a new `Prep` method `export_to(items, dest)`:

1. Check the call: `items` as for `take` (`{row, kind}` or `{row: "", kind: "merged"}`), `dest` optional.
2. The folder rule (section 5): `dest` through `dest_allowed_in`, the same function and the same faults as `export_all`.
3. Take the state of every item at this moment (`fps`, the same `fp` rule as `take`). Ready: copy. Not ready: answer 409
   with the states, and the page waits (section 4). Never copy a set whose `fp` is not the current one.
4. Copy each prepared file to `<dest>/<final name>.tmp`, then rename over the final name (one file at a time; the old
   file of that name keeps its bytes until the rename). A failed copy removes its temporary file and stops the export
   with the name of the file; the files before it stay (they are complete files).
5. Answer `{files, dest, midi, spikes, audio, merged, replaced, left_out}`. The paths go to `Shared.written`
   (`note_written`) so the drag guard accepts them, as for `export_all`.

The preview of the dialog, `plugin_deliver_plan {rows, kinds, merged, dest}`, answers `{names, count, replace, skipped}`
without copying (the replace count, the skipped rows with the reason `no hits` or `offline`).

## 3. The options of the dialog in a DAW

| Option | In a DAW | Maps to |
|---|---|---|
| Scope all / selected | rows, not sources | the row ids sent in `items` |
| MIDI, per track | one file per row | `Kind::Midi` of each row |
| MIDI, single file | one MIDI track with all drums, `Drums_merged.mid`, all rows | `Kind::Merged` (the file of the toolbar handle "Drag merged MIDI"; it is NOT the type 1 file with one MIDI track per drum of the app) |
| Spikes on, type Synthetic | one WAV per row | `Kind::Spike` |
| Spikes on, type Audio | one WAV per row | `Kind::Audio` |
| Spikes on, type Both | both WAVs per row | `Kind::Spike` and `Kind::Audio` |
| Markers in MIDI | stays absent (m1c1: a source time marker has no song time) | none |
| Compensate sampler offset | stays absent; `compensate: false` | none |
| Destination, Choose, Reset | as today | the folder rule (section 5) |

Single file with "Selected track": the merged file spans all rows, so the scope row "Selected track" and the single
mode exclude each other. In a DAW the single mode row is disabled while the scope is "Selected track" (the same `off`
class the dialog uses for a disabled row) and the text says why. With the scope back on "All tracks" it works again.

A first press on a wav kind starts the preparation (`want_kind`): the kinds Spike and Audio are `off` until first use
(ruling R3 of M1c3). Export is a first use, so the press prepares and then copies (section 4).

## 4. A set that is stale or preparing at the press of Export

Rule: Export never copies a set that is not ready for the CURRENT hits, and never says "done" with an old file. It waits.

- The page calls `plugin_deliver_export`. The plugin first asks for the wav kinds that the options need
  (`want_kind`, so `off` becomes `preparing`) and for a wake of the worker, then looks at the states.
- All items ready: copy, answer 200.
- Something is `preparing` or `off`: answer 409 with the DAW track and its state (`Kick: preparing`). The page shows the progress
  `Preparing n/m` (the progress bar of the export, `showProgress`) and asks again every 300 ms. The plugin call itself
  is short, no request thread waits. The wait ends at once on an `error` state.
- Limit: the existing 30 s of the dialog (`EX.waitMs`, `exWait`) counted from the press. On the limit the dialog says
  `export failed: <DAW track> is not ready after 30 s: nothing was written`; nothing was copied that is not ready.
  Files of the ready items are NOT copied in this case: all or nothing, so a folder never gets a part of a set
  without the user knowing (the toolbar handles do the same, m1c3 section 7).
- An `error` state (a failed job, the 30 s watchdog of the worker, `no export folder`, a row that is not done, M1d4):
  the dialog shows `export failed: <DAW track>: <reason>` and the press of Export is a new try of that set (`want_kind`
  clears the failure, as a press on a handle does).
- A hit that changes during the wait (the user edits, or a round ends): the `fp` changes, the set is `preparing` again, the
  page keeps asking; the file that is copied is the one for the `fp` at the moment of the copy. The check of the `fp`
  and the copy use the same snapshot of `ready` under the lock (a set cannot change under a copy: the worker renames a
  new file over the old name, the copy holds the open old file).
- Rows with no hit are skipped and named (`Skipped: Snare (no hits)`), as today. A row that is not done (M1d4) is
  `preparing` (in progress) or skipped (failed, offline).

## 5. Names, clashes, folders

- Names: the final names of the producer (`final_names`, `file_stems`): the DAW track name made safe, numbered on a clash
  without case, 80 bytes at most. The dialog invents none.
- A name that exists in the destination: the file is REPLACED, as the dialog does today. The dialog says before:
  `N files will be replaced` (the count of `plugin_deliver_plan`, names that exist as a regular file). This is the contract
  of the dialog and different from the take of a handle, which numbers a clash and never replaces (m1c3 section 5): a drag
  hands a file to another program that may still read the old one, an export into a folder the user chose is an act that
  names the folder and shows the count.
- Never replaced: a directory with the name, and a file that is protected (the audio files of the document, the
  document's own files). The plan checks the output names against the protected list before it copies anything;
  `drumspike_app::export::check_export_plan` with `Service::protected_files_everywhere` does it (both made `pub`, ruling R5; no copy of the rule in the plugin).
  The writes never leave `dest`: the name is one safe component (the producer's rule), the temporary
  file is `<name>.tmp` in `dest`.
- Two files of one export with equal names without case cannot occur (`final_names`).
- The folder: the default folder `Exports/` of the document, or a folder chosen in the sheet (this session or remembered
  in `export-folders.json`), exactly the rule of #579. `plugin_deliver_export` and `plugin_deliver_plan` use
  `deliver::dest_allowed_in`; the command list in it gets the two new commands. A `path` key, a subfolder or a folder
  that is not in the list is 403. The default folder is made when missing (`default_dest_made`). `.prepared/` and
  `Dragged/` are inside the default folder; a `dest` equal to them is refused (they are not the default folder).
- The files in a chosen folder join `Shared.written`, so the tile drag and the Show in Finder work as before.
- No write outside `dest` and `Exports/.prepared/`. The command changes no hit, edit or setting; the only state it
  changes is the first-use flag of the wav kinds (a flag of the worker in memory, as a press on a handle does), and the
  refresh of a stale follower that every export and the handles already do (`tracks_get`).

## 6. The page

Every change is behind `hostMode()`. The standalone page has no new element, nothing focusable, no changed value.

- `exTrackSets` in host mode returns rows (`names` = row ids, a label per row, `skipped` = rows with no hits). `exOpts` in
  host mode builds `{rows, kinds, merged, dest}` for the new commands. With the mode off the functions are the old ones,
  line for line.
- `exInfo`: replace count from `plugin_deliver_plan`. `exRun`: `B.deliverExport` with the wait loop of section 4, the summary
  text built from the answer (`exSummary` gets the counts: `N MIDI and N spike files`, the folder, the time).
- The result line names the hits that the song map left out (no region, before song 0). The producer gets one small additive
  function `left_out(row) -> (no_region, before_start)` in `prepare.rs` that calls `to_song` and returns its `Counts`; `mapped_events` and the
  files are not changed. The counts are in `plugin_deliver_export`'s answer, and the page text is the one of today.
- Tiles: the state of a tile is the aggregate (`DLV.agg`) of the chosen rows for the kind (the Spikes tile follows the spike type);
  a drag calls `DLV.drag` with the items of the chosen rows. They need no `export_drag_prepare`, no 400 ms timer, no
  hidden `.drag` folder in a DAW. `exClose` still clears `.drag` (it is empty, no harm; skipped in host mode).
- The stretch line and the tempo line use source names today; in a DAW they use the rows that hold those sources.
- Keyboard: Enter exports as before. The dialog adds no new focusable element.

## 7. Rulings of the coordinator (the decisions of this note, as built)

1. R1: the single file mode in a DAW is the merged MIDI file, `Drums_merged.mid`. The label in a DAW is "One MIDI track with all drums (one file)" and
   its tooltip says what it is. No text promises one track per drum.
2. R2: the single mode with the scope "Selected track" is disabled, with the reason as the tooltip of its row ("needs All tracks").
3. R3: a name that exists in the destination is replaced; the dialog shows the count before the export. A folder with that name and a protected file are
   never replaced. Every file is written to a temporary name in the destination and renamed.
4. R4: all or nothing. A set that is not ready for the hits of now is never copied; the dialog waits (progress, 30 s) and on the limit says which
   DAW track is not ready ("<track> is not ready after 30 s: nothing was written"). The copy checks the state again before the first rename: when the hits
   changed during the copy it removes its temporary files and waits for the new set.
5. R5: `check_export_plan` and `Service::protected_files_everywhere` in `crates/app/src/export.rs` are `pub` (two words, no logic change). The plugin calls
   them; it has no copy of the rule. This is the only change in `crates/`.
6. R6: the two tiles of the dialog show the prepared sets of the chosen DAW tracks and drag with `plugin_deliver_take`, like the handles. The dialog
   renders nothing, uses no `export_drag_prepare`, and has no 400 ms timer in a DAW.
7. R7: "Also updated: older spike files of the other type in this folder" (#274) is not done in a DAW: the names are the producer's, one set per DAW track.
8. R8: the copy runs on the request thread of the command (never the main thread, never the audio thread), in pieces of 1 MiB. The dialog shows
   `Exporting n/m` from `plugin_deliver_progress`. Cancel (the dialog's Cancel button while it runs) sets a flag that the copy reads between pieces and
   before the first rename. A failed or cancelled copy removes its temporary file; a failed copy names the DAW track. When a rename fails, the
   files renamed before it are complete files and the rest is removed.
9. R9: the dialog text names DAW tracks only; the page test of the words and the stage check that no text says "source".
10. R10: `plugin_deliver_plan` and `plugin_deliver_export` take only `{row, kind}` items from the page (a row id is a live row or "" for the merged
    file; any other key in an item, a name or a path, is 400), a `dest` that the rule of #579 allows (the commands are in `dest_allowed_in`), and a `start` flag.
    The file name is the producer's and is checked to be one component. The prepared and delivered folders are not destinations. Fault runs of the stage:
    `any-row` (a row id that is no live row), `trust-name` (a name from the page), `any-dest`, `prefix-dest`.
11. R11: the standalone dialog is not touched: a page test with the plugin mode off checks that it lists audio tracks and calls `export_all`,
    `export_existing` and `export_drag_prepare`.

12. R12: the files start at the session start, so a drop at the clip puts every note late (found in a DAW). The hint is "Drop at the start of the
    session, not at the clip" (`DLV_HINT`, and `DROP_HINT` in `plugins/ara/src/deliver.rs` for the payload: one text in two places, a stage compares them).
    It shows at the start of a drag ("Dragging n items: drop at the start of the session, not at the clip"), after the drop says "dragged n items. If the notes
    are late, move the clip to the start of the session", the toolbar hint has the accent colour, and the dialog shows it next to its tiles (host mode only).

## 8. Gate plan and measures

Stage `export-rows` (test host, mode `exportdialog` family, no system drag, no real panel, no real pasteboard): a DAW track with clips of
two audio files; the REAL dialog in the REAL page runs Export into `Exports/` and into a folder chosen with `DS_ARA_PICK_DIR`;
the stage reads: exactly one MIDI and one spike file for the row, the names of the handles, bytes equal to the files that
`plugin_deliver_take` gives (the same `fp`), the options (per track, single, Both) and the wait path (an edit during the dialog
gives `preparing`, then the new hit in the file; a job that never ends gives the error text, no old file). Fault runs:
`export-per-source` (the dialog calls `export_all`: two files, the stage fails for the count), `stale-copy` (the copy ignores the
`fp`: the old hit is in the file), `any-dest` and `prefix-dest` for the new commands, `no-wait` (a not ready set is skipped:
the file is missing). The page tests (`tests/e2e_app`) get the host-mode dialog cases; no expected value of a standalone test changes.
Unit tests: the plan (names, replace count, a directory with the name, protected names), the copy (temporary file removed on a failure,
the old file kept until the rename), `left_out`.

Measures (printed by a script that I will add under `plugins/ara/`, run with the gate lock, numbers in the report):

- The time to prepare the spike files of a 10 minute song with 12 tracks: 12 sources of 600 s at 48 kHz, 700 hits each, one clip per row,
  the real worker path (`Prep`), from the wake to all 12 `ready`, split in MIDI, spike, and the copy of the dialog into a folder. Also the
  size on disk (m1c3 estimates 115 MB per row).
- The cost of the idle look with 40 rows: one pass of `gather` and `fps` after a wake that changes nothing (`settle_dirty`), the
  CPU time of the worker over 10 s with no wake (expected 0, it waits on the condition variable), and in the page the time of
  `DLV.repaint` and the size of the `plugin-deliver` payload with 40 rows.

Not proved by these: a real DAW, a real folder sheet, a copy to a slow or network volume, the time on the owner's disk (the numbers are from
this machine).

## Round 2 (review of f1ee950)

- M1: before the first rename the export checks every target (a folder in the way, a protected file, a read-only file). When a rename still fails, the files before it
  are complete and the answer lists the files written and the DAW tracks not written; the dialog says: "2 of 5 files were written: a.mid, b.mid. Not written: C, D, E.
  Press Export again." Every temporary file is removed.
- S2: one export at a time (a flag per document; a second call answers 409 "an export is running" and changes nothing of the first). The same row and kind twice is one
  file. A temporary name is `.<name>.dsx-<pid>-<n>.tmp`, created new (an existing file of that name is never cut or followed).
- S3: a guard removes the temporary files on an error, a cancel and a panic. At the start of an export the plugin removes the temporary files of its own pattern that an
  earlier export of this process left, or that are older than an hour; any other file stays.
- S4: a poll that waits wakes the worker only for a set that is `off` or in error (a set that is `preparing` needs no look); the first call of an export wakes it once. The
  poll gathers once. The page asks for the replace count once per 150 ms (the newest wins).
- S5: tests for the hits that change during the copy, a copy that fails, a protected file through the real list of the service, and a stage part `export-rows-late` with a
  fault run `no-late-check` (the stage keeps the copy open with `slow-copy` and adds a hit). `stale-ok` now skips only the check before the copy.
- S6: a prepared file that is gone when the copy opens it is a 409 "preparing"; the page waits.
- N8: a cancel during the wait says "export cancelled: nothing was written". N10: `plan.exists` and `replaced` use `prepare::replaces`.
- N7: the state of a set is read under the lock of the worker's books for the time of a `stat` per file; this is short and unchanged by this ticket, no change.
  N9: the second `canonicalize` (the folder of the prepared files against the chosen folder) is cheap and runs once per call, no change.
