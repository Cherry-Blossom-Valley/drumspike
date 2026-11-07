//! Drum Spike service layer (P3-3a): the commands of the desktop app as plain Rust functions.
//!
//! [`Service`] holds the registry of open projects, the analysis job, the model and the
//! caches. Its methods (module [`commands`]) take plain arguments and return JSON with the
//! keys of the Python server's responses, or bytes. They have no Tauri type in their
//! signatures, so the tests run without a window and without a `WebView`, and the Intel Mac
//! host builds and tests this crate. `apps/desktop/src-tauri` holds the thin wrappers; the test bridge
//! of P3-7a (`bridge.rs`, `crates/devtools/src/bin/ds-bridge.rs`) calls the same methods.
//!
//! Errors are [`AppError`] with the server's codes (400, 404, 409) and a message.

pub mod agent;
pub mod agent_skill;
pub mod api;
pub mod autosave;
pub mod bridge;
pub mod call;
pub mod candidates;
pub mod cli;
pub mod command_policy;
pub mod commands;
pub mod confidence;
pub mod copyjob;
pub mod cross_bleed;
pub mod edit;
pub mod error;
pub mod export;
pub mod feedback;
pub mod flam;
pub mod follow;
pub mod ghost_gate;
pub mod headless;
pub mod histogram;
pub mod import;
pub mod jobs;
pub mod lanes;
// P4-7a and P4-7b: the one-shot layer of a track.
pub mod layer;
pub mod learn;
pub mod learn_filter;
pub mod learn_score;
pub mod lifecycle;
pub mod lock;
pub mod markers;
pub mod mcp;
pub mod media;
pub mod overlap;
pub mod patterns;
pub mod plugins;
pub mod policy;
pub mod reanalyze;
pub mod registry;
pub mod relink;
pub mod reply;
pub mod roles;
pub mod rowheights;
// P4-7c: the sampler calibration of a track and the MIDI placement that uses it.
pub mod sampler;
pub mod sensitivity;
pub mod shell;
// R2: the fixed session steps of the analysis job.
pub(crate) mod steps;
pub mod tempo;
pub mod tom_bleed;
pub mod trace;
pub mod tracks;
pub mod training;
pub mod transport;

use std::path::Path;
use std::sync::Arc;

use drumspike_core::refine::RefineRule;
use drumspike_core::rules::{ResearchOverrides, RuleSet};

use serde_json::Value;

pub use commands::{CHAIN, COMMANDS, DETECTOR, ListShape, app_info_value};
pub use copyjob::COPYJOB_COMMANDS;
pub use edit::EDIT_COMMANDS;
pub use error::AppError;
pub use histogram::HISTOGRAM_COMMANDS;
pub use import::IMPORT_COMMANDS;
pub use layer::LAYER_COMMANDS;
pub use learn::LEARN_COMMANDS;
pub use lifecycle::LIFECYCLE_COMMANDS;
pub use relink::RELINK_COMMANDS;
pub use sampler::SAMPLER_COMMANDS;
pub use shell::SHELL_COMMANDS;
pub use tempo::TEMPO_COMMANDS;
pub use tracks::TRACKS_COMMANDS;
pub use transport::{DeviceKey, TRANSPORT_COMMANDS};

/// A progress callback: gets `{"sid", "job"}` after every change of an analysis job, where
/// `job` is the value `analyze_status` returns. The Tauri wrapper sends it on a `Channel`.
pub type Progress = Arc<dyn Fn(Value) + Send + Sync>;

