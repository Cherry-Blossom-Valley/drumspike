# M1b10: the tracks show the names of the DAW tracks

Ticket #558, umbrella #503 (the DAW plugin program), part of M1 (#510). Found in the first
complete run in LUNA with the build of #554. The plugin lives in `plugins/ara/`. Nothing in
`crates/` changed. In `static/` only the two lines named in "What the page shows" changed.

## The problem

LUNA gives each audio source a machine name of 32 hex characters. The plugin made the track name
from the name of the audio source, so the page showed 32 hex characters as track names. The user
knows the track by the name in the DAW. The plugin ignored the two properties that hold it: the
name of the region sequence (the DAW track) and the name of the playback region.

## The rule

The page shows a display name. The display name of a track is the first one that exists, in this
order:

1. the name of the region sequence of the first playback region of the track,
2. the name of that playback region,
3. the name of the audio source (the track name as it was before this change).

The first playback region is the one with the lowest start in song time (`startInPlaybackTime`).
The region that the host made first wins a tie. An empty name or a name of white space is no
name; control characters leave the name and it is cut to 120 characters. A source with no playback
region has the third name, as before.

Two tracks with one display name (compared without case) show a difference: the first source (by
creation order) keeps the name, the next ones get a number, `Toms (2)`, `Toms (3)`. A numbered
name that another track already shows is skipped.

What the role models show: the ticket names Melodyne and Auto-Align 2 as editors that show the
name of the DAW track. This part did not run them; the rule above follows the ticket and the ARA
documentation of the two properties: the header of ARA 2 says the name of a region sequence is the
name the host displays, and it asks the host not to make up a name for a playback region (a missing
name means the host has none).

## The identity does not change

The display name is a label. The key of a track is the track name, made from the name of the
audio source as before. The key names the hits of the track, its files and its entry in the ARA
archive. The plugin writes no host name into the archive and no host name into the project. The
fault run `DS_ARA_NAMES_FAULT=key` lets the key follow the DAW name and the stage fails.

## Where the plugin keeps the names

- `createRegionSequence` gives the host a counter as the ref (never a pointer) and keeps the name in
  the document of the controller. `updateRegionSequenceProperties` replaces the name.
  `destroyRegionSequence` removes it. A region that points to a sequence that is gone falls back to
  its own name.
- A playback region keeps its sequence id and its name. `createPlaybackRegion` and
  `updatePlaybackRegionProperties` read them, only when the declared `structSize` covers the field
  (an old host may leave them out). A move of a region to another DAW track is an update with
  another `regionSequenceRef`.
- `names.rs` holds the rule as a pure function over plain data. A unit test reaches every case.

## A rename and a move: no round

A rename of a DAW track or a move of a region changes no sample and no region time, so the hits are
the same. The update does not start an analysis round (the move in playback time alone already
started none). The plugin compares the display names that differ from the track name with those
that the page was told last. When they differ it writes one report row `track_names` and pushes the
event `plugin-reload`; the shell script of the page then loads the tracks again. A document with no
host names makes no row and no reload.

## What the page gets, and what the page shows

The plugin adds one key, `display_name`, to every track of the answers of `tracks_get` and
`project_get`. It never changes another key. The standalone app does not have the key.

The page shows it (the coordinator accepted this change of `static/` for this ticket):

- `static/tracks.js`, the track header: text and title are `t.display_name || name`.
- `static/inspector.js`, the name field: the value is `t.display_name || name`. The field keeps the key
  in `dataset.name` and the shown text in `dataset.shown`; a blur renames the track only when the
  text differs from the shown text, so a blur that changed nothing never sends the display name as a
  new key. Select, rename, analyse and every data attribute use the key.

Without `display_name` (the standalone app) the page is as before; the existing page tests keep
their expected values. `tests/e2e_app/test_display_name.py` proves the other side: with a
`display_name` the header and the Inspector show it, and select, rename and analyse send the key.
The stage `editor-names` reads the text that the header draws (part `editor-names-header`).

## The role follows the DAW name (item 8)

The owner found all tracks grey with the role `other` and the note C1 (36) in the DAW: the plugin
guessed the role from the name of the audio source, 32 hex characters in LUNA. The guess now reads,
in this order (`service.rs`, `pick_role`):

1. the DAW name of the track: the name of the region sequence of the first playback region, else the
   name of that region (the rule of item 1 without the numbers),
2. the name of the audio source (as before),
3. the audio, as the scan of the app does for a name that gives no role: `role_from_audio` of
   `drumspike-core` (the hint mode, only a role with a confidence bar comes back, the first 30 s, no
   suggestion for a Drum Spike export name). It runs only when neither the DAW name nor the name of
   the source gives a role, and only when the audio of the source is here (a unit test proves that a
   name that gives a role never asks the audio). It runs once, when the track is made; a later new DAW
   name never asks the audio. The app uses it in `tracks_scan` and takes the hint for
   the new track; the plugin has no scan, so it takes the hint at the make of the track,
4. `other`.

The note is the default note of the role. The track gets `role_source` `guess` (a name) or `audio`,
and `role_match` with the matched words, as a track of the app does.

