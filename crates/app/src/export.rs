//! P3-3c: spike WAV export, Export all and the velocity recompute.
//!
//! | Command | Server route |
//! |---|---|
//! | `export_spikes` | new (route 33 has the MIDI part only): `Exports/<name>_spike.wav` |
//! | `export_all` | new: MIDI and spikes of every track |
//! | `velocities_recompute` | POST `/api/velocities/{sid}/{name}` (34) |
//!
//! The spike file has the rate, the length and the sample format of the source and its `bext`
//! time reference (`drumspike_core::export_spikes`, the function behind
//! `drumspike_core.write_spike_wav`). Only the header of the source is read (a memory map
//! that touches no sample), so an export of a 5-minute 96 kHz track reads no audio.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Instant;

use drumspike_core::io::{MappedWav, MixMode, SampleFormat, WavFormat};
use drumspike_core::midi::{MidiEvent, MidiTrack, export_midi_map as write_midi, export_multi_midi};
use drumspike_core::project::EXPORTS_DIR;
use drumspike_core::spike::{export_audio_spikes, export_spikes as write_spikes, hit_index};
use drumspike_core::tempo::TempoMap;
use drumspike_core::velocity::{VelocityMap, velocity_map};
use serde_json::{Value, json};

use crate::error::AppError;
use crate::media::mix_mode;
use crate::registry::{OpenProject, Shared, lock};
use crate::reply::Channels;
use crate::sampler::{MidiPlan, ShiftInfo, midi_events_shifted};
use crate::{Progress, Service};

/// MIDI ticks per quarter note and tempo of the MIDI files that Export all writes (the page's
/// values for Export MIDI).
pub const EXPORT_PPQ: u16 = 24_000;
/// Tempo in BPM of those files.
pub const EXPORT_TEMPO: f64 = 120.0;
/// The most worker threads of Export all. Each worker holds the signal and the bytes of one
/// spike file (up to about 300 MB for a 5-minute 96 kHz track).
pub const EXPORT_WORKERS: usize = 6;

/// The WAV sample format a spike file takes from its source: the same container width, float
/// stays float.
#[must_use]
pub fn spike_format(format: SampleFormat, bytes_per_sample: usize) -> WavFormat {
    match (format, bytes_per_sample) {
        (SampleFormat::Float, _) => WavFormat::Float32,
        (SampleFormat::Pcm, 2) => WavFormat::Pcm16,
        (SampleFormat::Pcm, 3) => WavFormat::Pcm24,
        (SampleFormat::Pcm, _) => WavFormat::Pcm32,
    }
}

/// The name of a [`WavFormat`] as `drumspike_core.write_spike_wav` takes it.
#[must_use]
pub fn format_name(f: WavFormat) -> &'static str {
    match f {
        WavFormat::Pcm16 => "pcm16",
        WavFormat::Pcm24 => "pcm24",
        WavFormat::Pcm32 => "pcm32",
        WavFormat::Float32 => "float32",
    }
}

/// P4-3: an event taken out of the output: the loudness cut (`"cut": true`; `histogram::mark_cut`), the tom bleed
/// rule (`"bleed": true`; [`crate::tom_bleed`]) or a hit the old Learn filter had taken out (`"filtered"`, #281 D16).
/// R16 (#40): the one predicate is [`crate::policy::is_out`]; this wrapper keeps the export name.
#[must_use]
pub fn is_cut(e: &Value) -> bool {
    crate::policy::is_out(e)
}

/// P4-3: the events that go into the output (MIDI, spikes, clicks): every event without the cut or bleed mark.
#[must_use]
pub fn uncut(events: &[Value]) -> Vec<Value> {
    events.iter().filter(|e| !is_cut(e)).cloned().collect()
}

/// The hit times of an event list: every event needs a numeric `t`.
pub fn hit_times(events: &[Value]) -> Result<Vec<f64>, AppError> {
    events
        .iter()
        .map(|e| {
            e.get("t")
                .and_then(Value::as_f64)
                .ok_or_else(|| AppError::bad("every event needs a numeric t"))
        })
        .collect()
}

/// The events of an event list for the MIDI file (`v` missing is 100, as the server).
pub(crate) fn midi_events(events: &[Value]) -> Result<Vec<MidiEvent>, AppError> {
    events
        .iter()
        .map(|e| {
            let t = e
                .get("t")
                .and_then(Value::as_f64)
                .ok_or_else(|| AppError::bad("every event needs a numeric t"))?;
            let v = match e.get("v") {
                None => 100,
                Some(v) => v.as_i64().ok_or_else(|| AppError::bad("v must be an integer"))?,
            };
            Ok(MidiEvent { t, v })
        })
        .collect()
}

/// P4-4: the number of hits that have no sample in a file of `n_samples` at `sr`: `round(t * sr)`
/// before the first or after the last sample. The spike file cannot hold them; the MIDI file
/// keeps them (a hit at or before 0 s is a note at 0 s). The answers carry the count as `outside`.
#[must_use]
pub fn outside_count(times: &[f64], sr: u32, n_samples: usize) -> usize {
    times.iter().filter(|&&t| hit_index(t, sr, n_samples).is_none()).count()
}

/// P4-4 fix round: the hits before the first sample (`round(t * sr) < 0`), a part of
/// [`outside_count`]. The MIDI file moves them to 0 s (the frozen rule of the MIDI export).
#[must_use]
pub fn before_count(times: &[f64], sr: u32) -> usize {
    times
        .iter()
        .filter(|&&t| hit_index(t, sr, usize::MAX).is_none())
        .count()
}

/// P4-4: the rate and length of a track's audio for the `outside` count of a MIDI export: the
/// source's header when the file is there (no sample is read), else the document's `sr` and
/// `n_samples`; `None` when neither is known.
pub(crate) fn audio_length(op: &crate::registry::Shared, name: &str) -> Option<(u32, usize)> {
    let (source, doc) = {
        let g = lock(op);
        let tr = g.track(name).ok()?;
        let doc = tr
            .sr
            .and_then(|s| u32::try_from(s).ok())
            .zip(tr.n_samples.and_then(|n| usize::try_from(n).ok()));
        (g.source(name).ok(), doc)
    };
    source
        .and_then(|p| MappedWav::open(&p).ok())
        .map(|w| (w.info.sample_rate, w.info.n_frames))
        .or(doc)
}

/// P4-4: the key under which a disk that ignores case and normalisation (APFS by default) sees a
/// file name: the canonical caseless form NFD(casefold(NFD(name))) of Unicode (full case
/// folding: `Straße` and `STRASSE`, `ΟΔΟΣ` and `οδος`; NFD: a precomposed and a decomposed `Ü`).
#[must_use]
pub fn file_name_key(name: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let folded = caseless::default_case_fold_str(&name.nfd().collect::<String>());
    folded.nfd().collect()
}

/// P4-4: true when two track names are one file name on such a disk ([`file_name_key`]): `Kick`
/// and `kick` share the export files and the `Analysis/` folder.
#[must_use]
pub fn same_file_name(a: &str, b: &str) -> bool {
    file_name_key(a) == file_name_key(b)
}

/// P4-4: the earlier track (in the document's order) whose export files are `name`'s on a disk
/// that ignores case (APFS by default): `Kick` and `kick` both write `Exports/kick_spike.wav`.
pub(crate) fn file_owner(g: &OpenProject, name: &str) -> Option<String> {
    g.project
        .doc
        .tracks()
        .map(|(n, _)| n)
        .take_while(|n| n.as_str() != name)
        .find(|n| same_file_name(n, name))
        .cloned()
}

