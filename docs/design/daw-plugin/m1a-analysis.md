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

PENDING - filled by the gate run (the wake in progress).

| Gate line | Result | Evidence |
| --- | --- | --- |
