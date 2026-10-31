//! Track import (P3-4c): the scan of dropped files and folders, and `tracks_add`.
//!
//! | Command | Server route (route map row) |
//! |---|---|
//! | `tracks_scan {paths, preset?}` | none: the list the page shows before it adds (role guess, rejected files) |
//! | `tracks_add {sid, paths, roles, notes, copy, progress}` | POST `/api/upload/{sid}` (23) |
//! | `tracks_remove {sid, names, snapshot?}`, `tracks_restore {sid, tracks}` | none (P4-39a: Track > Remove Track and its undo) |
//! | `kit_presets_list`, `kit_preset_save {name, rules}`, `kit_preset_load {name}` | none, see [`crate::roles`] |
//!
//! The server received bytes from the browser. The app gets paths: a file stays where it is
//! (`referenced`, decision 6 of the plan) unless `copy` is true; then every file is copied into
//! `Audio Files/` (byte-identical, the progress channel gets one message per file). Only the
//! header of a file is read (a memory map that touches no sample) plus the first MiB for the
//! source identity, so the import of 30 tracks of 5 minutes at 96 kHz reads 30 MiB.
//! #225: the scan of a file whose name gives no role decodes the file and runs the model-free
//! `role_from_audio` (#393) for the audio suggestion.
//!
//! A folder stands for its WAV files (not its sub-folders). A session folder with no WAV file
//! in it but with an `Audio Files/` folder stands for the files of that folder. Files the
//! analysis cannot read are not added; each comes back in `rejected` with the reason: AIFF and
//! other audio formats, a WAV file with a rate other than 44.1, 48 or 96 kHz, a file that is no
//! WAV file, a file with no samples, an unreadable file.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use drumspike_core::SourceStatus;
use drumspike_core::io::{MappedWav, MixMode};
use drumspike_core::project::{
    ANALYSIS_DIR, AUDIO_DIR, Track, add_track, build_source, check_track_name, copy_into_project_ctl,
};
use serde_json::{Value, json};

use crate::command_policy::opt_i64;
use crate::copyjob::Meter;
use crate::error::AppError;
use crate::registry::lock;
use crate::reply::Channels;
use crate::roles::{Rule, default_note, guess, is_role, role_color};
use crate::{Progress, Service};

/// The commands of this card, as the route map names them.
pub const TRACKS_COMMANDS: [&str; 8] = [
    "tracks_scan",
    "tracks_add",
    "tracks_remove",
    "tracks_restore",
    "track_set_role",
    "kit_presets_list",
    "kit_preset_save",
    "kit_preset_load",
];

/// The sample rates the analysis reads.
pub const RATES: [u32; 3] = [44_100, 48_000, 96_000];

/// A file the import will add: path and what the header says.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    /// Absolute path.
    pub path: PathBuf,
    /// File name with extension.
    pub name: String,
    /// File name without extension.
    pub stem: String,
    /// Size in bytes.
    pub size: u64,
    /// Sample rate.
    pub sr: u32,
    /// Channels.
    pub channels: u16,
    /// Bits per sample.
    pub bits: u16,
    /// Frames.
    pub n_frames: usize,
}

impl Probe {
    /// Duration in seconds.
    pub fn duration(&self) -> f64 {
        self.n_frames as f64 / f64::from(self.sr)
    }
}

/// A file the import does not add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reject {
    /// Absolute path.
    pub path: PathBuf,
    /// File name.
    pub name: String,
    /// The reason, in words for the user.
    pub reason: String,
}

impl Reject {
    fn new(path: &Path, reason: impl Into<String>) -> Reject {
        Reject {
            path: path.to_path_buf(),
            name: file_name(path),
            reason: reason.into(),
        }
    }