/// P4-4: 409 when `name`'s export files belong to an earlier track ([`file_owner`]), so an export
/// never replaces another track's files.
pub(crate) fn check_file_owner(g: &OpenProject, name: &str) -> Result<(), AppError> {
    match file_owner(g, name) {
        Some(first) => Err(AppError::conflict(format!(
            "the export files of track {name} are those of track {first} on this disk (the names \
             differ only in case): rename one of the two tracks"
        ))),
        None => Ok(()),
    }
}

/// R01 (#248): the disk key of a path: the canonical folder (symlinks and `..` resolved; the
/// folder is canonicalized when it is there, the file itself may be missing) plus the caseless
/// file-name key ([`file_name_key`], the APFS default). Two paths with the same key name one
/// file on this disk.
pub(crate) fn path_key(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let name = abs
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dir = match abs.parent() {
        Some(d) => std::fs::canonicalize(d).unwrap_or_else(|_| d.to_path_buf()),
        None => abs.clone(),
    };
    dir.join(file_name_key(&name))
}

/// R01: true when `a` and `b` name the same file on this disk ([`path_key`]): one spelling of
/// the same path, the same file through a symlink or a folder link, a `..` in the middle, or
/// names that differ only in case.
pub(crate) fn same_file_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    path_key(a) == path_key(b)
}

/// R01: 409 when one of `outputs` would write a `protected` file. The message names both.
pub(crate) fn check_export_plan(outputs: &[PathBuf], protected: &[PathBuf]) -> Result<(), AppError> {
    for out in outputs {
        for p in protected {
            if same_file_path(out, p) {
                return Err(AppError::conflict(format!(
                    "the export would overwrite the protected file {}: the output {} names the \
                     same file: choose another destination folder or rename the track",
                    p.display(),
                    out.display()
                )));
            }
        }
    }
    Ok(())
}

/// R10: the destinations with a running export (process-wide): an overlapping export to the
/// same folder waits until the running one is done, so the output set has a defined last
/// writer instead of two plans racing on the same files. Keyed by [`path_key`], so two
/// spellings (case, links, `..`) of one folder count as one destination.
fn export_dest_slots() -> &'static (Mutex<HashSet<PathBuf>>, Condvar) {
    static SLOTS: OnceLock<(Mutex<HashSet<PathBuf>>, Condvar)> = OnceLock::new();
    SLOTS.get_or_init(|| (Mutex::new(HashSet::new()), Condvar::new()))
}

/// R10: hold the guard for the whole of an export into `dir` ([`Service::export_all_opts`],
/// [`Service::export_spikes_to`] and the per-track MIDI export): a second export to the same
/// folder serializes behind it instead of writing the same files twice, interleaved.
pub(crate) fn export_dest_acquire(dir: &Path) -> DestGuard {
    let key = path_key(dir);
    let (m, cv) = export_dest_slots();
    let mut busy = m.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    while busy.contains(&key) {
        busy = cv.wait(busy).unwrap_or_else(std::sync::PoisonError::into_inner);
    }
    busy.insert(key.clone());
    DestGuard(key)
}

/// The lock of one export destination; released on drop.
pub(crate) struct DestGuard(PathBuf);

impl Drop for DestGuard {
    fn drop(&mut self) {
        let (m, cv) = export_dest_slots();
        let mut busy = m.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        busy.remove(&self.0);
        cv.notify_all();
    }
}

/// P4-30: the file name suffix of the multi-track MIDI file: `<project>_drums.mid`.
pub const MULTI_SUFFIX: &str = "_drums.mid";

/// P4-40g: the document key of the remembered "Markers in MIDI" checkbox of the Export dialog (a
/// boolean; absent means off, the default).
pub const EXPORT_MARKERS_KEY: &str = "export_markers";

/// The remembered "Markers in MIDI" checkbox of `doc` (absent: off).
pub fn doc_export_markers_on(doc: &drumspike_core::Document) -> bool {
    doc.extra
        .get(EXPORT_MARKERS_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The review markers of `doc` for the multi-track MIDI file (P4-40g): the entries with a numeric
/// `t`, sorted by time (stable), the note carried as it is; `multi_midi_bytes` writes the text or
/// the `Marker N` fallback.
#[must_use]
pub fn doc_export_markers(doc: &drumspike_core::Document) -> Vec<(f64, Option<String>)> {
    let mut ms: Vec<(f64, Option<String>)> = crate::markers::doc_markers(doc)
        .iter()
        .filter_map(|v| {
            Some((
                v.get("t")?.as_f64()?,
                v.get("note").and_then(Value::as_str).map(str::to_string),
            ))
        })
        .collect();
    ms.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    ms
}
/// P4-30: how the MIDI export writes its files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiMode {
    /// One file `<track>_spike.mid` per track (the default).
    PerTrack,
    /// One type 1 file `<project>_drums.mid` with one MIDI track per drum track.
    Single,
}

impl MidiMode {
    /// `per_track` (also when absent) or `single`; any other text is a 400.
    pub fn parse(s: Option<&str>) -> Result<MidiMode, AppError> {
        match s {
            None | Some("per_track") => Ok(MidiMode::PerTrack),
            Some("single") => Ok(MidiMode::Single),
            Some(o) => Err(AppError::bad(format!("midi_mode must be per_track or single, not {o}"))),
        }
    }
}

/// P4-30: what an export takes besides the project: the destination folder, the tracks, the formats and the
/// MIDI mode (`export_all`, `export_existing`, `export_drag_prepare`).
#[derive(Debug, Clone)]
pub struct ExportOpts {
    /// An absolute folder that exists and is writable; `None` is `Exports/` in the project folder.
    pub dest: Option<String>,
    /// How the MIDI files are written.
    pub midi_mode: MidiMode,
    /// The tracks to export, `None` for all of them.
    pub tracks: Option<Vec<String>>,
    /// Write MIDI.
    pub midi: bool,
    /// Write spike WAVs.
    pub spikes: bool,
    /// #209: write audio-spike WAVs (the track's own audio, 1.5 periods per hit). Off by
    /// default: the established export (MIDI and blip spikes) does not change.
    pub audio: bool,
    /// P4-7c: the MIDI notes of a track with a sampler calibration move by the offset (default true; a
    /// track without a calibration is not changed either way).
    pub compensate: bool,
}

impl Default for ExportOpts {
    fn default() -> Self {
        ExportOpts {
            dest: None,
            midi_mode: MidiMode::PerTrack,
            tracks: None,
            midi: true,
            spikes: true,
            audio: false,
            compensate: true,
        }
    }
}

impl ExportOpts {
    /// The options of a call: `dest` (text), `midi_mode`, `tracks` (list of names) and `formats` (a list of
    /// `midi` and `spikes`, at least one). A missing key is the default.
    pub fn from_args(args: &Value) -> Result<ExportOpts, AppError> {
        let strs = |k: &str| -> Result<Option<Vec<String>>, AppError> {
            match args.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Array(a)) => a
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| AppError::bad(format!("`{k}` must hold strings")))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(Some),
                Some(_) => Err(AppError::bad(format!("`{k}` must be a list"))),
            }
        };
        let dest = match args.get("dest") {
            None | Some(Value::Null) => None,
            Some(Value::String(d)) => Some(d.clone()),
            Some(_) => return Err(AppError::bad("`dest` must be a string")),
        };
        let midi_mode = MidiMode::parse(match args.get("midi_mode") {
            None | Some(Value::Null) => None,
            Some(Value::String(m)) => Some(m.as_str()),
            Some(_) => return Err(AppError::bad("`midi_mode` must be a string")),
        })?;
        let (mut midi, mut spikes, mut audio) = (true, true, false);
        if let Some(f) = strs("formats")? {
            midi = f.iter().any(|x| x == "midi");
            spikes = f.iter().any(|x| x == "spikes");
            audio = f.iter().any(|x| x == "audio");
            if let Some(bad) = f.iter().find(|x| *x != "midi" && *x != "spikes" && *x != "audio") {
                return Err(AppError::bad(format!(
                    "unknown format {bad}: use midi, spikes and audio"
                )));
            }
            if !midi && !spikes && !audio {
                return Err(AppError::bad("`formats` needs midi, spikes or audio"));
            }
        }
        let compensate = match args.get("compensate") {
            None | Some(Value::Null) => crate::command_policy::COMPENSATE_DEFAULT,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(AppError::bad("`compensate` must be true or false")),
        };
        Ok(ExportOpts {
            dest,
            midi_mode,
            tracks: strs("tracks")?,
            midi,
            spikes,
            audio,
            compensate,
        })
    }
}

