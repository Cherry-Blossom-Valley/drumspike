//! Lane preparation with progress (P3-4a, UX follow-up of P3-7a). The first open of a project
//! builds `Analysis/<track>/peaks.bin` for every track (about 9 s for 10 tracks of 353 s); the
//! page used to show nothing while it waited. `peaks_build_start` starts that build on a worker
//! thread, which builds up to six tracks at the same time (P3-15d: the workers of
//! `tracks_add`, [`crate::media::on_peak_workers`]), with the job pattern of the analysis: the same
//! `{state, tracks: {name: {state, stage?, error?}}}` value, a `progress` callback that gets
//! `{"sid", "job"}` after every change, and `peaks_build_status` to poll. Only tracks whose
//! cache is missing or stale and whose source is online are in the job; the other tracks need
//! no build, and `peaks_info` still answers for them as before (a build on demand is
//! unchanged, so a page that skips this step works).

use std::collections::HashMap;
use std::sync::Mutex;

use drumspike_core::cache::cache_is_current;
use serde_json::{Value, json};

use crate::error::AppError;
use crate::media::PeakRequest;
use crate::registry::lock;
use crate::{Progress, Service};

/// The job states of all projects.
#[derive(Default)]
pub struct LaneJobs {
    map: Mutex<HashMap<String, Value>>,
}

impl LaneJobs {
    /// The job of `sid`, or `{"state": "idle", "tracks": {}}`.
    pub fn status(&self, sid: &str) -> Value {
        lock(&self.map)
            .get(sid)
            .cloned()
            .unwrap_or_else(|| json!({"state": "idle", "tracks": {}}))
    }

    /// Replace the job of `sid` with `queued` entries for `targets` (`done` when there are none).
    pub fn queue(&self, sid: &str, targets: &[String]) -> Value {
        let tracks: serde_json::Map<String, Value> = targets
            .iter()
            .map(|t| (t.clone(), json!({"state": "queued"})))
            .collect();
        let job = json!({
            "state": if targets.is_empty() { "done" } else { "queued" },
            "total": targets.len(),
            "tracks": tracks,
        });
        lock(&self.map).insert(sid.to_string(), job.clone());
        job
    }

    /// Change the job of `sid` and return the new value.
    pub fn update(&self, sid: &str, f: impl FnOnce(&mut Value)) -> Value {
        let mut map = lock(&self.map);
        let job = map
            .entry(sid.to_string())
            .or_insert_with(|| json!({"state": "running", "total": 0, "tracks": {}}));
        f(job);
        job.clone()
    }

    /// Forget the job of `sid` (the project was closed).
    pub fn clear(&self, sid: &str) {
        lock(&self.map).remove(sid);
    }

    /// After a crash of the worker: every track that is neither `done` nor `error` becomes an
    /// error with `message`, and the job is `done`. Returns the new job value.
    pub fn fail_unfinished(&self, sid: &str, message: &str) -> Value {
        self.update(sid, |job| {
            if let Some(tracks) = job["tracks"].as_object_mut() {
                for t in tracks.values_mut() {
                    if t["state"] != "done" && t["state"] != "error" {
                        *t = json!({"state": "error", "error": message});
                    }
                }
            }
            job["state"] = Value::from("done");
        })
    }
}