**"Still the automatic guess" is `role_source`.** This is the field of the app. The app sets it to
`user` when the user sets the role in the page (`track_set_role`); every other value, and an absent
field, is a guess. The plugin stores nothing of its own. (A restore of the session writes the project document
again from the archive and guesses the roles again, as it did before M1b10: the archive holds no role.
This part does not change it, so a role set by hand does not survive a restore of the session; the
guess then reads the DAW names that the host has made by then.) The note: it follows the new role only when it is still the default note of the old
role. A note that the user changed by hand stays. (The app does not mark a hand-set note, so a note
that the user set to the default note of the old role is taken as automatic.) Known limit, the same
kind: a role that the user set by hand to the value that was already guessed is not protected. The
page writes nothing in that case (the app keeps `role_source` as it was when the role does not change), so
the plugin cannot tell it from the guess. A track from an archive or project of before this change has no
`role_source` and counts as automatic; that is wanted, because its role came from a hex name. Such a track
on a DAW track named as a snare gets the snare role once, keeps every hit (also hand-edited ones), and its
note follows only if it was the default note (unit test).

**A new DAW name.** The snapshot of a source carries the role that its DAW name gives. A name that
gives another role makes the source changed (`same_as`), and the plugin starts a round after the
settle time, like a change of a region. A rename that keeps the role (Kick In to Kick Inside) changes
nothing and starts no round. In that round the track record is made again with everything the user
set (the same path as a region change). If the role is not `user` and the new name gives another role,
the record gets the new role, its `role_source` `guess` and, when the note was the default, the new
default note. The round analyses the tracks together, as every round does, and the role steers the
detector of that track. When the new name gives no role, the role stays. The identity of the track
(its key) is not touched.

The page is told through the same `plugin-reload` as for a name.

**Cost and bounds (review round 1).**
- The role is computed only when a track is made new (`make_track`); a remake of the record for a region
  change takes only the layout (`track_layout`). The audio is read by `wav::read_head_mono`: the header
  and the first 30 s of frames, never the whole file (unit test with a 120 s file: a quarter of it is
  read). The `job` row names it in `role_audio` (track, bytes read, role).
- A new DAW name that gives another role but whose track has a role of the user, or already has that
  role, makes no change: the worker reads the record with `tracks_get` and keeps the snapshot, so the
  touch ends in a round that does nothing. "Kick" to "Mystery" to "Kick" is no change; "Kick" to
  "Mystery" to "Snare" is one.
- A rename storm marks a source once until a round has taken its snapshot (`role_touch_pending`), so the
  settle time of the worker makes one round of it.
- In the callbacks of the host the names are only marked; they are computed once when the outer edit
  cycle (`beginEditing`, `endEditing`) ends, or at once for a host without an edit cycle. 500 regions
  made in one cycle give one computation (unit test). One `track_names` row per real change.
- Numbering follows the DAW track order: the `orderIndex` of the region sequence of the first region,
  then the creation index of the source. The order in which the host made its objects does not change
  which of two equal names gets the number (unit test with both orders).
- The `state_call` rows are written with `log_deferred`; the worker (or the unload of the library)
  writes the file, so a state call of the host never waits for the report lock.

**Where the page still shows the key in a DAW** (not changed here; the coordinator writes a page ticket;
the list is from a search of `static/`, not exhaustive): the status line texts of an edit or a drag
(`tracks.js` "moving", `velocity.js` "Set Velocity"), the text " follows <key>" on a follower, the
follow selector and the track pick lists, the export dialog (the track list and the "Skipped" text),
the Re-analyze and Examples (Learn) texts of the Inspector, the labels of the undo steps (`edit.js`),
the context menu texts of a track, the tooltips that build a text from the key, and the mixer rows (the
mixer is absent in host mode).

Stages `editor-roles` and `editor-roles-fault`: four sources with hex names on DAW tracks named like
a kick, a snare, a hi-hat and with no role get the roles and notes; a rename of the last track to a
snare name changes its role with one new round; a role that the user set survives a rename. Fault
runs: `role-source`, `role-frozen`, `role-hand` (switch `DS_ARA_NAMES_FAULT`).

## The test host

The host makes region sequences (`--seq-names`, `--seq-of`) and region names (`--region-names`),
and in the editor mode `names` it renames a sequence (`--rename-seq <j>=<name>`) and moves a
region to another sequence (`--move-region <i>=<j>`) after the page reported. The update call of a
region carries its sequence and name again, as a real host does.

The stages `editor-names` (the CLAP and the plugin inside the VST3) and `editor-names-fault`:

| line | proof | fault run |
| --- | --- | --- |
| names | the display names of four tracks: the sequence name beats the region name, a sequence with no name gives the region name, two DAW tracks with one name get a number | `ignore`, `region-first`, `no-number` |
| rename | after a rename and a move the page gets the new names once the plugin said it reloaded | `no-rename`, `no-reload` |
| no round | one analysis job and one round in the whole run | `round` |
| identity | keys, events, notes before and after, and the stored archive are equal to a run with no names | `key` |

## "loadPreset failed" in LUNA

LUNA logs `VST3_PluginInstance <plugin> ::loadPreset failed` right after the user chooses the
plugin for a track. The measurements and the open point are in `hosts-luna.md`. This part adds
the report row `state_call`, so the next run in LUNA says whether LUNA calls `getState` or
`setState` of the plugin at all.
