//! Phase 2: onset detection in Rust (`docs/archive/phase2-detection-prompt.md`).
//!
//! The detector runs on a 44.1 kHz mono signal. The reader gives the native rate, so the
//! chain starts with [`resample`], then the log-filtered spectrogram ([`features`]) and the
//! peak picking ([`peaks`]). The DSP onset detector ([`onset`]) runs on the same
//! spectrogram. The attribution flags ([`attribution`], off by default) mark bleed and
//! resonance hits; they never remove a hit. The tom bleed rule ([`tom_bleed`]) marks the hits of a tom
//! close mic that the other toms' mics explain; the app takes them out of the output. The flam rule ([`flam`])
//! adds the second stroke of a flam inside a hit's lobe on a snare mic.
//!
//! Two reference chains exist (`docs/archive/phase2-report.md`, section "P2-1c R3 vs CLI
//! gap"). [`Chain`] selects one of them, or the mix of the CLI features and the madmom peak
//! picking (the default), for [`features`] and [`peaks`].

pub mod attribution;
pub mod cross_bleed;
pub mod features;
pub mod flam;
pub mod nmfd;
pub mod onset;
pub mod peaks;
pub mod resample;
pub mod tom_bleed;

use std::fmt;

use resample::ResampleError;

/// The number of classes the peak picking distinguishes (BD, SD, TT, HH, CY+RD). The class
/// count of the retired model era stays for the stored documents, the candidates and the similarity
/// recipes; there is no model on main any more.
pub const N_CLASSES: usize = 5;

/// The MIDI notes of the five model classes (BD, SD, TT, HH, CY+RD), as the reference detector
/// writes them (the CLI reference implementation, `LABELS_5`).
pub const CLASS_NOTES: [u8; N_CLASSES] = [35, 38, 47, 42, 49];

/// Errors of [`detect`]: the source rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectError {
    /// The source rate is not 44.1, 48 or 96 kHz.
    Resample(ResampleError),
}

impl fmt::Display for DetectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DetectError::Resample(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for DetectError {}

impl From<ResampleError> for DetectError {
    fn from(e: ResampleError) -> Self {
        DetectError::Resample(e)
    }
}

/// The preparation of the detector (P3-15b): [`resample`] to 44.1 kHz and the chain's
/// [`features`] (`(T, 84)` row-major).
///
/// # Errors
///
/// [`DetectError::Resample`] when `sr` is not 44.1, 48 or 96 kHz.
pub fn prepare(signal: &[f32], sr: u32, chain: Chain) -> Result<Vec<f32>, DetectError> {
    let y = resample::resample_to_44100(signal, sr)?;
    Ok(features::features(&y, chain.feature_recipe()))
}

/// The reference chain that [`features`] and [`peaks`] follow.
///
/// * [`Chain::Cli`]: the CLI reference (the `PyTorch` port
///   the CLI reference: `AudioProcessor` with the filterbank grid anchored at fmin = 20 Hz,
///   `1 + len / 441` frames, and the port's own `PeakPicker`). The Rust detector equals the
///   CLI with this chain.
/// * [`Chain::Madmom`]: the original reference pipeline on madmom 27f032e
///   (`LogarithmicFilteredSpectrogram` with the grid anchored at A4 = 440 Hz,
///   `ceil(len / 441)` frames, and `NotePeakPickingProcessor`), the features the reference
///   detector was trained on.
/// * [`Chain::Mix`] (the default): the CLI features with madmom's peak picking. Card P2-1d
///   (`docs/archive/phase2-report.md`, section "P2-1d Which chain is more precise") found
///   the two grids equally precise and madmom's picker closer to the attack in 965 of the
///   1,250 frames where the pickers differ; the controller chose this mix as the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Chain {
    /// The CLI reference chain.
    Cli,
    /// The madmom chain.
    Madmom,
    /// The CLI features with the madmom peak picking.
    #[default]
    Mix,
}

impl Chain {
    /// The three chains, in a fixed order.
    pub const ALL: [Chain; 3] = [Chain::Cli, Chain::Madmom, Chain::Mix];

