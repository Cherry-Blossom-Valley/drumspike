# M1b13: the page follows the selection of the DAW

Ticket #562, umbrella #503 (the DAW plugin program), part of M1 (#510). The host tells the plugin what it has selected and which
tracks it hides. The page shows both, as the editors of the role models do. This note says what ARA gives, what the plugin makes of
it, the rule for each case, and the proof. The last section is the hand check for a real DAW.

## 1. What ARA gives

ARA has one direction here. The host calls the plugin; the plugin has no call that sets the selection of the host. The only thing a
plugin can move in the host is its playhead (`requestSetPlaybackPosition`, #554, already in the plugin).

The host calls two functions of the editor view interface of the plugin:

- `notifySelection(ARAViewSelection)`. The header says it is not a setter, it is a notice: "the plug-in remains free to modify its internal
  selection at any time". It lists what a plugin may do with it: scroll to relevant positions, select the playback regions in its
  inspector, filter objects. The fields:

| Field | Meaning |
|---|---|
| `playbackRegionRefs` (and its count) | The playback regions the host selected explicitly. May be empty. A plugin that needs regions may derive them from the sequences and the time range. |
| `regionSequenceRefs` (and its count) | The region sequences (the DAW tracks) the host selected explicitly. May be empty. A plugin may derive them from the regions. |
| `timeRange` (a pointer) | The time range the host selected explicitly: `start` and `duration` in song seconds. NULL means "this call carries no time range". It is a range in the song, not in the audio of a source. |

  All pointers are valid only during the call. The plugin copies what it needs inside the call.
- `notifyHideRegionSequences(count, refs)`. The host hides tracks from view, they still play. Each call replaces the list of the call
  before. A call with an empty list shows every track again.

The struct of the host may be shorter than ours (`structSize`); the plugin reads a field only when the declared size reaches it.

## 2. What the DAW sends

Before this change the plugin wrote only a count row (`view_selection`: counts and whether a range came, one row a second).
A real DAW run showed that the DAW sends the selected track (a region sequence) and a time range. The row now carries what the page
needs to be checked from the report alone:

- `sequence_names`: the names of the selected DAW tracks,
- `range_start_s`, `range_duration_s`: the range of the host in song seconds,
- `tracks`, `none`, `range`, `range_why`: what the plugin made of it (the payload below).

The hide row (`view_hide_sequences`) carries `hidden_tracks`, the keys of the page tracks that are hidden. Both rows stay rate limited
(the first call, then one a second, the newest of a burst is kept) and are written by the worker, never by the callback.

What I could not see: what a real DAW sends for a click on a clip (regions only?), for a drag in the ruler (a range only?), or for a
change of the track order. The test host sends every combination, so the plugin handles each. The first rows of a real run, with
these fields, answer the question; they are the evidence for the hand check in section 8.

## 3. What the role models do

The ticket names Melodyne and Auto-Align 2. I did not run them for this note. What the ARA header allows an editor to do with this call
(section 1) is the list I followed: show the selected track, scroll to the selected range, filter by the selection. The hand check in
section 8 compares the plugin with the editor of the DAW owner's choice. Where the owner finds a difference, the table in section 4 is
the place to change.

## 4. The rules

The plugin turns the host's calls into one payload and sends it to the page as the event `plugin-selection`. The rules are pure
functions in `plugins/ara/src/selection.rs`; the page side is `static/hostsel.js`.

| Case | Rule |
|---|---|
| One region sequence, or one playback region, selected | The track of the page whose source has a region on that sequence (or that region) is the selected track of the page (`setSelTrack`). The page scrolls its row into view. |
| Several tracks | The tracks come in the order of the DAW (`orderIndex` of the region sequence; a sequence with no order after the others), then in the order of the page. The first one that is shown (not hidden, see below) is selected; when every selected track is hidden the selection of the page does not change. The others get a mark (a dashed outline and a dot after the name), which a later selection takes away. The order of the list the host sent does not matter. |
| A DAW track with no source of the plugin | The page changes nothing: no selection, no marks, no range, no scroll. The plugin still sends the event (with `none: true`), so a test can see that the page got it and ignored it. The range of such a selection is not used either. |
| Time range | The range of the host is one or more bands over the ruler and the lanes (`#hostRange`, one `.hostBand` for each part, teal). It is not the hit selection of the page (`LN.sel`, `S.sel`): nothing in the page treats it as a selection, Delete and the nudge keys do not see it. |
| Mapping | The song time of the range is mapped to the source time of the page through the playback regions of the first selected track, with the regions of the export (`deliver::ExportRegion`: the slice of the source, the place in the song and the length in the song). A region keeps the part of the range that lies in it and maps it linearly with its length in the song, so a region that the host stretches is right (the unit test has half and double length). The answer is one band for each part: two regions in one range are two bands and the gap between them is not drawn; parts that touch in the source are one band. A warp inside a region (non-linear stretch) is not handled. |
| Range outside every region | The band is not drawn and the range of an earlier selection is removed. The page shows no text (a range in a part of the song where this track has no audio is not an error). The report row says `range_why: outside`. |
| Empty range, no range, empty selection | The band is removed. The selected track stays. A range of no length, or a start that is not a number, is empty. |
| A range and no track | The range is mapped through the first track of the last selection that named a track of the plugin; with none yet, through the first track of the page. |
| Range out of view | The page moves its view (keeps the zoom) so that the first band is in the middle; a range longer than 90 percent of the view starts at the left. A range with a band that is partly in view leaves the view as it is: the user sees it already. |
| Hidden tracks | A page track is hidden when every region sequence that holds one of its regions is hidden. Its row is hidden. One short text in the status bar names how many ("2 tracks hidden in the DAW"), and nothing when none is hidden. A track with a region on no sequence is never hidden. The list of a call replaces the list of the call before. The hidden track keeps its hits and is still analysed and exported: hiding is display only, in the page as in the DAW. The page never selects a hidden track; the keys that move the track (`P` and `;`), Tab over the hits, and the review queue skip it; a hit that was selected on a track the DAW then hides is deselected, so no key acts on a hit nobody can see. |
| Follow off | View > Follow the DAW Selection in the page menu (host mode only). Off: the marks, the band and the hidden rows go, nothing follows, the selected track stays. On: the page takes the last selection of the host. The choice lasts for the page (it is kept in `localStorage` when the web view has one; the plugin view has none that survives, see M1b2). |
| A selection the user makes | While a pointer button or a key is down in the page, and for 300 ms after a key, the selection of the host waits. When the action ends the page takes it, unless the user changed the selection of the page during the action; then the host's selection is dropped. The hidden rows are applied in both cases, they are not a selection of the user. A host selection that was dropped counts as seen: a later load of the project does not select it over the user's choice. A text field that has the focus counts as an action: the host's selection waits until the field is left (focus out), under the same rule. An action that never ends (the window lost the up event) counts for 20 s. |
| Page opens, project loads again | The shell script of the page asks the plugin for the state once the project is loaded (`plugin_host_selection`), because an event from before the editor opened is not kept. After every load of the project the marks, the band and the hidden rows are made again; a load does not select again. A message older than the last one is ignored. |
| The other direction | A click on a track in the page does not change the selection of the DAW. ARA has no call for it (section 1). The page changes itself only. This is also why the two can differ after a click: the DAW stays as the user left it. |
| A double click on a hit | Moves the DAW playhead. This is #554 and is not changed here; the stages `editor-playback` and `editor-hit` of the check still run it. |

The page code runs only in host mode. With the mode off `hostSelInit()` returns at once, the page has no band, no text and no menu
item, and `drawHead` calls `hostSelPaint()`, which returns at once. The page test `test_standalone_page_has_nothing_of_the_follow`
checks it.

## 5. Threads and locks

The callbacks run on the main thread of the host. The compare of the new payload with the last one, its number, and the reading of the tracks and regions it is made from are under ONE lock (`selection_step`), so a callback of the host and the end of a round cannot leave the older payload in place of the newer. The lock is not held across the page message or the log call. The numbers and the facts are read once for each call; during an edit cycle of the host only a mark is set and the payload is computed at its end (the rule of #558). A burst of changes (a drag in the ruler of the DAW) reaches the page as at most one message in 50 ms, always with the newest state (a short thread sends the last one), and writes at most one `view_applied` row a second (the newest is kept).

The callbacks They copy the ids and the range, change two small in-memory states under short
locks, push one event into the in-memory log of the page, and offer the report row to the rate limiter. They write no file (the
worker writes the row), wait for no lock that is held across a call of the host, and make no call of the host.

A playback region of the host is named by a ref that is a pointer to the plugin's own record. The record is freed in
`destroyPlaybackRegion`, so a selection that names a destroyed region must not read it: the plugin keeps the set of the live refs and
reads only a ref that is in it. A region sequence is named by a counter, never a pointer.

## 6. Where it is

| Part | File |
|---|---|
| The rules and their unit tests | `plugins/ara/src/selection.rs` |
| The callbacks, the state, the event, the pull | `plugins/ara/src/lib.rs` (`ev_notifySelection`, `ev_notifyHideRegionSequences`, `DocShared::selection_*`) |
| The shell command `plugin_host_selection` | `plugins/ara/src/editor.rs` |
| The page | `static/hostsel.js`, a line in `drawHead` (`app.js`), `buildTrackDOM` (`tracks.js`), `measureGeo` (`render.js`), the menu item (`hostmenu.js`), the styles (`style.css`) |
| The pull of the first state | `plugins/ara/src/editor/shell.js` |

The mapping of the range of the host (song time) to the time of the page is in ONE function: `selection::range_to_source` in
`plugins/ara/src/selection.rs`, called from `selection::payload` only. The page maps nothing: the payload is already in the source
time of the first selected track, and `hostSelRange` in `static/hostsel.js` draws it as it is. Ticket #555 (the song-time timeline)
replaces `range_to_source`; the page stays as it is, or, if the page gets song time, `hostSelRange` is the one place to map.

`measureGeo` skips a hidden row: it has no size, and reading it would have set the lane heights of every other row to 0.

## 7. Proof

- Unit tests (`cargo test`, `selection.rs` and `lib.rs`): the rules of section 4 one by one, the event once per change, the same payload twice.
- Page tests (`tests/e2e_app/test_host_selection.py`, Chromium, host mode): scroll into view, several tracks, a DAW track with no source, the
  band and its place, the view, the hidden rows and their text, the menu switch, the pointer, the key and the user's own selection, an older
  message, a load of the project, no call to the host from a click, the standalone page.
- Plugin stage `editor-selection` (`plugins/ara/check.py`): the test host sends a selection and a hide list before the editor opens (the page must take them from the plugin), then 17 steps through the ARA editor view (selections
  and hide lists) and the page in the real web view reports what it shows after each. The pointer and the key are down in the page for
  steps 10 to 12 and the menu item is clicked for steps 13 and 14. Fifteen parts, each with a fault run (`editor-selection-fault`) that
  fails for the named reason. The faults are switches of the plugin (`DS_ARA_SELECTION_FAULT`) and of the page driver (`DS_ARA_EDITOR_FAULT`,
  values `sel-*`); a delivery build reads none.

What the stage does not prove: that a real DAW sends these calls in these combinations, that the band sits on the right bar of the
DAW's timeline (the song-time timeline of #555 does not exist yet; the mapping is through the regions, section 4), and that the
selection looks right next to the editors of the role models.

## 7a. What the report holds

The row `view_selection` carries the names of the selected DAW tracks (`sequence_names`) and the keys of the page tracks. Track names are
the names of the user's tracks, so the row is private data of the user, like the row `track_names`: it is written only when the report
switch (`DS_ARA_OUT`) is set, else it stays in the small in-memory ring and leaves with the process. Nothing sends it anywhere, and no
test data with real names goes into git. The rows are limited to one a second (`view_selection`, `view_hide_sequences`, `view_applied`).

The cache version of the page scripts (`?v=` in `index.html`) is not bumped: the earlier host-mode tickets (#559, #560) changed `app.js`
and others without a bump, and the plugin serves the page from its own bundle, so no old copy exists to refresh.

## 8. Hand check in a real DAW

1. Open a session with at least three tracks that the plugin analysed. Open the plugin window.
2. Click a track name in the DAW. Expect: the same track is selected in the page and its row is inside the lanes area.
3. Select two tracks in the DAW (Shift+click, or Cmd+click). Expect: the track that is higher in the DAW is selected, the other has a
   dashed outline.
4. Select a track that has no clip of the plugin (an empty track, or a track of another instrument). Expect: nothing in the page changes.
5. Drag a time range in the ruler or over a clip. Expect: a teal band over the ruler and the lanes at the same bars. If the range is
   outside the part of the song where the selected track has audio, expect no band. Select nothing: the band goes.
6. Zoom the page so that the range is out of view, then select the range again. Expect: the page scrolls to it.
7. Hide two tracks in the DAW. Expect: their rows go and the status bar says "2 tracks hidden in the DAW". Show them: the text goes.
8. Open the page menu (the trigram), View > Follow the DAW Selection. Switch it off, select another track in the DAW. Expect: the page
   does not follow. Switch it on: the page takes the last selection.
9. Press the mouse button in the page and keep it down while you select a track in the DAW. Expect: the page does not change until you
   let go. Do the same with a key.
10. Click another track in the page. Expect: the selection of the DAW does not change. Double click a hit: the DAW playhead moves.
11. Look at the report rows `view_selection` and `view_hide_sequences` of this run. Send me what a click on a clip, a drag in the ruler and a
    track move send (the counts and the new fields). That answers the open question in section 2.

## 9. Open points

- What a real DAW sends for each user gesture (section 2). If it sends a region sequence with every range, the rule "a range and no track"
  is not used; if it sends regions only, the track rule is the one that matters.
- Several tracks: the first in the order of the DAW is selected. The other choice is the one the user clicked last, which ARA does not tell.
- Warps inside a stretched region (a non-linear stretch) are not mapped; only the linear length of the region is.
- The song-time timeline of #555 will replace the mapping through the regions of one track; the payload keeps the range in the source time
  of the first selected track, so the page side stays as it is.
