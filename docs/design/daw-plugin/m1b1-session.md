# M1b1: one service and one project for each ARA document

Ticket #537, umbrella #503 (the DAW plugin program), part of M1 (#510). Follows M1a (#525)
and is the base of the editor window (M1b2). The plugin lives in `plugins/ara/`.

## What changed

Before: the plugin made a new Drum Spike service for every analysis job and dropped it. It
wrote one fixed project folder, and a later job wrote the document from only its own
sources, so the earlier tracks left the project. A destroyed source stayed in the plugin's
list until the controller ended. Two documents in one process shared the folder, the source
numbering and the file names.

Now: one `drumspike_app::Service` and one project folder for each ARA document controller,
for the life of the controller. Every analysed source stays a track. A destroyed source leaves
the list and the project. All tracks of a document are analysed together (the rule below). A
restore opens the project with the stored events and decodes nothing. Nothing in `crates/`
changed.

## The rule: all tracks together

Whenever the set of analysed tracks of a document changes (a request adds sources, or a source
is destroyed), the result of ALL tracks is byte for byte the result of one analysis of all
those tracks together, as the service reference gives it.

Why: the session steps of the service (cross bleed, ghost gate, tom bleed, flam) run over the
tracks of one job only. A DAW requests the analysis of sources one by one, as the user sets the
plugin on each track. If a round analysed only the new tracks, no track would ever see the
other microphones, and the plugin's result would not be the app's result. The first version of
this work had that fault (and a gate line that asked for it); the fault switch
`DS_ARA_ROUND_FAULT=new-only` brings it back so the check can refuse it.

How:

- Each round runs `analyze_start` over every track whose audio is here. The decode is cached by
  the service. A track restored from the archive whose audio the host has not given again is
  not analysed; it keeps its stored events.
- Requests that arrive close together are one round: the worker waits until no message came
  for 250 ms (at most 2 s in all). Eight tracks added one after the other are one analysis,
  not eight. The report names the number of rounds.
- After `destroyAudioSource` the remaining tracks are analysed together again, and their result
  equals the reference of the remaining set.
- The notes of an earlier source can change in a later round (a hit the bleed rules mark is no
  note). The plugin tells the host: the sources are collected, and in `notifyModelUpdates` (the
  only call in which ARA lets the plugin call back) the plugin calls the host's
  `notifyAudioSourceContentChanged` for exactly those sources. The test host records every
  notification. A source whose notes did not change gets none; a new source gets none (the
  host polls for its completion, as in M1a).

## The design

- **Service life.** The service is made in `createDocumentControllerWithDocument` and
  dropped when the controller is destroyed. The report names the services made and dropped.
  The service home is set inside the cache folder, so the service never writes below the
  user's home.
- **Folder per document.** `<cache>/documents/<id>/` holds `audio/` (the cached WAV of each
  source, numbered per document), `project/` (the Drum Spike project) and `service/` (the
  home of the service). Two documents never share any of it.
- **Document id.** The plugin makes the id once (64 random bits as hex) and stores it in the
  ARA archive (`doc_id`). The archive also stores the track name of each source (`track`).
  Both fields are optional on reading, so an archive of M1a loads and the committed fixture
  `fixtures/archive-v1.ara` still loads. A reader of M1a ignores the new fields. The archive
  version stays 1: no bump was needed. An archive without an id restores into a fresh id.
