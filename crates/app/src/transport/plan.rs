//! The play plan: the tracks of one project that an open transport plays (the [`Plan`]),
//! their hits, volumes and layer voices ([`LayerPlan`], the one-shot templates [`LayerTpls`]),
//! the hit conversion from event times, and the [`Service`] methods that build and refresh
//! them (P4-7a). See the module docs of `super` for the whole transport.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use drumspike_core::io::MappedWav;
use drumspike_core::layer::OneShot;
use drumspike_core::spike::hit_index;
use drumspike_core::transport::TrackSpec;
use drumspike_core::transport::clicks::role_pitch;
use drumspike_core::transport::layer::{LayerBank, LayerVoices};
use serde_json::Value;

use super::{Open, clamp_db, clamp_pan, dev_err};
use crate::Service;
use crate::error::AppError;
use crate::media::mix_mode;
use crate::registry::lock;

/// P4-7a: the layer of one track in a plan: the one-shot (its own rate) and per voice the time of
/// its first sample (seconds) and its gain.
#[derive(Debug, Clone)]
pub(crate) struct LayerPlan {
    pub(crate) path: PathBuf,
    pub(crate) shot: Arc<OneShot>,
    pub(crate) starts: Vec<(f64, f32)>,
}

impl PartialEq for LayerPlan {
    fn eq(&self, o: &LayerPlan) -> bool {
        self.path == o.path && (Arc::ptr_eq(&self.shot, &o.shot) || *self.shot == *o.shot) && self.starts == o.starts
    }
}

/// The tracks of an open transport: what the transport needs to open again.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    pub(crate) sid: String,
    pub(crate) names: Vec<String>,
    pub(crate) roles: Vec<String>,
    /// Path, mix mode and hits (at the track's own rate) per track.
    pub(crate) specs: Vec<TrackSpec>,
    /// P3-16: volume (dB) and pan per track, from the project.
    pub(crate) vols: Vec<(f64, f64)>,
    /// P4-7a: the layer of each track (`None`: none, off or its file missing).
    pub(crate) layers: Vec<Option<LayerPlan>>,
}

impl Plan {
    /// The same tracks (names and files) as `other`: an open transport can serve it.
    pub(crate) fn same_tracks(&self, other: &Plan) -> bool {
        self.sid == other.sid
            && self.names == other.names
            && self.specs.len() == other.specs.len()
            && self
                .specs
                .iter()
                .zip(&other.specs)
                .all(|(a, b)| a.path == b.path && a.mode == b.mode)
    }

    pub(crate) fn hits(&self) -> Vec<Vec<u64>> {
        self.specs.iter().map(|s| s.hits.clone()).collect()
    }
}

