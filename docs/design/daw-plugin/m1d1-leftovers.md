# M1d1: the page in a DAW shows no standalone leftovers

Ticket #575, umbrella #503 (the DAW plugin program), part of M1 (#510). It also covers the older ticket #569 (every place of the page
shows the DAW track name). The page of the plugin is the page of the standalone app. A UX audit of the host mode found places that make
sense only in the standalone app: machine names of tracks, a tempo that cannot be edited, a save dot, a development note, a manual for
the wrong product. This note gives the rule for each line, the list of the places that showed the machine key, the proof, and the hand
check. The owner rule is: the standalone app stays unchanged. Every new path of the page is behind the host-mode switch (`hostMode()`),
and the page tests show the standalone page as it was.

What changed outside `static/`: `plugins/ara/` (one reason in the page event of a round, the test host, the stage), this note and the
tests. Nothing in `crates/`.

## 1. The rules, line by line

### Line 1: DAW track names everywhere

One helper, `trackLabel(key)` in `static/state.js`. It returns the display name of the track (`display_name`, #558) and else the key.
Only the plugin sets `display_name`, so the standalone page returns the key everywhere, as before. A second helper, `trackText(text)`,
is for a text that already holds keys (an answer of the service, such as "Snare Top: 12 hits ..."). In host mode it replaces each key
whose label is another name with the label, in one pass. A label that is in the text already stays (the longest name wins), so a name is
never changed twice. With the mode off it returns the text as it is.

The rule for a new text: a track that the user reads goes through `trackLabel`. The key goes only into a command, a data attribute or
a lookup. As a net under the sites that were changed one by one, the status line (`setMsgLvl` in `static/status.js`), its detail
(`setMsgDetail`) and the progress text pass through `trackText`.

The places that showed the machine key before this change (found with a search of `static/` and the audit; the list is not a proof
that no other place exists, the net above catches a status text of a place that was missed):

| Place | File | Now |
|---|---|---|
| progress text of a running analysis ("Analyzing 2/4: key: stage") | `analysis.js` `pollJob` | label |
| the text of an analysis that failed, and the tooltip of the detector field (`showDetector`) | `analysis.js` | label |
| the Re-analyze dialog (a range and the list of tracks with edited hits) | `analysis.js` | label |
| the info line (field Track) and the status bar (`stSel`, "key: hit 3/40", "key: moving") | `status.js`, `tracks.js` | label |
| the status text of a velocity gesture | `velocity.js` | label |
| the export dialog: "Selected track (1): key", the list of skipped tracks, the tooltip of the sampler offset, the result of an export with a failed track | `export.js` | label |
| Track > Follow Hits Of in the page menu (one entry for each other track) | `hostmenu.js` | label |
| the follow selector of the Inspector, the text "follows key" in the Inspector and in the header of a follower | `inspector.js`, `tracks.js` | label |
| the status texts of a follower ("key follows key2: ...", "own detection"), a shift, a role change, Learn, review, audition and layer texts | `tracks.js`, `inspector.js` and others | label, through the net of `setMsg` and, for the first two, in the source |
| the labels of the undo steps and the text of Edit > Undo and Redo ("Delete Hits (key, 3 hits)", "Set Role (key)", where the step names its track) | `edit.js` | label |
| the header of a track (text and tooltip) | `tracks.js` | label, through the helper now (it was `display_name || name` in the source, #558) |
| the name field of the Inspector | `inspector.js` | label (#558, unchanged) |
| the native menu of the standalone app (Follow Hits Of) | `shell.js` `menuState` | key, unchanged: the native menu does not exist in a DAW, and the standalone page has no label |

Not changed on purpose: the text of an error that the service sends is changed only by the net, so an error from the service shows the
DAW name when the key is in the text and the key is a track key. A key that is also an ordinary word of the text is changed too (a
display name that is a number of another track, such as "Snare (2)" for the key "Snare", is handled by the longest-name rule).

### Line 2: the tempo is read-only in a DAW

In a DAW the host owns the tempo. The page draws the map of the host (`tmLoad`). Before this change a typed tempo, a typed meter, Set
Bar 1 and the bar-1 drag wrote the map of the service; the page then loaded the map of the host again, so the edit looked lost.

Rule: `tmHostLocked()` (`static/tempomap.js`). In host mode it stops every edit of the tempo map and sets one status text. It stops: the
fields of the tempo, meter and bar-1 rulers (`tmEditStart`), the drag of a tempo step, the drag and the double click of the bar-1
marker, Delete on a selected step, Set Bar 1 at Playhead and at Audio Start (`tmSetBar1`), and every commit (`tmCommit`, the net under
all paths). In the page menu the two Set Bar 1 items are greyed. The markers ruler is not part of the tempo and works as before.

What happens when the DAW gives no tempo: the page has no map (`TM.map` is empty). The tempo rulers draw nothing, no field opens, and
the status text says "The DAW gives no tempo: the rulers stay empty and the MIDI file has the tempo the export dialog names." The
MIDI file then gets one tempo from the process call of the host, else 120 BPM, and the export dialog says which (#560). The page does
not make a tempo up and does not let the user type one: a tempo that the DAW does not know would be wrong in the DAW anyway.

### Line 3: no "unsaved changes" dot

The DAW session is the only save. `setDirtyState` (`static/state.js`) keeps `S.dirty` as it is, but in host mode it hides the dot
and does not put the mark in the window title. With the mode off the dot and the title are as before.

### Line 4: the export dialog

In host mode:

- the development note ("Spike audio types (Audio, Both) come in a later part") is gone;
- the note says where to drop the files: "Files start at song position 0: drop them at the start of the session in your DAW";
- the destination row (the folder, Choose, Reset) is collapsed. One button ("Write to a folder instead...") opens it. A folder that was
  chosen, or a note about the folder, keeps it open. A new dialog starts collapsed again.

With the mode off the button is hidden and the row is shown as before (`exDestFold` in `static/export.js`; the class `exFold` is set
only in host mode).

Two assertions of the existing test `test_host_page.py` (`test_the_export_dialog_of_the_host_mode...`) pinned the old dialog: the text
"later part" in the note and the visibility of the button Choose. Both are the lines that this ticket changes. I changed these two
assertions (the first now says the text is gone) and moved the new proof to `test_host_leftovers.py`. No other expected value of an
existing test changed. This is the one place where I read the ticket and the rule "no expected value changes" against each other.

### Line 5: help and the first-open line

The help of the page in a DAW (section "In a DAW", `static/help.js`) starts with a path of five steps:

1. Put the plugin on the drum clips of the DAW.
2. Wait for the analysis. The page says when the hits are ready.
3. Check the hits and correct them.
4. Set the note and the role of each track in the Inspector.
5. Drag or export the MIDI and the spikes into the DAW. Drop them at the start of the session.

The list of what the DAW owns follows. The first open shows one line in the status bar: "Step 1 of 5: put the plugin on the drum clips.
Help shows all five steps." It goes away at the first click or key in the page (the capture listeners of `hostIntroInit`). The service
setting `host_intro_done` keeps that it went; the settings are inside the document folder in the plugin, so the line shows once for
each document. A key of the settings needs no change of the service (the key is `[a-z_]`).

### Line 6: a reload that the DAW caused

When the host replaces the samples of a source (`updateAudioSourceContent`), the plugin reads the new audio and analyses it again. The
page reloads (`plugin-reload`). Before this change the reload was silent. Now:

- the round names the tracks whose changed audio it analysed again (the source had the flag `reanalyse` and the round read the new
  generation). The page event carries them as `audio` (the keys), and the report row `job` carries `audio_changed`. A reload for
  another reason (a first analysis, a name, a restore) has an empty list;
- the shell script of the page (`editor/shell.js`) keeps the keys until the page has loaded the project again, then calls
  `hostAudioChanged(keys)`. A burst of events gives one message;
- the page writes one status text: "The DAW changed the audio of <DAW names>. The hits were detected again. The hits you added, moved
  or edited are kept." More than three tracks: "<three names> and N more tracks".

What is kept is what the plugin keeps (#538): the hits that the user added, moved or edited, and the settings of the tracks. A hit that
the user deleted can come back after a round (open point of #538, see the README). The text says "added, moved or edited" for this
reason, not "all hand edits". The ticket line says "hand edits are kept"; the words are the same but the text does not promise the
deleted hits.

## 2. What the tests prove

Page tests, `tests/e2e_app/test_host_leftovers.py`, each line with the mode on (`host_app`, the page as the plugin shell serves it)
and with the mode off (`app`, the standalone page):

| Line | Mode on | Mode off |
|---|---|---|
| 1 | the progress, info line, status bar, undo label, follower text, status text from the service, export dialog, Follow menu, Inspector selector and the tooltip show the DAW name; no text holds a key; `trackText` rules | the same calls show the keys; the native menu state still lists the keys |
| 2 | Set Bar 1 (both), the three typed fields, the double click on the tempo ruler, the double click and the drag of the bar-1 marker change nothing and send no tempo command; the menu items are greyed; a DAW with no tempo says so | a tempo edit works and sends the command; the field opens |
| 3 | the dot and the title mark stay hidden; `S.dirty` is true | the dot and the mark show |
| 4 | no development note, the drop hint, the destination row collapsed, one click opens it, a new dialog is collapsed, a chosen folder keeps it open | the row, the label and Choose show; the toggle and the note are hidden |
| 5 | the five steps; the first line shows, goes at a key and at a click, is kept, does not come back | no help path, no line |
| 6 | the message for two tracks, for many tracks, and no message for an unknown key or an empty list | no message |

Plugin side, `plugins/ara/check.py`, stage `editor-leftovers` (the real plugin in a real web view, the DAW names from the test host):

| Part | What it proves | Fault run |
|---|---|---|
| `editor-leftovers-names` | the page shows the DAW names in the progress, the info line, the status bar, the undo label, the follower text, the status text, the export dialog and the Follow menu, and no text holds the key of a track | `left-key` (`DS_ARA_EDITOR_FAULT`): the label helper returns the key; fails with "holds the key" |
| `editor-leftovers-page` | the dot is hidden, the export note has the drop hint and no development note, the destination row is collapsed | none (the page tests prove it with a mutation of the page) |
| `editor-leftovers-audio` | after the host replaced the audio of one source, the round names that track (`audio_changed`) and the page shows the message with the DAW name | `no-audio-reason` (`DS_ARA_LEFTOVER_FAULT`): fails with "no audio message" |

A run with no page report fails with "no usable run" (`editor-leftovers-fault-norun`).

## 3. What is not proved

- That any of this looks right in a real DAW. The stage runs the page in a web view of a test host, not in a DAW.
- Lines 2 to 5 have no fault run in the plugin stage. They are page-only and have page tests; the stage reads lines 3 and 4 as a smoke
  check.
- The tempo of a real DAW through ARA: the page tests give the map through the host transport stub.
- The key can still show in a text that the service builds from a key that the page does not know (a track that is not in the page yet).
- The help text and the first-open line are not measured with a user.

## 4. Hand check in a real DAW

1. Open a session with drum tracks that have names in the DAW. Open the plugin window. Expect: one line in the status bar names step 1.
   Click in the page. Expect: the line is gone. Close and open the window of the same session: the line does not come back.
2. Look at the track headers, select a hit (the info line), start Analyze All (the progress text), select a track and open Track >
   Follow Hits Of in the page menu, press Delete on a hit and open Edit. Expect: every text names the DAW track, none shows 32 hex
   characters or the name of the audio file.
3. Open Export. Expect: no text about a later part, one line that says to drop the files at the start of the session, the destination
   row closed. Click "Write to a folder instead...": the folder, Choose and Reset show.
4. Open the tempo ruler (View > Tempo Ruler). Double click it, double click and drag the bar-1 marker, use Track > Set Bar 1 at Playhead
   (greyed). Expect: nothing changes, the status line says that the DAW gives the tempo.
5. Edit a hit. Expect: no dot next to the logo, no mark in the title.
6. Open Help. Expect: five steps first.
7. Change the audio of one clip in the DAW (a destructive edit, a bounce in place). Expect: after a moment the status line names the
   DAW track and says that the hits were detected again and the hits you edited are kept.
8. Look at the report rows `job`: the field `audio_changed` names the track of step 7 and no other.
