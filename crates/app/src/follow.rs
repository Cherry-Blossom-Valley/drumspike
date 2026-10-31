//! P3-18: a track that follows another track's hits (owner decision 2026-10-12). A kick sub
//! mic carries the same hits as the kick in, 10 ms earlier and much softer; the model finds
//! none of them at the default thresholds. Instead of its own detection the sub takes the hits
//! of the kick in, sample-exact, with velocities from its own signal.
//!
//! One event store per source: the follower's events are derived and written into its record
//! (so the lanes, the click and the exports read them as before), and derived again when the
//! source is analysed again ([`crate::jobs`]), edited (`events_set`) or when the follow is set.
//! The page shows the follower read-only. The record carries `follows: <source name>`
//! (`project-format.md`; an older document has no key, format 1 stays loadable).
//!
//! Rules: a track follows one other track of the project; a source cannot follow (no chains);
//! a track with followers cannot follow. A follower whose source is gone (renamed records
//! follow the rename; a removed source) gets its own detection back with no events.

use drumspike_core::project::Event;
use drumspike_core::velocity::{VelocityMap, velocity_map};
use serde_json::{Map, Value, json};

use crate::Service;
use crate::error::AppError;
use crate::media::{Decoded, mix_mode};
use crate::registry::{OpenProject, Shared, lock};

/// The derived events of a follower: every hit of `src` at the same sample position in the
/// follower's rate (`round(t * sr) / sr`; equal rates and on-grid times give the same `t`),
/// the velocity from the follower's signal at that position, as a detected hit gets it
/// (`velocity_map`, `peak_in` over -5 ms to +60 ms). Only `t` and `v`: no `rise`, no edit marks.
pub fn derive_events(src: &[Event], dst_samples: &[f32], dst_sr: u32, map: &VelocityMap) -> Vec<Event> {
    let sr = f64::from(dst_sr);
    let times: Vec<f64> = src.iter().map(|e| (e.t * sr).round() / sr).collect();
    let vels = velocity_map(dst_samples, dst_sr, &times, map);
    times.into_iter().zip(vels).map(|(t, v)| Event::new(t, v)).collect()
}

/// The velocity map of a track's params (`vel_lo_db`, `vel_hi_db`; the defaults -40 and -6 dB).
pub fn velocity_map_of(params: Option<&Map<String, Value>>) -> VelocityMap {
    let p = |k: &str, d: f64| params.and_then(|m| m.get(k)).and_then(Value::as_f64).unwrap_or(d);
    VelocityMap {
        lo_db: p("vel_lo_db", -40.0),
        hi_db: p("vel_hi_db", -6.0),
        ..VelocityMap::default()
    }
}

/// The tracks that follow `source`, in the document's order.
pub fn followers_of(g: &OpenProject, source: &str) -> Vec<String> {
    g.project
        .doc
        .tracks()
        .filter(|(_, tr)| tr.follows.as_deref() == Some(source))
        .map(|(n, _)| n.clone())
        .collect()
}

/// 409 when `name` follows another track: its hits are not edited, the source's are.
///
/// # Errors
///
/// A 409 `AppError` when `name` follows a track; the lookup error when the project has no track `name`.
pub fn check_own(g: &OpenProject, name: &str) -> Result<(), AppError> {
    match g.track(name)?.follows.as_deref() {
        Some(src) => Err(AppError::conflict(format!(
            "track {name} follows {src}: edit the hits of {src}"
        ))),
        None => Ok(()),
    }
}

/// Store derived events in the follower's record (the shape `finish_one` writes).
fn store(g: &mut OpenProject, name: &str, events: Vec<Event>, dec: &Decoded) -> Result<usize, AppError> {
    let n = events.len();
    let tr = g.track_mut(name)?;
    tr.events = Some(events);
    tr.extra.shift_remove("events");
    tr.sr = Some(i64::from(dec.sr));
    tr.duration = Some(dec.samples.len() as f64 / f64::from(dec.sr));
    tr.n_samples = Some(dec.samples.len() as i64);
    g.mark_dirty();
    Ok(n)
}

