# M1a: the plugin analyses the host's tracks with the Drum Spike service (ARA, no editor)

Ticket #525, umbrella #503 (the DAW plugin program). First part of M1 (#510):
`plugins/ara/`, one CLAP plugin in Rust with ARA 2, wrapped as one self-contained VST3
(the CLAP inside the bundle). No editor: the plugin analyses, gives the hits back as ARA
notes and keeps its state in the ARA archive. It builds on the merged spikes S1 (#504),
S3 (#506) and on P1, P4 (#508, #519), and copies their patterns without changing them.

## The design

One chain: host -> CLAP -> ARA document controller in Rust -> the app service -> ARA notes.

- **How the host's audio reaches the service.** The plugin reads each ARA audio source
  one time, through the host's audio access controller, and writes it as one WAV file
  into the cache folder (`<cache>/audio/<n>-<stem>.wav`, streaming f32, temporary file +
  rename so a reader never sees half a file). The reader is created and destroyed on the
  host's main thread (the ARA rule); the reads run on their own threads, one per source.
- **Where the project lives.** One Drum Spike project per document controller, at
  `<cache>/session/`: one track per audio source, named `safe_name` from the source name,
  with the role guessed from the name (the app's own `tracks_scan` rule) and the source
  record of the cached WAV. The host's playback regions of a source are the segments of
  the track (P1: `start` at the region's `startInModificationTime`, `offset` the same,
  `length` the `durationInModificationTime`): ARA content is per audio source in the
  source's own time, so the track's timeline is the source's timeline on the covered
  slices; a hit of the source at `t` seconds exists only inside a region and sits at the
  sample `t * rate`. A source with no regions is a plain track. The analysis is the app
  service itself: `drumspike_app::headless::analyze_projects`, the same call
  `service_tool analyze` makes, so the event lists are the service's own, byte for byte.
- **What the archive holds.** One JSON blob with a version field (`version: 1`), the
  document archive ID, the events object of the last analysis and the notes of every
  source (`persistent_id`, name, sample rate, one note per event at the exact sample with
  its volume). `storeObjectsToArchive` writes it through the host's archive controller,
  `restoreObjectsFromArchive` reads it, matches the sources by persistent ID (name as the
  fallback) and sets the notes without running the service again - no file is decoded a
  second time. The committed fixture `fixtures/archive-v1.ara` loads in the check.
- **Threads.** No analysis on the audio thread and none on the host's main thread: the
  reads and the service job run on their own threads; `process` only passes audio
  through. The S3 block check is used again: audio blocks during the analysis have no
  overrun, and the fault switch (`DS_ARA_FAULT=in-process`) runs the analysis inside
  `process` once and makes the overrun check fail - the proof that the check can fail.
- **The boundary.** Every CLAP entry point and the service job run inside a panic guard;
  a caught panic answers the entry point's default (the S3 rule).
- **Test switches** (a real host sets none): `DS_ARA_CACHE` (the cache folder,
  default `~/Library/Caches/DrumSpike/ARA`), `DS_ARA_OUT` (the report, one JSON object
  per line) and `DS_ARA_FAULT`.

## The check

`plugins/ara/check.sh` (deps, build, check) is the gate; `check.py` runs one stage per
gate line. The fixture session is synthetic audio from the script: four tracks with bleed
between them, named so the roles come out kick, snare, tom and hat. The reference event
list comes from `service_tool analyze` on a project with the same audio and the same
roles (built by `tracks_add`, the app's own import). The regions fixture is one source
with two playback regions at different timeline positions, one of them trimmed, with a
hit in the gap and one behind the regions; the reference project carries the same
segments. pluginval 1.0.4 (strictness 5) runs on the wrapped VST3 and clap-validator
0.4.1 on the CLAP, both pinned; no `CLAP_PATH` is set anywhere, so the VST3 proves it
finds the CLAP inside its own bundle.

Two hosts read the ticket's "in the ARA test host": the plugin's own `ara-host` drives
the fixture session, the regions, the archive round trip, the notes and the thread
checks (it is the only host that can build these), and the `ARATestHost` of the ARA SDK
examples - built from the S1 pattern with the S1 patch - runs its own tests beside it:
ContentReading, PropertyUpdates, Archiving, PlaybackRendering and EditorView, on the
CLAP and on the wrapped VST3, each must exit 0 within its 120 s limit (a hung test is a
FAIL with its name). The plugin has no editor GUI, but it implements the EditorView role
interface and its playback renderer passes audio through, so no test is skipped.

The `tables` stage walks the empty-slot class (the crash of this ticket): `ara-host
tables` loads the plugin, binds all three roles and reads every ARA interface table up
to the structSize the plugin declares - the factory, the document controller and the
playback renderer, editor renderer and editor view tables. ARA marks optional parts by
structSize (`ARAInterface.h`), so every slot inside the declared size must carry a
function; the SDK hosts call the slots unchecked (`ARAHostDispatch.cpp`). The stage
fails on the first empty slot, on an assigned role without an interface, and on 0 slots
checked, and prints the number of slots it walked.

### What each stage calls

A stage proves the calls it names: the plugin's report rows (`DS_ARA_OUT`) exist only
because a host made the call, so a host that calls nothing cannot pass a stage below.

| Stage | Host calls the stage depends on |
| --- | --- |
| tables | `createDocumentControllerWithDocument`; the ARA plugin extension `bind_to_document_controller` with playback renderer, editor renderer and editor view roles; a read of every slot of the factory, the document controller and the three role tables |
| identity-clap, identity-vst3 | the CLAP entry (`init`, `get_factory`), `create_plugin`, `init`; ARA: `initializeARAWithConfiguration`, `createDocumentControllerWithDocument`, `beginEditing`, `endEditing`, `createAudioSource`, `createAudioModification`, `createPlaybackRegion`, `enableAudioSourceSamplesAccess`, `requestAudioSourceContentAnalysis`, `isAudioSourceContentAnalysisIncomplete`, `createAudioSourceContentReader`, `getContentReaderEventCount`, `getContentReaderDataForEvent`, `destroyContentReader`, `destroyDocumentController` |
| regions | the identity set, with `createPlaybackRegion` carrying the two region specs |
| archive | the identity set, plus `storeObjectsToArchive` and `restoreObjectsFromArchive` in a second process |
| threads | the identity set, plus the CLAP `process` loop on the audio thread while the analysis runs |
| conformance | the `ARATestHost` calls of its own test set (a second author's host; its sources are in the checked-out ARA_SDK) |
| fixture, reference, files | no plugin calls (fixture and reference build the compare base; files diffs the repo tree) |
| pluginval, validate, universal, standalone, crate, workspace | no ARA calls (validators, lipo, cargo, git) |

## Result table

Gate run of 2026-11-15 on the PR head 754e61cc: `plugins/ara/check.sh` rc=0, every stage
below ok (the run's `files` stage: no file of the repo changed, so the code the gate ran
is exactly 754e61cc; the commit that adds this table changes docs only).

| Gate line | Result | Evidence |
| --- | --- | --- |
| `deps` | ok | ARA_SDK 2.3.0 with the pinned ARA_API submodule, clap-wrapper 0.17.0 (+ patch), CLAP 1.1.9, VST3 SDK v3.8.0_build_66, pluginval 1.0.4, clap-validator 0.4.1 |
| `build` | ok | `DrumSpikeAra.clap` (universal), `DrumSpikeAra.vst3` with the CLAP inside, `ara-host`, `ARATestHost` |
| `fixture` | ok | 4 tracks with bleed between them (16 lead hits), `out/fixtures/` |
| `reference-project` | ok | project at `out/m1a-ref` with the copied audio (service_tool run rc=0) |
| `service-reference` | ok | 19 event(s) on 4 track(s), 733 bytes |
| `tables` | ok | 65 slots walked up to the declared structSizes (ARAFactory=3, documentController=54, playbackRenderer=2, editorRenderer=4, editorView=2); fails on an empty slot or on 0 |
| `identity-clap` | ok | 19 events on 4 tracks (analysis 628 ms) |
| `compare-clap-session` | ok | byte for byte the reference events (733 bytes) |
| `notes-clap-session` | ok | kick 5, snare 3, tom 7, hat 4 - at the exact samples |
| `identity-vst3` | ok | 19 events on 4 tracks (analysis 619 ms) |
| `compare-vst3-session` | ok | byte for byte the reference events (733 bytes) |
| `notes-vst3-session` | ok | kick 5, snare 3, tom 7, hat 4 - at the exact samples |
| `service-regions` | ok | 4 event(s) on 1 track(s), 156 bytes (analysis 251 ms) |
| `compare-regions` | ok | byte for byte the reference events (156 bytes) |
| `regions-trim` | ok | no event at the gap hit (1.5 s) or behind the regions (2.6 s); 4 events on the covered slices |
| `notes-regions` | ok | m1a_tom_96k_mono: 4 notes at the exact samples |
| `compare-archive-store` | ok | byte for byte the reference events (733 bytes) (analysis 612 ms) |
| `archive-decodes` | ok | the decode counter of the restore stays at 0 (analysis None ms: the restore decodes no file) |
| `compare-archive-restore` | ok | byte for byte the reference events (733 bytes) |
| `notes-archive-restore` | ok | kick 5, snare 3, tom 7, hat 4 - at the exact samples |
| `archive-fixture-version` | ok | the fixture carries version 1 |
| `compare-archive-fixture` | ok | byte for byte the reference events (733 bytes) |
| `threads` | ok | rc=0, 356283 blocks while the analysis ran, overruns=0, worst 23 µs of 10667 µs per block |
| `threads-fault` | ok | the fault switch ran the analysis inside `process`: rc=2, 1 block(s) over budget (worst 620981 µs), event_file=True |
| `conformance-clap-ContentReading` ... `conformance-clap-EditorView` | ok | ARATestHost ContentReading, PropertyUpdates, Archiving, PlaybackRendering, EditorView on the CLAP: all five rc=0 |
| `conformance-vst3-ContentReading` ... `conformance-vst3-EditorView` | ok | the same five on the wrapped VST3: all five rc=0 |
| `files-repo` | ok | no file of the repo changed during the runs (0 changed) |
| `files-fixtures` | ok | the four fixture WAVs: sha256 unchanged |
| `pluginval` | ok | strictness 5 on the wrapped VST3, rc=0 |
| `validate` | ok | clap-validator on the CLAP bundle, rc=0 |
| `universal` | ok | lipo -info: x86_64 arm64 (CLAP and VST3) |
| `standalone-tree` | ok | cargo tree -p drumspike-desktop names neither the plugin crate nor a plugin SDK crate (0 hits) |
| `standalone-metadata` | ok | cargo metadata --no-deps at the repo root does not name the plugin crate |
| `standalone-diff` | ok | the diff touches only plugins/ara/ and docs/design/daw-plugin/ (0 files outside) |
| `crate-fmt`, `crate-clippy`, `crate-test` | ok | rc=0 each (fmt, clippy, test of the plugin crate) |
| `workspace-fmt`, `workspace-clippy`, `workspace-test` | ok | rc=0 each (workspace without the ARA SDK) |
| total | PASS | M1A CHECK: PASS (tables, identity, regions, archive, notes, threads, conformance, files, pluginval, clap-validator, universal, standalone, crate, workspace) |
