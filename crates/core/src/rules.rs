//! The rule versions and the research numbers of one analysis (R6).
//!
//! Two kinds of input decide a detection besides the audio and the document parameters:
//!
//! - [`RuleSet`]: the versions of the rules that are not recorded in the project yet, the onset
//!   [`DeltaRule`] and the tempo referee [`TempoRule`]. A pin from the environment (tests,
//!   research) wins, else the current version. The refinement keeps its own
//!   [`crate::refine::RefineRule`]; the session rules keep their recorded versions in the app.
//! - [`ResearchOverrides`]: the typed A1, A5 and A6 numbers that a research run can set through
//!   `DRUMSPIKE_*` variables. `None` is the shipped value.
//!
//! The values are resolved once, at the edge (the app's `ServiceConfig::from_env`, a devtools
//! binary), and passed down as parameters. [`RuleSet::from_lookup`] and
//! [`ResearchOverrides::from_lookup`] parse a variable source; they hold the variable names and
//! the parse of each, so the names and their effect stay in one place.
//!
//! Parallel Change: the env readers [`RuleSet::from_env`] and [`ResearchOverrides::from_env`]
//! serve the old entry points (`OnsetParams::for_role`, `detect_onsets`, the `Default` of
//! `CrossBleedParams`) while they still have callers.

use crate::role::Role;

/// The onset delta rule (#281 A5, `DRUMSPIKE_DELTA_RULE`). The detector only asks "version 1 or
/// later", so every version from 1 up is [`DeltaRule::Gained`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeltaRule {
    /// Version 0: the shipped absolute `delta` (the frozen references pin it).
    Absolute,
    /// Version 1: a quiet track's spectrogram is gained up to the role's reference scale before
    /// the absolute `delta` applies (`detect::onset::gained_spec`). The product default.
    #[default]
    Gained,
}

impl DeltaRule {
    /// The rule of a new analysis.
    pub const CURRENT: DeltaRule = DeltaRule::Gained;

    /// The version number (0 or 1).
    #[must_use]
    pub fn version(self) -> u32 {
        match self {
            DeltaRule::Absolute => 0,
            DeltaRule::Gained => 1,
        }
    }

    /// The rule of a version number: 0 is [`DeltaRule::Absolute`], every other number
    /// [`DeltaRule::Gained`] (the detector compared `version >= 1`).
    #[must_use]
    pub fn from_version(v: u32) -> DeltaRule {
        if v >= 1 { DeltaRule::Gained } else { DeltaRule::Absolute }
    }

    /// The rule of a pin text (`DRUMSPIKE_DELTA_RULE`): unset or not a number is version 1.
    #[must_use]
    pub fn from_pin(v: Option<&str>) -> DeltaRule {
        DeltaRule::from_version(v.map_or(1, |v| v.parse().unwrap_or(1)))
    }
}

/// The tempo referee (#130, #180, `DRUMSPIKE_TEMPO_RULE`). The detection asks "version under 2"
/// and "version 3 or later", so three behaviours exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TempoRule {
    /// Versions 0 and 1: the shipped referee; with a snare the backbeat alone picks the octave.
    Shipped,
    /// Version 2: the #130 referee; the sub-beat shares corroborate the backbeat verdict.
    Corroborated,
    /// Version 3 and later: version 2 plus the #180 half-time bar 1. The product default.
    #[default]
    HalfTime,
}

impl TempoRule {
    /// The rule of a new detection.
    pub const CURRENT: TempoRule = TempoRule::HalfTime;

    /// The version number (1, 2 or 3).
    #[must_use]
    pub fn version(self) -> u32 {
        match self {
            TempoRule::Shipped => 1,
            TempoRule::Corroborated => 2,
            TempoRule::HalfTime => 3,
        }
    }

    /// The rule of a version number: 0 and 1 are [`TempoRule::Shipped`], 2 is
    /// [`TempoRule::Corroborated`], every larger number [`TempoRule::HalfTime`].
    #[must_use]
    pub fn from_version(v: u32) -> TempoRule {
        match v {
            0 | 1 => TempoRule::Shipped,
            2 => TempoRule::Corroborated,
            _ => TempoRule::HalfTime,
        }
    }

    /// The rule of a pin text (`DRUMSPIKE_TEMPO_RULE`): unset or not a number is version 3.
    #[must_use]
    pub fn from_pin(v: Option<&str>) -> TempoRule {
        TempoRule::from_version(v.map_or(3, |v| v.parse().unwrap_or(3)))
    }
}

/// The rule versions of one analysis or tempo detection that the project does not record yet.
/// [`Default`] is the current version of each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RuleSet {
    /// The onset delta rule.
    pub delta: DeltaRule,
    /// The tempo referee.
    pub tempo: TempoRule,
}

impl RuleSet {
    /// The rule set from a variable source (`DRUMSPIKE_DELTA_RULE`, `DRUMSPIKE_TEMPO_RULE`); a
    /// missing variable is the current version.
    #[must_use]
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> RuleSet {
        RuleSet {
            delta: DeltaRule::from_pin(get("DRUMSPIKE_DELTA_RULE").as_deref()),
            tempo: TempoRule::from_pin(get("DRUMSPIKE_TEMPO_RULE").as_deref()),
        }
    }

    /// [`RuleSet::from_lookup`] over the process environment.
    #[must_use]
    pub fn from_env() -> RuleSet {
        RuleSet::from_lookup(|n| std::env::var(n).ok())
    }
}

