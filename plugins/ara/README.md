# Drum Spike ARA plugin (M1a, #525; M1b1, #537)

The Drum Spike plugin for DAWs, without an editor: one CLAP plugin in Rust with ARA 2,
wrapped as one self-contained VST3 (the CLAP inside the bundle). The plugin takes the
audio sources and playback regions of the host, analyses them with the Drum Spike service
(`drumspike-app`, the same detection as the desktop app), gives the hits back to the host
as ARA notes, and keeps its state in the ARA archive of the session.

Since M1b1 the plugin holds one Drum Spike service and one project for each ARA document,
for the life of the document controller (see "The session" below).

The crate is outside the Cargo workspace members (the workspace builds without the ARA
SDK); it uses `drumspike-app` as a path dependency. The ARA 2 C API is generated with
bindgen from `$ARA_API_DIR/ARAInterface.h` (the ARA SDK 2.3.0; `check.sh deps` fetches it
into `.deps`).

## Build

One command builds the CLAP (universal: arm64 + x86_64, joined with lipo; the build fails
when `lipo -info` does not name both) and one self-contained VST3 bundle with the CLAP
inside (`.deps/build-wrapper/DrumSpikeAra.vst3/Contents/PlugIns/DrumSpikeAra.clap`; the
wrapper searches its own bundle's `PlugIns` first, so no `CLAP_PATH` is needed):

    ./check.sh all        # deps, build, check
    ./check.sh build      # the two bundles, both test hosts, service_tool
    ./check.sh check      # the gate: check.py all

`./check.sh deps` fetches the third-party parts into `.deps` (or `$DS_ARA_DEPS`): the ARA
SDK 2.3.0 API headers, the ARA examples (with `ARA_Library` and the CLAP 1.1.9 and
VST3 SDK v3.8.0_build_66 companions, patched like S1, for the independent conformance
host), clap-wrapper 0.17.0 with the patch of `patches/`, pluginval 1.0.4 (pinned SHA-256)
and clap-validator 0.4.1, and both macOS Rust targets. Heavy: run it under the host lock.

## Cache folder

The plugin writes only below one cache folder:

    ~/Library/Caches/DrumSpike/ARA

(`DS_ARA_CACHE` overrides it in the check.) Inside it, one folder for each ARA document:

    documents/<id>/audio/<n>-<stem>.wav   the source audio the host delivers through the
                                          ARA audio access controller, one file per ARA
                                          audio source of this document
    documents/<id>/project/               the Drum Spike project of the document (one track
                                          per audio source; the host's playback regions
                                          are the segments of the track)
    documents/<id>/service/               the home folder of the service (its recent list)

The plugin never writes into an audio file of the session, and nothing below the user's
home outside this folder.

## The session

One document controller owns one `drumspike_app::Service` (made with the controller,
dropped with it) and one project folder. The id of the folder is made by the plugin once per
document and stored in the ARA archive, so a restore in a new process finds the same folder.

- A request to analyse sources adds them as tracks to the open project. The rule: whenever
  the set of analysed tracks of a document changes (a request adds sources, a source is
  destroyed), the result of ALL tracks is the result of one analysis of all those tracks
  together, as the service gives it (the session steps cross bleed, ghost gate, tom bleed and
  flam run over the tracks of one job, so a job over the new tracks alone would never let a
  track see the other microphones). Requests that arrive within 250 ms of each other (at
  most 2 s in all) are one round; the report names the number of rounds. The project is saved
  after every round.
- The notes of an earlier source can change in a later round (a bleed hit leaves the output).
  The plugin then tells the host: in `notifyModelUpdates` it calls the host's
  `notifyAudioSourceContentChanged` for exactly those sources, and serves the new notes.
- `destroyAudioSource` removes the source from the controller's list at once; its track
  leaves the project (and its cached audio the cache) on the worker thread, and the tracks
  that are left are analysed together again.
- A restore from the archive writes the project document with the stored events and opens
  it. It decodes no audio file. A restored track's source points at the cache path where the
  reader would write the audio; that file does not exist until the host gives the audio
  again. `project_open` and `events_get` work without it; a call that needs the audio gets
  the app's own offline answer (a crate test), and a later round skips such a track and keeps
  its stored events. An archive of M1a (version 1, no document id) restores into a fresh id;
  the archive version stays 1 (the two new fields are optional).
- Two document controllers in one process have their own service, folder, source numbering
  and file names.
- All of this runs on the plugin's worker thread. Nothing runs on the audio thread, and the
  host's main thread only reads and writes the state it needs for the ARA calls.
  `destroyDocumentController` cancels a running analysis and returns within half a second in
  every case; the worker owns the document through shared ownership and finishes the save on
  its own.

## The check

`plugins/ara/check.sh check` (-> `check.py all`) runs every gate line of the ticket: the
fixture session (four tracks with bleed, roles from the names), the byte-for-byte compare
against `service_tool analyze` on the CLAP and on the plugin inside the VST3, the regions
fixture (two regions, one trimmed; the track is made from segments), the archive store /
restore with the decode counter at 0 and the committed fixture `fixtures/archive-v1.ara`,
the audio-thread check with its fault run, the file probe, pluginval (strictness 5) and
clap-validator, the standalone rule, and the crate and workspace trios. The M1b1 stages
(`doc-*`, each with a fault run) drive the host modes `grow` (sources arrive in batches, one
can be destroyed) and `docs` (two document controllers in one process): one service per
document, all tracks analysed together (a fixture whose split result differs), one round for
requests that arrive close together, the host told about changed notes, two documents apart, the document id in the archive, a destroyed
source leaves the project, the destroy of a controller during an analysis returns in under
1 s, and the home of the service (a run with `HOME` below `out/`
lists the files below it outside the cache folder: 0). The ticket's "in
the ARA test host" is two hosts: the plugin's own `ara-host` drives the stages above, and
the `ARATestHost` of the ARA SDK examples - built from the S1 pattern with the S1 patch -
runs its own tests (ContentReading, PropertyUpdates, Archiving, PlaybackRendering,
EditorView) on the CLAP and on the wrapped VST3, each must exit 0. The plugin has no
editor GUI, but it implements the EditorView role interface and its playback renderer
passes audio through, so no test is skipped.

## Test switches of the check

A real host sets none of these: `DS_ARA_CACHE` (the cache folder), `DS_ARA_OUT` (where the
plugin writes its report, one JSON object per line), `DS_ARA_FAULT=in-process` (the fault
switch: the analysis runs inside `process` on purpose, so the overrun check of the host
can fail), `DS_ARA_TABLES_FAULT=1` (the plugin serves a document controller table with
one emptied slot, so the table walk of the check can fail) and
`DS_ARA_RESTORE_FAULT=decode` (the restore decodes one source on purpose, so the
no-decode proof of the archive check can fail).

## Licences

The third-party parts of the plugin and of the check, one row each (the versions are the
ones `.deps` fetches):

| Part | Version | Licence | Where it is used |
| --- | --- | --- | --- |
| ARA SDK (ARA_API, ARA_Library, ARA_Examples, Celemony) | 2.3.0 | Apache-2.0 (its bundled examples: cpp-base64 zlib, pugixml MIT, ICST AudioFile BSD-2-Clause) | `ARA_API` headers feed the bindgen bindings; the examples build the independent `ARATestHost` of the conformance run. Nothing of it is linked into the plugin. |
| CLAP | 1.1.9 | MIT | The entry/factory ABI the plugin implements (headers only). |
| clap-wrapper | 0.17.0 | MIT | Compiled into the wrapped VST3 bundle. |
| VST3 SDK | 3.8.0_build_66 | MIT | Compiled into the wrapped VST3 bundle (the wrapper's module and `pluginterfaces`). |
| pluginval | 1.0.4 | GPL-3.0 (Tracktion) | Check-time tool only; never part of a bundle. |
| clap-validator | 0.4.1 | MIT (Robbert van der Helm) | Check-time tool only; never part of a bundle. |
| Rust crates of the plugin | `Cargo.lock` | 150 crates, all permissive (MIT, Apache-2.0, BSD-2/3-Clause, Zlib, ISC, Unlicense, Unicode-3.0, 0BSD); no copyleft | Compiled into the plugin binaries. `drumspike-app` and `drumspike-core` are this repository's own crates (Apache-2.0). |

The crate itself is repository code under the repo licence (Apache-2.0, see `NOTICE` at
the repository root). Before any delivery of a bundle (M1c), `NOTICE` must list the parts
that end inside it (the wrapper and the VST3 SDK module, the plugin's crates and their
notices) the way the app's `THIRD_PARTY.md` does.