/// Where the service finds its files. [`Default`] pins nothing (every rule at its current or
/// recorded version).
#[derive(Debug, Clone, Default)]
pub struct ServiceConfig {
    /// P4-12: a pinned refinement rule for every analysis and re-pick (`DRUMSPIKE_REFINE_RULE`,
    /// `1`, `2` or `3`; the parity checks pin the frozen engine's rule 1). `None`: a new analysis runs
    /// [`RefineRule::CURRENT`], a re-pick the rule its analysis recorded ([`jobs::recorded_rule`]).
    pub refine_rule: Option<RefineRule>,
    /// P4-14: a pinned tom bleed rule version for every analysis and re-pick (`DRUMSPIKE_BLEED_RULE`, `0` for no
    /// rule, `1` for rule CD, `2` for rule CD with the kick and snare sources, P4-17, `3` with the tom pair rule, P4-21). `None`: a new analysis runs [`tom_bleed::BLEED_RULE`], a re-pick the version its
    /// analysis recorded ([`tom_bleed::recorded_bleed_rule`]).
    pub bleed_rule: Option<u32>,
    /// P4-13 stage B: a pinned flam rule version for every analysis and re-pick (`DRUMSPIKE_FLAM_RULE`, `0` for no
    /// rule, `1` for the snare flam rule). `None`: a new analysis runs [`flam::FLAM_RULE`], a re-pick the version its
    /// analysis recorded ([`flam::recorded_flam_rule`]).
    pub flam_rule: Option<u32>,
    /// #281 A6: a pinned cross-track bleed rule for every analysis (`DRUMSPIKE_CROSS_BLEED`, `0`
    /// for no rule, `1` for the kick and snare sources). `None`: a new analysis runs
    /// [`crate::cross_bleed::CROSS_BLEED`].
    pub cross_bleed: Option<u32>,
    /// #281 A1: a pinned snare ghost-note gate for every analysis (`DRUMSPIKE_GHOST_GATE`,
    /// `0` for no clearance). `None`: a new analysis runs [`ghost_gate::GHOST_GATE`].
    pub ghost_gate: Option<u32>,
    /// R6: the onset delta rule and the tempo referee of every analysis and tempo detection
    /// (`DRUMSPIKE_DELTA_RULE`, `DRUMSPIKE_TEMPO_RULE`; unset: the current versions). The project
    /// records neither yet, so the pin or the current version decides.
    pub rules: RuleSet,
    /// R6: the A1, A5 and A6 research numbers (`DRUMSPIKE_A1_*`, `DRUMSPIKE_A5_*`,
    /// `DRUMSPIKE_A6_*`, `DRUMSPIKE_DELTA_REF_*`); [`Default`] overrides nothing.
    pub research: ResearchOverrides,
}

impl ServiceConfig {
    /// [`ServiceConfig::from_lookup`] over the process environment: the one place the service
    /// reads its rule pins and research numbers (R6).
    #[must_use]
    pub fn from_env(_models_dir: &Path) -> ServiceConfig {
        ServiceConfig::from_lookup(|name| std::env::var(name).ok())
    }

    /// The rule pins and the research numbers from a variable source: the refinement pin from
    /// `DRUMSPIKE_REFINE_RULE` ([`jobs::parse_rule`]), the session-rule pins
    /// (`DRUMSPIKE_BLEED_RULE`, `DRUMSPIKE_FLAM_RULE`, `DRUMSPIKE_CROSS_BLEED`,
    /// `DRUMSPIKE_GHOST_GATE`), the [`RuleSet`] and the [`ResearchOverrides`].
    #[must_use]
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> ServiceConfig {
        ServiceConfig {
            refine_rule: get("DRUMSPIKE_REFINE_RULE").and_then(|v| jobs::parse_rule(&v)),
            bleed_rule: get("DRUMSPIKE_BLEED_RULE").and_then(|v| tom_bleed::parse_bleed_rule(&v)),
            flam_rule: get("DRUMSPIKE_FLAM_RULE").and_then(|v| flam::parse_flam_rule(&v)),
            cross_bleed: get("DRUMSPIKE_CROSS_BLEED").and_then(|v| v.trim().parse::<u32>().ok()),
            ghost_gate: get("DRUMSPIKE_GHOST_GATE").and_then(|v| v.trim().parse::<u32>().ok()),
            rules: RuleSet::from_lookup(&get),
            research: ResearchOverrides::from_lookup(&get),
        }
    }
}

pub(crate) struct Inner {
    pub(crate) config: ServiceConfig,
    pub(crate) registry: registry::Registry,
    pub(crate) jobs: jobs::Jobs,
    pub(crate) detector: jobs::DetectorSlot,
    pub(crate) media: media::Media,
    pub(crate) life: lifecycle::State,
    pub(crate) copyjobs: copyjob::CopyJobs,
    pub(crate) transport: transport::TransportHost,
    /// P4-7a: the decoded one-shots and the per-hit alignments of the layers.
    pub(crate) layers: layer::LayerState,
    /// P4-5: the name this process puts in the lock files (`app`, `cli`, `mcp`); `None`: no project lock.
    pub(crate) lock_client: std::sync::Mutex<Option<String>>,
    /// #343: the Training Mode state per open project (`training.rs`).
    pub(crate) training: training::State,
}