/// P4-30: a destination folder: absolute, there, a folder and writable. R11: the probe is a
/// unique unpredictable temporary file of the shared helper (`create_new`, `io::atomic`),
/// removed by its guard, so only a file this call created is ever removed: a pre-existing
/// file or a symlink with an old probe name survives, and concurrent checks never collide.
/// 400 for a relative path, a file or a folder that refuses a write, 404 for a path that
/// does not exist.
pub fn check_dest(dest: &str) -> Result<PathBuf, AppError> {
    let p = Path::new(dest);
    if !p.is_absolute() {
        return Err(AppError::bad(format!(
            "the destination must be an absolute folder path: {dest}"
        )));
    }
    let meta = std::fs::metadata(p)
        .map_err(|_| AppError::not_found(format!("the destination folder does not exist: {dest}")))?;
    if !meta.is_dir() {
        return Err(AppError::bad(format!("the destination is not a folder: {dest}")));
    }
    let probe = drumspike_core::io::TempFile::create(&p.join(".drumspike-write-test"))
        .map_err(|e| AppError::bad(format!("the destination folder is not writable ({e}): {dest}")))?;
    drop(probe);
    Ok(p.to_path_buf())
}

/// P4-30: the folder an export writes to: `dest` when given (checked by [`check_dest`]), else `Exports/` of the
/// project (made when it is missing).
pub fn export_dir(g: &OpenProject, dest: Option<&str>) -> Result<PathBuf, AppError> {
    if let Some(d) = dest {
        check_dest(d)
    } else {
        let dir = g.project.subdir(EXPORTS_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| AppError::internal(format!("{}: {e}", dir.display())))?;
        Ok(dir)
    }
}

/// P4-30: the file name of the multi-track MIDI file of a project: `<name>_drums.mid` with the characters a file
/// name cannot hold replaced by `-`.
#[must_use]
pub fn multi_file_name(project_name: &str) -> String {
    let n: String = project_name
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let n = n.trim();
    format!("{}{MULTI_SUFFIX}", if n.is_empty() { "project" } else { n })
}

/// P4-30: `all` filtered to `want` (in the order of `all`); a name that no track has is a 400.
pub fn select_tracks(all: Vec<String>, want: Option<&[String]>) -> Result<Vec<String>, AppError> {
    let Some(want) = want else { return Ok(all) };
    if let Some(n) = want.iter().find(|n| !all.contains(n)) {
        return Err(AppError::bad(format!("no track {n}")));
    }
    Ok(all.into_iter().filter(|n| want.contains(n)).collect())
}

/// What one spike file needs from the project, taken while its lock is held.
struct Job {
    name: String,
    source: Result<PathBuf, AppError>,
    events: Result<Vec<Value>, AppError>,
    note: i64,
    /// The project's tempo map (P4-8): the default map writes the exports as before.
    map: TempoMap,
    /// P4-7c: the MIDI events of the track (the sampler offset applied) and what that did; `None` when the
    /// export writes no MIDI or the track's hits could not be read.
    midi: Option<Result<MidiPlan, AppError>>,
}

/// The reply accumulators of `export_all_opts` (R10): the written files with their counts, the
/// per-track errors, the hits with no sample in their file (`outside`), the sampler calibration
/// facts and the warnings (seeded with the #151 default-map note, so it stays the first warning).
#[derive(Default)]
struct ExportTally {
    /// The written files, as display strings.
    files: Vec<Value>,
    /// How many tracks wrote a MIDI file.
    midi: usize,
    /// How many tracks wrote a spike WAV.
    spikes: usize,
    /// How many tracks wrote an audio spike WAV.
    audio: usize,
    /// The per-track errors, as `{"name", "code", "message"}`.
    errors: Vec<Value>,
    /// P4-4: a track with hits of no sample in its file, mapped to their count.
    outside: serde_json::Map<String, Value>,
    /// The sampler calibration facts per track, as their JSON.
    sampler: serde_json::Map<String, Value>,
    /// The warnings of the reply (the #151 note first, then the sampler ones).
    warnings: Vec<String>,
}

/// The facts of a written spike file.
#[derive(Debug, Clone, PartialEq)]
pub struct SpikeFile {
    /// The file written.
    pub path: PathBuf,
    /// Size in bytes.
    pub size: u64,
    /// Rate of the source and of the spike file.
    pub sr: u32,
    /// Length of the source and of the spike file.
    pub n_samples: usize,
    /// Sample format of both.
    pub format: WavFormat,
    /// The source's `bext` time reference (`None` without a `bext` chunk).
    pub time_reference: Option<u64>,
    /// P4-4: the hits with no sample in the file ([`outside_count`]), not in the spike file.
    pub outside: usize,
}

/// Write the spike file `out` for `times` at the rate, length and sample format of `source`,
/// with the source's `bext` time reference. A source the core cannot read is a 400 that names
/// the file.
pub fn write_spike_file(source: &Path, out: &Path, times: &[f64]) -> Result<SpikeFile, AppError> {
    let wav = MappedWav::open(source).map_err(|e| {
        let e = AppError::from(e);
        AppError {
            code: e.code,
            message: format!("{}: {}", source.display(), e.message),
        }
    })?;
    let info = &wav.info;
    let format = spike_format(info.sample_format, info.bytes_per_sample);
    write_spikes(out, info.sample_rate, info.n_frames, times, info.bext.as_ref(), format)
        .map_err(|e| AppError::internal(format!("{}: {e}", out.display())))?;
    let size = std::fs::metadata(out).map_or(0, |m| m.len());
    Ok(SpikeFile {
        path: out.to_path_buf(),
        size,
        sr: info.sample_rate,
        n_samples: info.n_frames,
        format,
        time_reference: info.bext.as_ref().map(|b| b.time_reference),
        outside: outside_count(times, info.sample_rate, info.n_frames),
    })
}

/// True when `out` names the file `source` (the source exists; `out` may not): the source
/// canonical, `out` canonical when it is there, else its absolute path.
fn out_is_source(source: &Path, out: &Path) -> bool {
    if source == out {
        return true;
    }
    let src = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    if src == out {
        return true;
    }
    let dst =
        std::fs::canonicalize(out).unwrap_or_else(|_| std::path::absolute(out).unwrap_or_else(|_| out.to_path_buf()));
    src == dst
}

