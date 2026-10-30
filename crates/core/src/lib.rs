//! `drumspike-core`: the application logic of Drum Spike without any UI.
//!
//! The crate reproduces the Python engine (`engine.py`) bit for bit on the test corpus;
//! parity is proven by the harness under `tools/parity/`, not assumed. Modules:
//!
//! * [`io`]: memory-mapped WAV/BWF reader at native rate (PCM 16/24/32, IEEE float 32,
//!   `bext`, iXML, WAVE_FORMAT_EXTENSIBLE), the explicit [`MixMode`] downmix, a WAV writer
//!   and a rayon helper that decodes several files in parallel.
//! * [`peaks`]: the min/max/RMS pyramid of `engine.compute_peaks` and the tile API the
//!   waveform view reads (`peaks(level, from, to)`, `samples(from, to)`).
//! * [`velocity`]: `engine.peak_in` and `engine.velocity_map`.
//! * [`refine`]: the hybrid onset refinement (`engine._refine_sr`, `refine_track` with the
//!   neighbours `analyze_track` passes, the search window [`RefineWindow`]),
//!   `recompute_velocities`, and the rule versions [`RefineRule`] (P4-12: `refine_reach`).
//! * [`midi`]: `engine.export_midi` byte for byte (midly, 24,000 PPQ, 120 BPM).
//! * [`spike`]: the spike WAV export (a full-scale blip per hit at the native rate, the
//!   source's `bext` time reference).
//! * [`project`]: the `.drumspike` document (format 1) with load, migration from format 0,
//!   atomic save with the backup rotation, source records and their resolution order,
//!   copies into `Audio Files/`, consolidate and the `Analysis/` cache key (`project.py`).
//! * [`cache`]: the binary peak cache of the app, `Analysis/<track>/peaks.bin` (P3-2b).
//! * `transport` (feature `audio`): sample-exact native playback of a session (P3-5a).
//! * [`detect`]: phase 2, onset detection; today the fixed-ratio resampler to 44.1 kHz
//!   (`detect::resample`, P2-2a), the rest follows card by card.
//!
//! Rules: the reader never writes (I3); `unsafe` appears only in the mmap layer with a
//! comment that says why; Python numeric semantics (`int()` truncation, `round()` ties to
//! even, slice clamping) live in [`pyops`] so every port uses the same helpers.

#![warn(missing_docs)]

/// Version of the `drumspike-core` crate (the workspace version); the app and the PyO3 module report it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod cache;
// P3-2a: the analysis pipeline after the detector (engine.analyze_track).
pub mod analyze;

pub mod agreement;
pub mod backend;
pub use analyze::{Analysis, AnalyzeError, AnalyzeParams, Detector, analyze_detected, analyze_track};
pub use backend::{BackendError, CandidateList, DetectedHit, DetectorBackend, DspBackend, ExternalDetector};

pub mod detect;
pub mod io;
// P4-7a and P4-7b: the one-shot layer of a track (read, onset, resample, alignment, render).
pub mod layer;
pub mod midi;
pub mod peaks;
pub mod project;
pub mod pyops;
pub mod refine;
// #281 item 10: the model-free role suggestion from a track's audio.
pub mod role;
pub mod sampler;
// #293: the Learn similarity, DSP-only (features of a candidate, the score against the examples).
pub mod similarity;
pub mod spike;
// P4-8: the tempo and meter map (the document's `tempo_map`, the MIDI export and import of it).
pub mod tempo;
// P4-8 stage B: a suggested tempo map from the kick and snare hits.
pub mod tempo_detect;
// P3-5a: native playback (cpal, rtrb, rubato); the app turns the feature on, the PyO3 module does not.
#[cfg(feature = "audio")]
pub mod transport;
pub mod velocity;

pub use cache::{CacheError, CacheTile, PeakCache, build_cache, cache_is_current};
pub use detect::resample::{ResampleError, Resampler, TARGET_RATE, resample_to_44100};
pub use io::{Bext, MappedWav, MixMode, SampleFormat, WavError, WavInfo, decode_many, read_wav_bytes, read_wav_mono};
pub use midi::{
    MidiError, MidiEvent, MidiMarker, MidiTrack, NoteEvent, export_midi, export_midi_map, export_multi_midi,
    midi_bytes, midi_bytes_map, multi_midi_bytes, note_events,
};
pub use peaks::{PEAK_LEVELS, PeakLevel, PeakTile, SampleWindow, Track, compute_peaks, compute_peaks_levels};
pub use project::{Document, Event, Project, ProjectError, Source, SourceStatus};
pub use refine::{
    BackStep, Hit, ReachStep, RefineRule, RefineWindow, Refined, first_motion, hilbert_env, recompute_velocities,
    refine_first_motion, refine_reach, refine_rule, refine_sr, refine_track, refine_track_rule,
};
pub use spike::{
    AUDIO_BEXT_DESCRIPTION, AUDIO_DEFAULT_S, AUDIO_FADE_S, AUDIO_MAX_S, AUDIO_MIN_S, AUDIO_PERIOD_WINDOW_S,
    audio_snippet_lengths, audio_spike_bext, audio_spike_signal, export_audio_spikes, export_spikes, period_samples,
    spike_bext, spike_signal,
};
pub use tempo::{TempoMap, TickMap, import_midi};
pub use velocity::{VelocityMap, peak_in, velocity_map};

#[cfg(test)]
mod version_tests {
    #[test]
    fn version_is_the_cargo_package_version() {
        assert_eq!(super::VERSION, env!("CARGO_PKG_VERSION"));
        assert!(!super::VERSION.is_empty());
    }
}