/// The state of the app: cheap to clone (one `Arc`), shared by every command and by the
/// analysis worker threads.
#[derive(Clone)]
pub struct Service {
    pub(crate) inner: Arc<Inner>,
}

impl Service {
    /// A service with no open project.
    #[must_use]
    pub fn new(config: ServiceConfig) -> Service {
        let inner = Arc::new(Inner {
            config,
            registry: registry::Registry::new(),
            jobs: jobs::Jobs::default(),
            detector: jobs::DetectorSlot,
            media: media::Media::new(),
            life: lifecycle::State::from_env(),
            copyjobs: copyjob::CopyJobs::default(),
            transport: transport::TransportHost::new(),
            layers: layer::LayerState::default(),
            lock_client: std::sync::Mutex::new(None),
            training: training::State::default(),
        });
        autosave::spawn(&inner);
        Service { inner }
    }

    /// P4-5: lock every project this service opens for writing (`lock::ProjectLock`) and name this process
    /// `client` in the lock files. The app, the command-line tool and the MCP server call it; the tests do not.
    pub fn set_lock_client(&self, client: &str) {
        *registry::lock(&self.inner.lock_client) = Some(client.to_string());
    }

    /// The lock client name, when locking is on.
    #[must_use]
    pub fn lock_client(&self) -> Option<String> {
        registry::lock(&self.inner.lock_client).clone()
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &ServiceConfig {
        &self.inner.config
    }

    /// The analysis jobs and the re-pick stores (P4-3 fix round: the tests mark a store).
    #[must_use]
    pub fn jobs(&self) -> &jobs::Jobs {
        &self.inner.jobs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config from one variable, as `from_env` builds it from the process environment.
    fn with(name: &str, value: &str) -> ServiceConfig {
        ServiceConfig::from_lookup(|n| (n == name).then(|| value.to_string()))
    }

    #[test]
    fn the_config_reads_each_rule_pin_by_its_variable_name() {
        let none = ServiceConfig::from_lookup(|_| None);
        assert_eq!(none.refine_rule, None);
        assert_eq!(none.bleed_rule, None);
        // P4-12: the refinement pin
        assert_eq!(
            with("DRUMSPIKE_REFINE_RULE", "1").refine_rule,
            Some(RefineRule::Reference)
        );
        assert_eq!(with("DRUMSPIKE_REFINE_RULE", "2").refine_rule, Some(RefineRule::Reach));
        assert_eq!(
            with("DRUMSPIKE_REFINE_RULE", "3").refine_rule,
            Some(RefineRule::FirstMotion)
        );
        assert_eq!(with("DRUMSPIKE_REFINE_RULE", "x").refine_rule, None);
        // P4-14: the bleed rule pin
        assert_eq!(with("DRUMSPIKE_BLEED_RULE", "0").bleed_rule, Some(0));
        assert_eq!(with("DRUMSPIKE_BLEED_RULE", "1").bleed_rule, Some(1));
        assert_eq!(
            with("DRUMSPIKE_BLEED_RULE", "7").bleed_rule,
            None,
            "a version this build does not know"
        );
        // R6: the delta rule, the tempo referee and a research number
        assert_eq!(none.rules, RuleSet::default());
        assert_eq!(none.research, ResearchOverrides::default());
        assert_eq!(
            with("DRUMSPIKE_DELTA_RULE", "0").rules.delta,
            drumspike_core::rules::DeltaRule::Absolute
        );
        assert_eq!(
            with("DRUMSPIKE_TEMPO_RULE", "2").rules.tempo,
            drumspike_core::rules::TempoRule::Corroborated
        );
        assert_eq!(with("DRUMSPIKE_A5_GAIN_ATTACK", "0").research.a5_gain_attack, Some(0.0));
    }

    #[test]
    fn a_new_service_has_no_project_and_an_idle_job() {
        let s = Service::new(ServiceConfig::default());
        assert_eq!(s.project_list(ListShape::Sessions), serde_json::json!([]));
        assert_eq!(s.analyze_status("x")["state"], "idle");
        assert!(!s.jobs().running("x") && !s.jobs().storing("x"));
        assert_eq!(s.project_get("x").unwrap_err().code, 404);
    }
}