/// Write the audio-spike file `out` for `times`, cut from the audio of `source` (1.5 periods
/// per hit, #209): mono at the rate and length of `source`, in the source's sample format,
/// with the source's `bext` time reference. Unlike [`write_spike_file`] this reads the whole
/// audio (the snippets are the source's own samples). `out` equal to `source` is a 400 (the
/// export would overwrite the track's file); the source is never written. A source the core
/// cannot read is a 400 that names the file. Returns the facts and the number of hits that
/// got the default length (no period found).
pub fn write_audio_spike_file(source: &Path, out: &Path, times: &[f64]) -> Result<(SpikeFile, usize), AppError> {
    if out_is_source(source, out) {
        return Err(AppError::bad(format!(
            "the output path is the track's source file: {}",
            source.display()
        )));
    }
    let wav = MappedWav::open(source).map_err(|e| {
        let e = AppError::from(e);
        AppError {
            code: e.code,
            message: format!("{}: {}", source.display(), e.message),
        }
    })?;
    let info = &wav.info;
    let format = spike_format(info.sample_format, info.bytes_per_sample);
    let samples = wav.decode_mono(MixMode::Ffmpeg).map_err(|e| {
        let e = AppError::from(e);
        AppError {
            code: e.code,
            message: format!("{}: {}", source.display(), e.message),
        }
    })?;
    let n_frames = samples.len();
    let no_period = export_audio_spikes(out, info.sample_rate, times, &samples, info.bext.as_ref(), format)
        .map_err(|e| AppError::internal(format!("{}: {e}", out.display())))?;
    let size = std::fs::metadata(out).map_or(0, |m| m.len());
    Ok((
        SpikeFile {
            path: out.to_path_buf(),
            size,
            sr: info.sample_rate,
            n_samples: n_frames,
            format,
            time_reference: info.bext.as_ref().map(|b| b.time_reference),
            outside: outside_count(times, info.sample_rate, n_frames),
        },
        no_period,
    ))
}

fn spike_json(f: &SpikeFile, n_events: usize) -> Value {
    json!({
        "path": f.path.display().to_string(),
        "size": f.size,
        "n_events": n_events,
        "sr": f.sr,
        "n_samples": f.n_samples,
        "format": format_name(f.format),
        "time_reference": f.time_reference,
        "outside": f.outside,
    })
}

impl Service {
    /// R01 (#248): the protected files of every open project: each track's source, its one-shot
    /// (the layer file) and its sampler bounce. A file that is offline or missing from the
    /// record is not listed.
    pub(crate) fn protected_files_everywhere(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for op in self.inner.registry.all_open() {
            let g = lock(&op);
            out.extend(crate::layer::protected_files(&g));
            let names: Vec<String> = g.project.doc.tracks().map(|(n, _)| n.clone()).collect();
            for name in names {
                let Ok(tr) = g.track(&name) else { continue };
                let Some(s) = crate::sampler::sampler_of(&tr.extra) else {
                    continue;
                };
                if s.file.is_empty() {
                    continue;
                }
                let p = PathBuf::from(&s.file);
                out.push(if p.is_absolute() { p } else { g.project.dir.join(p) });
            }
        }
        out
    }

    /// The `events` of a track as the document holds them (typed or untyped).
    pub(crate) fn track_events(&self, sid: &str, name: &str) -> Result<Vec<Value>, AppError> {
        let op = self.inner.registry.get(sid)?;
        let g = lock(&op);
        let m = g.track_object(name)?;
        Ok(m.get("events").and_then(Value::as_array).cloned().unwrap_or_default())
    }

    /// `export_spikes {sid, name, events?}`: write `Exports/<name>_spike.wav`. The hits are
    /// `events` when the call holds them, else the track's events in the document. The file
    /// has the source's rate, length, sample format and `bext` time reference. Returns
    /// `{path, size, n_events, sr, n_samples, format, time_reference, outside}` (`outside`: the
    /// hits with no sample in the file, P4-4). 409 for an offline track, a track whose files are
    /// another track's ([`check_file_owner`]) and a stale follower that cannot be derived; 400
    /// for a source that is not a WAV file the core reads. A follower exports its own derived
    /// hits (the document's, made fresh by [`crate::follow::fresh_for_export`]), never `events`.
    pub fn export_spikes(&self, sid: &str, name: &str, events: Option<&[Value]>) -> Result<Value, AppError> {
        self.export_spikes_to(sid, name, events, None)
    }