/// Derive the events of the follower `name` from its source and store them. Returns the
/// number of hits. A follower whose source is gone gets its own detection back (no `follows`,
/// no events) and 0. 400 when `name` follows nothing, 409 when its audio is offline.
///
/// # Errors
///
/// A 400 `AppError` when `name` follows no track; the lookup error for an unknown track; a 409 when the follower's
/// audio is offline; the decode error when its audio cannot be read.
pub fn derive_one(svc: &Service, op: &Shared, name: &str) -> Result<usize, AppError> {
    let (src_events, path, mix, map, cut) = {
        let mut g = lock(op);
        let tr = g.track(name)?;
        let Some(src) = tr.follows.clone() else {
            return Err(AppError::bad(format!("track {name} follows no track")));
        };
        let Ok(source) = g.track(&src) else {
            let tr = g.track_mut(name)?;
            tr.follows = None;
            tr.events = Some(Vec::new());
            tr.extra.shift_remove("events");
            g.mark_dirty();
            return Ok(0);
        };
        let src_events = source.events().to_vec();
        let path = g.source(name)?;
        (
            src_events,
            path,
            mix_mode(tr.mix_mode_or_default()),
            velocity_map_of(tr.params.as_ref()),
            crate::histogram::cut_of(tr.params.as_ref()),
        )
    };
    let dec = svc.inner.media.decode_cached(&path, mix)?;
    // P4-3: the hits the source's loudness cut took out are not followed; the follower's own cut marks its quiet ones
    let src_events: Vec<Event> = src_events
        .into_iter()
        .filter(|e| !crate::histogram::event_cut(e))
        .collect();
    let mut events = derive_events(&src_events, &dec.samples, dec.sr, &map);
    crate::histogram::mark_cut_events(&mut events, &dec.samples, dec.sr, cut);
    let mut g = lock(op);
    store(&mut g, name, events, &dec)
}

/// Derive the events of every follower of `source` again (after an analysis, an edit or a
/// relink of the source). Returns the followers with their hit counts, or the error of each.
pub fn refresh_followers(svc: &Service, op: &Shared, source: &str) -> Vec<(String, Result<usize, AppError>)> {
    let names = followers_of(&lock(op), source);
    names
        .into_iter()
        .map(|n| {
            let r = derive_one(svc, op, &n);
            (n, r)
        })
        .collect()
}

/// P4-4: the hit times a follower must hold: every uncut hit of its source at the nearest sample
/// of the follower's rate (the times [`derive_events`] writes). `None` when `name` follows no
/// track, its source is gone, or its rate is not known.
pub(crate) fn expected_times(g: &OpenProject, name: &str) -> Option<Vec<f64>> {
    let tr = g.track(name).ok()?;
    let source = g.track(tr.follows.as_deref()?).ok()?;
    let sr = tr.sr.filter(|s| *s > 0)? as f64;
    Some(
        source
            .events()
            .iter()
            .filter(|e| !crate::histogram::event_cut(e))
            .map(|e| (e.t * sr).round() / sr)
            .collect(),
    )
}

/// P4-4: before an export, a follower whose hits no longer match its source's (its audio was
/// offline when the source was edited, so [`refresh_followers`] failed) is derived again. A
/// follower that cannot be derived now is a 409 that names the cause, so no export writes the
/// old hits. Answers whether the follower was derived again; `false` for any other track.
pub(crate) fn fresh_for_export(svc: &Service, op: &Shared, name: &str) -> Result<bool, AppError> {
    let (stale, source, old_files) = {
        let g = lock(op);
        let Some(want) = expected_times(&g, name) else {
            return Ok(false);
        };
        let have: Vec<f64> = g.track(name)?.events().iter().map(|e| e.t).collect();
        let dir = g.project.subdir(drumspike_core::project::EXPORTS_DIR);
        let old: Vec<String> = ["mid", "wav"]
            .iter()
            .map(|ext| format!("{name}_spike.{ext}"))
            .filter(|f| dir.join(f).is_file())
            .map(|f| format!("Exports/{f}"))
            .collect();
        (have != want, g.track(name)?.follows.clone().unwrap_or_default(), old)
    };
    if !stale {
        return Ok(false);
    }
    derive_one(svc, op, name).map_err(|e| {
        // P4-4 fix round: the files of an earlier export stay; the message names them
        let old = if old_files.is_empty() {
            String::new()
        } else {
            format!(
                "; {} of an earlier export still hold its old hits",
                old_files.join(" and ")
            )
        };
        AppError::conflict(format!(
            "track {name} follows {source}, whose hits changed while it could not follow them ({}): \
             bring its audio back, then export again{old}",
            e.message
        ))
    })?;
    Ok(true)
}

/// P4-4 fix round: derive again every follower whose hits no longer match its source's and that
/// can be derived now (its audio and its source are online), as an edit of the source does. A
/// relink and `tracks_get` (the page loads the project with it, also after a file came back) call
/// this, so the lanes, the click and the exports see fresh hits. Answers the names derived.
/// Round 2: it holds the project's store guard from the snapshot to the store, so it cannot
/// overwrite a concurrent `track_velocity_set`; when the guard is not free (a store or an
/// analysis runs) it does nothing and the follower stays stale until the next call or export.
pub(crate) fn refresh_stale(svc: &Service, op: &Shared) -> Vec<String> {
    let sid = lock(op).sid.clone();
    let Some(_guard) = svc.inner.jobs.begin_store(&sid) else {
        return Vec::new();
    };
    if svc.inner.jobs.running(&sid) {
        return Vec::new();
    }
    let names: Vec<String> = lock(op)
        .project
        .doc
        .tracks()
        .filter(|(_, tr)| tr.follows.is_some())
        .map(|(n, _)| n.clone())
        .collect();
    names
        .into_iter()
        .filter(|n| matches!(fresh_for_export(svc, op, n), Ok(true)))
        .collect()
}

