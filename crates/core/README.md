# drumspike-core

The application logic of Drum Spike in Rust, without any UI. The crate reproduces the results of the
retired Python server on the test corpus; those results are frozen in `tests/golden/`. The desktop app uses this crate through `crates/app`.

## Public API: the module paths

The module paths are the API. Callers write `drumspike_core::<module>::<item>`, for example
`drumspike_core::refine::refine_sr` or `drumspike_core::role::Role` (also re-exported as `detect::onset::Role`).

- An item that no code outside the crate uses is `pub(crate)` (R8b). "Outside" means `crates/app`,
  `crates/devtools`, `apps/desktop/src-tauri`, and the crate's own `src/bin` and `tests/` targets.
- The tuning constants and stage helpers of the detection rules (for example `refine::walk::FM_SPAN_S`)
  are crate-internal. `cargo doc -p drumspike-core --document-private-items` shows them.
- The crate root re-exports only the names that callers import as `drumspike_core::<name>` (R8c):
  `VERSION`, `CacheError`, `PeakCache`, `build_cache`, `MappedWav`, `MixMode`, `WavError`, `WavInfo`,
  `decode_many`, `read_wav_bytes`, `read_wav_mono`, `MidiError`, `Track` (the peak pyramid), `compute_peaks`,
  `Document`, `Event`, `Project`, `ProjectError`, `Source`, `SourceStatus`, `Hit`, `RefineWindow`,
  `refine_track`, `export_spikes`, `VelocityMap`, `velocity_map`, `AnalyzeParams`, `Detector`, `analyze_track`.
  New code imports from the module path.
- To make an item public, use it from outside the crate in the same change. To demote one, check that
  `cargo check --workspace --all-targets --features drumspike-core/audio` stays free of warnings.

| Module | Content |
|---|---|
| `io` | WAV/BWF reader at the native rate (PCM 16/24/32, float 32, `bext`, iXML, extensible header): `WavInfo::parse`, `read_wav_mono`, `read_wav_bytes`, `MappedWav`, `decode_many` (parallel). `MixMode` selects the downmix (`ffmpeg` or `average`). A WAV writer: `write_wav`, `encode_wav` |
| `peaks` | Min/max/RMS pyramid: `compute_peaks`. `Track` with the tile API `peaks(level, from, to)` and `samples(from, to)` |
| `cache` | The binary peak cache `Analysis/<track>/peaks.bin`: `PeakCache`, `build_cache`, `cache_is_current` |
| `velocity` | `velocity_map`, `VelocityMap` (level in dB to MIDI velocity) |
| `refine` | Onset refinement: `refine_sr`, `refine_track`, `refine_rule`, the rule versions `RefineRule`, the search window `RefineWindow` (default -2..+12 ms around the detector time, P2-9), `recompute_velocities`. `refine.rs` only re-exports; the code is in one private module per mechanism: `reference` (version 1, the search window), `reach` (version 2), `walk` (the first-motion back-track of versions 3 to 7), `edge` (the steep edge of versions 6 and 7), `track` (`RefineRule`, the dispatch to a rule, one hit and a whole track) |
| `analyze`, `backend` | One track from samples to events: `analyze_track`, `analyze_detected`, `AnalyzeParams`; the detector backends `DspBackend`, `ExternalDetector`, `DetectorBackend` |
| `detect` | Detection: `detect::{resample, features, peaks, onset, nmfd, attribution, cross_bleed, tom_bleed, flam}`. `resample` (polyphase FIR to 44.1 kHz), `onset` (the DSP detector, `detect_onsets`), `attribution` (bleed and resonance flags), `cross_bleed`, `tom_bleed` and `flam` (the session rules the app steps call; `cross_bleed` runs as phase functions over one crate-internal `CrossBleedContext`) |
| `dsp` | Filter and statistics primitives; `FilterKind`, `BandStage` and `median` are public, the rest is crate-internal |
| `role` | The one `Role` type (`parse_or_other` at the string edge, two note tables: `detector_note` and `default_note`), `role_note`, `class_note`, `role_classes`, and the role suggestion from audio (`role_from_audio`) |
| `rules` | The rule versions and research numbers of one analysis: `RuleSet` (`DeltaRule`, `TempoRule`) and `ResearchOverrides` (A1, A5, A6). `from_lookup` holds the `DRUMSPIKE_*` names; the caller resolves them once and passes them down |
| `similarity`, `agreement` | The Learn similarity score (`score`, `filter_detected`, `auto_threshold`); the detector-refine agreement |
| `midi` | `export_midi`, `midi_bytes`, `note_events` (24,000 PPQ, same bytes as `engine.export_midi`), `export_multi_midi` |
| `spike` | Spike WAV export: `spike_signal`, `export_spikes` (one full-scale blip per hit), `export_audio_spikes` |
| `layer`, `sampler` | The one-shot layer of a track (`read_one_shot`, `render_layer`); the sampler calibration (`measure_offset`) |
| `tempo`, `tempo_detect` | The tempo and meter map (`TempoMap`, `TickMap`, `import_midi`); the suggested tempo map (`tempo_detect::detect`) |
| `project` | `.drumspike` document (format 1): `load_project`, `new_project`, `save_project`, `save_as`, `consolidate`, migration from format 0, source records and their resolution, backup rotation, `Analysis/` cache key |
| `transport` | Native playback (feature `audio`): `Transport`, `Offline` |
| `pyops` | Python number rules (`int()`, `round()`, slice clamp) that every port shares |

Every public item has a doc comment (`#![warn(missing_docs)]`). Run `cargo doc -p drumspike-core --open`.

## Build and test

```bash
cargo build --release -p drumspike-core        # the library
cargo test -p drumspike-core
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

- `crates/devtools/src/bin/parity.rs`: the binary `tests/test_golden.py` calls (LAYOUT 2: the tool binaries live in `crates/devtools`).
- `crates/devtools/src/bin/bench.rs`: the benchmark. `./target/release/bench [--tracks 30] [--seconds 300] [--format pcm24|pcm16|float32] [--runs 5]`.
- `tests/ffmpeg_downmix.rs`: compares the downmix with ffmpeg's frozen output (no ffmpeg needed).
- `tests/project_cross_load.rs`: documents written by the retired Python layer (frozen) load in Rust.
- `tests/test_golden.py` (repository root): the golden regression against the frozen references (decode, peaks,
  refinement, velocities, MIDI and spike export, the analysis). `tools/parity/make_corpus.py` writes its synthetic corpus.

## Rules

- The frozen results of the retired Python server are the reference. Every change keeps the golden
  regression (`tests/test_golden.py`, `tests/golden/frozen/`) green.
- The reader never writes. Source audio stays unchanged.
- Dependencies have exact versions. `Cargo.lock` is committed. No C or C++ dependency.
- `unsafe` appears only in the memory-map layer, with a comment that says why.