    /// P4-30: [`Service::export_spikes`] into the folder `dest` (see [`check_dest`]); `None` is `Exports/`.
    pub fn export_spikes_to(
        &self,
        sid: &str,
        name: &str,
        events: Option<&[Value]>,
        dest: Option<&str>,
    ) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let (source, dir, follower) = {
            let mut g = lock(&op);
            g.track(name)?;
            check_file_owner(&g, name)?;
            g.resolve(); // a file that went away since the open is a 409
            (g.source(name)?, export_dir(&g, dest)?, g.track(name)?.follows.is_some())
        };
        // R10: a second export to the same folder serializes behind this one
        let _dest_guard = export_dest_acquire(&dir);
        let refreshed = crate::follow::fresh_for_export(self, &op, name)?;
        // R01: the output must not be a protected file (the source of any track of any open
        // project, a one-shot or a sampler bounce), in any spelling of the path
        let out = dir.join(format!("{name}_spike.wav"));
        check_export_plan(&[out], &self.protected_files_everywhere())?;
        let events = uncut(&match events {
            Some(e) if !follower => e.to_vec(),
            _ => self.track_events(sid, name)?,
        });
        let times = hit_times(&events)?;
        let f = write_spike_file(&source, &dir.join(format!("{name}_spike.wav")), &times)?;
        let mut out = spike_json(&f, events.len());
        if refreshed {
            out["followers"] = json!([name]);
        }
        Ok(out)
    }

    /// `export_all {sid, progress}`: for every track, `Exports/<name>_spike.mid` (the track's
    /// note, 24,000 PPQ, 120 BPM) and `Exports/<name>_spike.wav`, from the events in the
    /// document. The tracks run on up to [`EXPORT_WORKERS`] threads. A track that fails (an
    /// offline file, an unsupported format) does not stop the others: its error is in
    /// `errors`. `progress` gets `{"sid", "export": {state, done, total, track}}` after each
    /// track and a last `state: "done"` message. Returns `{files, midi, spikes, errors,
    /// elapsed_ms, outside, followers}`. P4-4: `outside` maps a track to its hits with no sample
    /// in its file (tracks with none are left out); a follower whose hits are stale is derived
    /// first (`followers` lists them; one that cannot be derived is an error and gets no files);
    /// a track whose files are an earlier track's ([`check_file_owner`]) is an error.
    pub fn export_all(&self, sid: &str, progress: Option<Progress>) -> Result<Value, AppError> {
        self.export_all_opts(sid, &ExportOpts::default(), progress)
    }

    /// P4-30: [`Service::export_all`] with options: the tracks (`tracks`), the formats (`midi`, `spikes`), the folder
    /// (`dest`) and the MIDI mode. `MidiMode::Single` writes one type 1 file `<project>_drums.mid` (the project's
    /// tempo map; one MIDI track per exported track on its own note) instead of the `<track>_spike.mid` files; a
    /// track that failed is left out of it and named in `errors`. The answer also has `dest` (the folder) and
    /// `midi_file` (the path of the single file, else null). 400 for a bad `dest`, an unknown track name.
    pub fn export_all_opts(&self, sid: &str, opts: &ExportOpts, progress: Option<Progress>) -> Result<Value, AppError> {
        let started = Instant::now();
        let op = self.inner.registry.get_writable(sid)?;
        crate::registry::resolve_shared(&op);
        let all: Vec<String> = lock(&op).project.doc.tracks().map(|(n, _)| n.clone()).collect();
        let names = select_tracks(all, opts.tracks.as_deref())?;
        let dir = export_dir(&lock(&op), opts.dest.as_deref())?;
        // R10: a second export to the same folder serializes behind this one
        let _dest_guard = export_dest_acquire(&dir);
        let single_name = multi_file_name(&lock(&op).project.name());
        let (mut refused, refreshed) = self.refresh_followers(&op, &names);
        let mut jobs = Self::build_jobs(&op, &names, &mut refused)?;
        self.precompute_midi(sid, opts, &mut jobs);
        self.check_plan_staged(&dir, opts, &names, &single_name, &refused)?;
        let total = jobs.len();
        let say = |v: Value| {
            if let Some(p) = &progress {
                p(json!({"sid": sid, "export": v}));
            }
        };
        let results = Self::run_export_workers(&jobs, &dir, opts, total, &say);
        let warnings = Self::default_map_warning(&op, opts)?;
        let mut tally = Self::collect_results(&jobs, &results, warnings);
        let midi_file = if opts.midi && opts.midi_mode == MidiMode::Single {
            Self::write_single_midi(&op, &dir, &single_name, &jobs, &mut tally)?
        } else {
            Value::Null
        };
        let out = json!({
            "dest": dir.display().to_string(),
            "midi_file": midi_file,
            "files": tally.files,
            "midi": tally.midi,
            "spikes": tally.spikes,
            "audio": tally.audio,
            "errors": tally.errors,
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "outside": tally.outside,
            "followers": refreshed,
            "sampler": tally.sampler,
            "warnings": tally.warnings,
        });
        say(json!({"state": "done", "done": total, "total": total, "result": out}));
        Ok(out)
    }

    /// The owner check and the follower refresh of `export_all_opts`: a track whose files are an
    /// earlier track's ([`check_file_owner`]) or whose followers cannot be derived is refused with
    /// its error; a follower refreshed here is named in the second list.
    fn refresh_followers(
        &self,
        op: &Shared,
        names: &[String],
    ) -> (std::collections::HashMap<String, AppError>, Vec<Value>) {
        let mut refused = std::collections::HashMap::new();
        let mut refreshed = Vec::new();
        for name in names {
            // round 2: the owner check first, so a 409 never names another track's files as this track's
            if let Err(e) = check_file_owner(&lock(op), name) {
                refused.insert(name.clone(), e);
                continue;
            }
            match crate::follow::fresh_for_export(self, op, name) {
                Ok(true) => refreshed.push(Value::from(name.as_str())),
                Ok(false) => {}
                Err(e) => {
                    refused.insert(name.clone(), e);
                }
            }
        }
        (refused, refreshed)
    }

    /// The export jobs of `export_all_opts`: per track the source, the uncut events (or the error
    /// that refused the track, removed from `refused` here), the note and the project's tempo map.
    fn build_jobs(
        op: &Shared,
        names: &[String],
        refused: &mut std::collections::HashMap<String, AppError>,
    ) -> Result<Vec<Job>, AppError> {
        let g = lock(op);
        let mut jobs = Vec::with_capacity(names.len());
        let map = crate::tempo::doc_map(&g.project.doc)?;
        for name in names.iter().cloned() {
            let note = g.track(&name)?.note.unwrap_or(36);
            let events = match refused.remove(&name) {
                Some(e) => Err(e),
                None => Ok(()),
            }
            .and_then(|()| g.track_object(&name))
            .map(|m| {
                m.get("events")
                    .and_then(Value::as_array)
                    .map_or_else(Vec::new, |a| uncut(a))
            });
            jobs.push(Job {
                source: g.source(&name),
                events,
                note,
                map: map.clone(),
                midi: None,
                name,
            });
        }
        Ok(jobs)
    }

    /// P4-7c: the notes of a track with a sampler calibration, before the workers start (the lags of the
    /// layer's alignment read the track's audio). `opts.midi` gates it: without MIDI nothing precomputes.
    fn precompute_midi(&self, sid: &str, opts: &ExportOpts, jobs: &mut [Job]) {
        if opts.midi {
            for job in jobs.iter_mut() {
                let midi = match &job.events {
                    Ok(events) => Some((|| {
                        let shift = if opts.compensate {
                            self.sampler_shift(sid, &job.name, events)?
                        } else {
                            None
                        };
                        midi_events_shifted(events, shift.as_ref())
                    })()),
                    Err(_) => None,
                };
                job.midi = midi;
            }
        }
    }

    /// R01: the whole plan is checked before anything is written: one collision refuses the
    /// entire export, no output of it lands (the folder listing stays as it was).
    fn check_plan_staged(
        &self,
        dir: &Path,
        opts: &ExportOpts,
        names: &[String],
        single_name: &str,
        refused: &std::collections::HashMap<String, AppError>,
    ) -> Result<(), AppError> {
        let mut plan: Vec<PathBuf> = Vec::new();
        if opts.midi && opts.midi_mode == MidiMode::Single {
            plan.push(dir.join(single_name));
        }
        for name in names {
            if refused.contains_key(name) {
                continue;
            }
            if opts.midi && opts.midi_mode == MidiMode::PerTrack {
                plan.push(dir.join(format!("{name}_spike.mid")));
            }
            if opts.spikes {
                plan.push(dir.join(format!("{name}_spike.wav")));
            }
            if opts.audio {
                plan.push(dir.join(format!("{name}_audio_spike.wav")));
            }
        }
        check_export_plan(&plan, &self.protected_files_everywhere())?;
        Ok(())
    }

    /// The worker pool of `export_all_opts`: up to [`EXPORT_WORKERS`] threads (the machine's
    /// parallelism, at least 1) pull the next job index and write each [`TrackResult`] into its
    /// slot; `say` reports `{state: "running", done, total, track}` after each track.
    fn run_export_workers(
        jobs: &[Job],
        dir: &Path,
        opts: &ExportOpts,
        total: usize,
        say: &(impl Fn(Value) + Sync),
    ) -> Vec<Option<TrackResult>> {
        let results: Mutex<Vec<Option<TrackResult>>> = Mutex::new((0..total).map(|_| None).collect());
        let next = AtomicUsize::new(0);
        let finished = AtomicUsize::new(0);
        let workers = std::thread::available_parallelism()
            .map_or(2, usize::from)
            .clamp(1, EXPORT_WORKERS)
            .min(total.max(1));
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::SeqCst);
                        let Some(job) = jobs.get(i) else { break };
                        let r = export_track(job, dir, opts);
                        lock(&results)[i] = Some(r);
                        let done = finished.fetch_add(1, Ordering::SeqCst) + 1;
                        say(json!({"state": "running", "done": done, "total": total, "track": job.name}));
                    }
                });
            }
        });
        results.into_inner().unwrap()
    }

    /// #151: a MIDI export on the default map uses the default 120 BPM: the reply says so (only then).
    fn default_map_warning(op: &Shared, opts: &ExportOpts) -> Result<Vec<String>, AppError> {
        if opts.midi && crate::tempo::doc_map(&lock(op).project.doc)?.is_default() {
            return Ok(vec![
                "No tempo set: the MIDI export uses the default 120 BPM.".to_string(),
            ]);
        }
        Ok(Vec::new())
    }

    /// The reply accumulators of `export_all_opts` from the worker slots: the files of each track
    /// in order, the counts, the per-track errors, the outside hits and the sampler facts.
    fn collect_results(jobs: &[Job], results: &[Option<TrackResult>], warnings: Vec<String>) -> ExportTally {
        let mut tally = ExportTally {
            warnings,
            ..Default::default()
        };
        for (job, r) in jobs.iter().zip(results.iter()) {
            let Some(r) = r else { continue };
            for p in [&r.midi, &r.spike, &r.audio].into_iter().flatten() {
                tally.files.push(Value::from(p.display().to_string()));
            }
            if r.outside > 0 {
                tally.outside.insert(job.name.clone(), Value::from(r.outside));
            }
            tally.midi += usize::from(r.midi.is_some());
            tally.spikes += usize::from(r.spike.is_some());
            tally.audio += usize::from(r.audio.is_some());
            if let Some(i) = &r.sampler {
                tally.sampler.insert(job.name.clone(), i.to_json());
                tally.warnings.extend(i.warning(&job.name));
            }
            for e in &r.errors {
                tally
                    .errors
                    .push(json!({"name": job.name, "code": e.code, "message": e.message}));
            }
        }
        tally
    }

    /// `MidiMode::Single`: stage the per-track MIDI plans into one type 1 file `<project>_drums.mid`
    /// (the project's tempo map, one MIDI track per exported track on its own note); a track whose
    /// events failed is named in `errors` and left out of the file. Pushes the written file onto
    /// `tally.files`, sets `tally.midi` to 1 and answers the path for the reply's `midi_file`
    /// (`Null` when the file could not be written).
    fn write_single_midi(
        op: &Shared,
        dir: &Path,
        single_name: &str,
        jobs: &[Job],
        tally: &mut ExportTally,
    ) -> Result<Value, AppError> {
        let mut staged = Vec::new();
        for job in jobs {
            match &job.midi {
                Some(Ok((e, info))) => {
                    if let Some(i) = info {
                        tally.sampler.insert(job.name.clone(), i.to_json());
                        tally.warnings.extend(i.warning(&job.name));
                    }
                    staged.push((job.name.as_str(), job.note, e.clone()));
                }
                Some(Err(e)) => tally
                    .errors
                    .push(json!({"name": job.name, "code": e.code, "message": e.message})),
                None => {}
            }
        }
        let tracks: Vec<MidiTrack> = staged
            .iter()
            .map(|(n, note, e)| MidiTrack {
                name: n,
                note: *note,
                events: e,
            })
            .collect();
        let out = dir.join(single_name);
        let (map, midi_markers) = {
            let g = lock(op);
            let map = crate::tempo::doc_map(&g.project.doc)?;
            let markers = if doc_export_markers_on(&g.project.doc) {
                doc_export_markers(&g.project.doc)
            } else {
                Vec::new()
            };
            (map, markers)
        };
        let markers: Vec<drumspike_core::midi::MidiMarker> = midi_markers
            .iter()
            .map(|(t, note)| drumspike_core::midi::MidiMarker {
                t: *t,
                note: note.as_deref(),
            })
            .collect();
        match export_multi_midi(&tracks, &markers, &out, EXPORT_PPQ, &map, EXPORT_TEMPO) {
            Ok(()) => {
                tally.files.push(Value::from(out.display().to_string()));
                tally.midi = 1;
                Ok(Value::from(out.display().to_string()))
            }
            Err(e) => {
                let e = AppError::from(e);
                tally
                    .errors
                    .push(json!({"name": "(all tracks)", "code": e.code, "message": e.message}));
                Ok(Value::Null)
            }
        }
    }

    /// `velocities_recompute {sid, name, events?}` (route 34): keep the hit times, recompute
    /// `v` from the track's decoded audio with the track's `vel_lo_db` and `vel_hi_db`
    /// (default -40 and -6 dB; velocities 20 to 127). Every other key of an event stays.
    /// Returns `{events}`. The events are `events` when the call holds them, else the track's.
    pub fn velocities_recompute(&self, sid: &str, name: &str, events: Option<&[Value]>) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        crate::registry::resolve_shared(&op);
        let (source, mix, map) = {
            let g = lock(&op);
            let tr = g.track(name)?;
            let p = |k: &str, d: f64| {
                tr.params
                    .as_ref()
                    .and_then(|m| m.get(k))
                    .and_then(Value::as_f64)
                    .unwrap_or(d)
            };
            let map = VelocityMap {
                lo_db: p("vel_lo_db", -40.0),
                hi_db: p("vel_hi_db", -6.0),
                ..VelocityMap::default()
            };
            (g.source(name)?, mix_mode(tr.mix_mode_or_default()), map)
        };
        let events = match events {
            Some(e) => e.to_vec(),
            None => self.track_events(sid, name)?,
        };
        let times = hit_times(&events)?;
        let d = self.inner.media.decode_cached(&source, mix)?;
        let vels = velocity_map(&d.samples, d.sr, &times, &map);
        let mut out = Vec::with_capacity(events.len());
        for (e, v) in events.into_iter().zip(vels) {
            let mut e = e;
            match e.as_object_mut() {
                Some(m) => {
                    // P4-40f: a hand velocity (`edited: true`) survives the recompute; its `v` stays
                    if m.get("edited").and_then(Value::as_bool) != Some(true) {
                        m.insert("v".into(), Value::from(v));
                    }
                }
                None => return Err(AppError::bad("every event must be an object")),
            }
            out.push(e);
        }
        Ok(json!({"events": out}))
    }
}

