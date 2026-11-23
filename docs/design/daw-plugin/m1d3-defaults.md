# M1d3: user defaults and presets of the plugin

Ticket #577, umbrella #503 (the DAW plugin program), part of M1 (#510), fourth part of M1d (the usability audit of 2026-11-22).
This note is the design (Gate line 1). The code places are those of the base commit `fec062b` (#576 merged). A fact that we did
not measure says so.

Goal: a user sets the MIDI map and the typical settings ONCE. Every new session and every new track starts right, as in the
role-model editors (`role-models.md`), and the state of the session always wins.

## 1. What exists today

| Item | Where it lives now | Code place |
|---|---|---|
| Role, note, detection params, velocity map, mix, Learn, layer, follow of a track | the project document of the session, and per track in the ARA archive (`ArchiveSource.settings`) | `service.rs` `SETTINGS`, `settings_of`, `overlay_settings`; `lib.rs` `dc_storeObjectsToArchive` |
| First values of a NEW track | `make_track_audited`: the role from `pick_role`, `note = default_note(role)` (36 kick, 38 snare, 48 tom, 42 hat, 49 cymbal, 36 other), the rest of the record empty | `service.rs`, called by `DocService::analyse` in the loop over `new_sources` |
| Detection params | `Track.params` (`thresholds`, `min_gap_ms`, `refine`, `vel_lo_db`, `vel_hi_db`, `vel_cut_db`, `shift_samples`); the page fills missing keys from `defaultParams(role)` only for display | `crates/core/src/project/document.rs`, `static/tracks.js` `defaultParams` |
| Velocity setting | the keys `vel_lo_db`, `vel_hi_db`, `vel_cut_db` of `params`; `velocity_map_of` reads them (defaults -40 and -6) at the first analysis | `crates/app/src/follow.rs`, `histogram.rs` `set_velocity_params` |
| Follow options | page settings: `followMode` (smooth, page, off), `ui.suspendFollowOnEdit`, and the host-only switch "Follow the DAW Selection" (`localStorage` key `dsHostFollowSel`). The per-track `follows` (a leader track) is session state, not a default | `static/state.js`, `commands.js`, `hostsel.js`, `hostmenu.js` `HM_EXTRA` |
| Export settings | `export_last` (`formats`, `midi_mode`, `compensate`, `scope`, `track`, `dest`) and `export_compensate` in the project document (`doc.extra`), per session, in the cache folder. NOT in the archive | `crates/app/src/shell.rs` `export_last_set`, `last_from_args` |
| Remembered export folder | `export-folders.json` next to `project/` in the document folder of the cache; only folders chosen in the plugin's sheet | `deliver.rs` `FOLDERS_FILE`, `load_folders`, `save_folders` |
| User-level files of the plugin | NONE. The plugin writes only below `~/Library/Caches/DrumSpike/ARA` (README "Cache folder"; stage `files`; stage `doc-home` counts files below `HOME` outside the cache: must be 0) | `lib.rs` `cache_default`, `check.py` `check_home` |
| App settings of the page (`settings.json`) | in the service home of the DOCUMENT, `documents/<id>/service/` (so per session, lost when the cache goes) | `crates/app/src/lifecycle.rs` `app_settings_get`, `service.rs` `home_of` |
| Kit presets | `<home>/kit-presets/<name>.json` of the same service home (per document in a plugin). The commands are in the page allow list but no page element calls them in a DAW | `crates/app/src/roles.rs` `kit_preset_*`, `editor.rs` `ALLOWED` |
| The menu of the page in a DAW | one button `#hostMenuBtn`; panel built from `DSKeys.COMMANDS` plus `HM_EXTRA` (host-only check items) | `static/hostmenu.js` |

Consequence: the MIDI map and the typical settings are set again in each session, per track, in the Inspector (audit 2026-11-22, section 3).

## 2. The three levels

1. Built-in defaults: the values of section 1 ("First values of a NEW track"). They are in the code and never change by a file.
2. User defaults of the plugin: ONE file at user level (section 4). New in this ticket.
3. Session state: the project document and the ARA archive of the session (exists).

Rules:

- Order: built-in < user defaults < session state. A value that the session has is never read from the defaults again.
- Defaults apply to NEW tracks only: a source that the document sees for the first time. An existing session opens exactly as it was
  saved, also when the defaults file changed, is damaged or is gone. Reading a changed file never changes a track that exists.
- Presets (section 7) change existing tracks only after a confirm and as one undo step. They are the only way to change many tracks
  at once.

### The exact point where defaults apply

`DocService::analyse` (`service.rs`), in the loop `for (s, name) in new_sources.iter().zip(&names)`, where it calls
`make_track_audited(s, &dir)`. That function sets `t.note = Some(default_note(role))` after `pick_role`. The change:

- A new function `make_new_track(s, dir, &RoleDefaults)` = `make_track_audited` plus the defaults of the role of the new track:
  `note` and `params` (and so the velocity setting). The record goes into `tracks_restore` with these values BEFORE the first round,
  so the first analysis already uses the thresholds and the velocity map. No new call of the service and no change in `crates/`.
- The worker takes ONE snapshot of the defaults at the start of `analyse` (`userdefaults::current()`), so all tracks of one round
  use one state of the file.
- `make_track` and `make_track_audited` stay as they are for `restore` (`DocService::restore`, `guessed()`), for the remake of a
  track record after a region change (`track_layout`) and for the tests. A restore NEVER reads the defaults.
- A "new track" is a source whose creation index is not in `self.tracks` AND whose key has no entry in `doc.settings` (the map that
  the archive restore fills). The second part matters for a source that the archive holds with no result (F5, `restore_no_result`
  in `lib.rs`): its settings wait in `doc.settings` until its first round, so it is not new. Open question 4 says what happens to
  those settings today.
- A role change later (the DAW name changes the role: `apply_label_role`) changes `role` and `note` only. It never applies the
  params of the new role (a track that exists). One fix is needed there: `note_is_default` compares the note with
  `default_note(current)`; with user defaults it must compare with the note that a NEW track of the old role would get now
  (user default, else built-in). If the file changed since, the note counts as set by the user and stays. This is the safe side.
- A track without a role (`other`, no guess, no audio hint) uses the key `other` of the file. If the file has no `other` entry,
  the built-in values apply (note 36, as today). The roles are the six of `roles.rs` `ROLES`: kick, snare, tom, hat, cymbal, other.
  The role is the one `pick_role` gives (DAW name first, #558). A rule on the name (tom 1 and tom 2 with two notes) is not part of
  this ticket (open question 6).

## 3. What is in the defaults

One JSON object, versioned:

```
{
  "v": 1,
  "roles": {
    "kick":  { "notes": [36], "params": { "thresholds": [0.22, 0.24, 0.32, 0.22, 0.30], "min_gap_ms": 20, "refine": true,
                                        "vel_lo_db": -40, "vel_hi_db": -6, "vel_cut_db": -50 } },
    "snare": { "notes": [38] }
  },
  "follow": { "mode": "smooth", "suspend_on_edit": false, "host_selection": true },
  "export": { "formats": ["midi", "spikes"], "midi_mode": "per_track", "compensate": true }
}
```

- `roles.<role>.notes`: list of 1 to 8 integers, each 0 to 127 (a single `note` in a file is read as a list of one). `roles.<role>.params`: only the keys of the table below; each key is optional.
  A role or a key that is absent means "built-in". A new track of a role takes the first note of the list that no other track of the same role in the document uses; when all are used, the last. Several new tracks of one round are served in DAW track order. The velocity setting is the three `vel_*` keys of `params` (one place, as the
  service has it); `shift_samples`, `weak_signal`, `click_freq` and every other key are NOT defaults (they belong to the audio of
  one track or to the monitor).
- `follow`: the page options of section 1 (`mode` is smooth, page or off). They are global, not per role. The leader of a
  track (`follows`) is a track name and has no default. The page reads them only where the document has no value of its own
  (the `settings.json` of the document and the `localStorage` key); see section 5.
- `export`: the shape for a NEW document (a document the plugin creates with no archive): `formats` (midi, spikes, audio),
  `midi_mode` (per_track or single), `compensate`. It seeds `export_last` once, at the first creation of the project of a
  document that is not restored (`DocService::ensure_open`, when `doc.json` does not exist). A restored document never reads it.
  `dest`, `scope` and `track` are never defaults (a folder is a choice of the user in the sheet, rule of #532).
- Unknown fields, at the top and inside a role, are kept when the plugin writes the file again (section 4).

Ranges (every value is checked on read, never trusted):

| Key | Valid | Else |
|---|---|---|
| `v` | integer 1 | see "version" in section 4 |
| role name | one of the six roles | entry kept as unknown data, not used |
| `notes` (or `note`) | list of 1 to 8 integers, each 0 to 127 (`note`: one integer) | field dropped |
| `thresholds` | list of 5 numbers, each 0.01 to 0.85 (`SENS.FLOOR`, `SENS.CEIL`) | field dropped |
| `min_gap_ms` | number 1 to 1000 | field dropped |
| `refine` | boolean | field dropped |
| `vel_lo_db`, `vel_hi_db`, `vel_cut_db` | finite number -96 to 0; `vel_lo_db` < `vel_hi_db`, else both dropped; `vel_cut_db` below `vel_hi_db` | field dropped |
| `follow.mode` | smooth, page, off | field dropped |
| `follow.suspend_on_edit`, `follow.host_selection` | boolean | field dropped |
| `export.formats` | non-empty list of midi, spikes, audio, no repeat | field dropped |
| `export.midi_mode` | per_track or single | field dropped |
| `export.compensate` | boolean | field dropped |

## 4. The defaults file

Path: `~/Library/Application Support/DrumSpike ARA/defaults.json`. This is the plugin's own folder. It is not `Drum Spike/` (the
detector folder of the app, `crates/app/src/plugins.rs`) and not `com.cbv.drumspike` (the app data folder, `agent.rs`), and it is
never a system plug-in folder. The plugin keeps no other user-level file today, so the path is new. Sibling: `Presets/` (section 7).

Contradiction with the README, to fix in the same change: the README says "The plugin writes only below one cache folder" and
stage `files` proves it. A cache folder can be cleaned by the system or by a cleaner tool, so user choices must not live there
(the audit proposed it; this note does not follow it). The README text and `files` change to: "the cache folder, and the folder of
the user defaults, and the latter only after an explicit Save". The plugin creates the folder on the first Save, never on a read,
so a run that saves nothing still writes 0 files outside the cache (stage `doc-home` stays true unchanged).

How a gate run is kept away from the real file:

- The path comes from ONE function `userdefaults::dir()`. In a delivery build it is always the path above (no variable is read:
  the rule of `test_switch`). In a gate build (cargo feature `test-switches`) it is the value of the new scenario switch
  `DS_ARA_USER_DIR`. In a gate build WITHOUT that switch `dir()` answers "none": nothing is read, nothing is written, and one
  report row `defaults_path_refused` says so. So a stage that forgets the switch cannot reach the real file.
- `check.py` `host_run` (the one function that starts every host process) sets `DS_ARA_USER_DIR=<out>/user-<label>` for every run
  and refuses to start a run whose environment has another value (a Python assertion, with a unit test).
- New stage `defaults-real-path`: a run WITHOUT the switch and with `HOME` below `out/`; it needs the row `defaults_path_refused`
  and 0 files and 0 folders named `DrumSpike ARA` below that `HOME`. Fault run `DS_ARA_DEFAULTS_FAULT=real-path` turns the guard off
  (the path resolves below `HOME`, which is below `out/` in this run, so the real home is never touched): the stage must FAIL because
  the refusal row is missing (a read makes no folder, so the folder cannot be the sign). Register `DS_ARA_USER_DIR` (kind scenario) and `DS_ARA_DEFAULTS_FAULT` (kind fault) in `switches.json`, so the
  start guard of `check.sh` fails when one of them is set in the caller's environment (an owner shell never carries them into a gate).
- The cargo unit tests build the path by an argument (`Defaults::at(dir)`), never from `HOME`. A unit test fails when
  `dir()` of a test build answers a path that starts with the real `HOME` (it reads `HOME` and compares).

Reading (`userdefaults::load`):

- One `symlink_metadata` call: a file that is not a plain file (a link, a folder) is "none". Size bound: 256 KB (six roles need
  about 1 KB); a bigger file is "none". One `read` of the whole file, then `serde_json::from_slice` (its depth limit is 128).
- `v` missing, not an integer, 0 or negative: the file is invalid, "none". `v` = 1: read. `v` above 1 (a newer plugin wrote it):
  read the fields that pass the ranges, and treat the file as READ-ONLY: a Save answers the error "The defaults file is from a newer
  version of the plugin. Update the plugin, or move the file." and writes nothing. This is the same rule as the archive (section 8):
  a newer version is not guessed, and is never damaged.
- Each field is checked alone. A bad field gives the built-in value for THAT field; the others stay. A bad file gives all built-in
  values. In both cases ONE report row `defaults_invalid` per load of a changed file (keyed by size and mtime, so a file that
  stays bad writes one row, not one per track) with the count and the first five paths (`roles.snare.note`). The page can ask for
  the same text once, in the status line of the menu panel: "The defaults file is damaged; built-in values are used."
- A defaults or preset file that is a symlink, a directory or unreadable gives built-in values and one report row; it is never followed and never a crash (ruling R8).
- A Save that fails (read-only folder, sandboxed host) shows `error: the defaults could not be saved: <reason>` in the status of the page (ruling R12).
- The plugin never stops because of this file. A read error, a missing folder and a permission error are "none" and one row.
- Cache: the parsed result is kept with `(mtime, size)`; each use compares them, so a Save in another host process takes effect on
  the next new track without a restart.

Writing (`userdefaults::save_role`, `reset_role`, `save_all`):

- Read the file now, change ONLY the named role (or `follow`, or `export`), keep every other field including unknown ones, then
  `drumspike_core::io::write_atomic(path, bytes)` (a temporary file with a name of its own in the same folder, then a rename; the
  core test `concurrent_write_atomic_never_shares_a_temporary_and_keeps_payloads_whole` proves it). Create the folder if needed
  (mode as the umask gives). Never write a file larger than the bound.
- If the existing file is damaged (not valid JSON or no valid `v`), copy it once to `defaults.bad.json` (replaced each time), then
  write the new content. The user's text is not lost.
- Concurrency: inside one process one `Mutex` covers read-modify-write. Two DAW processes, or two documents, that save at the same
  time: the read-modify-write window is a few milliseconds; the later rename wins ("last writer wins"), and a reader never sees a
  half file because the rename is atomic. The loser can lose its role entry in this window; we accept it (a lock file would stay
  behind after a crash and block every later Save). A stage proves "never a half file" (section 10).

## 5. The page: which of the three levels the page sees

New commands of the plugin shell (class Shell in `editor.rs`, in `SHELL_COMMANDS`; none of them exists in the standalone app,
whose Tauri commands and bridge never get these names). All answer the editor of one document; none takes a path:

| Command | Does |
|---|---|
| `plugin_defaults_get` | `{roles, follow, export, presets: [names], builtin: ["General MIDI"], status}` with the VALID values only, and `status` = ok, none or damaged (text) |
| `plugin_defaults_save_role {role, note, params}` | Save as default (section 6); `params` is filtered to the keys of section 3 |
| `plugin_defaults_reset_role {role}` | Reset to default (section 6) |
| `plugin_preset_get {name}` | the roles of a preset (built-in or file) |
| `plugin_preset_save {name, roles}` | write `Presets/<stem>.json`; errors say why (name, size, number of presets) |

The page asks `plugin_defaults_get` once at load in host mode. The follow options take the file only for a key that the document has
not set (no key in `settings.json` of the document, no `localStorage` value). The follow options are page state outside the archive;
a document that lost its cache starts again from the defaults file for them. We accept this (they are view options, not session data).

## 6. The Inspector (host mode only)

Two buttons in the Inspector, under the role and note line (`insRoleLine`, `insRoleControls` in `static/inspector.js`). They are made
by new code in a new file `static/hostdefaults.js` that returns at once when `hostMode()` is false, so the standalone page has no new
element, nothing focusable and no new text. Both act on the ROLE of the selected row, never on the track:

- "Save as default for new snare tracks": writes the MIDI notes of ALL tracks of that role in the document (DAW track order; for toms the text is "for new tom tracks: 50, 48, 47") and the detection settings (`params` filtered to section 3) of the
  selected row as the entry `snare` of the file. Tooltip: "New snare tracks in every session start with this MIDI note and these
  detection settings. Tracks that exist keep theirs." Status line after: "Saved: new snare tracks start with note 38 (D1) and these settings."
- "Reset to default for new snare tracks": removes the entry `snare` from the file. New snare tracks start with the built-in values
  again. Tooltip: "New snare tracks start with the built-in values again. Tracks that exist keep theirs." Status line after:
  "New snare tracks start with the built-in values."
- For a track with role `other` the word is "tracks with no role". The buttons are disabled with a reason in the tooltip when no
  row is selected, when the row follows another track ("its hits come from the leader"), when an analysis runs, or when the file is
  from a newer plugin.
- A failed write shows the error text in the status line ("Could not save the default: <reason>"); the Inspector stays as it was.
- "Reset" does not change a track that exists. A button "Apply the default to this track" is not in this ticket (open question 3).

The words are checked by the wording test of the page and by a page test that reads the text of both buttons for each of the six roles.

## 7. Presets

Folder: `Presets/` beside `defaults.json` (same `dir()`, same gate rule). One file per preset, `<stem>.json`.

Format. The kit presets of the app (`crates/app/src/roles.rs`, `kit_preset_save`, `kit_preset_load`) are
`{ "name": "...", "rules": [ { "pattern": "k in", "role": "kick", "note": 36 } ] }`: a rule matches a TRACK NAME and gives a role and a
note. They have no parameters and no velocity, and they decide the role from the name, which the plugin does already (#558). So the
format does not fit the need (a note map per role with settings), and the plugin presets do not reuse it. What is reused: the envelope
(one JSON object per file, `name` inside, the file name derived from the name) and the atomic write. The folder and the files of the
app are never used (the app keeps them per service home; the plugin's page commands `kit_preset_*` stay unused in a DAW).

A preset: `{ "v": 1, "name": "My Kit", "roles": { ...the same object as roles in the defaults... } }`. One parser for both, so
the ranges of section 3 apply to a preset file. Unknown fields are kept when a preset is written again by the plugin.

- Names: the user types the name in a page dialog (below). The file stem is `deliver::safe_stem` of the name (control characters and
  `/ \ : * ? " < > |` become `-`, leading dots go, at most 80 bytes). A stem that is empty, or that is equal (compared with
  `drumspike_app::export::file_name_key`, case and form blind, as the file system sees it) to a built-in name or to a file of another
  preset name, is refused with a text: "A preset named General MIDI is built in. Choose another name." The name inside the file is the
  typed text, trimmed, at most 80 characters.
- Built-in: "General MIDI" is code, not a file. It is first in the list, it cannot be overwritten, renamed or deleted, and `Presets/`
  can hold no file that shadows it. Map (per role, notes of the General MIDI drum map): kick 36, snare 38, tom 48, hat 42,
  cymbal 49, tom [50, 48, 47, 45, 43, 41]. It has no `params`, so a load changes notes only. The role `other` is not in it. General MIDI has several tom notes
  (41, 43, 45, 47, 48, 50); one role has one note, so the tom entry is a list (ruling R3).
- Delete: each user preset in the Load Preset dialog has a Delete control with a confirm in the page ("Delete preset "X"? The file is removed."); the built-in one has none. The plugin command `plugin_preset_delete {name}` removes only a plain file of `Presets/`.
- Bounds: at most 200 preset files are listed (the first 200 by name); each file at most 64 KB; a file that fails the checks of
  section 4 is skipped with one report row `preset_invalid`; it never stops the list.
- Menu (`static/hostmenu.js`): two new host-only entries in the group "Track" (where the other role commands live): "Load Preset..." and "Save Preset...". `HM_EXTRA` has check items only today, so it gets a second kind
  (`run` without a check mark); the audit function `hostMenuAudit` and its e2e test keep counting only the table, as for `#562`.
  "Load Preset..." opens a page dialog (a list: General MIDI first, then the presets by name, a Cancel button). "Save Preset..."
  opens a page dialog with a text field (`maxlength` 80) and Save and Cancel, built like `#renameModal`. Both dialogs are made by
  `hostdefaults.js` on first use. No system dialog and no real panel.
- Save Preset: takes the roles that the document has. For each role the note and the `params` (section 3 keys) of the FIRST row of
  that role in the DAW order; the dialog says so ("Saves the note and the settings of the first track of each role."). A role with no
  track is not in the file.
- Load Preset: the page builds the plan: for each row (a DAW track, `HT.srcsOf`) whose role is in the preset, the new note
  and the new keys of `params` that differ. Nothing is written yet. A dialog (`askConfirm`, buttons "Load" and "Cancel") says:
  "Load preset "My Kit"? 5 tracks change: the MIDI note of 4 tracks, the detection settings of 3 tracks. The hits stay as they are;
  Re-analyze uses the new settings. You can undo this in one step." With nothing to change: "Preset "My Kit" changes no track." and
  no undo step. Cancel (or Escape) changes nothing.
- Apply: after "Load", for every changed row the page calls `track_set_note` for each source of the row (as `saveMetaNote` does)
  and `track_params_set` (merge: the keys of the preset replace those keys, the others stay), and keeps the old values. The steps go
  into ONE undo entry by `pushGroup("Load Preset (My Kit)", steps)` (`static/edit.js`). Two step kinds: a new `note` step
  (`applyOp` gets one branch, `noteApplyOp`, that calls `B.setNote` with `before` or `after`; undoMeta gets a label) and the existing
  `params` step (`hgApplyParamsOp`). `groupApplyOp` is all or nothing: a refused step takes the steps before it back. The note of the
  app has no undo step today ("a note has no undo step", `saveMetaNote`); the new step is reachable only from host-mode code, so
  the standalone undo stays as it is.
- A preset changes the role of no track and creates no track. The prepared files wake by themselves (section 9).

## 8. The ARA archive keeps the export settings (Gate line 4)

Today `state::ArchiveState` is `{version, archive_id, doc_id, events, sources[]}`; `sources[].settings` holds the per-track settings.
The version rule today (`state.rs`, `m1b1-session.md`): `ARCHIVE_VERSION` is 1; `decode` refuses another number ("a restore refuses
another one"); a new field is added as `#[serde(default)]` and optional in both directions, with NO version bump; there is no
`deny_unknown_fields`, so an older reader ignores new fields and a newer archive with more fields loads, the extra fields are
ignored and are not written back; `settings` is read leniently (`lenient_map`: a wrong type means "none", never a failed archive).
The committed fixture `fixtures/archive-v1.ara` is an archive of M1a and a test loads it.

The change follows that rule exactly:

- New optional top-level field `export`: an object `{ "formats": [...], "midi_mode": "...", "compensate": true }`, read with a
  lenient reader like `lenient_map` (a field of another type is "none"), skipped when empty
  (`skip_serializing_if`). `scope`, `track` and `dest` are NOT stored: a `dest` is an absolute folder (a path that the user's
  archive must not carry, the rule of `archive_name_of`, and a write target that only a folder chosen in this editor may be, #532);
  `scope` and `track` are a choice of the moment. Open question 5.
- `ARCHIVE_VERSION` stays 1. An archive without `export` restores as today (the document keeps what its cache holds, nothing is
  invented). An archive with unknown fields, inside `export` or beside it, is read; the unknown fields are not kept (the rule today).
  An archive with another `version` is still refused.
- Store: `DocShared` gets `export: Mutex<Option<Map>>`, filled in `record_page_edit` (the same call that reads the settings, same lock
  order: events, settings, export) from `export_last_get` of the service, only when `export_last` is not null. `dc_storeObjectsToArchive`
  copies it into the blob. A change of the export settings is a page command that writes the project (`export_last_set` is a document
  write), so the existing wake path stores it.
- Restore: `dc_restoreObjectsFromArchive` hands the object to `DocService::restore`, which checks each key with the same rules as
  `export_last_set` (`formats`, `midi_mode`, `compensate`) by calling `export_last_set` and `export_compensate_set` of the service after
  `ensure_open` (no new function in `crates/`), so a hostile archive gets the same refusals as a hostile page. A refused value is
  dropped with one report row `restore_refused`, as the other archive fields do.
- A restore never reads the defaults file for `export`. A new document (no archive) is seeded from the defaults (section 3).

## 9. The prepared drag files of #576

`prepare.rs` `fingerprint(row, kind, name, tempo_key, bump)` hashes the clips, the sources (key, rate, wav path), the time of every
event, and, for the kind MIDI only, the velocity of every hit, `row.note` and the tempo key. The merged MIDI file hashes the MIDI
fingerprint of every row. `Prep::gather` reads `note` from `tracks_get` (refresh) and `fps` builds the fingerprints. A command that
writes the project (`track_set_note` is `Effect::Doc`) wakes the worker through `command_done` (`editor.rs` `changed_after`). So:

- A changed MIDI note (a Save as default does not change a track that exists; Load Preset, the Inspector note field and a new track
  with a default note do) makes the MIDI set of that row and the merged set stale, and they are made again. The spike sets do not
  depend on the note and stay ready. This is already true; this ticket adds a test that proves it for the Load Preset path
  (stage `deliver-defaults`).
- Export settings are not inputs of the prepared files (ruling R5: no rule for them). A test proves that a change of them makes no job. Whether `compensate` applies to dragged files is ticket #589.
- Presets and defaults apply their changes through the same page commands as the Inspector, so no new wake path exists.

## 10. Stages and fault runs

Each new stage has a fault run that fails for its named reason. Every stage uses the gate lock and `DS_ARA_USER_DIR` below `out/`.
No stage or test starts a system drag, posts a system event, shows a real panel or writes a system pasteboard.

| Stage | Proves | Fault run (switch) and the reason it fails |
|---|---|---|
| unit tests in the crate (`crate` stage) | parser and ranges; hostile files (too big, not JSON, link, `v` 0, 2, a string, NaN as text, deep nesting, a note 300, thresholds of 4, lo above hi, unknown role, duplicate keys); unknown fields kept on rewrite; `defaults.bad.json`; newer file is read-only; the six roles with the buttons' texts; the name rules and the reserved name; the plan of Load Preset; the archive `export` field (round trip, old archive, fixture, newer fields, damaged field) | none (the stages below are the fault runs of the same rules) |
| `defaults-new-track` | a new snare track gets note and params from the file before its first round; a track of an archive restore does not (the file changed, is gone, is damaged: the track equals the stored one); a role with no entry gets built-in values; `other` has no entry and gets note 36; the first analysis uses the thresholds | `always` (defaults also on restore): the restored track differs; `session-loses` (defaults overwrite the settings of the archive): the stored note is lost; `no-role` (new track without role takes the entry of snare) |
| `defaults-hostile` | each hostile file gives the built-in value of the bad field, the good fields stay, ONE `defaults_invalid` row per load, the plugin runs on | `trust`: no range check; the note 300 reaches the record |
| `defaults-real-path` | gate build without `DS_ARA_USER_DIR`: row `defaults_path_refused`, 0 files and 0 folders | `real-path`: the path resolves below `HOME`; the folder appears |
| `defaults-writers` | six host processes and eight threads save roles in a loop; every read in between parses; at the end the file is valid and holds the roles of the last writers | `in-place`: the write is not atomic; a reader sees a half file |
| `editor-defaults` (editor mode `defaults`, DOM actions only) | Inspector buttons: text per role, Save writes the role, Reset removes it, a new track gets it; menu: Load Preset lists General MIDI first; the confirm shows the counts; Cancel changes nothing; Load changes notes and params and is ONE undo step (undo and redo move all of them); Save Preset with a typed name writes the file; a reserved or empty name is refused | `no-confirm` (load applies at once), `two-steps` (one undo entry per track), `all-roles` (the buttons write all roles) |
| `doc-archive-export` (extends `doc-archive`) | the export settings survive store and restore; an archive without the field restores as today; an archive with unknown fields and one with `version` 2 behave by the rule of section 8; `dest` never in the archive | `export-drop`: the field is not stored |
| `deliver-defaults` (extends `deliver`) | a note change by Load Preset makes the MIDI set and the merged set stale and ready again with the new note in the file, the spike set stays ready; a change of the export settings makes no job and no state change | `fp-no-note`: the note is out of the fingerprint; the MIDI set stays "ready" with the old note |
| page tests `tests/e2e_app/test_host_defaults.py` | host mode: the buttons, the menu entries and the dialogs exist and work in the real page with a stub of the plugin commands; standalone: none of those elements exists, the menu model and the Inspector DOM equal the snapshot of the base, no call of a `plugin_defaults_*` or `plugin_preset_*` command | `leak`: the host items are made without `hostMode()` |
| Gate line 6 (see below) | the standalone app never touches the plugin's files | `touch`: a probe that does touch them |

Gate line 6, the proof. (a) A workspace test (`cargo test`, in the app crate or a root test) scans the sources of `crates/` and `apps/`
and fails when the folder name `DrumSpike ARA` or the file names `defaults.json` and `Presets` of the plugin appear in code: the
standalone code has no path to them. (b) A pytest with the standalone app (the bridge, `HOME` below a temp folder): plant a
sentinel `defaults.json` and `Presets/Sentinel.json` with known bytes in `<HOME>/Library/Application Support/DrumSpike ARA/`, run
a session that adds tracks, sets notes and saves and loads a kit preset, then compare: bytes, modification time and the list of the
folder are equal, and no file below `HOME` has the plugin's names except the sentinels. The comparison is a function with a unit test
that fires on a changed byte, a changed mtime and a new file (the fault run `touch`), so the check is proved able to fail.
(c) The same harness runs once against the plugin editor in a gate build with `DS_ARA_USER_DIR` set to that folder, saves a default, and
the comparison MUST fire (a positive control: the harness can see a write).

## 11. Standalone and gate rules

- Every page change is behind `hostMode()`: `hostdefaults.js` (a new file; it enters `JS_FILES` of `tests/jscov.py` and the script
  list of `static/index.html`) returns at once when the mode is off; `hostmenu.js` and `inspector.js` get one call each, inside
  existing `hostMode()` branches. The step type `note` of `edit.js` is reachable only from this code. No expected value of an existing
  page test changes; the page tests above compare the standalone DOM and menu model with the base.
- No element is added to `index.html`; the two dialogs and the two buttons are made in JS in host mode.
- Gate safety: no system dialog (the name is typed in a page dialog), no real panel, no system drag, no system event, no system
  pasteboard.
- Scope: `plugins/ara/` (new module `userdefaults.rs`, edits in `service.rs`, `state.rs`, `lib.rs`, `editor.rs`, `prepare.rs` tests,
  `check.py`, `switches.json`, README), `static/` (`hostdefaults.js`, small edits in `hostmenu.js`, `inspector.js`, `edit.js`,
  `backend.js`, `index.html`, `help.js` text if the DAW help names the new menu entries), `docs/design/daw-plugin/`. Outside these:
  `tests/` (new page test and the Gate line 6 test, one entry in `tests/jscov.py`). No change in `crates/`.

## 12. Open questions and risks

1. The path. The brief asks for Application Support; the README rule "only below the cache folder" must change (section 4). A host
   that sandboxes the plugin (the LUNA case is not measured) may refuse the write; then Save shows the error and the plugin works with
   built-in values.
2. The follow options. The ticket says "the follow options"; the code has no per-role follow option. This note takes the page options
   (mode, suspend on edit, follow the DAW selection) as global defaults. If the owner meant the leader of a track, say so.
3. "Reset to default" removes the role entry for new tracks. If the owner wants a button that sets an existing track back to the role
   default, it is one more Inspector action ("Apply the default to this track"), host-only, undoable; not planned here.
4. A source that the archive holds with no result (F5) keeps its settings in `doc.settings`, but its first round makes a NEW record
   with `make_track_audited`; I did not find where those kept settings are laid over the new record (the comment in `lib.rs` says "a round
   reads the settings of the service"). With defaults the track would get defaults, not the archive values, if no code lays them over.
   The developer proves this with a test before the code (stage `defaults-new-track`, case "restored without result") and fixes it,
   or keeps the track out of "new" by its `doc.settings` entry as in section 2.
5. The archive stores `formats`, `midi_mode`, `compensate`. If the owner wants the chosen scope or folder to come back after a reopen,
   that is a different rule (the folder rule of #532 and #579).
6. Tom notes. A role has one note; General MIDI has six tom notes, and "Tom 1" to "Tom 4" are different drums. A rule on the DAW name
   (pattern to note, the shape of the app's kit rules) would solve it; it is a larger change and is not in this ticket.
7. Export settings and the prepared files (section 9): the brief and the code disagree; the note keeps the code and tests the
   independence. If `compensate` should apply to dragged files, that is a change of `prepare.rs` and of its fingerprint.
8. (closed by R6) Delete is in the Load Preset dialog.
9. Two DAW processes that save at the same moment: last writer wins and one role entry may be lost in a window of a few milliseconds.
10. `export-folders.json` uses a fixed temporary name (`json.tmp`, `save_folders` in `deliver.rs`). It is not part of this ticket. The new
    file uses `write_atomic`, which has a temporary name of its own.

## 13. Order of work

Commit 1: this note. Then, one commit per line or pair: (2) `userdefaults.rs`, the new-track point, the commands, the stages
`defaults-new-track`, `defaults-hostile`, `defaults-real-path`, `defaults-writers`; the Inspector buttons and their page tests;
(3) presets, the menu entries, the dialogs and the undo step with `editor-defaults`; (4) the archive field with `doc-archive-export`
and `deliver-defaults`; (5) the unit tests of the hostile files (with line 2); (6) the standalone proof; README, `switches.json` and
the stage list in `check.py` with each commit that adds a switch or a stage.

## 14. Rulings of the coordinator (applied above)

R1 path accepted, folder created at the first Save only; the README sentence and stage `files` allow exactly this folder. R2 apply point
accepted; Q4 is tested FIRST (a source that the archive holds without a result must keep the session values). R3 notes are a list per role
(section 3). R4 follow options are the page options; export defaults seed a new document only. R5 export settings do not touch the prepared files; #589
decides `compensate`. R6 own preset format; Delete in the Load Preset dialog; the `note` step of `edit.js` with a page test that the standalone
undo list is unchanged. R7 no "Apply default to this track". R8 symlink, directory, unreadable: built-in values and one row. R9 the archive `export`
field is range-checked like the file. R10 the standalone proof uses a temp `HOME`. R11 `HM_EXTRA` second kind, `hostMenuAudit` still passes, the two
entries exist in host mode only. R12 see section 4. Report rows carry the fixed word `defaults-folder`, never an absolute path; the rows are
listed in the privacy class list of the README.

## 15. What was built (state at the end of the work)

Built and proved: the file module `plugins/ara/src/userdefaults.rs` (unit tests, hostile matrix, race test), the start values of new tracks (`make_new_track` is
`apply_defaults` in `service.rs`; one snapshot per round; the order of the DAW tracks by `SourceSnapshot.order`), the fix of Q4 (`#577 fix:` commit), the archive
field `export` (unit tests, stage `doc-archive-export`), the shell commands of `editor.rs`, the page (`static/hostdefaults.js`, tests in
`tests/e2e_app/test_host_defaults.py`), Gate line 6 (`tests/test_standalone_plugin_files.py`, stage `defaults-standalone-control`).
Not built: a stage `editor-defaults` that drives the REAL page in the plugin's web view (the page is proved in Chromium against the bridge with the plugin
commands stubbed; the plugin side of the commands is proved by unit tests and the stages above). The page has no control for the `follow` and `export` parts
of the file; they are read from a file that the user may edit by hand. The store half of the archive field (`record_page_edit`) has a unit test of the
state and a restore stage, but no stage that sets an export setting in a page and stores the archive.
