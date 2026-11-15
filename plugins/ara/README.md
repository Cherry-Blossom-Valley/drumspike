# Drum Spike ARA plugin (M1a, #525)

The Drum Spike plugin for DAWs, without an editor: one CLAP plugin in Rust with ARA 2,
wrapped as one self-contained VST3 (the CLAP inside the bundle). The plugin takes the
audio sources and playback regions of the host, analyses them with the Drum Spike service
(`drumspike-app`, the same detection as the desktop app), gives the hits back to the host
as ARA notes, and keeps its state in the ARA archive of the session.

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

(`DS_ARA_CACHE` overrides it in the check.) Inside it: `audio/<n>-<stem>.wav`, the source
audio the host delivers through the ARA audio access controller, one file per ARA audio
source, and `session/`, the Drum Spike project the analysis runs on (one track per audio
source; the host's playback regions are the segments of the track). The plugin never
writes into an audio file of the session.

## The check

`plugins/ara/check.sh check` (-> `check.py all`) runs every gate line of the ticket: the
fixture session (four tracks with bleed, roles from the names), the byte-for-byte compare
against `service_tool analyze` on the CLAP and on the plugin inside the VST3, the regions
fixture (two regions, one trimmed; the track is made from segments), the archive store /
restore with the decode counter at 0 and the committed fixture `fixtures/archive-v1.ara`,
the audio-thread check with its fault run, the file probe, pluginval (strictness 5) and
clap-validator, the standalone rule, and the crate and workspace trios. The ticket's "in
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
