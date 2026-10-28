# drumspike-core

The application logic of Drum Spike in Rust, without any UI. The crate reproduced `engine.py`
and `project.py` of the Python server on the test corpus; the server was retired in P3-12 and its
results are frozen in `tests/golden/`. The desktop app uses this crate through `crates/app`.

## Public API by module

| Module | Content |
|---|---|
| `io` | WAV/BWF reader at the native rate (PCM 16/24/32, float 32, `bext`, iXML, extensible header): `WavInfo::parse`, `read_wav_mono`, `read_wav_bytes`, `MappedWav`, `decode_many` (parallel). `MixMode` selects the downmix (`ffmpeg` or `average`). A WAV writer: `write_wav`, `encode_wav` |
| `peaks` | Min/max/RMS pyramid: `compute_peaks`, `compute_peaks_levels`. `Track` with the tile API `peaks(level, from, to)` and `samples(from, to)` |
| `velocity` | `peak_in`, `velocity_map` (level in dB to MIDI velocity) |
| `refine` | Onset refinement: `refine_sr`, `refine_track` with the search window `RefineWindow` (default -2..+12 ms around the detector time, P2-9), `hilbert_env`, `recompute_velocities` |
| `midi` | `export_midi`, `midi_bytes`, `note_events` (24,000 PPQ, same bytes as `engine.export_midi`) |
| `spike` | Spike WAV export: `spike_signal`, `export_spikes` (one full-scale blip per hit) |
| `project` | `.drumspike` document (format 1): `load_project`, `new_project`, `save_project`, `save_as`, `consolidate`, migration from format 0, source records and their resolution, backup rotation, `Analysis/` cache key |
| `detect` | Detection (phase 2): `detect::{resample, features, peaks, onset, attribution}`. `resample` (polyphase FIR to 44.1 kHz), `features` (CLI and madmom recipes), `peaks` (CLI and madmom pickers), `onset` (the DSP detector), `attribution` (bleed and resonance flags, off by default); `detect::detect` runs the whole chain |
| `pyops` | Python number rules (`int()`, `round()`, slice clamp) that every port shares |

Every public item has a doc comment (`#![warn(missing_docs)]`). Run `cargo doc -p drumspike-core --open`.

## Build and test

```bash
cargo build --release -p drumspike-core        # the library
cargo test -p drumspike-core
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

- `crates/devtools/src/bin/parity.rs`: the binary the parity harnesses call (LAYOUT 2: the tool binaries live in `crates/devtools`).
- `crates/devtools/src/bin/bench.rs`: the benchmark. `./target/release/bench [--tracks 30] [--seconds 300] [--format pcm24|pcm16|float32] [--runs 5]`.
- `tests/ffmpeg_downmix.rs`: compares the downmix with ffmpeg's frozen output (no ffmpeg needed).
- `tests/project_cross_load.rs`: documents written by the retired Python layer (frozen) load in Rust.
- `tools/parity/` (repository root): the Python harnesses against the frozen references (`make_corpus.py`, `check_parity.py`,
  `check_refine.py`, `check_midi.py`, `check_spike.py`, `check_analyze.py`). The
  usage is in each docstring.

## Rules

- The frozen results of the retired Python server are the reference. Every change keeps the parity
  harnesses and the golden files (`tests/test_golden.py`, `tests/golden/frozen/`) green.
- The reader never writes. Source audio stays unchanged.
- Dependencies have exact versions. `Cargo.lock` is committed. No C or C++ dependency.
- `unsafe` appears only in the memory-map layer, with a comment that says why.
- Known differences from Python are listed under Deviations in `docs/archive/phase1-report.md`.