impl Service {
    /// Build the missing or stale `peaks.bin` files of `tracks` (default: every track) on a
    /// worker thread, up to [`crate::media::PEAK_WORKERS`] tracks at the same time (the
    /// folder lock of [`crate::media::Media::peak_cache`] still builds each folder once).
    /// `progress` gets `{"sid", "job"}` after every change, in the order of the changes.
    /// Answers `{"started": [names that need a build]}`; the job is `done` at once when that
    /// list is empty. A track error (an offline track without a cache is not in the list; a
    /// file that cannot be decoded is) ends that track only.
    pub fn peaks_build_start(
        &self,
        sid: &str,
        tracks: Option<Vec<String>>,
        progress: Option<Progress>,
    ) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let needed: Vec<String> = {
            let g = lock(&op);
            let names = match tracks {
                Some(t) if !t.is_empty() => t,
                _ => g.project.doc.track_order(),
            };
            let mut needed = Vec::new();
            for name in names {
                let req = PeakRequest::of(&g, &name)?;
                if !cache_is_current(&req.dir, &req.key) && req.source.is_ok() {
                    needed.push(name);
                }
            }
            needed
        };
        let first = self.inner.life.lanes.queue(sid, &needed);
        if needed.is_empty() {
            if let Some(p) = progress.as_ref() {
                p(json!({"sid": sid, "job": first}));
            }
            return Ok(json!({"started": needed}));
        }
        let svc = self.clone();
        let sid = sid.to_string();
        let started = needed.clone();
        std::thread::spawn(move || {
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                svc.run_lane_job(&sid, &needed, progress.as_ref());
            }));
            if run.is_err() {
                let job = svc
                    .inner
                    .life
                    .lanes
                    .fail_unfinished(&sid, "waveform build stopped by an internal error");
                if let Some(p) = progress.as_ref() {
                    p(json!({"sid": sid, "job": job}));
                }
            }
        });
        Ok(json!({"started": started}))
    }

    /// The job of [`Service::peaks_build_start`].
    #[must_use]
    pub fn peaks_build_status(&self, sid: &str) -> Value {
        self.inner.life.lanes.status(sid)
    }

    fn run_lane_job(&self, sid: &str, targets: &[String], progress: Option<&Progress>) {
        let lanes = &self.inner.life.lanes;
        // a change and its message under one lock: the messages leave in the order of the changes
        let order = Mutex::new(());
        let emit = |f: &dyn Fn(&mut Value)| {
            let _o = lock(&order);
            let job = lanes.update(sid, f);
            if let Some(p) = progress {
                p(json!({"sid": sid, "job": job}));
            }
        };
        emit(&|j| j["state"] = Value::from("running"));
        let reqs: Vec<Result<PeakRequest, AppError>> = match self.inner.registry.get(sid) {
            Ok(op) => {
                let g = lock(&op);
                targets.iter().map(|name| PeakRequest::of(&g, name)).collect()
            }
            Err(e) => targets.iter().map(|_| Err(e.clone())).collect(),
        };
        crate::media::on_peak_workers(targets.len(), |i| {
            let name = targets[i].as_str();
            emit(&|j| j["tracks"][name] = json!({"state": "running", "stage": "building"}));
            let _span = crate::trace::stage("lanes:track");
            let result = reqs[i]
                .clone()
                .and_then(|r| self.inner.media.peak_cache(&r))
                .map(|_| ());
            let entry = match result {
                Ok(()) => json!({"state": "done"}),
                Err(e) => json!({"state": "error", "error": e.message}),
            };
            emit(&|j| j["tracks"][name] = entry.clone());
        });
        emit(&|j| j["state"] = Value::from("done"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_job_list_is_idle_then_queued_then_failed_to_done() {
        let j = LaneJobs::default();
        assert_eq!(j.status("s"), json!({"state": "idle", "tracks": {}}));
        let q = j.queue("s", &["a".into(), "b".into()]);
        assert_eq!(q["state"], "queued");
        assert_eq!(q["total"], 2);
        assert_eq!(q["tracks"]["b"], json!({"state": "queued"}));
        j.update("s", |v| v["tracks"]["a"] = json!({"state": "done"}));
        let f = j.fail_unfinished("s", "boom");
        assert_eq!(f["state"], "done");
        assert_eq!(f["tracks"]["a"]["state"], "done");
        assert_eq!(f["tracks"]["b"], json!({"state": "error", "error": "boom"}));
        j.clear("s");
        assert_eq!(j.status("s")["state"], "idle");
    }

    #[test]
    fn an_empty_target_list_is_done_at_once() {
        let j = LaneJobs::default();
        let q = j.queue("s", &[]);
        assert_eq!(q, json!({"state": "done", "total": 0, "tracks": {}}));
    }
}
