//! P3-4b: the service commands of the native shell: `export_existing` (the Export dialog says before it writes
//! which files it replaces). P4-30: `export_existing` takes the dialog's options, `export_dest_set` keeps the
//! destination folder per project, `export_drag_prepare` pre-renders the files of the dialog's drag tiles into
//! `Exports/.drag/` and `export_drag_take` moves the dragged ones into `Exports/` (#433).
//! P4-31: `export_last_get`/`export_last_set` keep the export shape of the project (the choices the Export
//! dialog shows again and the Export again key repeats).
//! Everything else of the shell (menus, dialogs, close prompt, Finder open) is in
//! `apps/desktop/src-tauri/src/{menu,window,open}.rs` and `static/shell.js`.

use std::path::PathBuf;

use drumspike_core::project::EXPORTS_DIR;
use serde_json::{Value, json};

use crate::Service;
use crate::error::AppError;
use crate::export::{ExportOpts, MidiMode, check_dest, check_export_plan, multi_file_name, select_tracks};
use crate::registry::lock;

/// The commands of this card that the service owns (the bridge and the Tauri wrapper use the names).
pub const SHELL_COMMANDS: [&str; 8] = [
    "export_existing",
    "export_dest_set",
    "export_markers_set",
    "export_compensate_set",
    "export_drag_prepare",
    "export_drag_take",
    "export_last_get",
    "export_last_set",
];

/// The document key of the remembered export folder (P4-30; optional, absent means `Exports/`).
pub const DEST_KEY: &str = "export_dest";

/// The document key of the export shape of the project (P4-31; absent: the project never exported and the
/// dialog keeps no choices yet).
pub const LAST_KEY: &str = "export_last";

/// #433: the hidden folder of the pre-rendered drag files, inside the project's `Exports/` folder. A drag moves
/// its files out of it into `Exports/` under the export names, so the dragged file is a real export.
pub const DRAG_DIR: &str = ".drag";

/// #433: the drag folder of a project: `<project>/Exports/.drag`.
pub(crate) fn drag_dir_of(g: &crate::registry::OpenProject) -> PathBuf {
    g.project.subdir(EXPORTS_DIR).join(DRAG_DIR)
}

impl Service {
    /// #433: the drag folder of an open project (`Exports/.drag`).
    ///
    /// # Errors
    /// 404 when `sid` is not an open project.
    pub fn drag_dir(&self, sid: &str) -> Result<PathBuf, AppError> {
        let op = self.inner.registry.get(sid)?;
        let dir = drag_dir_of(&lock(&op));
        Ok(dir)
    }
}

