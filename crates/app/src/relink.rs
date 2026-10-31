//! Relink (P3-4d): `project_relink` (the file picker of today, route 11) and `relink_search`
//! (the folder search, new): offline audio is found again in one action.
//!
//! | Command | Server route (route map row) |
//! |---|---|
//! | `project_relink {sid, track, path, force}` | POST `/api/project/{sid}/relink` (11) |
//! | `relink_search {sid, folder, progress?}` | none (the search is new in the app) |
//!
//! `relink_search` walks the folder (sub-folders too), looks for every offline track by file
//! name, then by size and head hash (the identity of the source record), and relinks each
//! track for which it finds exactly one file. A track with more than one matching file is
//! listed as ambiguous and none is chosen; a track without a match is listed as missing, with
//! the files of the same name whose content differs. The events, roles, notes and peak caches
//! of the tracks are not touched (the record keeps its size, hash and `mtime`, so the cache
//! key stays the same).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use drumspike_core::SourceStatus;
use drumspike_core::project::{
    AUDIO_DIR, SearchHit, SearchTarget, abspath, build_source, relink_search, relinked_source, resolve_source,
    source_matches,
};
use serde_json::{Value, json};

use crate::Service;
use crate::error::AppError;
use crate::registry::lock;
use crate::reply::Channels;
use crate::tracks::probe;

/// P4-7a fix 6: a layer file is a search target with the key `<track>` + this suffix (a control character no
/// track name has), so that the results map back to the track and to the layer.
const LAYER_KEY: &str = "\u{1f}layer";

/// The track name and whether the target is its layer file.
fn split_key(key: &str) -> (&str, bool) {
    key.strip_suffix(LAYER_KEY).map_or((key, false), |t| (t, true))
}

/// The commands of this card for the relink flow.
pub const RELINK_COMMANDS: [&str; 2] = ["project_relink", "relink_search"];

fn paths_json(v: &[PathBuf]) -> Value {
    Value::Array(v.iter().map(|p| json!(p.display().to_string())).collect())
}