    fn to_json(&self) -> Value {
        json!({"path": self.path.display().to_string(), "name": self.name, "reason": self.reason})
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

fn ext_lower(p: &Path) -> String {
    p.extension()
        .map_or_else(String::new, |e| e.to_string_lossy().to_lowercase())
}

/// True for the WAV extensions the import reads.
fn is_wav_ext(ext: &str) -> bool {
    matches!(ext, "wav" | "wave" | "bwf")
}

/// True for an audio format that is not WAV (listed as rejected, with the reason).
fn is_other_audio_ext(ext: &str) -> bool {
    matches!(
        ext,
        "aif"
            | "aiff"
            | "aifc"
            | "flac"
            | "mp3"
            | "m4a"
            | "aac"
            | "ogg"
            | "oga"
            | "opus"
            | "caf"
            | "wma"
            | "mp4"
            | "mov"
    )
}

/// The first twelve bytes tell AIFF from WAV whatever the extension says.
fn magic(path: &Path) -> std::io::Result<[u8; 12]> {
    use std::io::Read;
    let mut b = [0u8; 12];
    let mut f = std::fs::File::open(path)?;
    let mut got = 0;
    while got < b.len() {
        let n = f.read(&mut b[got..])?;
        if n == 0 {
            break;
        }
        got += n;
    }
    Ok(b)
}

/// Read the header of `path` and decide: a [`Probe`] for a file the analysis reads, else the
/// [`Reject`] with the reason. No sample is read.
///
/// # Errors
/// A [`Reject`] when the file is missing or unreadable, is not a file, is AIFF or another
/// non-WAV audio format, is not a WAV file, has an unsupported WAV format or sample rate, or
/// holds no samples.
pub fn probe(path: &Path) -> Result<Probe, Reject> {
    let no = |why: String| Reject::new(path, why);
    let meta = std::fs::metadata(path).map_err(|e| {
        no(if e.kind() == std::io::ErrorKind::NotFound {
            "the file does not exist".into()
        } else {
            format!("cannot read the file: {e}")
        })
    })?;
    if !meta.is_file() {
        return Err(no("this is not a file".into()));
    }
    let ext = ext_lower(path);
    let head = magic(path).map_err(|e| no(format!("cannot read the file: {e}")))?;
    if &head[0..4] == b"FORM" && (&head[8..12] == b"AIFF" || &head[8..12] == b"AIFC") {
        return Err(no(
            "AIFF files are not supported: convert the file to WAV in your DAW (WAV, 44.1, 48 or 96 kHz)".into(),
        ));
    }
    if is_other_audio_ext(&ext) {
        return Err(no(format!(
            "{} files are not supported: convert the file to WAV in your DAW (WAV, 44.1, 48 or 96 kHz)",
            ext.to_uppercase()
        )));
    }
    if !is_wav_ext(&ext) && &head[0..4] != b"RIFF" {
        return Err(no(
            "this is no audio file Drum Spike reads (WAV, 44.1, 48 or 96 kHz)".into()
        ));
    }
    let wav = MappedWav::open(path).map_err(|e| match e {
        drumspike_core::WavError::Io(e) => no(format!("cannot read the file: {e}")),
        drumspike_core::WavError::Format(m) => no(format!("this WAV file is not supported ({m})")),
    })?;
    let i = &wav.info;
    if !RATES.contains(&i.sample_rate) {
        return Err(no(format!(
            "the sample rate {} Hz is not supported: Drum Spike reads 44.1, 48 or 96 kHz; convert the file in your DAW",
            i.sample_rate
        )));
    }
    if i.n_frames == 0 {
        return Err(no("the file holds no audio samples".into()));
    }
    Ok(Probe {
        path: path.to_path_buf(),
        name: file_name(path),
        stem: path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
        size: meta.len(),
        sr: i.sample_rate,
        channels: i.channels,
        bits: i.bits_per_sample,
        n_frames: i.n_frames,
    })
}

/// The files of a folder that the import looks at: WAV files and other audio formats, sorted by
/// name, without hidden files (`._k in.wav` of a Mac archive).
fn folder_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && !file_name(p).starts_with('.')
                        && (is_wav_ext(&ext_lower(p)) || is_other_audio_ext(&ext_lower(p)))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|p| file_name(p).to_lowercase());
    v
}

/// A folder named `<name>.partial`: a copy Save As that is being built or was left by a killed process.
fn is_partial_dir(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".partial"))
}

