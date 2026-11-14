# ARA stack spike (seed for ticket #504)

Throwaway spike code of 2026-11-14, copied as it ran. It is the start for spike S1, not product code.
It is not a member of the Cargo workspace and no build or release of the Drum Spike app uses it.

Question: does ARA 2 work from a CLAP plugin written in Rust, and does `clap-wrapper` carry it into a VST3?
Answer: yes. `./spike.sh` exits 0 and prints `SPIKE: PASS (CLAP and wrapped VST3)`.

## What is here

- `src/lib.rs` (by hand): the CLAP entry, the ARA factory, the document controller, the playback renderer.
- `src/stubs.rs` (generated): the trivial callbacks and the interface tables. `gen_stubs.py <bindgen output> <stubs.rs>` writes it. The input is the bindgen output of `ARAInterface.h` without doc comments.
- `build.rs`: bindgen on `$ARA_API_DIR/ARAInterface.h`.
- `Info.plist`: the plist of the CLAP bundle (`DrumSpikeAraSpike.clap/Contents/`; the library goes to `Contents/MacOS/DrumSpikeAraSpike`).
- `patches/`: the local changes of the ARA examples 2.3.0 and of `clap-wrapper` 0.17.0 (see the findings).
- `check.py`: makes the fixtures and compares. `spike.sh`: runs everything.

## What the check proves, on both paths

1. The plugin reads every sample of four sources (48, 44.1 and 96 kHz, mono and stereo) through ARA, on a background thread. The hash of all samples equals the hash of the fixture.
2. The plugin gives its hits back to the host as ARA notes: 14 notes, each at the exact sample.
3. The host renders two regions back to back through a plugin instance. The host's buffer holds one click per hit, 0 samples difference, and no other non-zero sample.
4. The host tests EditorView, PropertyUpdates and Archiving exit 0.
5. The check can fail: a hit moved by one sample, a changed hash digit, a moved click and a missing CLAP each gave FAIL.

The detector of the spike is a threshold on channel 0. It is not the Drum Spike detector.

## Build notes

These are the commands of the first run, on Apple silicon with the command line tools and no Xcode. No script does them yet: that is work of the ticket. `spike.sh` expects the built parts in `.deps/` (or in the folder that `DS_ARA_DEPS` names).

- ARA SDK: release 2.3.0 with its submodules; its scripts fetch the CLAP and VST3 SDKs. Apply `patches/ara-examples-2.3.0.patch` in `ARA_Examples`.
- Test host: `cmake -S ARA_SDK/ARA_Examples -B build-examples -DARA_SETUP_DEBUGGING=OFF -DCMAKE_BUILD_TYPE=Debug -DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_OSX_SYSROOT=<versioned macOS SDK path> -DXCODE_VERSION=16.0`, with `XCODE_VERSION=16.0` and `SDKROOT` also in the environment. Always pass `-DARA_SETUP_DEBUGGING=OFF`: the default writes symlinks into the user's plugin folder.
- Plugin: `ARA_API_DIR=<ARA_SDK>/ARA_API LIBCLANG_PATH=<command line tools>/usr/lib cargo build`, then put the library into the bundle.
- VST3: `clap-wrapper` 0.17.0 with `patches/clap-wrapper-0.17.0.patch`, `-DCMAKE_OSX_DEPLOYMENT_TARGET=12.0 -DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=TRUE -DCLAP_WRAPPER_OUTPUT_NAME="DrumSpikeAraSpike"`. The wrapper finds the CLAP through `CLAP_PATH`.

## Findings

1. ARA rule: an audio reader is created and destroyed on the host's main thread. Only the reads run on another thread.
2. `clap-wrapper` reads the VST3 host context without a null check (`ClapAsVst3::initialize`). The ARA test host passes null. A real DAW passes a context. Worth a report upstream.
3. `clap-wrapper` renders only after the VST3 call `setProcessing(true)`. The ARA test host does not make that call. Real DAWs make it.
4. The WAV loader of the test host loops forever on a file without an iXML chunk. The fixtures carry one.
5. The test host loader of CLAP needs a mono in-place audio port.
6. The ARA part by hand is small: 15 real callbacks of the document controller, 39 generated trivial ones.

## Not covered

A web view in the plugin window, the app service inside the plugin, a real DAW, AAX, a universal binary, state in the session archive, and a region that is cut or moved.

## Licences

Our code here: Apache 2.0, as the repo. ARA SDK: Apache 2.0. `clap-wrapper`, CLAP, `clap-sys`: MIT. VST3 SDK: MIT. The patches change files of the ARA examples and of `clap-wrapper` and keep their licences.
