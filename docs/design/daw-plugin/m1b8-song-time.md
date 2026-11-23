# M1b8: the page in the plugin shows song time

Ticket #555, umbrella #503 (the DAW plugin program), part of M1 (#510). In host mode the timeline of the page is the song time of
the DAW. One row of the page is one DAW track (one region sequence of ARA). Every clip of the row lies at its song position. The
playhead is the position of the host, 1:1. The standalone app does not change.

Why now: a hand check in a real DAW had one DAW track with two clips from two audio files. The page made two rows, selected the
first, and showed "only the first clip". The playhead was mapped through one region and jumped at play start. The role models
(a pitch editor and an alignment tool of the same kind) show the DAW track as one row, with each clip where the DAW has it, and
their playhead is the host position. This note follows them.

## 1. What the code does today (facts checked on the base commit)

- The page row is the audio source: one project track per source, key = track name (`service.rs:1385` `track_layout` makes the
  track of a source from its regions; segments carry `start = offset = start_mod`). Hit times are source time.
- The plugin knows the region sequence of every playback region: `RegionSlot.sequence` (`lib.rs:309`), the names and the DAW order in
  `DocShared.sequences` (`lib.rs:391`, `SeqInfo {name, order}`, `lib.rs:320`). Today only the display names (`names.rs:131`,
  `display_names`), the selection (`selection.rs`, `TrackFacts.regions`) and the label for the role guess read it.
- The live mapping of a region exists for the export: `export_region_list` (`lib.rs:1091`) with `merge_regions` (`deliver.rs:44`) gives
  `start_mod, dur_mod, start_play` and the live `dur_play` (`play_len`, `lib.rs:411`). The export maps a hit with `to_song`
  (`crates/app/src/export_song.rs:68`).
- The poller maps song time to page time with `transport::page_time` (`transport.rs:335`) through the regions of the selected track
  (`Shared::mapping_regions`, `editor.rs:1462`; the page tells the track with `plugin_host_track`, `editor.rs:985`). A click goes the
  other way with `transport::song_time` (`transport.rs:394`). The range of the host selection goes through
  `selection::range_to_source` (`selection.rs:152`).
- The page has one view and two global functions, `tToX` (`static/app.js:176`) and `xToT`, with no track argument; `maxDur`
  (`app.js:178`) is the longest track. Hit drawing, hit test, edits, velocity lane, range and snap read source time
  (`drawEventLines` `wave.js:312`, `drawVelLane` `velocity.js:77`, `hitEvent` `tracks.js:356`, `snapSample` `app.js:179`).

## 2. Row = DAW track (decision 1)

A row is the set of playback regions that lie on one region sequence, over all the sources they use.

- **Identity.** The row id is the id of the region sequence that the plugin made (`"q<id>"`, the counter of `lib.rs:315`, never a
  pointer). It is stable inside one run of the plugin. Nothing persistent uses it: the stored data (events, Learn, params, the ARA
  archive) stay per source, keyed by the track name as today. After a reload the ids are new and the rows are rebuilt from the
  regions.
- **Order.** The DAW order: `SeqInfo.order`, `None` last, then the lowest creation index of a source of the row (the same sort as
  `names::display_names`, `names.rs:131`). Rows of sources with no region come after the DAW rows, by creation index.
- **Name.** The name of the sequence (cleaned with the rule of `names.rs`), else the name of its first region, else the track name of
  its first source. A number `(2)` goes only between two ROWS that show one name, as today between tracks. Two sources inside one
  row never give a number to the display name (this retires the "X" and "X (2)" of the hand check).
- **Source with no region** (no round saw a region, or the host has no regions): its own row with one identity clip
  `{a: 0, b: len, s: 0, k: 1}`, named by the track name. This is the standalone-like case and keeps the rule "no region: song time =
  source time" of `m1c1-delivery.md`.