/// The files `paths` stand for, in order and without duplicates: a file stands for itself, a
/// folder for its audio files (see the module docs).
pub fn expand(paths: &[String]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        let path = Path::new(p.trim());
        // P4-5 (M1): an unfinished Save As (`<name>.partial`) is no source of audio: not the folder, not a file in it
        // (the copy is built in `<name>.partial/Audio Files/`, so every ancestor counts, not only the parent)
        if path.ancestors().any(is_partial_dir) {
            continue;
        }
        let found = if path.is_dir() {
            let mut f = folder_files(path);
            if f.is_empty() {
                f = folder_files(&path.join(AUDIO_DIR));
            }
            f
        } else {
            vec![path.to_path_buf()]
        };
        for f in found {
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

/// The track name of a file: letters, digits and `-` stay, every other run becomes one `_`
/// (`server.safe_name`); `track<n>` when nothing is left.
pub fn safe_name(stem: &str, idx: usize) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() || c == '-' {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    if out.is_empty() { format!("track{idx}") } else { out }
}

fn str_list(v: Option<&Value>) -> Vec<Value> {
    v.and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The progress sender of `tracks_add`: state, done, file, result.
type Emit<'a> = &'a dyn Fn(&str, usize, &str, Option<&Value>);

impl Service {
    /// Build `Analysis/<track>/peaks.bin` for `names` on up to [`crate::media::PEAK_WORKERS`] threads, so the lanes
    /// are ready when `tracks_add` answers (the 30 tracks of the exit test E3 take 1 to 2 s, one
    /// after the other 3 to 4 s). `on_done(done, total)` runs after each track, in order. A track
    /// whose build fails is left alone: `peaks_build_start` meets it later and reports the error
    /// for that track. Returns the time in ms (0 when nothing needed a build).
    fn prebuild_peaks(
        &self,
        op: &crate::registry::Shared,
        names: &[String],
        on_done: impl Fn(usize, usize) + Sync,
    ) -> f64 {
        let t0 = Instant::now();
        let reqs: Vec<crate::media::PeakRequest> = {
            let g = lock(op);
            names
                .iter()
                .filter_map(|n| crate::media::PeakRequest::of(&g, n).ok())
                .filter(|r| r.source.is_ok() && !drumspike_core::cache::cache_is_current(&r.dir, &r.key))
                .collect()
        };
        if reqs.is_empty() {
            return 0.0;
        }
        let finished = std::sync::Mutex::new(0usize);
        crate::media::on_peak_workers(reqs.len(), |i| {
            let _span = crate::trace::stage("tracks_add:peaks");
            let _ = self.inner.media.peak_cache(&reqs[i]);
            let mut n = lock(&finished);
            *n += 1;
            on_done(*n, reqs.len());
        });
        t0.elapsed().as_secs_f64() * 1000.0
    }

    /// `tracks_scan {paths, preset?}`: the files that `paths` stand for, each with the guessed
    /// role and note, and the rejected files with the reason. With `preset` the rules of that
    /// kit preset come first. Reads headers only, plus the model-free audio suggestion of
    /// [`Service::audio_suggestion`]
    /// for the files whose name gives no role (#225): their row carries the suggested role with
    /// `guess: "audio"`.
    ///
    /// # Errors
    /// The error of `load_rules` (for example 404 for an unknown kit preset) when `preset` is
    /// given. A file that cannot be read is listed in `rejected`, not an error.
    pub fn tracks_scan(&self, paths: &[String], preset: Option<&str>) -> Result<Value, AppError> {
        let t0 = Instant::now();
        let rules: Vec<Rule> = match preset.map(str::trim).filter(|p| !p.is_empty()) {
            Some(name) => self.load_rules(name)?.1,
            None => Vec::new(),
        };
        let mut files = Vec::new();
        let mut rejected = Vec::new();
        let mut guessed = 0;
        for f in expand(paths) {
            match probe(&f) {
                Ok(p) => {
                    let mut g = guess(&p.stem, &rules);
                    // a Drum Spike export (#216) gets no role, neither from its name nor from its audio
                    if g.source == crate::roles::Source::None
                        && !crate::roles::is_export_name(&p.stem)
                        && let Some(role) = self.audio_suggestion(&p).unwrap_or(None)
                    {
                        g = crate::roles::Guess {
                            role: role.to_string(),
                            note: default_note(role),
                            source: crate::roles::Source::Audio,
                            matched: None,
                        };
                    }
                    if matches!(g.source, crate::roles::Source::Preset | crate::roles::Source::Scheme) {
                        guessed += 1;
                    }
                    files.push(json!({
                        "path": p.path.display().to_string(), "name": p.name, "stem": p.stem,
                        "track": safe_name(&p.stem, files.len()),
                        "size": p.size, "sr": p.sr, "channels": p.channels, "bits": p.bits,
                        "n_samples": p.n_frames, "duration": p.duration(),
                        "role": g.role, "note": g.note, "guess": g.source.as_str(),
                    }));
                }
                Err(r) => rejected.push(r.to_json()),
            }
        }
        Ok(json!({
            "files": files, "rejected": rejected, "guessed": guessed,
            "elapsed_ms": t0.elapsed().as_secs_f64() * 1000.0,
        }))
    }

    /// #225 from audio: the role the audio of one probed file suggests, through the model-free
    /// `drumspike_core::role::role_from_audio` (#393, the hint mode: only the roles whose
    /// threshold bar exists can come back). Only the first 30 s are analysed (#406). Any
    /// decode error is no suggestion: the scan never fails on one file.
    fn audio_suggestion(&self, p: &Probe) -> Result<Option<&'static str>, AppError> {
        const SUGGEST_S: usize = 30;
        let decoded = match crate::media::decode(&p.path, MixMode::Ffmpeg) {
            Ok(d) => d,
            Err(_) => return Ok(None),
        };
        let n = decoded.samples.len().min(decoded.sr as usize * SUGGEST_S);
        Ok(drumspike_core::role::role_from_audio(&decoded.samples[..n], decoded.sr)
            .map(drumspike_core::role::Role::as_str))
    }

    /// `tracks_add {sid, paths, roles, notes, copy, progress}` (route 23): add one track per
    /// readable file. `roles` and `notes` are lists in the order of the files `paths` stand for
    /// (see [`expand`]); an entry that is missing, `null` or wrong gets the guess from the name
    /// (the note: the default note of the role). The source record is `referenced`; with
    /// `copy` the file is copied into `Audio Files/` and the record is `copied`. `progress`
    /// gets `{sid, add: {state, done, total, file}}` before and after each file (`state` is
    /// `running`, then `peaks` after each waveform file, then `done` with the result). The peak
    /// cache of the new tracks is built before the call answers, on up to six threads, so the
    /// lanes are ready; `peaks_build_start` then finds nothing to do. Answers
    /// `{added: [names], tracks: [{name, role, note, path, mode}], rejected, peaks_ms, elapsed_ms}`; files
    /// that are not added are in `rejected` (a request with only such files is not an error).
    ///
    /// # Errors
    /// 404 when `sid` is not an open project; 409 when the project is open read-only or the copy
    /// job cannot start; an error from `add_track` (for example a bad track name) after a file
    /// was accepted. A file that cannot be probed or copied is reported in `rejected`, not here.
    pub fn tracks_add(
        &self,
        sid: &str,
        paths: &[String],
        roles: &[Value],
        notes: &[Value],
        copy: bool,
        progress: Option<Progress>,
    ) -> Result<Value, AppError> {
        let t0 = Instant::now();
        let op = self.inner.registry.get_writable(sid)?;
        let dir = lock(&op).project.dir.clone();
        let list = expand(paths);
        let total = list.len();
        let emit = |state: &str, done: usize, file: &str, result: Option<&Value>| {
            if let Some(p) = progress.as_ref() {
                let mut m = json!({"state": state, "done": done, "total": total, "file": file});
                if let Some(r) = result {
                    m["result"] = r.clone();
                }
                p(json!({"sid": sid, "add": m}));
            }
        };
        let mode = if copy { "copied" } else { "referenced" };
        // P3-4d: a copy is a job of the project for `job_cancel`; a referenced add needs no cancel
        let job = if copy {
            Some(self.inner.copyjobs.start(sid, "add")?)
        } else {
            None
        };
        let meter = job.as_ref().map(|j| Meter::new(j, sid, None, total, 0));
        let mut created: Vec<PathBuf> = Vec::new();
        let (mut pending, mut rejected) = (Vec::new(), Vec::new());
        for (i, file) in list.iter().enumerate() {
            if job.as_ref().is_some_and(|j| j.cancelled()) {
                return Ok(self.add_cancelled(&created, &rejected, t0, &emit, meter.as_ref()));
            }
            emit("running", i, &file_name(file), None);
            let pr = match probe(file) {
                Ok(p) => p,
                Err(r) => {
                    rejected.push(r.to_json());
                    continue;
                }
            };
            let g = guess(&pr.stem, &[]);
            let named = roles
                .get(i)
                .and_then(Value::as_str)
                .is_none_or(|r| is_role(r) && r == g.role);
            let role = roles
                .get(i)
                .and_then(Value::as_str)
                .filter(|r| is_role(r))
                .map_or(g.role.clone(), str::to_string);
            let note = notes.get(i).and_then(Value::as_i64).filter(|n| (0..=127).contains(n));
            let note = note.unwrap_or(if role == g.role { g.note } else { default_note(&role) });
            if let Some(m) = &meter {
                m.file_start(i, &file_name(file));
            }
            let rec = build_source(&pr.path, &dir, "referenced").and_then(|r| match &meter {
                Some(m) if copy => match m.with_ctl(|ctl| copy_into_project_ctl(&pr.path, &r, &dir, None, ctl))? {
                    Some(o) => {
                        created.extend(o.created);
                        Ok(Some(o.source))
                    }
                    None => Ok(None),
                },
                _ => Ok(Some(r)),
            });
            let rec = match rec {
                Ok(Some(r)) => r,
                Ok(None) => return Ok(self.add_cancelled(&created, &rejected, t0, &emit, meter.as_ref())),
                Err(e) => {
                    rejected.push(Reject::new(&pr.path, format!("cannot add the file: {e}")).to_json());
                    continue;
                }
            };
            if let Some(m) = &meter {
                m.file_done(i, &file_name(file));
            }
            pending.push((i, pr, rec, role, note, named, g.source.as_str(), g.matched));
            emit("running", i + 1, &file_name(file), None);
        }
        if job.as_ref().is_some_and(|j| j.cancelled()) {
            return Ok(self.add_cancelled(&created, &rejected, t0, &emit, meter.as_ref()));
        }
        if let Some(m) = &meter {
            m.finish("done", None, None); // the peak build below is short and has no cancel
        }
        let (mut added, mut tracks) = (Vec::new(), Vec::new());
        for (i, pr, rec, role, note, named, role_source, role_match) in pending {
            let name = {
                let mut g = lock(&op);
                let base = safe_name(&pr.stem, i);
                let mut name = base.clone();
                let mut k = 1;
                // P4-4: `Kick` and `kick` would share their files on a disk that ignores case
                while g
                    .project
                    .doc
                    .tracks()
                    .any(|(n, _)| crate::export::same_file_name(n, &name))
                {
                    k += 1;
                    name = format!("{base}_{k}");
                }
                let path = rec.path.clone().map(PathBuf::from);
                let tr = add_track(&mut g.project, &name, Some(rec), &role, note, None)?;
                tr.sr = Some(i64::from(pr.sr));
                tr.n_samples = Some(pr.n_frames as i64);
                tr.duration = Some(pr.duration());
                tr.color = Some(role_color(&role).to_string());
                // 0.9.1: the role came from the name when the caller kept the guess (the hint of
                // the Inspector tells the two apart); a role of the caller is the user's
                if named {
                    tr.role_source = Some(role_source.to_string());
                    tr.role_match = role_match;
                }
                // the record was built from this file a moment ago: no second hash to resolve it
                g.paths.insert(name.clone(), path);
                g.status.insert(name.clone(), SourceStatus::Ok);
                g.mark_dirty();
                name
            };
            let row = json!({"name": name, "role": role, "note": note,
                "path": pr.path.display().to_string(), "mode": mode});
            added.push(name);
            tracks.push(row);
        }
        let peaks_ms = self.prebuild_peaks(&op, &added, |done, total| {
            if let Some(p) = progress.as_ref() {
                p(json!({"sid": sid, "add": {"state": "peaks", "done": done, "total": total, "file": ""}}));
            }
        });
        let result = json!({
            "added": added, "tracks": tracks, "rejected": rejected,
            "peaks_ms": peaks_ms, "elapsed_ms": t0.elapsed().as_secs_f64() * 1000.0,
        });
        emit("done", total, "", Some(&result));
        Ok(result)
    }
}

impl Service {
    /// The end of a cancelled copy-mode `tracks_add`: remove the files this call copied (the
    /// project got no track, so it is as it was), end the job as `cancelled`, tell the page.
    fn add_cancelled(
        &self,
        created: &[PathBuf],
        rejected: &[Value],
        t0: Instant,
        emit: Emit<'_>,
        meter: Option<&Meter>,
    ) -> Value {
        for f in created {
            let _ = std::fs::remove_file(f);
        }
        if let Some(m) = meter {
            m.finish("cancelled", None, None);
        }
        let result = json!({
            "cancelled": true, "added": [], "tracks": [], "rejected": rejected,
            "peaks_ms": 0.0, "elapsed_ms": t0.elapsed().as_secs_f64() * 1000.0,
        });
        emit("cancelled", 0, "", Some(&result));
        result
    }
}

impl Service {
    /// `track_set_role {sid, name, role, note?, source?, role_match?, events?}` (0.9.1): set the role of one
    /// track in the Inspector. `role` must be one of [`crate::roles::ROLES`]. `note` on request
    /// only (the page offers the new role's default note). `source` records where the role comes
    /// from (`user`, `guess`, `audio`; the undo also takes `scheme`, `none`, `preset`); without
    /// it a changed role counts as `user`, an unchanged one keeps its source. `role_match` (the
    /// undo) restores the matched words of the name; a role of `user` or `audio` has none. With
    /// `events` (the undo of the page) the hits are restored exactly
    /// and nothing re-runs; otherwise a changed role re-analyzes the track: a stored re-pick at
    /// the stored params (`track_repick_keep`, what a re-analyze gives at the same params) from
    /// the kept activations — skipped (and reported as `reanalyzed: false`) for a follower, an
    /// offline track or a track without activations, and 409 while an analysis or another write
    /// of the project runs. R09: the command validates and prepares first; a rejection (a busy
    /// project, a non-list `events`, a failing re-pick) leaves the document and the dirty flag
    /// as they were. Answers
    /// `{name, role, note, role_source, suggest_note, reanalyzed, previous: {role, note,
    /// role_source, role_match, events}}`; `previous` is the undo state, `suggest_note` the new role's
    /// default note when the stored note was the old role's default and no note was given.
    ///
    /// # Errors
    /// 400 when `role` is not a known role, `source` is not a known source, `note` is outside 0
    /// to 127, or `events` is not a list; 404 when `sid` is not an open project or `name` is not a
    /// track; 409 when the project is open read-only or an analysis or another write runs; any
    /// error of the re-pick, after which the role fields are put back.
    #[allow(clippy::too_many_arguments)]
    pub fn track_set_role(
        &self,
        sid: &str,
        name: &str,
        role: &str,
        note: Option<i64>,
        source: Option<&str>,
        role_match: Option<&str>,
        events: Option<&Value>,
    ) -> Result<Value, AppError> {
        if !is_role(role) {
            return Err(AppError::bad(format!(
                "`role` must be one of {}",
                crate::roles::ROLES.join(", ")
            )));
        }
        if let Some(s) = source
            && !matches!(s, "user" | "guess" | "audio" | "scheme" | "none" | "preset")
        {
            return Err(AppError::bad(format!(
                "wrong `source` {s} (user, guess, audio; the undo also takes scheme, none, preset)"
            )));
        }
        if let Some(n) = note
            && !(0..=127).contains(&n)
        {
            return Err(AppError::bad(format!("note must be 0 to 127, not {n}")));
        }
        let op = self.inner.registry.get_writable(sid)?;
        // R09: a busy project (an analysis queued or running, another write active) refuses the
        // whole command before anything changes. The documented re-pick skips (a follower, an
        // offline track, no kept activations) are not busy: they keep the role change and come
        // back as `reanalyzed: false`.
        self.check_idle(sid)?;
        let previous_events = self.events_of(sid, name)?;
        let old = {
            let g = lock(&op);
            let tr = g.track(name)?;
            (
                tr.role.clone(),
                tr.note,
                tr.role_source.clone(),
                tr.role_match.clone(),
                default_note(tr.role.as_deref().unwrap_or("other")),
            )
        };
        let changed = old.0.as_deref() != Some(role);
        let source = source.map(str::to_string).unwrap_or_else(|| {
            if changed {
                "user".into()
            } else {
                old.2.clone().unwrap_or_else(|| "user".into())
            }
        });
        let skip = changed && events.is_none() && self.repick_skip(&op, name)?;
        // the role fields and the hits go in as one command: the undo's `events` are stored
        // first (a non-list refuses with 400 before the role goes in), and a failing re-pick
        // puts the role fields back, so a rejection leaves the document (and the dirty flag)
        // as it was
        if let Some(e) = events {
            self.events_set(sid, name, e)?;
        }
        {
            let mut g = lock(&op);
            let tr = g.track_mut(name)?;
            tr.role = Some(role.to_string());
            if let Some(n) = note {
                tr.note = Some(n);
            }
            tr.role_source = Some(source.clone());
            // the hint's matched words: a role of the user or of the audio has none; the undo
            // restores the recorded ones (`role_match`), a kept source keeps the old ones
            tr.role_match = match source.as_str() {
                "user" | "audio" => None,
                _ => role_match.map(str::to_string).or_else(|| old.3.clone()),
            };
            g.mark_dirty();
        }
        // the hits: the undo's `events` restore the old ones exactly, a changed role re-picks
        let (mut reanalyzed, new_events) = if events.is_some() {
            (false, None)
        } else if changed && !skip {
            match self.track_repick_keep(sid, name, None, true, true) {
                Ok(r) => (true, r.get("events").cloned()),
                Err(e) => {
                    // R09: the re-pick ran with the new role and failed; the role fields go back
                    let mut g = lock(&op);
                    let tr = g.track_mut(name)?;
                    tr.role = old.0.clone();
                    tr.note = old.1;
                    tr.role_source = old.2.clone();
                    tr.role_match = old.3.clone();
                    g.clean_if_saved();
                    return Err(e);
                }
            }
        } else {
            (false, None)
        };
        if new_events.is_some() {
            reanalyzed = true;
        }
        let suggest_note = if changed && note.is_none() && old.1 == Some(old.4) {
            Some(default_note(role))
        } else {
            None
        };
        let (note, role_source) = {
            let g = lock(&op);
            let tr = g.track(name)?;
            (tr.note, tr.role_source.clone())
        };
        Ok(json!({
            "name": name, "role": role, "note": note, "role_source": role_source,
            "suggest_note": suggest_note, "reanalyzed": reanalyzed,
            "previous": {"role": old.0, "note": old.1, "role_source": old.2,
                         "role_match": old.3, "events": Value::Array(previous_events)},
        }))
    }

    /// R09: has the role change of `name` its documented re-pick skip? `Ok(true)`: a follower,
    /// an offline track or a track without kept activations — the re-pick refuses these with
    /// its 409, and `track_set_role` keeps the role change and reports `reanalyzed: false`
    /// instead. The same cases as `pick_inputs` (which never stores), without its busy and
    /// params checks: a busy project refused above, wrong params or a failing decode fail the
    /// re-pick after the role went in and put the role fields back.
    fn repick_skip(&self, op: &crate::registry::Shared, name: &str) -> Result<bool, AppError> {
        crate::registry::resolve_shared(op);
        let st = self.track_state(op, name)?;
        if st.follows.is_some() || st.source.is_err() {
            return Ok(true);
        }
        Ok(self.kept_detection(&st)?.is_err())
    }

    /// `tracks_remove {sid, names, snapshot?}` (P4-39a): take the tracks out of the project document (the record with its
    /// hits, params, Learn examples, layer and every other key; the place in `order`) and delete the folder
    /// `Analysis/<name>/` (peak cache, activations, Learn scores) of each. A follower of a removed track that stays becomes
    /// a normal track (it loses `follows`, its hits stay as they are). The audio files are not touched. The project is
    /// dirty. 400 for an empty list or an unknown name (nothing is removed then), 409 while an analysis, a lane build or a
    /// copy job of the project runs. Answers `{sid, removed, unfollowed, order, tracks}` (`order` and `tracks` as
    /// `tracks_get` has them for the tracks that are left); with `snapshot` also `snapshot`: one `{name, index,
    /// order_index, track, followers}` per removed track, the input of [`Service::tracks_restore`].
    ///
    /// # Errors
    /// 404 when `sid` is not an open project or a name is not a track; 409 when the project is
    /// open read-only or an analysis, lane build or copy job runs; 400 when `names` is empty;
    /// 500 when an `Analysis/<name>/` folder cannot be deleted or a track record cannot be
    /// serialized.
    pub fn tracks_remove(&self, sid: &str, names: &[String], snapshot: bool) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let mut want: Vec<String> = Vec::new();
        for n in names {
            if !want.contains(n) {
                want.push(n.clone());
            }
        }
        if want.is_empty() {
            return Err(AppError::bad("names must hold at least one track"));
        }
        let mut g = lock(&op);
        for n in &want {
            g.track(n)?;
        }
        for (what, state) in [
            ("an analysis", self.inner.jobs.status(sid)["state"].clone()),
            ("a lane build", self.inner.life.lanes.status(sid)["state"].clone()),
            ("a copy job", self.inner.copyjobs.status(sid)["state"].clone()),
        ] {
            if state == "queued" || state == "running" {
                return Err(AppError::conflict(format!(
                    "{what} of this project runs: remove the track after it"
                )));
            }
        }
        // the places and the snapshots before anything changes: a restore puts the tracks back in increasing order
        let order_before = g.project.doc.track_order();
        let mut rows = Vec::new();
        for n in &want {
            let index = g
                .project
                .doc
                .tracks
                .as_ref()
                .and_then(|m| m.get_index_of(n))
                .unwrap_or(0);
            let order_index = order_before.iter().position(|x| x == n).unwrap_or(0);
            let track = serde_json::to_value(g.track(n)?).map_err(|e| AppError::internal(format!("{n}: {e}")))?;
            let followers: Vec<String> = crate::follow::followers_of(&g, n)
                .into_iter()
                .filter(|f| !want.contains(f))
                .collect();
            rows.push(
                json!({"name": n, "index": index, "order_index": order_index, "track": track, "followers": followers}),
            );
        }
        // the folders first: a failure leaves the document as it is (the caches are made again by the lane job)
        let adir = g.project.subdir(ANALYSIS_DIR);
        for n in &want {
            let dir = adir.join(n);
            if check_track_name(n).is_ok() && dir.is_dir() {
                std::fs::remove_dir_all(&dir).map_err(|e| AppError::internal(format!("{}: {e}", dir.display())))?;
            }
        }
        let mut unfollowed = Vec::new();
        for n in &want {
            g.project.doc.tracks_mut().shift_remove(n);
            g.paths.remove(n);
            g.status.remove(n);
        }
        if let Some(order) = g.project.doc.order.as_mut() {
            order.retain(|x| !want.contains(x));
        }
        for (name, tr) in g.project.doc.tracks_mut().iter_mut() {
            if tr.follows.as_ref().is_some_and(|f| want.contains(f)) {
                tr.follows = None;
                unfollowed.push(name.clone());
            }
        }
        g.mark_dirty();
        let mut tracks = serde_json::Map::new();
        let left: Vec<String> = g.project.doc.tracks().map(|(n, _)| n.clone()).collect();
        for n in &left {
            tracks.insert(n.clone(), g.public_track(n)?);
        }
        let mut out = json!({
            "sid": sid, "removed": want, "unfollowed": unfollowed,
            "order": g.project.doc.track_order(), "tracks": Value::Object(tracks),
        });
        if snapshot {
            out["snapshot"] = Value::Array(rows);
        }
        Ok(out)
    }

    /// `tracks_restore {sid, tracks}` (P4-39a, the undo of `tracks_remove`): put the tracks of a `snapshot` back at their
    /// places with every key they had (hits, params, extra fields), link the followers that followed them again, resolve
    /// the sources and build the peak caches that the removal deleted. 400 for a snapshot that is no list or holds a bad
    /// record, 409 when a track of that name exists. The activations and Learn scores of the deleted folders come back with
    /// the next analysis. Marks the project dirty. Answers `{restored, order}`.
    ///
    /// # Errors
    /// 404 when `sid` is not an open project; 409 when the project is open read-only or a track of
    /// that name exists; 400 when `tracks` is not a list, a row has no valid `name`, or a track
    /// record cannot be parsed.
    pub fn tracks_restore(&self, sid: &str, tracks: &Value) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let list = tracks
            .as_array()
            .ok_or_else(|| AppError::bad("tracks must be a list of snapshot rows"))?;
        let mut rows: Vec<(String, usize, usize, Track, Vec<String>)> = Vec::new();
        for r in list {
            let name = r
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::bad("a snapshot row needs a `name`"))?;
            check_track_name(name).map_err(|e| AppError::bad(e.to_string()))?;
            let place = |k: &str| r.get(k).and_then(Value::as_u64).map_or(usize::MAX, |v| v as usize);
            let track: Track = serde_json::from_value(r.get("track").cloned().unwrap_or(Value::Null))
                .map_err(|e| AppError::bad(format!("{name}: the track record is bad: {e}")))?;
            let followers = r
                .get("followers")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            rows.push((name.to_string(), place("index"), place("order_index"), track, followers));
        }
        rows.sort_by_key(|r| r.1);
        let restored: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
        let order = {
            let mut g = lock(&op);
            for (name, ..) in &rows {
                if g.project
                    .doc
                    .tracks()
                    .any(|(n, _)| crate::export::same_file_name(n, name))
                    || restored.iter().filter(|x| *x == name).count() > 1
                {
                    return Err(AppError::conflict(format!("a track named {name:?} exists")));
                }
            }
            for (name, index, order_index, track, followers) in rows {
                let mut order = g.project.doc.track_order();
                order.insert(order_index.min(order.len()), name.clone());
                let tracks = g.project.doc.tracks_mut();
                tracks.shift_insert(index.min(tracks.len()), name.clone(), track);
                g.project.doc.order = Some(order);
                for f in followers {
                    if let Some(tr) = g.project.doc.track_mut(&f).filter(|t| t.follows.is_none()) {
                        tr.follows = Some(name.clone());
                    }
                }
            }
            g.mark_dirty();
            g.project.doc.track_order()
        };
        crate::registry::resolve_shared(&op);
        self.prebuild_peaks(&op, &restored, |_, _| {});
        Ok(json!({"restored": restored, "order": order}))
    }
}

