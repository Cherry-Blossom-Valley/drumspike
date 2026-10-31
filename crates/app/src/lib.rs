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
pub mod plugins;
pub mod policy;
pub mod registry;
pub mod relink;
pub mod reply;
pub mod roles;
pub mod rowheights;
// P4-7c: the sampler calibration of a track and the MIDI placement that uses it.
pub mod sampler;
pub mod sensitivity;
pub mod shell;
pub mod tempo;
pub mod tom_bleed;
pub mod trace;
pub mod tracks;
pub mod transport;

use std::path::Path;
use std::sync::Arc;

use drumspike_core::refine::RefineRule;

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

/// Where the service finds its files.
#[derive(Debug, Clone)]
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
}

impl ServiceConfig {
    /// The refinement pin from `DRUMSPIKE_REFINE_RULE` ([`jobs::parse_rule`]).
    pub fn from_env(_models_dir: &Path) -> ServiceConfig {
        ServiceConfig {
            refine_rule: std::env::var("DRUMSPIKE_REFINE_RULE")
                .ok()
                .and_then(|v| jobs::parse_rule(&v)),
            bleed_rule: std::env::var("DRUMSPIKE_BLEED_RULE")
                .ok()
                .and_then(|v| tom_bleed::parse_bleed_rule(&v)),
            flam_rule: std::env::var("DRUMSPIKE_FLAM_RULE")
                .ok()
                .and_then(|v| flam::parse_flam_rule(&v)),
            cross_bleed: std::env::var("DRUMSPIKE_CROSS_BLEED")
                .ok()
                .and_then(|v| v.trim().parse::<u32>().ok()),
            ghost_gate: std::env::var("DRUMSPIKE_GHOST_GATE")
                .ok()
                .and_then(|v| v.trim().parse::<u32>().ok()),
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
}

/// The state of the app: cheap to clone (one `Arc`), shared by every command and by the
/// analysis worker threads.
#[derive(Clone)]
pub struct Service {
    pub(crate) inner: Arc<Inner>,
}

impl Service {
    /// A service with no open project.
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
    pub fn lock_client(&self) -> Option<String> {
        registry::lock(&self.inner.lock_client).clone()
    }

    /// The configuration.
    pub fn config(&self) -> &ServiceConfig {
        &self.inner.config
    }

    /// The analysis jobs and the re-pick stores (P4-3 fix round: the tests mark a store).
    pub fn jobs(&self) -> &jobs::Jobs {
        &self.inner.jobs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_follows_the_engine_rule() {
        let dir = std::env::temp_dir().join(format!("ds_app_cfg_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("probe.bin"), b"x").unwrap();
        // P4-12: the refinement pin (this test owns DRUMSPIKE_REFINE_RULE as well)
        unsafe {
            std::env::remove_var("DRUMSPIKE_REFINE_RULE");
        }
        assert_eq!(ServiceConfig::from_env(&dir).refine_rule, None);
        unsafe {
            std::env::set_var("DRUMSPIKE_REFINE_RULE", "1");
        }
        assert_eq!(ServiceConfig::from_env(&dir).refine_rule, Some(RefineRule::Reference));
        unsafe {
            std::env::set_var("DRUMSPIKE_REFINE_RULE", "2");
        }
        assert_eq!(ServiceConfig::from_env(&dir).refine_rule, Some(RefineRule::Reach));
        unsafe {
            std::env::set_var("DRUMSPIKE_REFINE_RULE", "3");
        }
        assert_eq!(ServiceConfig::from_env(&dir).refine_rule, Some(RefineRule::FirstMotion));
        unsafe {
            std::env::set_var("DRUMSPIKE_REFINE_RULE", "x");
        }
        assert_eq!(ServiceConfig::from_env(&dir).refine_rule, None);
        unsafe {
            std::env::remove_var("DRUMSPIKE_REFINE_RULE");
        }
        // P4-14: the bleed rule pin (this test owns DRUMSPIKE_BLEED_RULE as well)
        unsafe {
            std::env::remove_var("DRUMSPIKE_BLEED_RULE");
        }
        assert_eq!(ServiceConfig::from_env(&dir).bleed_rule, None);
        unsafe {
            std::env::set_var("DRUMSPIKE_BLEED_RULE", "0");
        }
        assert_eq!(ServiceConfig::from_env(&dir).bleed_rule, Some(0));
        unsafe {
            std::env::set_var("DRUMSPIKE_BLEED_RULE", "1");
        }
        assert_eq!(ServiceConfig::from_env(&dir).bleed_rule, Some(1));
        unsafe {
            std::env::set_var("DRUMSPIKE_BLEED_RULE", "7");
        }
        assert_eq!(
            ServiceConfig::from_env(&dir).bleed_rule,
            None,
            "a version this build does not know"
        );
        unsafe {
            std::env::remove_var("DRUMSPIKE_BLEED_RULE");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_new_service_has_no_project_and_an_idle_job() {
        let s = Service::new(ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        assert_eq!(s.project_list(ListShape::Sessions), serde_json::json!([]));
        assert_eq!(s.analyze_status("x")["state"], "idle");
        assert!(!s.jobs().running("x") && !s.jobs().storing("x"));
        assert_eq!(s.project_get("x").unwrap_err().code, 404);
    }
}
