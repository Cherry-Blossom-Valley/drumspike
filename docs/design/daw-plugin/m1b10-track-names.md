# M1b10: the tracks show the names of the DAW tracks

Ticket #558, umbrella #503 (the DAW plugin program), part of M1 (#510). Found in the first
complete run in LUNA with the build of #554. The plugin lives in `plugins/ara/`. Nothing in
`static/` or `crates/` changed (see "What the page must change").

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

## What the page gets, and what the page must change

The plugin adds one key, `display_name`, to every track of the answers of `tracks_get` and
`project_get`. It never changes another key. The standalone app does not have it.

The page does not read `display_name` yet. The scope of this ticket is `plugins/ara/` and the
docs, so `static/` is not changed here. The exact need:

- `static/tracks.js`, in `renderTracks`, the line that writes the name into the header
  (`nm.textContent = name; nm.title = name;`): write `t.display_name || name`, keep the key in
  `row.dataset` and in every call.
- `static/inspector.js`, `insFill`: the name field (`nm.value = name`) shows
  `t.display_name || name` and stays read-only in host mode.
- Any other place that shows a track name to the user (the mixer rows are absent in host mode) must
  do the same. No place may use the display name as a key.

The stage `editor-names` therefore checks the answer that the page gets (`tracks_get`), not the
text the page draws. It prints the text that the header shows at the moment: the key.

## The role follows the DAW name (item 8)

The owner found all tracks grey with the role `other` and the note C1 (36) in the DAW: the plugin
guessed the role from the name of the audio source, 32 hex characters in LUNA. The guess now reads,
in this order (`service.rs`, `pick_role`):

1. the DAW name of the track: the name of the region sequence of the first playback region, else the
   name of that region (the rule of item 1 without the numbers),
2. the name of the audio source (as before),
3. the audio, as the scan of the app does for a name that gives no role: `role_from_audio` of
   `drumspike-core` (the hint mode, only a role with a confidence bar comes back, the first 30 s, no
   suggestion for a Drum Spike export name). The app uses it in `tracks_scan` and takes the hint for
   the new track; the plugin has no scan, so it takes the hint at the make of the track,
4. `other`.

The note is the default note of the role. The track gets `role_source` `guess` (a name) or `audio`,
and `role_match` with the matched words, as a track of the app does.

**"Still the automatic guess" is `role_source`.** This is the field of the app. The app sets it to
`user` when the user sets the role in the page (`track_set_role`); every other value, and an absent
field, is a guess. The plugin stores nothing of its own, so a hand-set role survives a restore of the
session too. The note: it follows the new role only when it is still the default note of the old
role. A note that the user changed by hand stays. (The app does not mark a hand-set note, so a note
that the user set to the default note of the old role is taken as automatic: this is the one case the
plugin cannot tell.)

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