/// #433: remove the pre-rendered drag files (the dialog closes or cancels, an option changes, the project closes).
/// Nothing there is not an error, and a file in `Exports/` is never touched.
pub(crate) fn drag_clear_dir(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

impl Service {
    /// `export_existing {sid, dest?, tracks?, formats?, midi_mode?}`: the files that the next `export_all` with the
    /// same options would replace. `files` are `Exports/<file>` for the default folder and `<file>` for a `dest`;
    /// the single MIDI file counts instead of the per-track MIDI files. Answer `{count, files, dest,
    /// default_dest, remembered}`: `dest` is the folder checked, `remembered` the folder the project keeps
    /// (`export_dest_set`) or null. A bad `dest` is the 400 or 404 of [`check_dest`].
    ///
    /// # Errors
    /// The same as [`Service::export_existing_opts`] with the default options (so only 404 when
    /// `sid` is not an open project).
    pub fn export_existing(&self, sid: &str) -> Result<Value, AppError> {
        self.export_existing_opts(sid, &ExportOpts::default())
    }

    /// P4-30: [`Service::export_existing`] with the options of the dialog.
    ///
    /// # Errors
    /// 404 when `sid` is not an open project; the 400 or 404 of [`check_dest`] for a bad `dest`;
    /// an error from `select_tracks` for an unknown track name in `opts.tracks`.
    pub fn export_existing_opts(&self, sid: &str, opts: &ExportOpts) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        let g = lock(&op);
        let default_dir = g.project.subdir(EXPORTS_DIR);
        let remembered = g
            .project
            .doc
            .extra
            .get(DEST_KEY)
            .and_then(Value::as_str)
            .map(str::to_string);
        let (dir, custom) = match &opts.dest {
            Some(d) => (check_dest(d)?, true),
            None => (default_dir.clone(), false),
        };
        let all: Vec<String> = g.project.doc.tracks().map(|(n, _)| n.clone()).collect();
        let names = select_tracks(all, opts.tracks.as_deref())?;
        let shown = |file: &str| {
            if custom {
                file.to_string()
            } else {
                format!("{EXPORTS_DIR}/{file}")
            }
        };
        let mut files = Vec::new();
        if opts.midi && opts.midi_mode == MidiMode::Single {
            let f = multi_file_name(&g.project.name());
            if !names.is_empty() && dir.join(&f).is_file() {
                files.push(Value::from(shown(&f)));
            }
        }
        for name in &names {
            if crate::export::file_owner(&g, name).is_some() {
                continue; // P4-4: Export All does not write this track's files (they are another track's)
            }
            let mut exts = Vec::new();
            if opts.midi && opts.midi_mode == MidiMode::PerTrack {
                exts.push("mid");
            }
            if opts.spikes {
                exts.push("wav");
            }
            for ext in exts {
                let file = format!("{name}_spike.{ext}");
                if dir.join(&file).is_file() {
                    files.push(Value::from(shown(&file)));
                }
            }
            if opts.audio {
                // #218: the audio-spike file of the track counts as replaced too
                let file = format!("{name}_audio_spike.wav");
                if dir.join(&file).is_file() {
                    files.push(Value::from(shown(&file)));
                }
            }
        }
        Ok(
            json!({"count": files.len(), "files": files, "dest": dir.display().to_string(),
                  "default_dest": default_dir.display().to_string(), "remembered": remembered,
                  "export_markers": crate::export::doc_export_markers_on(&g.project.doc),
                  "export_compensate": crate::export::doc_export_compensate(&g.project.doc),
                  "export_last": g.project.doc.extra.get(LAST_KEY).cloned().unwrap_or(Value::Null)}),
        )
    }

    /// `export_dest_set {sid, dest?}`: keep `dest` as the export folder of the project (the document field
    /// `export_dest`; the project is dirty until it is saved). A null or empty `dest` removes the field (back
    /// to `Exports/`). The folder must pass [`check_dest`]. Answer `{export_dest}`.
    ///
    /// # Errors
    /// 404 when `sid` is not an open project; 409 when the project is open read-only; the 400 or
    /// 404 of [`check_dest`] for a `dest` that is not a usable folder.
    pub fn export_dest_set(&self, sid: &str, dest: Option<&str>) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let dest = dest.filter(|d| !d.is_empty());
        let keep = match dest {
            Some(d) => Some(check_dest(d)?.display().to_string()),
            None => None,
        };
        let mut g = lock(&op);
        let before = g
            .project
            .doc
            .extra
            .get(DEST_KEY)
            .and_then(Value::as_str)
            .map(str::to_string);
        if before != keep {
            match &keep {
                Some(d) => {
                    g.project.doc.extra.insert(DEST_KEY.into(), Value::from(d.as_str()));
                }
                None => {
                    g.project.doc.extra.shift_remove(DEST_KEY);
                }
            }
            g.mark_dirty();
        }
        Ok(json!({"export_dest": keep}))
    }

    /// `export_markers_set {sid, on}`: keep the Export dialog's "Markers in MIDI" checkbox for the project
    /// (the document field `export_markers` of `crate::export`; the project is dirty until it is saved).
    /// Off removes the field (the default). Answer `{export_markers}`.
    ///
    /// # Errors
    /// 404 when `sid` is not an open project; 409 when the project is open read-only.
    pub fn export_markers_set(&self, sid: &str, on: bool) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let mut g = lock(&op);
        let before = crate::export::doc_export_markers_on(&g.project.doc);
        if before != on {
            if on {
                g.project
                    .doc
                    .extra
                    .insert(crate::export::EXPORT_MARKERS_KEY.into(), Value::from(true));
            } else {
                g.project.doc.extra.shift_remove(crate::export::EXPORT_MARKERS_KEY);
            }
            g.mark_dirty();
        }
        Ok(json!({"export_markers": on}))
    }

    /// `export_compensate_set {sid, compensate}`: keep the Export dialog's "Compensate" checkbox for
    /// the project (#232, the document field `export_compensate` of `crate::export`; the project is
    /// dirty until it is saved). An `export_all` without a per-call `compensate` uses it. Answer
    /// `{export_compensate}`.
    ///
    /// # Errors
    /// 404 when `sid` is not an open project; 409 when the project is open read-only.
    pub fn export_compensate_set(&self, sid: &str, compensate: bool) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let mut g = lock(&op);
        let before = crate::export::doc_export_compensate(&g.project.doc);
        let default = crate::command_policy::COMPENSATE_DEFAULT;
        if before != compensate {
            if compensate != default {
                g.project
                    .doc
                    .extra
                    .insert(crate::export::EXPORT_COMPENSATE_KEY.into(), Value::from(compensate));
            } else {
                g.project.doc.extra.shift_remove(crate::export::EXPORT_COMPENSATE_KEY);
            }
            g.mark_dirty();
        }
        Ok(json!({"export_compensate": compensate}))
    }

    /// `export_last_get {sid}`: the export shape of the project (the choices the Export dialog shows again and
    /// the Export again key repeats), or `null` before the first export. Answer `{export_last}`.
    ///
    /// # Errors
    /// 404 when `sid` is not an open project.
    pub fn export_last_get(&self, sid: &str) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        let g = lock(&op);
        Ok(json!({
            "export_last": g.project.doc.extra.get(LAST_KEY).cloned().unwrap_or(Value::Null),
            "export_compensate": crate::export::doc_export_compensate(&g.project.doc),
        }))
    }

    /// `export_last_set {sid, formats, midi_mode?, compensate?, scope?, track?, dest?}`: keep the export shape
    /// of the project (the document field `export_last`: `{formats, midi_mode, compensate, scope, track, dest}`;
    /// the project is dirty until it is saved). The page records the dialog's choices at once (like the folder
    /// and the markers checkbox) and what an export ran with. `dest: null` is `Exports/` in the project folder;
    /// a `dest` must pass [`check_dest`]. `scope: "selected"` names the `track`. Answer `{export_last, changed}`.
    ///
    /// # Errors
    /// 404 when `sid` is not an open project; 409 when the project is open read-only; 400 when
    /// `formats` is missing, empty or unknown, `midi_mode`, `compensate`, `scope`, `track` or
    /// `dest` has the wrong type or value, a selected scope has no `track`, or `dest` fails
    /// [`check_dest`].
    pub fn export_last_set(&self, sid: &str, args: &Value) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let last = last_from_args(args)?;
        let mut g = lock(&op);
        let changed = g.project.doc.extra.get(LAST_KEY) != Some(&last);
        if changed {
            g.project.doc.extra.insert(LAST_KEY.into(), last.clone());
            g.mark_dirty();
        }
        Ok(json!({"export_last": last, "changed": changed}))
    }

    /// `export_drag_prepare {sid, clear: true}`: remove the pre-rendered drag files (the dialog closes or cancels).
    /// Answers the empty form of [`Service::export_drag_prepare`].
    ///
    /// # Errors
    /// 404 when `sid` is not an open project.
    pub fn export_drag_clear(&self, sid: &str) -> Result<Value, AppError> {
        let dir = self.drag_dir(sid)?;
        drag_clear_dir(&dir);
        Ok(
            json!({"dir": dir.display().to_string(), "midi": [], "spikes": [], "errors": [], "outside": {}, "followers": []}),
        )
    }

    /// `export_drag_prepare {sid, tracks?, formats?, midi_mode?}`: pre-render the files of the dialog's drag tiles
    /// into `Exports/.drag/` (#433: the same names and bytes as Export; the folder is replaced first, so an option
    /// change leaves only the files of the new options; the `dest` option is ignored). Nothing reaches `Exports/`
    /// until [`Service::export_drag_take`]. Answer `{dir, midi: [paths], spikes: [paths], errors, outside,
    /// followers}`; `midi` holds the one file of the single mode. The render runs the same derivation as the export
    /// it previews ([`Service::export_all_opts`]), so `followers` names the tracks whose hits the render derived
    /// again (P4-4): the page applies them like an export's, or a derive that happened while the dialog was open
    /// never reaches it (the real export then finds everything fresh and names none).
    ///
    /// # Errors
    /// 404 when `sid` is not an open project; 409 when the project is open read-only; 500 when the
    /// drag folder cannot be created; any error of [`Service::export_all_opts`] for the options.
    pub fn export_drag_prepare(&self, sid: &str, opts: &ExportOpts) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        drop(op);
        let dir = self.drag_dir(sid)?;
        drag_clear_dir(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| AppError::internal(format!("{}: {e}", dir.display())))?;
        let own = ExportOpts {
            dest: Some(dir.display().to_string()),
            ..opts.clone()
        };
        let r = self.export_all_opts(sid, &own, None)?;
        let mut midi = Vec::new();
        let mut spikes = Vec::new();
        for f in r["files"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if f.ends_with(".mid") {
                midi.push(Value::from(f));
            } else {
                spikes.push(Value::from(f));
            }
        }
        Ok(json!({"dir": dir.display().to_string(), "midi": midi, "spikes": spikes,
                  "errors": r["errors"], "outside": r["outside"], "followers": r["followers"]}))
    }

    /// `export_drag_take {sid, paths}`: a drag starts. Each pre-rendered file of `paths` (a file of
    /// `Exports/.drag/` from [`Service::export_drag_prepare`]) moves to `Exports/` under its own name, one rename
    /// (atomic, same folder tree), so the dragged file is the export the Export command writes. Answer
    /// `{files: [paths in Exports/]}`. A path outside the drag folder is a 400, a file that is not there (never
    /// rendered, or already taken) a 404, and an output that would replace a protected file a 409 ([`check_export_plan`]).
    ///
    /// # Errors
    /// 404 when `sid` is not an open project, or a drag file is not rendered; 409 when the project
    /// is open read-only or an output would replace a protected file; 400 when a path is not in
    /// the project's drag folder; 500 when `Exports/` cannot be created or a rename fails.
    pub fn export_drag_take(&self, sid: &str, paths: &[String]) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let (drag, exports) = {
            let g = lock(&op);
            (drag_dir_of(&g), g.project.subdir(EXPORTS_DIR))
        };
        drop(op);
        std::fs::create_dir_all(&exports).map_err(|e| AppError::internal(format!("{}: {e}", exports.display())))?;
        let protected = self.protected_files_everywhere();
        let mut files = Vec::with_capacity(paths.len());
        for p in paths {
            let src = PathBuf::from(p);
            let name = match (src.parent(), src.file_name()) {
                (Some(parent), Some(name)) if parent == drag => name.to_os_string(),
                _ => return Err(AppError::bad(format!("not a drag file of this project: {p}"))),
            };
            if !src.is_file() {
                return Err(AppError::not_found(format!("the drag file is not rendered: {p}")));
            }
            let out = exports.join(name);
            check_export_plan(std::slice::from_ref(&out), &protected)?;
            std::fs::rename(&src, &out).map_err(|e| AppError::internal(format!("{}: {e}", out.display())))?;
            files.push(Value::from(out.display().to_string()));
        }
        Ok(json!({"files": files}))
    }
}

