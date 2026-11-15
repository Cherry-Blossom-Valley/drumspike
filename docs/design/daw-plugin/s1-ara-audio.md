# S1: an ARA 2 plugin reads the audio of many tracks (#504)

Spike result: a CLAP plugin in Rust speaks ARA 2 through the plain C interface, and
`clap-wrapper` 0.17.0 carries it into a VST3. The plugin reads every sample of every audio
source of the document on a background thread, without playback, and returns its hits to the
host as ARA notes. This page carries the result table of the gate and the facts behind it.
It is a spike report: no product code.

## Result table

One row per question of the gate, one column per host. The agent fills the rows of the hosts
it can run; the coordinator fills the LUNA row after the merge.

| Question | Test host (ARA examples, script run) | Reaper | LUNA | Pro Tools |
|---|---|---|---|---|
| Does one instance see all tracks? | pending: filled from the check on this host | not on this host (no Reaper on x86-mac) | coordinator fills after the merge | n/a until Avid admits the plugin |
| How is the plugin added (track, event, take)? | pending | not on this host | coordinator fills after the merge | n/a until Avid admits the plugin |
| Are the playback regions correct after a clip is cut and moved? | pending | not on this host | coordinator fills after the merge | n/a until Avid admits the plugin |
| Is the tempo map filled? | pending | not on this host | coordinator fills after the merge | n/a until Avid admits the plugin |
| Is the audio readable before the first playback? | pending | not on this host | coordinator fills after the merge | n/a until Avid admits the plugin |

## What the script proves

`plugins/ara-spike/check_s1.sh` (exit 0 = pass):

1. It generates three WAV fixtures: 3 s mono at 48 kHz, 1 s stereo at 48 kHz, 2 s mono at
   96 kHz (different lengths, one at another sample rate), each with an iXML chunk.
2. The test host of the ARA examples loads the plugin - the CLAP and the wrapped VST3 - with
   `-test ContentReading` and no playback. The plugin creates its audio readers on the host's
   main thread, reads every sample on a background thread, and writes one JSON line per
   source: sample count, sample rate, channel count, FNV-1a hash of all samples.
3. The script walks each WAV file itself (RIFF `fmt ` and `data` chunks) and compares every
   value with what the plugin reported. It prints the number of sources it compared and
   fails on 0.
4. `pluginval` 1.0.4 at strictness 5 passes on the wrapped VST3; `clap-validator` 0.4.1
   passes on the CLAP (pluginval 1.0.4 has no CLAP support).
5. The standalone rule: `cargo metadata --no-deps` at the repo root does not name the spike
   crate, and the diff of this head touches no file outside `plugins/ara-spike/` and
   `docs/design/daw-plugin/`.

`./spike.sh all` (the one command of the folder README) runs the fetch, the build and this
check; the seed check of the README (four sources, the render check, EditorView,
PropertyUpdates, Archiving) runs after it.

## The ARA document controller works from Rust

The document controller is plain Rust: `build.rs` runs bindgen on `ARAInterface.h`, and
`src/lib.rs` implements the 15 real callbacks of the document controller by hand;
`gen_stubs.py` generates the 39 trivial ones from the bindgen output. No C++ part on the ARA
library of the SDK was needed. The audio readers are created and destroyed on the host's main
thread; only the reads run on another thread (the ARA rule; the compare fails a read that ran
on the host's calling thread).

## Dependencies and licences

- Our code in `plugins/ara-spike/`: Apache 2.0, as the repo.
- ARA SDK 2.3.0 (API, examples, test host): Apache 2.0.
- CLAP, `clap-sys`, `clap-wrapper` 0.17.0: MIT.
- VST3 SDK (fetched by the wrapper and the ARA examples): MIT.
- `clap-validator` 0.4.1: MIT (test tool only).
- `pluginval` 1.0.4: GPL-3.0 (test tool only, not linked into the plugin; its release zip is
  SHA-256 pinned in `spike.sh`).
- The two patches in `patches/` change files of the ARA examples and of `clap-wrapper` and
  keep their licences. No SDK file of Avid is in the repo.

## The two patches, and why

- `ara-examples-2.3.0.patch`: the examples build with the Makefile generator and without
  Xcode (the generator check is removed; OBJC/OBJCXX are enabled); the host side needs no
  AudioUnitSDK; the VST3 workflow of the host calls `setProcessing(true)` before `process()`;
  the first render can be dumped to a file for the external check.
- `clap-wrapper-0.17.0.patch`: `ClapAsVst3::initialize` reads the VST3 host context without a
  null check; the ARA test host passes null. Worth a report upstream.

## Not covered

A user interface, detection of the Drum Spike quality (the spike detector is a threshold on
channel 0), AAX, signing, a universal binary (each host builds natively), state in the
session archive, and a region that is cut or moved on a real DAW.
