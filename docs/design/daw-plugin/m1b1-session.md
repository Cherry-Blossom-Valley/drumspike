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
for the life of the controller. Every analysed source stays a track. A destroyed source
leaves the list and the project. A restore opens the project with the stored events and
decodes nothing. Nothing in `crates/` changed.

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
- **Tracks are added.** A request marks sources. When every marked source of a document has
  its WAV, the worker thread runs one round: the sources are added to the open project
  with `tracks_restore` (one track record each, with the segments of the host's playback
  regions), the service analyses those tracks only, and the project is saved. The tracks of
  earlier rounds are not analysed again; their events are read back and compared with the
  events of the round before (`earlier_changed` in the report; the check compares the bytes
  itself).
- **Tracks are removed.** `destroyAudioSource` takes the source out of the controller's list
  at once. The worker removes the track with `tracks_remove`, saves the project and deletes
  the cached WAV of the source. A numbering counter per document makes sure an index is
  never reused.
- **Restore.** `restoreObjectsFromArchive` sets the notes from the blob as before, takes the
  stored id and sends the stored events to the worker. The worker writes the project
  document from the stored events and opens it. It decodes no file; the decode counter of
  the service is part of the report and must be 0.
- **Threads.** The service work (rounds, removals, restore, close) runs on the one worker
  thread, in message order, so a removal never overlaps a round. The audio thread is not
  touched. The destroy of a controller cancels a running job, sends a close message and
  waits for the worker for at most 20 s. Every `extern "C"` function stays inside the
  panic guard.

## The check

New ara-host modes: `grow` (sources arrive in batches, each batch is analysed and waited for,
one source can be destroyed) and `docs` (two document controllers in one process). The plugin
reports one `job` row per round (document id, folder, project tracks, the events as the
bytes the service gave), a `services` row, `doc_restored` rows and `source_destroyed` rows.

Each new stage has a fault run. A switch of the plugin brings back the M1a behaviour for
that line, and the same check function must refuse it. The existing stages only changed in
where they look for the document and the cache WAVs (the new folder layout); the check of
the restore now looks for a WAV anywhere below the cache folder.

## The result (the full run on the PR head)

`./check.sh build` and `./check.sh check`: exit 0, `M1A CHECK: PASS`, 79 `ok` lines, 0 `FAIL`
(62 existing, 17 new). The log is `out/final-check.log` (not committed).

| Stage | Result | Measured value |
| --- | --- | --- |
| doc-service | ok | 2 requests in 1 document: 1 service made, 1 dropped |
| doc-service-fault | ok | per-job switch: 3 services made, the check refuses |
| doc-tracks | ok | 4 tracks, 19 events; the first 2 tracks byte for byte after the second request; 4 tracks in the document on disk |
| doc-tracks-fault | ok | forget switch: 2 tracks, 0 of 2 earlier tracks kept, the check refuses |
| service-m1b1-ref-a, -b (reference of each document alone) | ok | 8 events on 2 tracks, 11 events on 2 tracks |
| doc-two | ok | 19 events; own folders; each document byte for byte its reference; 13 files of the first folder unchanged while the second worked |
| doc-two-fault | ok | shared-folder switch: the check refuses |
| doc-archive | ok | same folder after a restore in a new process; 19 events, 4 tracks byte for byte; 0 read rows, decode counter 0, 4 cache WAVs unchanged |
| doc-archive-v1 | ok | the committed version-1 archive restores into a fresh id with 19 stored events, 0 read rows, decode counter 0 |
| doc-archive-fault | ok | newid and empty switches: both refused |
| doc-destroy | ok | 3 sources, 1 destroyed: list 2, project 2 tracks, document on disk 2 tracks, other events byte for byte, cache WAV gone |
| doc-destroy-fault | ok | keep switch: 3 tracks stay, the check refuses |
| doc-home | ok | 0 files below the home outside the cache folder, 13 inside; service home inside the cache folder with its recent list |
| doc-home-fault | ok | home switch: 1 file below the home outside the cache folder, the check refuses |
| archive-decodes, archive-decodes-fault | ok | 0 read rows and 0 cache WAVs; the fault run shows 1 and 1 |
| identity-clap, identity-vst3 | ok | 19 events on 4 tracks, byte for byte the reference |
| regions (all lines) | ok | 4 events; update and destroy records as before |
| threads (all lines) | ok | 0 overruns in 239291 blocks; the analysis off the request thread; the fault run fires |
| conformance (10 runs) | ok | rc 0 on the CLAP and on the VST3 |
| files-repo | ok | 974 files, 0 changed |
| pluginval, validate, universal | ok | strictness 5 rc 0; clap-validator rc 0; arm64 and x86_64 |
| standalone-tree, -metadata, -diff | ok | 0 plugin hits; 0 files outside `plugins/ara/` and `docs/design/daw-plugin/` |
| crate-fmt, -clippy, -test | ok | 19 unit tests, 0 warnings |
| workspace-fmt, -clippy, -test | ok | rc 0 |

One more measure, not a gate: the events of the 2+2 run equal the events of the four sources
analysed at once (the identity reference), on this fixture. Cross-track rules judge the
tracks of a round, so another kit may differ; this was not measured.

## The red proofs

The new stages ran against the CLAP of the base commit, with the new check and the new
ara-host (`out/red-proof-final.log`, not committed). Each main stage fails:

- doc-service: `0 analysis requests in 0 document(s): None service(s) made` (the base reports no
  service count).
- doc-tracks: `0 job rows, want 2; the document on disk has 2 tracks, want 4` (the base lost the
  first two tracks in the second round).
- doc-two: `ara-host docs rc=2` (the host gave up waiting for the analysis after its 30 s; the
  base shares one folder and one source numbering between the two documents, but the cause
  of the hang was not looked at further).
- doc-archive: `the archive stores no document id (None)`; doc-archive-v1 fails the same way.
- doc-destroy: `the document on disk has 3 tracks after the destroy, want 2`.
- doc-home: `the service home is inside the cache folder with its recent list: False` (the base
  sets no service home).

Several of these fail on a missing report row, not on a measured wrong value. The fault runs
are the other half of the proof: each shows the check function refusing a wrong value.

## Open points for the editor ticket (M1b2)

- After a restore the track sources point at the cache path of the WAV, which may not exist
  in a new process. The editor needs the audio: the plugin has no lazy re-read yet.
- The archive takes its events from the plugin's copy of the last round. Edits made in the
  editor must update that copy, or the store must read the events from the service.
- A later request adds tracks to an open project with `tracks_restore`; it builds the peak
  caches of the new tracks (a decode on the worker thread). A track with segments can be
  added only this way; `tracks_add` makes plain tracks. A small `crates/` entry for segments
  would be cleaner (not done: out of scope).
- The service has no event emitter and no lock client yet: the editor will need both.
- Destroying a controller waits at most 20 s for the worker. A long job is cancelled first.
- `analyze_start` with a list of names reads patterns (glob, ranges). Track names are made
  from file stems, so an exact name wins, but a stem such as `tom-1-3` is an edge case.
- A source whose read fails is marked complete without content, as before; the host cannot
  tell the two cases apart.