/// The names of the followers that were derived again, as the answers carry them.
pub fn refreshed_names(rows: &[(String, Result<usize, AppError>)]) -> Vec<Value> {
    rows.iter()
        .filter(|(_, r)| r.is_ok())
        .map(|(n, _)| Value::from(n.as_str()))
        .collect()
}

impl Service {
    /// `track_set_follow {sid, name, source}`: the track `name` takes the hits of `source`
    /// (another track of the project), or its own detection again with `source` null. With a
    /// source the events are derived at once (a backup of the previous events is kept) and
    /// derived again at every change of the source. Without one the current events stay
    /// until the next analysis. 400 for the track itself, an unknown source, a source that
    /// follows a track, or a track that has followers; 409 when the follower is offline.
    /// Answers `{follows, n_events}`.
    ///
    /// # Errors
    ///
    /// An `AppError` for an unknown or read-only session or an unknown track; a 400 for the cases above (itself,
    /// unknown source, chained follow, a track with followers); the error of the first derivation ([`derive_one`]),
    /// after which the previous `follows` is restored.
    pub fn track_set_follow(&self, sid: &str, name: &str, source: Option<&str>) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        crate::registry::resolve_shared(&op);
        let previous = {
            let mut g = lock(&op);
            g.track(name)?;
            let previous = g.track(name)?.follows.clone();
            match source {
                None => {
                    if previous.is_some() {
                        g.track_mut(name)?.follows = None;
                        g.mark_dirty();
                    }
                    return Ok(json!({"follows": Value::Null, "n_events": g.n_events(name)}));
                }
                Some(src) if src == name => {
                    return Err(AppError::bad("a track cannot follow itself"));
                }
                Some(src) => {
                    let s = g
                        .track(src)
                        .map_err(|_| AppError::bad(format!("unknown track {src}")))?;
                    let mine = followers_of(&g, name);
                    if !mine.is_empty() {
                        return Err(AppError::bad(format!(
                            "{name} has followers ({}): set them to their own detection first",
                            mine.join(", ")
                        )));
                    }
                    if let Some(up) = &s.follows {
                        return Err(AppError::bad(format!("{src} follows {up}: follow {up} instead")));
                    }
                    if previous.as_deref() != Some(src) {
                        crate::jobs::backup_events(&g, name)?;
                    }
                    g.track_mut(name)?.follows = Some(src.to_string());
                    g.mark_dirty();
                }
            }
            previous
        };
        match derive_one(self, &op, name) {
            Ok(n) => Ok(json!({"follows": source, "n_events": n})),
            Err(e) => {
                // the follow is not set when its first derivation fails (an offline follower)
                let mut g = lock(&op);
                if let Ok(tr) = g.track_mut(name) {
                    tr.follows = previous;
                }
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn spike(db: f64, at: usize, n: usize) -> Vec<f32> {
        let mut x = vec![0.0f32; n];
        x[at] = 10f64.powf(db / 20.0) as f32;
        x
    }

    #[test]
    fn derived_events_keep_the_sample_positions_and_take_the_followers_level() {
        let src = [Event::new(0.5, 127), Event::new(1.0, 127)];
        let mut x = spike(-6.0, 24_000, SR as usize * 2);
        x[48_000] = 10f32.powf(-40.0 / 20.0);
        let ev = derive_events(&src, &x, SR, &VelocityMap::default());
        assert_eq!(ev.iter().map(|e| e.t).collect::<Vec<_>>(), [0.5, 1.0]);
        assert_eq!(ev.iter().map(|e| e.v).collect::<Vec<_>>(), [Some(127), Some(20)]);
        assert!(
            ev.iter()
                .all(|e| e.rise.is_none() && e.added.is_none() && e.orig.is_none())
        );
        // another rate: the same time rounded to the follower's grid
        let ev = derive_events(
            &[Event::new(0.5 + 1.0 / 96_000.0, 100)],
            &spike(-6.0, 22_050, 44_100),
            44_100,
            &VelocityMap::default(),
        );
        assert_eq!(ev[0].t, 22_050.0 / 44_100.0);
        assert!(derive_events(&[], &x, SR, &VelocityMap::default()).is_empty());
    }

    #[test]
    fn the_velocity_map_reads_the_params_and_defaults() {
        assert_eq!(velocity_map_of(None), VelocityMap::default());
        let m: Map<String, Value> = serde_json::from_str(r#"{"vel_lo_db": -50, "vel_hi_db": -3}"#).unwrap();
        let v = velocity_map_of(Some(&m));
        assert_eq!((v.lo_db, v.hi_db, v.lo_v, v.hi_v), (-50.0, -3.0, 20, 127));
    }
}