- **Source whose regions are all gone** (every clip deleted in the DAW; `export_region_list` reports it as `gone`): no row. The page
  shows one count in the status line ("N sources have no clip in the DAW"). Its hits stay stored.
- **Region with sequence 0** (the host gave none, `lib.rs:310`): the source forms its own row, keyed `"s<creation index>"`.
- **A source on two sequences:** it is a member of both rows, with only the clips of that row's regions in each. Its hits are
  drawn in each row where a clip holds them. Its project track is one; the row that comes first in the DAW order is its "home" row
  for the settings (section 4). Limit: the page has one DOM row per source and a row is drawn by the DOM row of its home source,
  so a source can be the home of one row only; a row whose every source is already the home of an earlier row is not shown, and the
  status line counts it.

## 3. Clip model (decision 2)

A row holds a list of clips `{src, a, b, s, k}`:

| Field | Meaning |
|---|---|
| `src` | The track key of the source (the page's `S.tracks` key). |
| `a`, `b` | The slice of the source: `start_mod`, `start_mod + dur_mod` (source seconds). |
| `s` | `start_play`, where the slice starts in the song. |
| `k` | `dur_play / dur_mod`, the stretch factor (1 when the host does not stretch). Live value as in `export_region_list`. |

A hit with source time `t` in a clip with `a <= t < b` has song time `s + (t - a) * k`. This is the formula of `to_song`
(`export_song.rs:68`); a page test and the plugin test compare the two on one list. The hit is drawn once per clip that holds it.
Source audio outside every clip is not drawn; its hits are counted ("N hits outside every region", the `outside_song` word of the
export). Overlapping clips of one source or of two sources: both are drawn, the later in the DAW order on top; no hit is lost.

Inverse, for a pointer at song time `x`: for a clip with `s <= x < s + (b - a) * k`, `t = a + (x - s) / k`, then
`snapSample(t, src)`. The error of the round trip is at most half a sample of the source rate for `k = 1`; for `k != 1` the bound
after `snapSample` is 1 sample of the source rate (the rate of `src`, not of the row). A test proves both, with a cut start, two
clips of one source, two sources, a stretch and a point in a gap (answer: no clip).

## 4. Settings of a row (decision 3)

Per source today (project track, `crates/core/src/project/document.rs:195`): role, note, params (threshold and the rest), velocity
cut, Learn, volume, pan, `follows`, colour. Page state per name: mute, solo.

Rule: the **row owns** these settings: role, note, params, volume, pan, colour, mute, solo and hide (the host's hide of #562 is
per sequence already). The page shows the values of the home source of the row (the first source by creation index) and a change
goes to every source of the row. What the code needs: every call that writes one of these takes a track name (`B.setNote`,
`B.trackSetRole`, `B.paramsSet`, `B.trackSetMix`, `backend.js:245,416,121,527`). Page code wraps them with one loop over
`HT.srcsOf(name)`. Mute and solo are keys of `S.muted` and `S.solo`; `applyMixer` (`mixer.js`) reads the row's flags for each of its
sources. This is about 10 call sites in the page and no change in the service. Defaults agree already: the role guess reads
the name of the sequence (`label`, `lib.rs:286`), the same for all sources of a row.

Not owned by the row, per source on purpose: **Learn, the velocity cut and the hits themselves.** A Learn mark is a pattern found in
the audio of one source; the same pattern at the same time in another source means nothing. A Learn command works on the source of
the selected hit, or of the clip under the pointer. The velocity gestures work in a clip, so on its source. The cost of a row-wide
Learn (a new fan-out with a mapping of the marks, the histograms of two files) is high; it is left out.
A smaller first step is not needed: role, note, params and mix fan out cheaply, Learn and hits are naturally per source.

Edits find the source from the clip under the pointer (`toSource`, section 6). A selected hit is `{track: src, index}`, as today, so
undo, `events_edit` and the whole backend see only the source and source time.

## 5. The project does not change (decision 4)

Display only. The project and the service keep one track per audio file; no file format changes, no new command of the app. The
plugin gives the page `plugin_host_regions` (new shell command; `SHELL_COMMANDS` and the lists in `editor.rs`, `shell.js` and the
test host list):

```
{gen, rows: [{id, name, srcs: [key...], clips: [{src, a, b, s, k}...]}], song_end, gone: [key...]}
```

built from the live regions (the `export_region_list` merge, plus the sequence id of each slot; `plugins/ara/src/rows.rs`). The
plugin pushes the same answer as the page event `plugin-regions` whenever a region, a sequence or a round changes the rows (the
document compares the text with what the page was told last), so no counter has to travel with the transport message. The page
counts the hits outside every clip itself (the stored hits and the clips are both in the page). The page module `static/hosttime.js` (new,
pure functions, no DOM, in `JS_FILES`) keeps the rows. The page rows are built from it: the DOM row of the home source
draws all sources of the row; the other source rows of a merged row are hidden (`.hostMerged`), not deleted. Stored times stay source
times: `S.events`, the undo steps and every call of the backend never see song time. Only the module converts, at drawing and at
input. Without `window.__DS_HOST_MODE__` the module answers `on() === false` and every guarded path returns the old code.

## 6. Transport and selection (decision 5)

- Follow: the poller sends the song position unchanged. `page_time`, `Mapped` and the `mapped_by` log field go. The page timeline
  is song time, so `PH.v` is the host position.
- Locate: the page sends song time, bounded 0 to 24 h; `song_time`, `LocateRule` and `plan`'s mapping go.
- The mapping of #554 by selected track retires: `Shared::mapping_regions`, `plugin_host_track`, `select_track`, `sel_gen`
  (`editor.rs:1462,985,1457,275`). It stays for nobody: with no region at all the identity clip makes song time equal source time.
  Tests of the removed functions are deleted in that commit; the stage `editor-song-transport` and the unit tests of the new rule
  cover the same invariant (position 1:1, no dependence on the selection).
- #562: `selection::range_to_source` (`selection.rs:152`) is replaced by the song-time range itself: the payload carries it as one
  `{t0, t1}` in song seconds (`ranges`); the page draws one band over all rows. The track list of the payload stays a list of source
  keys; the page maps each to the DOM row that shows it (`HT.homeFor`), so a DAW track selects its row. Deleted with
  `range_to_source`: the faults `song-range` and `outside-draws` of the selection (the song range is now the rule), the cap of 64 bands.

## 7. Export and drag (decision 6)

Left out of this ticket, left to #576 (one MIDI and one spike file per DAW track with the hits of all its sources merged). Reason: the
export is built per project track (`build_jobs`; `ExportTime::track`, `deliver.rs:90`, gives the regions of one track). A merge of two
sources into one file needs a group job in the app crate (events of several tracks, one file stem, one length), which is a change
of core export code with the identity-snapshot proof. It is not display work. What is cheap now: nothing that would be thrown away.
What stays true: the files of #560/#579 are per source, in song time, named after the display name. A second source of one DAW track
gets the number `(2)` in its file name (the stem rule `file_stems`, `deliver.rs:167`, needs unique names); the page row name has none.
The note says so, and the page does not hide the limit.

## 8. Commit plan (decision 7)

Built order (the coordinator's points of 2026-11-22): the transport and the selection range go direct in the SAME commit as the drawing,
because a page that draws song time with a playhead in source time is not a state that can be looked at. So: 1 the note, 2 the
plugin gives the rows, 3 the page draws song time and the transport and the range are direct (the owner's two defects), 4 input, edits,
the settings of a row and the keys, 5 what is left (stages retargeted, text, final runs). The list below is the first plan.

Each stage has a fault run that fails for its named reason (`DS_ARA_SONG_FAULT`, listed in `plugins/ara/switches.json`).

1. This note.
2. Plugin: `plugin_host_regions`, `rgen`, row building (`rows.rs`, pure, unit tested: identity, order, name, no region, gone, one
   source on two sequences, sequence 0). Stage `editor-song-rows`; fault `round-regions` takes the regions of the round: a moved
   clip is not seen and the stage fails with "start_play is old".
3. Page: `hosttime.js` and drawing in host mode (clips, hits of all sources of a row, peaks per clip, ruler, `maxDur`, the band
   of #562). Page unit tests of the round trip (fault `no-cut-start` ignores `a`: the cut-start case fails). Stage
   `editor-song-draw`: two sources on one DAW track and a second track at another position; the page reports the pixel of a hit
   per row. Fault `source-x` draws in source time: the pixels differ by the offset.
4. Page: input and edits (inverse map, add, drag, velocity, range, snap) and the settings fan-out of section 4. Stages
   `editor-song-edit` and `editor-song-store` (events, archive, notes for the host and export in source mode compared with the run
   with song display off: only the added hit differs). Fault `store-song`: the page sends song time, the stored `t` is off by the
   offset. Fault `home-only`: a role change reaches only the home source; the stage fails on the second source.
5. Transport and selection direct (section 6). Stage `editor-song-transport`: change the selection, the playhead does not move; the
   click time reaches the host as it is; a host position inside the second clip of a row shows at its song position. Fault
   `selected-track` keeps the mapping of #554: the playhead moves with the selection.

Standalone proof (all commits): `HT.on()` is false without `window.__DS_HOST_MODE__` (test); no expected value of a page test
changes (`git diff` of `tests/` shows only added tests); fault `leak` turns `HT.on()` on without the flag and a standalone page
test fails; the diff of old functions shows only `if (!HT.on())` guards. Gate line 4 has its own unit test: calls of
`plugin_host_regions` and a pure display change leave the hash of `events_get` equal.

## 9. Risks and what stays out (decision 8)

Risks:

1. Two sources with one role guess but a hand edit in one: the row shows the home source; a change then overwrites the other. This is
   the rule "the row owns the settings", and the inspector says "all N sources of this track".
2. A source on two sequences shows its hits in two rows; an edit changes both. Clear to the user, since it is one audio file.
3. Peaks of a clip with `k != 1` are drawn scaled (no warp markers). Stated limit.
4. #562 (selection) and #560/#579 (export) are merged first and use the per-source model; commit 5 swaps the selection mapping, and
   the export keeps per-source files until #576.
5. The tempo grid and the bars of the page are in source time; with no tempo map from the host they are off in song mode.

Out: tempo and bars from the host, time stretch (warp markers), loop, row-wide Learn, one file per DAW track (#576).

## 10. What was built, and where it differs from the plan above

- **Settings of a row.** The audio group of the page (mute, solo, volume, pan, the mixer) is absent in a DAW (`HOST_ABSENT`, #539), so
  those settings need no fan-out. Role, note, Sensitivity, Loudness (the velocity map) and Shift apply to every source of the row as
  ONE undo step: the page has an undo entry `{type: "group", steps}` (`pushGroup`, `groupApplyOp` in `edit.js`) that holds the step of each source, undo
  takes them back in reverse order, redo does them again. With one source (a standalone project, or a row with one source) `pushGroup` pushes
  the plain step, so the standalone undo path is the one of today. Sensitivity and Loudness run the same service call for each other source after the first
  (`rowRepick`, `rowLoud`), Shift runs `shiftApply` for each. The first source of the row is the one whose histograms the Inspector shows.
  An undo of the role of a row puts all sources back to the role of the first source.
- **Learn is not fanned out.** A Learn mark is a pattern of one source's audio (`track_learn_mark` reads the features at the marked time of that
  file); the rule set (the examples with their fingerprints, the Match and Add lines) belongs to that source. Applying it to the other sources of
  the row needs a new service command that takes the marks of one track and measures them on another track's audio (and a decision whether the examples of
  two files may mix). That is more than a fan-out of the existing call, so Learn stays with the source of the clip you work in, and the Inspector line says so.
- **Hand values.** A hand value of role or note in a non-first source is overwritten the next time the row is changed (the row owns role and note);
  until then it is kept and shown in the history of that source only. The Inspector shows the first source.
- **Outside every clip.** The status line says "N hits outside every clip" when a source has stored hits that no clip of its row shows (they stay stored and are not
  in the files); at zero there is no text (`HT.noteOutside`).
- **Waveform without WebGL2.** The page draws the waveform of each clip in the Canvas 2D path too (the min/max envelope from the
  peak arrays, `drawClipEnvelopes` in `wave.js`); it has no sample-exact window there. The web view of the plugin has WebGL2 on macOS,
  so this path is the fallback after a lost GL context only.
- **Sample-exact zoom.** The exact samples window of the GL lane is one window for each source. When two clips of ONE source are both
  inside a sample-exact view (about 5 s or less across), one of them shows the exact wave and the other the coarse peak tiles (the
  finest level, 256 samples a column): never empty, never at the wrong place, only less sharp until it is the one in the window.
- **Faults.** The fault runs of the page are a flag, `HT.fault` of `static/hosttime.js`, which the page code reads; only the test driver of
  the plugin sets it (from `DS_ARA_SONG_FAULT`), so a delivery build never does. `round-regions` is a switch in the plugin. The e2e
  test `test_the_fault_flags_change_what_the_page_does` runs the flags in the app tests.
- **Keys.** A merged source is hidden through `.hostMerged`; `hostHidden` (Tab over all tracks, `;`, `P`) skips it, `setSelTrack` maps a source to
  the row that shows it, Tab on a row walks the hits of all its sources in song order, the review queue (D) takes the hits of
  merged sources with their DAW track and puts the playhead at the song time of the hit.
- **Not built.** Learn on a row (see above). A source that is the home of two rows shows its first row only.

## 11. Review round (PR 585)

- **Menu Add Hit at the playhead** goes through the clip under the playhead (`HT.toSource`), to the source of that clip, in source time; no clip: no hit and one text. The add by double click or Option-click, and the
  nudge, keep the hit inside its clip (`HT.clampInto`, `HT.clampTo`; an attack snap that lands past the edge is moved back into the clip by a second step).
- **Tab** does not snap a song time to the grid of one source (the compare finds the same hit again otherwise); same for the review queue.
- **Role of a row:** each source keeps its own `previous` (its hits, its hand edits); the change is ONE group step of `role` steps; a source that refuses stops the row,
  the entry holds exactly the sources that changed, and one text names the refused source. An undo step applies its own source only.
  A note has no undo step; a refusal of one source says which.
- **Group undo is all or nothing:** a refused step takes the steps applied before it back. `hgRepickBusy()` stays true until every source of the row answered
  (`HG.rowBusy`), so no newer step lands before the group.
- **Readers of the playhead and of a hit time:** Go to End is `HT.songEnd()`; the zoom toggle and the review queue use song time; the tempo map (Set Bar 1 at Playhead), the
  markers (M, Option+M) are in the time of the audio of each source and are inert in a DAW with one text; `agent.js` select uses song time.
- **Plugin cost:** `rows_changed` builds the rows once and pushes through the 50 ms gate of the selection (the newest state always arrives); a unit test makes 500 changes.
- **Bad values:** a clip with a value that is not finite, or a stretch outside 1/64 to 64, is dropped and counted; one log row `rows_dropped` says how many.
- **New document:** the payload names its document (`doc`); the page starts counting again when the id changes (the counter of a new `DocShared` starts at 0).
- **Selection band** at the song position of an offset clip: the page test `test_the_range_of_the_daw_stands_at_the_song_position_of_an_offset_clip` with the fault flag `range-source`
  (the removed selection faults of the plugin are replaced by it).
- **`HT.dropped`** (a track with no row of its own) is shown in the status line: "N DAW tracks not shown".
