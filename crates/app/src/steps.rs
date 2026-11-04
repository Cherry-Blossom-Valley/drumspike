//! The session steps of the analysis job (R2, Branch by Abstraction): after every track of a job is stored,
//! [`ANALYSIS_STEPS`] run in a fixed order over the analysed tracks.
//!
//! The order is part of the rules: #281 A6, the cross-track bleed rule, runs first (it re-detects the tom targets; the
//! tom bleed rule judges the hits it leaves). #281 A1, the snare ghost-note gate's busy clearance, is next (it
//! re-detects the snare targets with the soft rescue denied to the busy candidates; the flam rule and the snare rescue
//! judge the hits it leaves). P4-14, the tom bleed rule, follows over the session's tom close mics, and P4-13 stage B,
//! the flam rule on the analysed snare close mics, comes last, after the bleed rules.
//!
//! This list is for the analysis job only. The re-pick (`histogram.rs`) runs its own explicit tom bleed and flam
//! calls with the recorded versions, and Reanalyze selection runs no session step.

use serde_json::{Value, json};

use crate::registry::Shared;
use crate::{Service, ServiceConfig};

/// A step over track names (`run_step` of the cross-track bleed rule and the ghost gate).
type RunNames = fn(&Service, &Shared, &[String]) -> Option<Value>;

/// A step over track names with a version each (`run_step` of the tom bleed rule and the flam rule).
type RunVersioned = fn(&Service, &Shared, &[(String, u32)]) -> Option<Value>;

/// How a step takes the analysed tracks.
#[derive(Clone, Copy)]
enum Run {
    /// The track names; the job runs the step only when its analysis version is above 0.
    Names(RunNames),
    /// Each track name with the analysis version; the step runs for every version (version 0 clears its record).
    Versioned(RunVersioned),
}

/// One session step of the analysis job.
pub(crate) struct AnalysisStep {
    /// The key of the step's outcome in the job answer and in each track's entry of it.
    pub(crate) id: &'static str,
    /// The version a new analysis runs: the configuration's pin, else the step's current version.
    pub(crate) analysis_version: fn(&ServiceConfig) -> u32,
    run: Run,
    /// The entry of one track (`tracks.<name>.<id>`) from the session's outcome and the track's value in it.
    track_entry: fn(&Value, &Value) -> Value,
}

impl AnalysisStep {
    /// Run the step over the analysed tracks with the analysis version. `None`: the step did not run or had no target.
    pub(crate) fn run(&self, svc: &Service, op: &Shared, analysed: &[String]) -> Option<Value> {
        let v = (self.analysis_version)(&svc.inner.config);
        match self.run {
            Run::Names(run) => {
                if v > 0 {
                    run(svc, op, analysed)
                } else {
                    None
                }
            }
            Run::Versioned(run) => {
                let targets: Vec<(String, u32)> = analysed.iter().map(|n| (n.clone(), v)).collect();
                run(svc, op, &targets)
            }
        }
    }

    /// Write the step's outcome `out` into the job answer `job`: per track of `out.tracks` the track's entry, then
    /// the session's outcome under [`AnalysisStep::id`].
    pub(crate) fn emit(&self, job: &mut Value, out: &Value) {
        if let Some(tracks) = out["tracks"].as_object() {
            for (name, value) in tracks {
                job["tracks"][name.as_str()][self.id] = (self.track_entry)(out, value);
            }
        }
        job[self.id] = out.clone();
    }
}

/// The track's value as it is (the cross-track bleed rule and the ghost gate).
fn as_is(_out: &Value, value: &Value) -> Value {
    value.clone()
}

/// The tom bleed rule: the session's `rule`, `on` and `reason` with the track's `removed` count.
fn bleed_entry(out: &Value, removed: &Value) -> Value {
    json!({"rule": out["rule"], "on": out["on"], "reason": out["reason"], "removed": removed})
}

/// The flam rule: the session's `rule` with the track's `added` count.
fn flam_entry(out: &Value, added: &Value) -> Value {
    json!({"rule": out["rule"], "added": added})
}

/// The session steps of the analysis job in their fixed order (see the module documentation).
pub(crate) const ANALYSIS_STEPS: [AnalysisStep; 4] = [
    AnalysisStep {
        id: "cross_bleed",
        analysis_version: crate::cross_bleed::analysis_cross_bleed,
        run: Run::Names(crate::cross_bleed::run_step),
        track_entry: as_is,
    },
    AnalysisStep {
        id: "ghost_gate",
        analysis_version: crate::ghost_gate::analysis_ghost_gate,
        run: Run::Names(crate::ghost_gate::run_step),
        track_entry: as_is,
    },
    AnalysisStep {
        id: "bleed",
        analysis_version: crate::tom_bleed::analysis_bleed_rule,
        run: Run::Versioned(crate::tom_bleed::run_step),
        track_entry: bleed_entry,
    },
    AnalysisStep {
        id: "flam",
        analysis_version: crate::flam::analysis_flam_rule,
        run: Run::Versioned(crate::flam::run_step),
        track_entry: flam_entry,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The job answer keeps its shape per step: the order of the steps and the keys of a track's entry.
    #[test]
    fn steps_keep_their_order_and_job_answer_shape() {
        let ids: Vec<&str> = ANALYSIS_STEPS.iter().map(|s| s.id).collect();
        assert_eq!(ids, ["cross_bleed", "ghost_gate", "bleed", "flam"]);
        let outs = [
            json!({"rule": 8, "tracks": {"t1": {"removed": 2, "redetected": 1}}}),
            json!({"rule": 1, "tracks": {"s": {"cleared": 3}}}),
            json!({"rule": 5, "on": false, "reason": "r", "played": [], "tracks": {"t1": 4}}),
            json!({"rule": 2, "tracks": {"s": 1}}),
        ];
        let mut job = json!({"tracks": {"t1": {"state": "done"}, "s": {"state": "done"}}});
        for (step, out) in ANALYSIS_STEPS.iter().zip(&outs) {
            step.emit(&mut job, out);
        }
        assert_eq!(
            job,
            json!({
                "tracks": {
                    "t1": {"state": "done", "cross_bleed": {"removed": 2, "redetected": 1},
                           "bleed": {"rule": 5, "on": false, "reason": "r", "removed": 4}},
                    "s": {"state": "done", "ghost_gate": {"cleared": 3}, "flam": {"rule": 2, "added": 1}},
                },
                "cross_bleed": outs[0], "ghost_gate": outs[1], "bleed": outs[2], "flam": outs[3],
            })
        );
        let t1: Vec<&String> = job["tracks"]["t1"].as_object().unwrap().keys().collect();
        assert_eq!(t1, ["state", "cross_bleed", "bleed"]);
        let bleed: Vec<&String> = job["tracks"]["t1"]["bleed"].as_object().unwrap().keys().collect();
        assert_eq!(bleed, ["rule", "on", "reason", "removed"]);
    }
}