- **Tracks are added.** The sources are added to the open project with `tracks_restore` (one
  track record each, with the segments of the host's playback regions), then the round runs.
- **Tracks are removed.** `destroyAudioSource` takes the source out of the controller's list
  at once. The worker removes the track with `tracks_remove`, saves, deletes the cached WAV of
  the source, and starts a round over the tracks that are left. A numbering counter per
  document makes sure an index is never reused.
- **Restore.** `restoreObjectsFromArchive` sets the notes from the blob as before, takes the
  stored id and sends the stored events to the worker. The worker writes the project document
  from the stored events and opens it. The decode counter of the service is part of the report
  and must be 0.
- **What a restored track points to.** The source record of a restored track (and the segments
  of a segmented track) points at the cache path `<cache>/documents/<id>/audio/<n>-<stem>.wav`
  where the reader would write the audio. In a new process that file does not exist until the
  host enables the samples access again. Before that: `project_open` and `events_get` work (the
  stored events are in the document), the track is offline, and a call that needs the audio
  answers the app's normal offline error, not a panic. The crate test
  `a_restored_track_without_audio_answers_the_offline_error_not_a_panic` checks this. A round
  skips such a track.
- **Close.** `destroyDocumentController` stops the readers, cancels a running analysis, sends
  a close message and waits at most 500 ms for the worker, then returns. The worker owns the
  document through shared ownership (`Arc`); it finishes the save or drops the service on its
  own. Nothing of the controller is used after it is freed: the readers are joined first, the
  worker holds only the shared document.

## Rules that still hold, and the stage that proves each

| Rule | Stage |
| --- | --- |
| Nothing of this runs on the audio thread: 0 overruns while the analysis runs | threads, threads-blocks (fault: threads-fault) |
| The analysis runs on the worker thread, not on the request caller's thread | threads-off-request (fault: threads-fault-thread) |
| The host's main thread does no service work and never waits long: the destroy of a controller during an analysis returns in under 1 s | doc-close (fault: doc-close-fault) |
| Readers are created and destroyed on the main thread; ARA calls follow the SDK's own tests | conformance (10 runs of the ARATestHost) |
| Every `extern "C"` function runs inside the panic guard; no unwrap on a host pointer | crate-clippy, the ARATestHost runs, doc-close (rc 0, no panic text) |
| The service writes only below the cache folder | doc-home (fault: doc-home-fault), files-repo |

## The check

Host modes: `grow` (sources arrive in batches, or one after the other with `--gap-ms`, one can
be destroyed), `docs` (two document controllers in one process) and `close-busy` (destroy a
controller while its analysis runs and time the call). The test host implements the model
update controller and prints every `notifyAudioSourceContentChanged` as `host_content_changed`.
The plugin reports one `job` row per round (document id, folder, the tracks analysed, the
events as the bytes the service gave, the sources whose notes changed), a `services` row
(made, dropped, rounds), `doc_restored` rows and `source_destroyed` rows.

The fixture that makes the split result differ: two toms that hear the snare (a quieter copy
2 and 3 ms after each of its 8 hits), the snare with 8 hits, and the kick. Analysed together,
the tom bleed rule marks the 16 bleed hits (`bleed: true`, out of the output, so no note).
Analysed in two requests of two without the others, it cannot. Several tries first: the M1a
fixture (the bleed of 3 snare hits) and a snare with 3 hits give no mark, because the rule
needs at least 5 hits of the source to learn the transfer gain. The cross bleed rule alone
only adds evidence keys (`a6_lead_ms`), which also differ in the bytes but give no note change.

Each new stage has a fault run. A switch of the plugin brings back one wrong behaviour, and
the same check function must refuse it.

## The result (the full run on the PR head)

`./check.sh build` and `./check.sh check`: exit 0, `M1A CHECK: PASS`, 89 `ok` lines, 0 `FAIL`
(62 existing, 27 new). The log is `out/final-check.log` (not committed).

| Stage | Result | Measured value | Fault switch that makes it fail, and the failure line |
| --- | --- | --- | --- |
| doc-service | ok | 2 requests in 1 document, 2 rounds: 1 service made, 1 dropped | `DS_ARA_SERVICE_FAULT=per-job`: 3 services made |
| doc-tracks | ok | 4 tracks, 37 events byte for byte the reference of all four together (0 differ); round 2 analysed 4 tracks; notes changed and told to the host: both toms, no other source | `DS_ARA_ROUND_FAULT=new-only`: 16 of 37 events differ, round 2 analysed 2 tracks; `DS_ARA_TRACKS_FAULT=forget`: 2 tracks and 13 events, 24 of 37 differ |
| doc-rounds | ok | 4 requests 40 ms apart: 1 round, events byte for byte the reference | `DS_ARA_SETTLE_FAULT=none`: 3 rounds, events differ |
| doc-two | ok | 19 events; own folders; each document byte for byte its reference; 13 files of the first folder unchanged while the second worked | `DS_ARA_FOLDER_FAULT=shared`: own folders False, 0 files quiet |
| doc-archive | ok | same folder after a restore in a new process; 19 events, 4 tracks byte for byte; 0 read rows, decode counter 0, 4 cache WAVs unchanged | `DS_ARA_RESTORE_FAULT=newid` and `=empty`: both refused |
| doc-archive-v1 | ok | the committed version-1 archive restores into a fresh id with 19 stored events, 0 read rows, decode counter 0 | (covered by the archive fault runs) |
| doc-destroy | ok | 3 sources together equal the reference; 1 destroyed: list 2, project 2 tracks, document on disk 2 tracks; the rest analysed together again, equal to the reference of the remaining set (24 of 32 events differ from before); both toms told to the host; cache WAV gone | `DS_ARA_DESTROY_FAULT=keep`: 3 tracks stay; `DS_ARA_ROUND_FAULT=new-only`: no round after the removal |
| doc-close | ok | destroy of the controller with the analysis running: 505 ms (limit 1000 ms); no round finished; rc 0; 0 files in the cache folder outside the documents folder | `DS_ARA_CLOSE_FAULT=wait`: 4669 ms |
| doc-home | ok | 0 files below the home outside the cache folder, 13 inside; service home inside the cache folder with its recent list | `DS_ARA_HOME_FAULT=1`: 1 file below the home outside the cache folder (`recent.json`) |
| the 12 fault stages and the 3 reference lines | ok | each fault run was refused by its check | (see above) |
| the 62 stages of M1a | ok | identity 19 events on 4 tracks byte for byte; archive-decodes 0 read rows; threads 0 overruns in 599145 blocks; conformance 10 runs rc 0; files-repo 0 changed | |
| standalone-tree, -metadata, -diff | ok | 0 plugin hits; 0 files outside `plugins/ara/` and `docs/design/daw-plugin/` | |
| crate-fmt, -clippy, -test; workspace trio | ok | 21 unit tests, 0 warnings | |

The crate test of the restore (offline answer) is part of crate-test.

## The red proofs

The new stages ran against the CLAP of the commit before this follow-up (the first version:
new tracks only, a close that waited), with the new check and the new ara-host
(`out/red2-proof-final.log`, not committed):

- doc-tracks: `byte for byte the reference of all four together: False (16 of 37 events differ);
  round 2 analysed 0 tracks` (the first version analysed the new tracks only).
- doc-rounds: `None round(s) (want 1), 4 job row(s)` (every request was a round).
- doc-destroy: `0 request round(s) and 0 round(s) after the removal` (no round after the removal).
- doc-close: `destroy of the controller with the analysis running: 4595.3 ms (limit 1000 ms)`.

For the first version of M1b1 the base commit run (`out/red-proof-final.log`) failed doc-service,
doc-tracks, doc-two, doc-archive, doc-archive-v1, doc-destroy and doc-home, several of them on a
missing report row. The measured-value proofs are the fault switches in the table above.

## Open points for the editor ticket (M1b2)

- After a restore the track sources point at a cache path without a file. The editor needs the
  audio: the plugin has no lazy re-read yet (the host must enable the samples access again). Until
  then a restored track is offline and is not analysed again.
- The archive takes its events from the plugin's copy of the last round. Edits made in the
  editor must update that copy, or the store must read the events from the service.
- A later request adds tracks to an open project with `tracks_restore`; it builds the peak
  caches of the new tracks (a decode on the worker thread). A track with segments can be added
  only this way; `tracks_add` makes plain tracks. A small `crates/` entry for segments would be
  cleaner (not done: out of scope).
- The service has no event emitter and no lock client yet: the editor will need both, and it will
  have to show a round in progress and its cancel.
- A round analyses every track again; for a large session the cost grows with the number of
  tracks. The settle time (250 ms quiet, 2 s at most) bounds the number of rounds, not their
  length. Cancel works between tracks, not in the middle of one.
- The close returns in 500 ms at most; a round that is cancelled leaves the tracks of the round
  without events in the saved project until the next round.
- `analyze_start` with a list of names reads patterns (glob, ranges). Track names are made
  from file stems, so an exact name wins, but a stem such as `tom-1-3` is an edge case.
- A source whose read fails is marked complete without content, as before; the host cannot
  tell the two cases apart.