impl Service {
    /// `project_relink {sid, track, path, force}` (route 11): point the track at the file
    /// `path`. The file must have the size and head hash of the track's source record unless
    /// `force`; 404 for a path that is no file, 409 for a file that does not match without
    /// `force`. A file inside `Audio Files/` makes the source `copied`. A different file
    /// (`force`) refreshes the track's rate and length from its header; the hit times stay.
    /// Answers `{track, status, source, matched, followers}` (P4-4: the followers derived again,
    /// [`crate::follow::refresh_stale`]).
    ///
    /// # Errors
    ///
    /// Returns a 404 error for an unknown project id or a `path` that is no file, a 400 error for an unknown
    /// track or an unreadable audio header, and a 409 error when the project is open read-only or the file does
    /// not match the source record without `force`. Building the new source record can also fail.
    pub fn project_relink(&self, sid: &str, track: &str, path: &str, force: bool) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let path = abspath(Path::new(path));
        let (rec, dir) = {
            let g = lock(&op);
            let tr = g.track(track)?;
            (tr.source_ref().cloned().unwrap_or_default(), g.project.dir.clone())
        };
        if !path.is_file() {
            return Err(AppError::not_found("no such file"));
        }
        let matches = source_matches(&rec, &path);
        if !matches && !force {
            return Err(AppError::conflict(
                "file does not match the track's source (size or head hash differ); send force=true to relink anyway",
            ));
        }
        let mode = if path.parent() == Some(abspath(&dir).join(AUDIO_DIR).as_path()) {
            "copied"
        } else {
            "referenced"
        };
        let header = if matches {
            None
        } else {
            Some(probe(&path).map_err(|r| AppError::bad(format!("{}: {}", r.name, r.reason)))?)
        };
        let new = build_source(&path, &dir, mode)?;
        let mut g = lock(&op);
        let tr = g.track_mut(track)?;
        tr.source = Some(Some(new.clone()));
        if let Some(p) = header {
            tr.sr = Some(i64::from(p.sr));
            tr.n_samples = Some(p.n_frames as i64);
            tr.duration = Some(p.duration());
        }
        g.paths.insert(track.to_string(), Some(path));
        g.status.insert(track.to_string(), SourceStatus::Ok);
        g.mark_dirty();
        drop(g);
        // P4-4: the followers this relink brings back (the track itself, or the followers of a source)
        let followers = crate::follow::refresh_stale(self, &op);
        Ok(json!({
            "track": track, "status": "ok",
            "source": serde_json::to_value(&new).unwrap_or(Value::Null),
            "matched": matches, "followers": followers,
        }))
    }

    /// `relink_search {sid, folder, progress?}`: find the audio of the offline tracks under
    /// `folder` and relink every track with exactly one match (see the module docs). Answers
    /// ```text
    /// {offline, relinked, followers, found: [{track, path, by}], ambiguous: [{track, candidates}],
    ///  missing: [{track, differs}], files_seen, dirs_seen, truncated, cancelled, elapsed_ms}
    /// ```
    /// `by` is `name` or `content`. The search is a job for `job_cancel`: a cancel returns what
    /// was done so far with `cancelled: true` and relinks nothing. 404 for a folder that does
    /// not exist, 409 while another job of the project runs. A project with no offline track
    /// answers `offline: 0`.
    ///
    /// # Errors
    ///
    /// Returns a 404 error for an unknown project id or a `folder` that is not a directory, and a 409 error when
    /// the project is open read-only or another job of the project runs.
    pub fn relink_search(&self, sid: &str, folder: &str) -> Result<Value, AppError> {
        let _span = crate::trace::stage("relink:search");
        let t0 = Instant::now();
        let op = self.inner.registry.get_writable(sid)?;
        let folder = abspath(Path::new(folder.trim()));
        if !folder.is_dir() {
            return Err(AppError::not_found(format!("no such folder: {}", folder.display())));
        }
        let (targets, dir) = {
            let mut g = lock(&op);
            g.resolve(); // a file moved since the open or the last look shows up now
            let mut t = Vec::new();
            for (name, tr) in g.project.doc.tracks() {
                if g.offline(name)
                    && let Some(rec) = tr.source_ref()
                {
                    t.push(SearchTarget {
                        track: name.clone(),
                        record: rec.clone(),
                    });
                }
                // the layer file of the track: offline when its record does not resolve to the file
                if let Some(l) = crate::layer::layer_of(&tr.extra)
                    && resolve_source(Some(&l.source), &g.project.dir).1 != SourceStatus::Ok
                {
                    t.push(SearchTarget {
                        track: format!("{name}{LAYER_KEY}"),
                        record: l.source,
                    });
                }
            }
            (t, g.project.dir.clone())
        };
        let job = self.inner.copyjobs.start(sid, "relink")?;
        let meter = crate::copyjob::Meter::new(&job, sid, None, targets.len(), 0);
        let cancelled = || job.cancelled();
        let report = match relink_search(&folder, &targets, &cancelled) {
            Ok(r) => r,
            Err(e) => {
                meter.finish("error", None, Some(&e.to_string()));
                return Err(e.into());
            }
        };
        let (mut found, mut ambiguous, mut missing) = (Vec::new(), Vec::new(), Vec::new());
        let mut relinked = 0usize;
        if !report.cancelled {
            let mut g = lock(&op);
            for r in &report.results {
                match &r.hit {
                    SearchHit::Found { path, by } => {
                        let (tname, layer) = split_key(&r.track);
                        let old = targets.iter().find(|t| t.track == r.track).map(|t| t.record.clone());
                        let (Some(old), true) = (old, g.project.doc.track(tname).is_some()) else {
                            continue;
                        };
                        let new = match relinked_source(&old, path, &dir) {
                            Ok(n) => n,
                            Err(e) => {
                                missing.push(
                                    json!({"track": tname, "layer": layer, "differs": [], "error": e.to_string()}),
                                );
                                continue;
                            }
                        };
                        if layer {
                            // the layer record of the track (its other settings stay)
                            if let Some(tr) = g.project.doc.track_mut(tname)
                                && let Some(mut l) = crate::layer::layer_of(&tr.extra)
                            {
                                l.source = new;
                                tr.extra.insert("layer".into(), l.to_json());
                            }
                        } else {
                            if let Some(tr) = g.project.doc.track_mut(tname) {
                                tr.source = Some(Some(new));
                            }
                            g.paths.insert(tname.to_string(), Some(path.clone()));
                            g.status.insert(tname.to_string(), SourceStatus::Ok);
                        }
                        relinked += 1;
                        found.push(
                            json!({"track": tname, "layer": layer, "path": path.display().to_string(), "by": by}),
                        );
                    }
                    SearchHit::Ambiguous(c) => {
                        let (tname, layer) = split_key(&r.track);
                        ambiguous.push(json!({"track": tname, "layer": layer, "candidates": paths_json(c)}));
                    }
                    SearchHit::Missing { differs } => {
                        let (tname, layer) = split_key(&r.track);
                        missing.push(json!({"track": tname, "layer": layer, "differs": paths_json(differs)}));
                    }
                }
            }
            if relinked > 0 {
                g.mark_dirty();
            }
        }
        // P4-4: followers that went stale while offline are derived again
        let followers = if relinked > 0 {
            crate::follow::refresh_stale(self, &op)
        } else {
            Vec::new()
        };
        // a relinked layer file reaches an open transport at once
        let transport_error = if found.iter().any(|f| f["layer"] == true) {
            self.transport_refresh_layers(sid).err().map(|e| e.to_string())
        } else {
            None
        };
        let result = json!({
            "transport_error": transport_error,
            "followers": followers,
            "offline": targets.len(), "relinked": relinked,
            "found": found, "ambiguous": ambiguous, "missing": missing,
            "files_seen": report.files_seen, "dirs_seen": report.dirs_seen,
            "truncated": report.truncated, "cancelled": report.cancelled,
            "elapsed_ms": t0.elapsed().as_secs_f64() * 1000.0,
        });
        meter.finish(if report.cancelled { "cancelled" } else { "done" }, None, None);
        Ok(result)
    }
}