/// The validated `export_last_set` arguments as the document keeps them (P4-31): `formats` needs midi, spikes
/// and/or audio (#218: the spike type of the dialog), `midi_mode` is `per_track` (default) or `single`,
/// `compensate` is on unless it is off, `scope` is `all` (default) or `selected` and a selected scope names the
/// `track`, `dest` passes [`check_dest`] (or is `null` for `Exports/`).
fn last_from_args(args: &Value) -> Result<Value, AppError> {
    let formats = match args.get("formats") {
        Some(Value::Array(a)) => {
            let f: Vec<String> = a
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| AppError::bad("`formats` must hold strings"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(bad) = f.iter().find(|x| *x != "midi" && *x != "spikes" && *x != "audio") {
                return Err(AppError::bad(format!(
                    "unknown format {bad}: use midi, spikes and audio"
                )));
            }
            if f.is_empty() {
                return Err(AppError::bad("`formats` needs midi, spikes or audio"));
            }
            f
        }
        _ => return Err(AppError::bad("`formats` needs midi, spikes or audio")),
    };
    let midi_mode = match args.get("midi_mode") {
        None | Some(Value::Null) => MidiMode::PerTrack,
        Some(Value::String(m)) => MidiMode::parse(Some(m.as_str()))?,
        Some(_) => return Err(AppError::bad("`midi_mode` must be a string")),
    };
    let midi_mode = match midi_mode {
        MidiMode::PerTrack => "per_track",
        MidiMode::Single => "single",
    };
    let compensate = match args.get("compensate") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(AppError::bad("`compensate` must be true or false")),
    };
    let scope = match args.get("scope") {
        None | Some(Value::Null) => "all",
        Some(Value::String(s)) if s == "all" || s == "selected" => s.as_str(),
        Some(Value::String(s)) => return Err(AppError::bad(format!("scope must be all or selected, not {s}"))),
        Some(_) => return Err(AppError::bad("`scope` must be a string")),
    };
    let track = match args.get("track") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) if !t.is_empty() => Some(t.clone()),
        Some(_) => return Err(AppError::bad("`track` must be a string")),
    };
    if scope == "selected" && track.is_none() {
        return Err(AppError::bad("scope selected needs the track name"));
    }
    let dest = match args.get("dest") {
        None | Some(Value::Null) => None,
        Some(Value::String(d)) if d.is_empty() => None,
        Some(Value::String(d)) => Some(check_dest(d)?.display().to_string()),
        Some(_) => return Err(AppError::bad("`dest` must be a string")),
    };
    Ok(json!({
        "formats": formats, "midi_mode": midi_mode, "compensate": compensate,
        "scope": scope, "track": track, "dest": dest,
    }))
}