/// The files and errors of one track of Export all.
struct TrackResult {
    midi: Option<PathBuf>,
    spike: Option<PathBuf>,
    /// #209: the audio-spike file of the track.
    audio: Option<PathBuf>,
    errors: Vec<AppError>,
    /// Hits with no sample in the spike file (P4-4).
    outside: usize,
    /// P4-7c: what the sampler offset did to the per-track MIDI file.
    sampler: Option<ShiftInfo>,
}

fn export_track(job: &Job, dir: &Path, opts: &ExportOpts) -> TrackResult {
    let _span = crate::trace::stage("export:track");
    let mut r = TrackResult {
        midi: None,
        spike: None,
        audio: None,
        errors: Vec::new(),
        outside: 0,
        sampler: None,
    };
    let events = match &job.events {
        Ok(e) => e,
        Err(e) => {
            r.errors.push(e.clone());
            return r;
        }
    };
    if opts.midi && opts.midi_mode == MidiMode::PerTrack {
        match &job.midi {
            Some(Ok((ev, info))) => {
                let out = dir.join(format!("{}_spike.mid", job.name));
                match write_midi(ev, job.note, &out, EXPORT_PPQ, &job.map, EXPORT_TEMPO) {
                    Ok(()) => {
                        r.midi = Some(out);
                        r.sampler = info.clone();
                    }
                    Err(e) => r.errors.push(AppError::from(e)),
                }
            }
            Some(Err(e)) => r.errors.push(e.clone()),
            None => {}
        }
    }
    if !opts.spikes && !opts.audio {
        return r;
    }
    if opts.spikes {
        let spike = hit_times(events).and_then(|times| match &job.source {
            Ok(src) => write_spike_file(src, &dir.join(format!("{}_spike.wav", job.name)), &times),
            Err(e) => Err(e.clone()),
        });
        match spike {
            Ok(f) => {
                r.outside = f.outside;
                r.spike = Some(f.path);
            }
            Err(e) => r.errors.push(e),
        }
    }
    if opts.audio {
        // #209: the audio-spike file, cut from the track's own audio (1.5 periods per hit)
        let audio = hit_times(events).and_then(|times| match &job.source {
            Ok(src) => write_audio_spike_file(src, &dir.join(format!("{}_audio_spike.wav", job.name)), &times),
            Err(e) => Err(e.clone()),
        });
        match audio {
            Ok((f, _no_period)) => r.audio = Some(f.path),
            Err(e) => r.errors.push(e),
        }
    }
    r
}