#[cfg(test)]
thread_local! {
    /// Test hook: how often this thread built layer plans.
    pub(crate) static LAYER_PLAN_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// P4-7a: the one-shots at a device rate and at twice it (interleaved), by (one-shot identity, rate).
/// Keyed by the Arc pointer, not by path: a replaced file of the same length must not reuse the
/// stale template.
pub(crate) type LayerTpls = HashMap<(usize, u32), (Arc<OneShot>, Arc<Vec<f32>>, Arc<Vec<f32>>)>;

/// The most one-shot templates the host keeps (each at one device rate).
const LAYER_TPLS_MAX: usize = 32;

/// P4-7a: the layer bank of `plan` at the device `rate`: the one-shots resampled to the rate and to
/// twice it (kept in `tpls`), the voices at their frames.
pub(crate) fn layer_bank(tpls: &mut LayerTpls, plan: &Plan, rate: u32) -> LayerBank {
    plan.layers
        .iter()
        .map(|l| {
            let l = l.as_ref()?;
            let key = (Arc::as_ptr(&l.shot) as *const () as usize, rate);
            if !tpls.contains_key(&key) && tpls.len() >= LAYER_TPLS_MAX {
                tpls.clear();
            }
            // the entry holds the Arc, so its address cannot be reused while it is cached; a hit is
            // checked by identity all the same
            let fresh = || {
                (
                    Arc::clone(&l.shot),
                    Arc::new(l.shot.resampled(rate).interleaved()),
                    Arc::new(l.shot.resampled(2 * rate).interleaved()),
                )
            };
            let e = tpls.entry(key).or_insert_with(fresh);
            if !Arc::ptr_eq(&e.0, &l.shot) {
                *e = fresh();
            }
            let (tpl, half) = (e.1.clone(), e.2.clone());
            Some(LayerVoices::new(l.shot.channels(), tpl, half, &l.starts, rate))
        })
        .collect()
}

/// Hand the layers of the open plan to its transport (none when no track has a layer and the
/// transport never had one). The update queue drains once per callback: retry briefly.
pub(crate) fn push_layers(tpls: &mut LayerTpls, o: &mut Open) -> Result<(), AppError> {
    let rate = o.transport.sample_rate();
    let bank = layer_bank(tpls, &o.plan, rate);
    let mut r = o.transport.set_layers(bank.clone());
    for _ in 0..50 {
        if r.is_ok() {
            break;
        }
        sleep(Duration::from_millis(2));
        r = o.transport.set_layers(bank.clone());
    }
    r.map_err(dev_err)
}

/// Hit samples (at the track's own rate) from event times in seconds.
pub(crate) fn hits_from_times(times: impl Iterator<Item = f64>, rate: u32, n_samples: Option<i64>) -> Vec<u64> {
    let n = n_samples.filter(|n| *n > 0).map_or(usize::MAX, |n| n as usize);
    let mut v: Vec<u64> = times
        .filter(|t| t.is_finite())
        .filter_map(|t| hit_index(t, rate, n))
        .map(|i| i as u64)
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// The times of one `events` entry of `transport_play`: numbers, or objects with a `t`.
pub(crate) fn times_of(list: &Value) -> Vec<f64> {
    list.as_array()
        .map(|a| {
            a.iter()
                .filter(|e| !crate::export::is_cut(e)) // P4-3: a hit of the loudness cut gets no click
                .filter_map(|e| e.as_f64().or_else(|| e.get("t").and_then(Value::as_f64)))
                .collect()
        })
        .unwrap_or_default()
}

impl Service {
    /// The plan for a project: the tracks in order that have a source, with their hits from
    /// `events` (name to times) or, without it, from the project's own events.
    pub(crate) fn transport_plan(&self, sid: &str, events: Option<&Value>) -> Result<Plan, AppError> {
        let op = self.inner.registry.get(sid)?;
        struct Row {
            name: String,
            role: String,
            path: PathBuf,
            mode: drumspike_core::MixMode,
            n_samples: Option<i64>,
            times: Vec<f64>,
            vol: (f64, f64),
        }
        crate::registry::resolve_shared(&op);
        let rows: Vec<Row> = {
            let g = lock(&op);
            let mut rows = Vec::new();
            for name in g.project.doc.track_order() {
                let Ok(tr) = g.track(&name) else { continue };
                let Ok(path) = g.source(&name) else { continue };
                let times = match events.and_then(|e| e.get(&name)) {
                    Some(list) => times_of(list),
                    None => tr
                        .events()
                        .iter()
                        .filter(|e| !crate::histogram::event_cut(e))
                        .map(|e| e.t)
                        .collect(),
                };
                rows.push(Row {
                    vol: (tr.volume_db.map_or(0.0, clamp_db), tr.pan.map_or(0.0, clamp_pan)),
                    role: tr.role.clone().unwrap_or_else(|| "other".into()),
                    mode: mix_mode(tr.mix_mode_or_default()),
                    n_samples: tr.n_samples,
                    name,
                    path,
                    times,
                });
            }
            rows
        };
        if rows.is_empty() {
            return Err(AppError::conflict("no track with an audio file to play"));
        }
        let names: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();
        let layers = self.layer_plans(sid, &names);
        let mut plan = Plan {
            sid: sid.to_string(),
            names: Vec::new(),
            roles: Vec::new(),
            specs: Vec::new(),
            vols: Vec::new(),
            layers,
        };
        for r in rows {
            let rate = MappedWav::open(&r.path)
                .map_err(|e| AppError::internal(format!("{}: {e}", r.path.display())))?
                .info
                .sample_rate;
            plan.specs.push(TrackSpec {
                hits: hits_from_times(r.times.into_iter(), rate, r.n_samples),
                path: r.path,
                mode: r.mode,
                click_pitch: role_pitch(&r.role),
            });
            plan.names.push(r.name);
            plan.roles.push(r.role);
            plan.vols.push(r.vol);
        }
        Ok(plan)
    }

    /// P4-7a: the layer plan of each track in `names` (the document's uncut hits; see the module docs).
    fn layer_plans(&self, sid: &str, names: &[String]) -> Vec<Option<LayerPlan>> {
        #[cfg(test)]
        LAYER_PLAN_CALLS.with(|c| c.set(c.get() + 1));
        let Ok(op) = self.inner.registry.get(sid) else {
            return vec![None; names.len()];
        };
        let jobs: Vec<Option<crate::layer::LayerJob>> = {
            let g = lock(&op);
            names
                .iter()
                .map(|n| crate::layer::layer_job(&g, n).ok().flatten())
                .collect()
        };
        names
            .iter()
            .zip(jobs)
            .map(|(n, job)| {
                let job = job?;
                let (shot, starts) = self.layer_voices(sid, n, &job, &job.hits)?;
                Some(LayerPlan {
                    path: job.file.clone()?,
                    shot,
                    starts,
                })
            })
            .collect()
    }

    /// P4-40e: the length of a track's one-shot in seconds (the document's layer record), for the
    /// preview window's end. 409 without a layer.
    pub(crate) fn layer_len_s(&self, sid: &str, name: &str) -> Result<f64, AppError> {
        let op = self.inner.registry.get(sid)?;
        let g = lock(&op);
        let tr = g.track(name)?;
        let layer = crate::layer::layer_of(&tr.extra)
            .ok_or_else(|| AppError::conflict(format!("track {name} has no layer")))?;
        Ok(layer.frames as f64 / layer.sr as f64)
    }

    /// P4-40e: append the preview voice (the one-shot once, its first sample at `at_s`) to the
    /// layer plan of `name`. The gain is the layer's at velocity 127 with its polarity (auto
    /// behaves as normal: the preview runs no alignment). A layer that is off (or without voices)
    /// still gets its voice. 409 without a layer or its file.
    pub(crate) fn plan_with_preview(&self, sid: &str, name: &str, plan: &mut Plan, at_s: f64) -> Result<(), AppError> {
        let op = self.inner.registry.get(sid)?;
        let job = {
            let g = lock(&op);
            crate::layer::layer_job(&g, name)?
                .ok_or_else(|| AppError::conflict(format!("track {name} has no layer")))?
        };
        let idx = plan.names.iter().position(|n| n == name).ok_or_else(|| {
            AppError::bad(format!(
                "preview track \"{name}\" is not in the project's playable tracks"
            ))
        })?;
        let sign = if job.layer.polarity == crate::layer::Polarity::Invert {
            -1
        } else {
            1
        };
        let gain = job.layer.gain(127, sign);
        let file = job
            .file
            .clone()
            .ok_or_else(|| AppError::conflict(format!("the layer file of track {name} is missing")))?;
        let shot = self.layer_shot(&file, None)?;
        match plan.layers[idx].as_mut() {
            Some(l) => l.starts.push((at_s, gain)),
            None => {
                plan.layers[idx] = Some(LayerPlan {
                    path: file,
                    shot,
                    starts: vec![(at_s, gain)],
                });
            }
        }
        Ok(())
    }

    /// P4-7a: after a layer setting changed, give the open transport of `sid` its new layers (no
    /// transport open for the project: nothing to do; the next play builds them). The plans are
    /// built once, before the host lock (mic decode, alignment and the project lock are slow). A
    /// failed hand-over to the transport is the `Err`.
    pub(crate) fn transport_refresh_layers(&self, sid: &str) -> Result<(), AppError> {
        let names = {
            let st = lock(&self.host().st);
            match &st.open {
                Some(o) if o.plan.sid == sid => o.plan.names.clone(),
                _ => match &st.lost {
                    Some(l) if l.plan.sid == sid => l.plan.names.clone(),
                    _ => return Ok(()),
                },
            }
        };
        let layers = self.layer_plans(sid, &names);
        self.apply_layers(sid, &names, layers)
    }

    /// Put `layers` (one per track of `names`) into the open plan and its transport, and into a
    /// lost plan. Holds the host lock only for that: no plan is built here.
    pub(crate) fn apply_layers(
        &self,
        sid: &str,
        names: &[String],
        layers: Vec<Option<LayerPlan>>,
    ) -> Result<(), AppError> {
        let mut st = lock(&self.host().st);
        let st = &mut *st;
        let mut result = Ok(());
        if let Some(o) = st.open.as_mut().filter(|o| o.plan.sid == sid && o.plan.names == names)
            && o.plan.layers != layers
        {
            o.plan.layers = layers.clone();
            result = push_layers(&mut st.layer_tpls, o);
        }
        if let Some(l) = st.lost.as_mut().filter(|l| l.plan.sid == sid && l.plan.names == names) {
            l.plan.layers = layers;
        }
        result
    }
}