/// The per-role A5 reference contrast scales (`DRUMSPIKE_DELTA_REF_<ROLE>`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DeltaRefs {
    /// `DRUMSPIKE_DELTA_REF_KICK`.
    pub kick: Option<f32>,
    /// `DRUMSPIKE_DELTA_REF_SNARE`.
    pub snare: Option<f32>,
    /// `DRUMSPIKE_DELTA_REF_HAT`.
    pub hat: Option<f32>,
    /// `DRUMSPIKE_DELTA_REF_TOM`.
    pub tom: Option<f32>,
    /// `DRUMSPIKE_DELTA_REF_CYMBAL`.
    pub cymbal: Option<f32>,
    /// `DRUMSPIKE_DELTA_REF_OTHER`.
    pub other: Option<f32>,
}

impl DeltaRefs {
    /// The override of `role`.
    #[must_use]
    pub fn of(&self, role: Role) -> Option<f32> {
        match role {
            Role::Kick => self.kick,
            Role::Snare => self.snare,
            Role::Hat => self.hat,
            Role::Tom => self.tom,
            Role::Cymbal => self.cymbal,
            Role::Other => self.other,
        }
    }
}

/// The research numbers of the A1, A5 and A6 rules (#281). `None` is the shipped value, which
/// stays at the rule that uses it. [`Default`] overrides nothing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ResearchOverrides {
    /// A1 snare onset `delta` (`DRUMSPIKE_A1_DELTA`).
    pub a1_delta: Option<f32>,
    /// A1 soft gate similarity (`DRUMSPIKE_A1_SOFT_SIM`; 0 or less turns the soft gate off).
    pub a1_soft_sim: Option<f32>,
    /// A1 soft gate depth in dB (`DRUMSPIKE_A1_SOFT_DB`).
    pub a1_soft_db: Option<f32>,
    /// A1 second template similarity (`DRUMSPIKE_A1_SOFT_T2`; 0 or less is off).
    pub a1_soft_t2: Option<f32>,
    /// A1 soft rescue attack ceiling in dB (`DRUMSPIKE_A1_SOFT_ATTACK`).
    pub a1_soft_attack: Option<f32>,
    /// A1 overhead denial margin in dB (`DRUMSPIKE_A1_OH_MARGIN_DB`).
    pub a1_oh_margin_db: Option<f64>,
    /// A5 attack floor of a gain-dependent admit in dB (`DRUMSPIKE_A5_GAIN_ATTACK`).
    pub a5_gain_attack: Option<f32>,
    /// A5 minimum gain factor, all roles (`DRUMSPIKE_A5_QUIET_GAIN`).
    pub a5_quiet_gain: Option<f32>,
    /// A5 reference contrast scale per role.
    pub delta_ref: DeltaRefs,
    /// A6 fixed lead bound in ms (`DRUMSPIKE_A6_MIN_LEAD`).
    pub a6_min_lead: Option<f64>,
    /// A6 level test margin in dB (`DRUMSPIKE_A6_MARGIN_DB`).
    pub a6_margin_db: Option<f64>,
    /// #296 aligned-pair coincidence count (`DRUMSPIKE_A6_ALIGN_MIN_COINS`, read as a number).
    pub a6_align_min_coins: Option<f64>,
    /// #296 aligned-pair median |lead| in ms (`DRUMSPIKE_A6_ALIGN_MED_ABS_LEAD`).
    pub a6_align_med_abs_lead: Option<f64>,
}

impl ResearchOverrides {
    /// The overrides from a variable source. A missing variable or one that does not parse as a
    /// number overrides nothing (the shipped value stays), as before R6.
    #[must_use]
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> ResearchOverrides {
        let f32_of = |name: &str| get(name).and_then(|v| v.parse::<f32>().ok());
        let f64_of = |name: &str| get(name).and_then(|v| v.parse::<f64>().ok());
        ResearchOverrides {
            a1_delta: f32_of("DRUMSPIKE_A1_DELTA"),
            a1_soft_sim: f32_of("DRUMSPIKE_A1_SOFT_SIM"),
            a1_soft_db: f32_of("DRUMSPIKE_A1_SOFT_DB"),
            a1_soft_t2: f32_of("DRUMSPIKE_A1_SOFT_T2"),
            a1_soft_attack: f32_of("DRUMSPIKE_A1_SOFT_ATTACK"),
            a1_oh_margin_db: f64_of("DRUMSPIKE_A1_OH_MARGIN_DB"),
            a5_gain_attack: f32_of("DRUMSPIKE_A5_GAIN_ATTACK"),
            a5_quiet_gain: f32_of("DRUMSPIKE_A5_QUIET_GAIN"),
            delta_ref: DeltaRefs {
                kick: f32_of("DRUMSPIKE_DELTA_REF_KICK"),
                snare: f32_of("DRUMSPIKE_DELTA_REF_SNARE"),
                hat: f32_of("DRUMSPIKE_DELTA_REF_HAT"),
                tom: f32_of("DRUMSPIKE_DELTA_REF_TOM"),
                cymbal: f32_of("DRUMSPIKE_DELTA_REF_CYMBAL"),
                other: f32_of("DRUMSPIKE_DELTA_REF_OTHER"),
            },
            a6_min_lead: f64_of("DRUMSPIKE_A6_MIN_LEAD"),
            a6_margin_db: f64_of("DRUMSPIKE_A6_MARGIN_DB"),
            a6_align_min_coins: f64_of("DRUMSPIKE_A6_ALIGN_MIN_COINS"),
            a6_align_med_abs_lead: f64_of("DRUMSPIKE_A6_ALIGN_MED_ABS_LEAD"),
        }
    }

    /// [`ResearchOverrides::from_lookup`] over the process environment.
    #[must_use]
    pub fn from_env() -> ResearchOverrides {
        ResearchOverrides::from_lookup(|n| std::env::var(n).ok())
    }
}

#[cfg(test)]
mod tests;