/// The bridge side of the three commands (`bridge.rs` calls this for their names).
pub fn bridge_call(
    s: &Service,
    channels: &std::sync::Arc<Channels>,
    cmd: &str,
    args: &Value,
) -> Result<Value, AppError> {
    let str_arg = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{k}` (string)")))
    };
    let events = match args.get("events") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => Some(a.as_slice()),
        Some(_) => return Err(AppError::bad("`events` must be a list")),
    };
    match cmd {
        "export_spikes" => s.export_spikes_to(
            str_arg("sid")?,
            str_arg("name")?,
            events,
            ExportOpts::from_args(args)?.dest.as_deref(),
        ),
        "velocities_recompute" => s.velocities_recompute(str_arg("sid")?, str_arg("name")?, events),
        "export_all" => {
            let progress = args
                .get("progress")
                .and_then(|c| c.get("__channel"))
                .and_then(Value::as_str)
                .map(|id| {
                    let (ch, id) = (channels.clone(), id.to_string());
                    let p: Progress = std::sync::Arc::new(move |v: Value| ch.push(&id, v));
                    p
                });
            s.export_all_opts(str_arg("sid")?, &ExportOpts::from_args(args)?, progress)
        }
        other => Err(AppError::not_found(format!("unknown command {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drumspike_core::io::WavSpec;

    #[test]
    fn spike_format_keeps_the_width_and_float() {
        assert_eq!(spike_format(SampleFormat::Pcm, 2), WavFormat::Pcm16);
        assert_eq!(spike_format(SampleFormat::Pcm, 3), WavFormat::Pcm24);
        assert_eq!(spike_format(SampleFormat::Pcm, 4), WavFormat::Pcm32);
        assert_eq!(spike_format(SampleFormat::Float, 4), WavFormat::Float32);
    }

    #[test]
    fn format_names_are_the_ones_of_write_spike_wav() {
        let names: Vec<_> = [WavFormat::Pcm16, WavFormat::Pcm24, WavFormat::Pcm32, WavFormat::Float32]
            .into_iter()
            .map(format_name)
            .collect();
        assert_eq!(names, ["pcm16", "pcm24", "pcm32", "float32"]);
    }

    #[test]
    fn is_cut_and_uncut_drop_the_events_of_the_loudness_cut_and_the_bleed_rule() {
        let ev = [
            json!({"t": 1, "cut": true}),
            json!({"t": 2, "cut": false}),
            json!({"t": 3}),
            json!({"t": 4, "bleed": true}),
            json!({"t": 5, "bleed": false}),
            json!({"t": 6, "filtered": true}),
            json!({"t": 7, "filtered": false}),
        ];
        assert!(is_cut(&ev[0]) && !is_cut(&ev[1]) && !is_cut(&ev[2]) && is_cut(&ev[3]) && !is_cut(&ev[4]));
        assert!(
            is_cut(&ev[5]) && !is_cut(&ev[6]),
            "P4-6c: a hit the Learn filter takes out leaves the output"
        );
        assert_eq!(
            uncut(&ev),
            vec![ev[1].clone(), ev[2].clone(), ev[4].clone(), ev[6].clone()]
        );
    }

    #[test]
    fn outside_count_counts_the_hits_with_no_sample() {
        let sr = 48_000;
        assert_eq!(outside_count(&[], sr, 10), 0);
        // sample -1, 0 (rounds from -1e-9), 9 (the last), 10 (one past), 0.5 s
        let t = [-1.0 / 48_000.0, -1e-9, 9.0 / 48_000.0, 10.0 / 48_000.0, 0.5];
        assert_eq!(outside_count(&t, sr, 10), 3);
        assert_eq!(outside_count(&[0.0], sr, 0), 1, "an empty file has no sample");
    }

    #[test]
    fn before_count_counts_the_hits_before_the_first_sample() {
        let t = [-1.0 / 48_000.0, -1e-9, 0.0, 0.5, 99.0];
        assert_eq!(before_count(&t, 48_000), 1, "-1e-9 rounds to sample 0");
        assert_eq!(before_count(&[], 48_000), 0);
    }

    #[test]
    fn same_file_name_ignores_case_only() {
        assert!(same_file_name("Kick", "kick") && same_file_name("K In", "k in"));
        assert!(same_file_name("Überkopf", "überkopf"));
        assert!(!same_file_name("Kick", "Kick 2") && !same_file_name("k_in", "k in"));
    }

    #[test]
    fn same_file_name_folds_as_apfs_does() {
        // full case folding: ß is ss; final sigma, sigma and capital sigma are one letter
        assert!(same_file_name("Straße", "STRASSE") && same_file_name("straße", "strasse"));
        assert!(same_file_name("ΟΔΟΣ", "οδος") && same_file_name("ΟΔΟΣ", "οδοσ"));
        // normalisation: a precomposed Ü (NFC) and U + combining diaeresis (NFD)
        assert!(same_file_name("\u{dc}ber", "U\u{308}ber") && same_file_name("\u{dc}ber", "u\u{308}ber"));
        assert_eq!(file_name_key("\u{dc}BER"), file_name_key("u\u{308}ber"));
        assert!(!same_file_name("Strasse", "Strase") && !same_file_name("Uber", "Über"));
    }

    #[test]
    fn hit_times_need_a_numeric_t() {
        let ok = hit_times(&[json!({"t": 1}), json!({"t": 0.5, "v": 3})]).unwrap();
        assert_eq!(ok, [1.0, 0.5]);
        assert_eq!(hit_times(&[json!({"v": 1})]).unwrap_err().code, 400);
        assert_eq!(hit_times(&[json!({"t": "x"})]).unwrap_err().code, 400);
    }

    #[test]
    fn midi_events_default_v_and_reject_a_non_integer_v() {
        let ev = midi_events(&[json!({"t": 0.5}), json!({"t": 1.0, "v": 90})]).unwrap();
        assert_eq!((ev[0].v, ev[1].v), (100, 90));
        assert_eq!(midi_events(&[json!({"t": 0.5, "v": 1.5})]).unwrap_err().code, 400);
    }

    #[test]
    fn write_spike_file_names_an_unreadable_source() {
        let d = std::env::temp_dir().join(format!("ds_app_export_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("x.mp3"), b"ID3 not a wav file at all").unwrap();
        let e = write_spike_file(&d.join("x.mp3"), &d.join("o.wav"), &[0.1]).unwrap_err();
        assert_eq!(e.code, 400);
        assert!(e.message.contains("x.mp3"), "{e}");
        assert!(!d.join("o.wav").exists());
        let e = write_spike_file(&d.join("none.wav"), &d.join("o.wav"), &[0.1]).unwrap_err();
        assert_eq!(e.code, 500);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_audio_export_refuses_an_output_path_equal_to_the_source() {
        let d = std::env::temp_dir().join(format!("ds_app_audio_export_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let spec = WavSpec {
            sample_rate: 48_000,
            channels: 1,
            format: WavFormat::Pcm16,
            extensible: false,
            channel_mask: 0,
        };
        let x: Vec<f32> = (0..48_000)
            .map(|i| (std::f64::consts::TAU * 240.0 * i as f64 / 48_000.0).sin() as f32)
            .collect();
        let src = d.join("kick in.wav");
        drumspike_core::io::write_wav(&src, &spec, &x, None).unwrap();
        // the literal path, and the same file through another spelling of it
        for out in [src.clone(), d.join("./kick in.wav")] {
            let e = write_audio_spike_file(&src, &out, &[0.5]).unwrap_err();
            assert_eq!(e.code, 400, "{e}");
            assert!(e.message.contains("source file"), "{e}");
        }
        assert!(!d.join("other.wav").exists(), "nothing was written on the refusal");
        // a different name exports
        write_audio_spike_file(&src, &d.join("out.wav"), &[0.5]).unwrap();
        assert!(d.join("out.wav").is_file());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_audio_export_leaves_the_source_unchanged_and_writes_the_snippets() {
        let d = std::env::temp_dir().join(format!("ds_app_audio_export2_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        // a stereo source: the snippets come from the mono mix
        let spec = WavSpec {
            sample_rate: 48_000,
            channels: 2,
            format: WavFormat::Pcm16,
            extensible: false,
            channel_mask: 0,
        };
        let x: Vec<f32> = (0..96_000)
            .map(|i| (std::f64::consts::TAU * 240.0 * i as f64 / 48_000.0).sin() as f32)
            .collect();
        let stereo: Vec<f32> = x.iter().flat_map(|&v| [v, v * 0.5]).collect();
        let src = d.join("snare.wav");
        drumspike_core::io::write_wav(&src, &spec, &stereo, None).unwrap();
        let before = std::fs::read(&src).unwrap();
        let (f, no_period) = write_audio_spike_file(&src, &d.join("snare_audio_spike.wav"), &[0.5]).unwrap();
        assert_eq!(no_period, 0);
        assert_eq!(f.n_samples, 96_000);
        assert_eq!(f.sr, 48_000);
        assert_eq!(std::fs::read(&src).unwrap(), before, "the source's bytes are unchanged");
        let (y, info) = drumspike_core::read_wav_mono(&d.join("snare_audio_spike.wav"), MixMode::Ffmpeg).unwrap();
        assert_eq!(
            info.n_frames, 96_000,
            "the audio-spike file has the length of the source"
        );
        let start = 24_000usize;
        // the snippet's end: the first sample of a real zero run (the source has exact-zero
        // crossing samples inside the snippet, they are part of the copy)
        let end = (start + 1..)
            .find(|&j| y[j..(j + 64).min(y.len())].iter().all(|&v| v == 0.0))
            .unwrap();
        assert!(
            ((end - start) as i64 - 300).abs() <= 1,
            "1.5 periods of 200 samples, got {}",
            end - start
        );
        assert!(y[..start].iter().all(|&v| v == 0.0), "exact silence before the snippet");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_formats_parse_midi_spikes_and_audio() {
        let o = ExportOpts::from_args(&json!({ "formats": ["audio"] })).unwrap();
        assert!(!o.midi && !o.spikes && o.audio);
        let o = ExportOpts::from_args(&json!({ "formats": ["midi", "audio"] })).unwrap();
        assert!(o.midi && o.audio && !o.spikes);
        assert_eq!(
            ExportOpts::from_args(&json!({ "formats": ["audio", "x"] }))
                .unwrap_err()
                .code,
            400
        );
        assert!(!ExportOpts::default().audio, "the default export does not change");
    }

    #[test]
    fn midi_mode_parses_the_two_words() {
        assert_eq!(MidiMode::parse(None).unwrap(), MidiMode::PerTrack);
        assert_eq!(MidiMode::parse(Some("per_track")).unwrap(), MidiMode::PerTrack);
        assert_eq!(MidiMode::parse(Some("single")).unwrap(), MidiMode::Single);
        assert_eq!(MidiMode::parse(Some("one")).unwrap_err().code, 400);
    }

    #[test]
    fn the_single_file_name_is_the_project_name_without_path_characters() {
        assert_eq!(multi_file_name("Song"), "Song_drums.mid");
        assert_eq!(multi_file_name("a/b:c"), "a-b-c_drums.mid");
        assert_eq!(multi_file_name("  "), "project_drums.mid");
    }

    #[test]
    fn select_tracks_keeps_the_document_order_and_refuses_an_unknown_name() {
        let all = || vec!["k".to_string(), "s".to_string(), "t".to_string()];
        assert_eq!(select_tracks(all(), None).unwrap(), all());
        assert_eq!(
            select_tracks(all(), Some(&["t".to_string(), "k".to_string()])).unwrap(),
            ["k", "t"]
        );
        assert_eq!(select_tracks(all(), Some(&["x".to_string()])).unwrap_err().code, 400);
    }

    #[test]
    fn check_dest_leaves_a_preexisting_probe_name_and_symlink_alone() {
        // the old probe name `.drumspike-write-test-<pid>`: a file and a symlink that are
        // there before the check must survive it (R11)
        let d = std::env::temp_dir().join(format!("ds_app_checkdest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::create_dir_all(d.join("b")).unwrap();
        let old = format!(".drumspike-write-test-{}", std::process::id());
        std::fs::write(d.join("a").join(&old), b"keep").unwrap();
        let target = d.join("b").join("target.txt");
        std::fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, d.join("b").join(&old)).unwrap();
        check_dest(d.join("a").to_str().unwrap()).unwrap();
        check_dest(d.join("b").to_str().unwrap()).unwrap();
        assert_eq!(
            std::fs::read(d.join("a").join(&old)).unwrap(),
            b"keep",
            "the pre-existing file survived"
        );
        let link = std::fs::symlink_metadata(d.join("b").join(&old)).unwrap();
        assert!(link.file_type().is_symlink(), "the pre-existing symlink survived");
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"keep",
            "the symlink target is untouched"
        );
        assert_eq!(
            std::fs::read_dir(d.join("a")).unwrap().flatten().count(),
            1,
            "no probe left behind"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn overlapping_exports_to_one_folder_serialize() {
        let d = std::env::temp_dir().join(format!("ds_app_destguard_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let first = export_dest_acquire(&d);
        let (tx, rx) = std::sync::mpsc::channel();
        let d2 = d.clone();
        std::thread::spawn(move || {
            let _second = export_dest_acquire(&d2);
            std::fs::write(d2.join("second.marker"), b"done").unwrap();
            tx.send(()).expect("receiver alive");
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            !d.join("second.marker").exists(),
            "the second export waits while the first holds the folder"
        );
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(300)).is_err(),
            "still serialized"
        );
        drop(first);
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("the second export ran after the release");
        assert!(d.join("second.marker").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn concurrent_check_dest_checks_never_collide() {
        let d = std::env::temp_dir().join(format!("ds_app_checkdest_par_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let old = format!(".drumspike-write-test-{}", std::process::id());
        std::fs::write(d.join(&old), b"keep").unwrap();
        let errors: Vec<String> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let dir = d.to_str().unwrap().to_string();
                    s.spawn(move || check_dest(&dir).map(|_| ()).map_err(|e| e.to_string()))
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().unwrap().err()).collect()
        });
        assert!(errors.is_empty(), "threads failed check_dest: {errors:?}");
        assert_eq!(
            std::fs::read(d.join(&old)).unwrap(),
            b"keep",
            "the pre-existing file survived"
        );
        assert_eq!(
            std::fs::read_dir(&d).unwrap().flatten().count(),
            1,
            "no probe left behind"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn check_dest_and_the_options_default() {
        assert_eq!(check_dest("rel").unwrap_err().code, 400);
        assert_eq!(check_dest("/definitely/not/here/ds").unwrap_err().code, 404);
        let o = ExportOpts::default();
        assert!(o.midi && o.spikes && o.dest.is_none());
        assert!(
            ExportOpts::from_args(&json!({"formats": ["midi", "spikes"]}))
                .unwrap()
                .spikes
        );
    }

    #[test]
    fn bridge_call_checks_its_arguments() {
        let s = Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        let ch = std::sync::Arc::new(Channels::default());
        assert_eq!(bridge_call(&s, &ch, "export_spikes", &json!({})).unwrap_err().code, 400);
        assert_eq!(
            bridge_call(&s, &ch, "export_spikes", &json!({"sid": "x", "name": "n", "events": 3}))
                .unwrap_err()
                .code,
            400
        );
        assert_eq!(
            bridge_call(&s, &ch, "velocities_recompute", &json!({"sid": "x", "name": "n"}))
                .unwrap_err()
                .code,
            404
        );
        assert_eq!(
            bridge_call(&s, &ch, "export_all", &json!({"sid": "x"}))
                .unwrap_err()
                .code,
            404
        );
        assert_eq!(bridge_call(&s, &ch, "nope", &json!({})).unwrap_err().code, 404);
    }
}