/// Run a command of this card for the bridge; `None` when `cmd` is not one of them.
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    if !SHELL_COMMANDS.contains(&cmd) {
        return None;
    }
    let Some(sid) = args.get("sid").and_then(Value::as_str) else {
        return Some(Err(AppError::bad("missing or wrong argument `sid` (string)")));
    };
    Some(match cmd {
        "export_existing" => ExportOpts::from_args(args).and_then(|o| svc.export_existing_opts(sid, &o)),
        "export_drag_prepare" if args.get("clear").and_then(Value::as_bool) == Some(true) => svc.export_drag_clear(sid),
        "export_drag_prepare" => ExportOpts::from_args(args).and_then(|o| svc.export_drag_prepare(sid, &o)),
        "export_drag_take" => match args.get("paths") {
            Some(Value::Array(a)) => a
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| AppError::bad("`paths` must hold strings"))
                })
                .collect::<Result<Vec<_>, _>>()
                .and_then(|paths| svc.export_drag_take(sid, &paths)),
            _ => Err(AppError::bad("`paths` must be a list of file paths")),
        },
        "export_markers_set" => match args.get("on") {
            Some(Value::Bool(on)) => svc.export_markers_set(sid, *on),
            _ => Err(AppError::bad("`on` must be true or false")),
        },
        "export_compensate_set" => match args.get("compensate") {
            Some(Value::Bool(b)) => svc.export_compensate_set(sid, *b),
            _ => Err(AppError::bad("`compensate` must be true or false")),
        },
        "export_last_get" => svc.export_last_get(sid),
        "export_last_set" => svc.export_last_set(sid, args),
        _ => match args.get("dest") {
            None | Some(Value::Null) => svc.export_dest_set(sid, None),
            Some(Value::String(d)) => svc.export_dest_set(sid, Some(d)),
            Some(_) => Err(AppError::bad("`dest` must be a string")),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_knows_only_its_command_and_checks_the_argument() {
        let svc = Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        assert!(dispatch(&svc, "project_open", &json!({})).is_none());
        assert_eq!(
            dispatch(&svc, "export_existing", &json!({})).unwrap().unwrap_err().code,
            400
        );
        assert_eq!(
            dispatch(&svc, "export_existing", &json!({"sid": "nope"}))
                .unwrap()
                .unwrap_err()
                .code,
            404
        );
    }

    #[test]
    fn the_command_list_names_the_command() {
        assert_eq!(
            SHELL_COMMANDS,
            [
                "export_existing",
                "export_dest_set",
                "export_markers_set",
                "export_compensate_set",
                "export_drag_prepare",
                "export_drag_take",
                "export_last_get",
                "export_last_set"
            ]
        );
    }

    #[test]
    fn export_last_arguments_are_validated_and_the_defaults_fill_in() {
        let last = last_from_args(&json!({"formats": ["midi"], "midi_mode": "single"})).unwrap();
        assert_eq!(
            last,
            json!({"formats": ["midi"], "midi_mode": "single", "compensate": true, "scope": "all", "track": null, "dest": null})
        );
        assert_eq!(
            last_from_args(&json!({"formats": [], "scope": "selected"}))
                .unwrap_err()
                .code,
            400
        );
        assert_eq!(last_from_args(&json!({"formats": ["zip"]})).unwrap_err().code, 400);
        assert_eq!(
            last_from_args(&json!({"formats": ["midi"], "scope": "selected", "track": ""}))
                .unwrap_err()
                .code,
            400
        );
        assert_eq!(
            last_from_args(&json!({"formats": ["midi"], "scope": "both"}))
                .unwrap_err()
                .code,
            400
        );
        assert_eq!(
            last_from_args(&json!({"formats": ["midi"], "dest": "relative/dir"}))
                .unwrap_err()
                .code,
            400
        );
        let sel = last_from_args(&json!({"formats": ["spikes"], "scope": "selected", "track": "Kick In"})).unwrap();
        assert_eq!(sel["track"], "Kick In");
        // #218: the spike type of the dialog is part of the record
        let audio = last_from_args(&json!({"formats": ["midi", "audio"]})).unwrap();
        assert_eq!(audio["formats"], json!(["midi", "audio"]));
        let both = last_from_args(&json!({"formats": ["spikes", "audio"]})).unwrap();
        assert_eq!(both["formats"], json!(["spikes", "audio"]));
    }
}