// ---------- the bridge ----------

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{key}` (string)")))
}

/// Run a command of this card for the bridge; `None` when `cmd` is not one of them. The
/// argument names are those of the Tauri wrappers in `apps/desktop/src-tauri/src/jobs.rs`.
pub fn dispatch(svc: &Service, _channels: &Arc<Channels>, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    let run = || -> Result<Value, AppError> {
        match cmd {
            "project_relink" => svc.project_relink(
                arg_str(args, "sid")?,
                arg_str(args, "track")?,
                arg_str(args, "path")?,
                args.get("force").and_then(Value::as_bool).unwrap_or(false),
            ),
            "relink_search" => svc.relink_search(arg_str(args, "sid")?, arg_str(args, "folder")?),
            _ => Err(AppError::not_found(format!("unknown command {cmd}"))),
        }
    };
    RELINK_COMMANDS.contains(&cmd).then(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_ignores_other_commands_and_checks_arguments() {
        let svc = Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        let ch = Arc::new(Channels::default());
        assert!(dispatch(&svc, &ch, "project_open", &json!({})).is_none());
        let e = dispatch(&svc, &ch, "relink_search", &json!({"sid": "x"}))
            .unwrap()
            .unwrap_err();
        assert_eq!(e.code, 400);
        let e = dispatch(&svc, &ch, "relink_search", &json!({"sid": "x", "folder": "/tmp"}))
            .unwrap()
            .unwrap_err();
        assert_eq!(e.code, 404, "no such project");
    }

    #[test]
    fn paths_json_lists_paths_as_strings() {
        assert_eq!(paths_json(&[PathBuf::from("/a/b.wav")]), json!(["/a/b.wav"]));
    }
}