    /// Parse `cli`, `madmom` or `mix` (case-insensitive).
    #[must_use]
    pub fn parse(s: &str) -> Option<Chain> {
        match s.to_ascii_lowercase().as_str() {
            "cli" => Some(Chain::Cli),
            "madmom" => Some(Chain::Madmom),
            "mix" => Some(Chain::Mix),
            _ => None,
        }
    }

    /// The name that [`Chain::parse`] reads.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Chain::Cli => "cli",
            Chain::Madmom => "madmom",
            Chain::Mix => "mix",
        }
    }

    /// The chain whose feature recipe this chain uses ([`Chain::Cli`] or [`Chain::Madmom`]).
    #[must_use]
    pub fn feature_recipe(self) -> Chain {
        match self {
            Chain::Madmom => Chain::Madmom,
            Chain::Cli | Chain::Mix => Chain::Cli,
        }
    }

    /// The chain whose peak picker this chain uses ([`Chain::Cli`] or [`Chain::Madmom`]).
    #[must_use]
    pub fn picker(self) -> Chain {
        match self {
            Chain::Cli => Chain::Cli,
            Chain::Madmom | Chain::Mix => Chain::Madmom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kicks like `tests/helpers.kick_hit` (125 -> 55 Hz sweep, 0.5 ms attack, 0.12 s decay)
    /// at `onsets` seconds, -3 dB, at `sr`.
    fn kicks(onsets: &[f64], dur: f64, sr: u32) -> Vec<f32> {
        let n = (dur * f64::from(sr)) as usize;
        let mut x = vec![0f32; n];
        for &t0 in onsets {
            let i0 = (t0 * f64::from(sr)).round() as usize;
            let mut ph = 0.0f64;
            for k in 0..((0.4 * f64::from(sr)) as usize).min(n - i0) {
                let t = k as f64 / f64::from(sr);
                ph += 2.0 * std::f64::consts::PI * (55.0 + 70.0 * (-t / 0.03).exp()) / f64::from(sr);
                let env = (t / 0.0005).min(1.0) * (-t / 0.12).exp();
                x[i0 + k] += (ph.sin() * env * 10f64.powf(-3.0 / 20.0)) as f32;
            }
        }
        x
    }

    #[test]
    fn detect_errors_name_the_cause() {
        let e: DetectError = ResampleError::UnsupportedRate(22_050).into();
        assert_eq!(e, DetectError::Resample(ResampleError::UnsupportedRate(22_050)));
        assert!(e.to_string().contains("22050"), "{e}");
        assert_eq!(CLASS_NOTES, [35, 38, 47, 42, 49]);
    }

    #[test]
    fn prepare_is_the_resample_and_the_features_and_rejects_an_unknown_rate() {
        let x = kicks(&[0.25], 1.0, 48_000);
        for chain in Chain::ALL {
            let f = prepare(&x, 48_000, chain).unwrap();
            let y = resample::resample_to_44100(&x, 48_000).unwrap();
            assert_eq!(f, features::features(&y, chain.feature_recipe()), "{chain:?}");
            assert_eq!(f.len() % features::N_BANDS, 0);
        }
        // 44.1 kHz: no resampling, the features of the signal itself
        let x = kicks(&[0.25], 1.0, 44_100);
        assert_eq!(
            prepare(&x, 44_100, Chain::Mix).unwrap(),
            features::features(&x, Chain::Cli)
        );
        assert_eq!(
            prepare(&[0.0; 100], 22_050, Chain::Mix).unwrap_err(),
            DetectError::Resample(ResampleError::UnsupportedRate(22_050))
        );
    }

    #[test]
    fn chain_names_round_trip_and_mix_is_the_default() {
        for c in Chain::ALL {
            assert_eq!(Chain::parse(c.name()), Some(c));
            assert_eq!(Chain::parse(&c.name().to_uppercase()), Some(c));
        }
        assert_eq!(Chain::parse("librosa"), None);
        assert_eq!(Chain::default(), Chain::Mix);
    }

    #[test]
    fn mix_takes_the_cli_features_and_the_madmom_picker() {
        assert_eq!(Chain::Mix.feature_recipe(), Chain::Cli);
        assert_eq!(Chain::Mix.picker(), Chain::Madmom);
        for c in [Chain::Cli, Chain::Madmom] {
            assert_eq!((c.feature_recipe(), c.picker()), (c, c));
        }
    }
}