// ---------- the bridge ----------

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{key}` (string)")))
}

fn string_list(args: &Value, key: &str) -> Result<Vec<String>, AppError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| AppError::bad(format!("`{key}` must hold strings")))
            })
            .collect(),
        Some(_) => Err(AppError::bad(format!("`{key}` must be a list"))),
    }
}

/// Run a command of this card for the bridge; `None` when `cmd` is not one of them. The
/// argument names are those of the Tauri wrappers in `apps/desktop/src-tauri/src/tracks.rs`.
pub fn dispatch(svc: &Service, channels: &Arc<Channels>, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    let run = || -> Result<Value, AppError> {
        match cmd {
            "tracks_scan" => svc.tracks_scan(&string_list(args, "paths")?, args.get("preset").and_then(Value::as_str)),
            "tracks_add" => {
                let progress = args
                    .get("progress")
                    .and_then(|c| c.get("__channel"))
                    .and_then(Value::as_str)
                    .map(|id| {
                        let (ch, id) = (Arc::clone(channels), id.to_string());
                        let p: Progress = Arc::new(move |v: Value| ch.push(&id, v));
                        p
                    });
                svc.tracks_add(
                    arg_str(args, "sid")?,
                    &string_list(args, "paths")?,
                    &str_list(args.get("roles")),
                    &str_list(args.get("notes")),
                    args.get("copy").and_then(Value::as_bool).unwrap_or(false),
                    progress,
                )
            }
            "tracks_remove" => svc.tracks_remove(
                arg_str(args, "sid")?,
                &string_list(args, "names")?,
                args.get("snapshot").and_then(Value::as_bool).unwrap_or(false),
            ),
            "tracks_restore" => svc.tracks_restore(arg_str(args, "sid")?, args.get("tracks").unwrap_or(&Value::Null)),
            "track_set_role" => svc.track_set_role(
                arg_str(args, "sid")?,
                arg_str(args, "name")?,
                arg_str(args, "role")?,
                opt_i64(args, "note")?,
                args.get("source").and_then(Value::as_str),
                args.get("role_match").and_then(Value::as_str),
                args.get("events").filter(|v| !v.is_null()),
            ),
            "kit_presets_list" => svc.kit_presets_list(),
            "kit_preset_save" => svc.kit_preset_save(arg_str(args, "name")?, args.get("rules").unwrap_or(&Value::Null)),
            "kit_preset_load" => svc.kit_preset_load(arg_str(args, "name")?),
            _ => Err(AppError::not_found(format!("unknown command {cmd}"))),
        }
    };
    TRACKS_COMMANDS.contains(&cmd).then(run)
}

#[cfg(test)]
mod tests;
