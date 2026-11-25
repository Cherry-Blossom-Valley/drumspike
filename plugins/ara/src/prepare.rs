//! M1c3 (#576): the files that leave the plugin, made BEFORE the user asks for them.
//!
//! The design is `docs/design/daw-plugin/m1c3-one-drag.md`. A row of the page is one DAW track (`rows.rs`); this module makes ONE file per
//! row and kind (MIDI, synthetic spike, audio spike) with the hits of every clip of the row at its song position, plus one merged
//! MIDI file for all rows. The files are written to `Exports/.prepared/` by a worker thread (never in an audio callback, never on the
//! main thread) and linked into `Exports/` under their final names when they are ready. Every set carries `fp`, a hash of everything
//! the file is made from: a set whose `fp` differs from the current one is stale, is never taken, and is made again.
//!
//! The pure parts (mapping, names, hashes, the state of a handle, the sum of two audio-spike files) have unit tests; the worker and the
//! shell commands are proved by the stages `prepare-*` of `check.py`.

use crate::rows::{Clip, Rows};
use crate::service::PageHandle;
use crate::tempo_ara::Answer;
use drumspike_app::export::{uncut, write_audio_spike_file_song, write_spike_file_song, EXPORT_TEMPO};
use drumspike_app::export_song::{song_end, song_pairs, to_song, SongRegion};
use drumspike_core::io::{read_wav_mono, write_wav, Bext, MixMode, WavFormat, WavSpec};
use drumspike_core::midi::{export_merged_midi, export_midi_map, export_multi_midi, export_ppq, MidiEvent, MidiTrack};
use drumspike_core::tempo::TempoMap;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

/// The folder of the prepared sets, a fixed child of the default export folder. Only this module writes it.
pub const PREPARED_DIR: &str = ".prepared";
/// The most files one drag may carry (the limit of `start_drag_command`, unit tested).
pub const MAX_DRAG_FILES: usize = 512;
/// How long a handle may stay `preparing` for one state of the events before it says so (the 30 s limit of the dialog).
pub const WATCHDOG: Duration = Duration::from_secs(30);
/// The bound of `.prepared/` (ruling R11): a set that would take the folder over it fails with a reason.
pub const CAP_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// The fixed plugin-owned folder below `Exports/` that a take hard-links its files into (round 2: the worker never writes in `Exports/`).
pub const DRAGGED_DIR: &str = "Dragged";
/// What the sum of several audio-spike parts may hold in memory (round 2, S8).
pub const MEM_CAP_BYTES: u64 = 1024 * 1024 * 1024;
/// The fixed stem of the merged MIDI file (ruling R7).
pub const MERGED_STEM: &str = "Drums_merged";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Midi,
    Spike,
    Audio,
    Merged,
}

impl Kind {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Kind::Midi => "midi",
            Kind::Spike => "spike",
            Kind::Audio => "audio",
            Kind::Merged => "merged",
        }
    }
    #[must_use]
    pub fn parse(s: &str) -> Option<Kind> {
        [Kind::Midi, Kind::Spike, Kind::Audio, Kind::Merged]
            .into_iter()
            .find(|k| k.word() == s)
    }
    fn suffix(self) -> &'static str {
        match self {
            Kind::Midi => "_spike.mid",
            Kind::Merged => ".mid",
            Kind::Spike => "_spike.wav",
            Kind::Audio => "_audio_spike.wav",
        }
    }
    fn is_wav(self) -> bool {
        matches!(self, Kind::Spike | Kind::Audio)
    }
}

/// The worker threads that run (a plugin unload waits for them), and the stop sign that unload sets.
static LIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How many worker threads of the preparation run now.
#[must_use]
pub fn live_threads() -> usize {
    LIVE.load(std::sync::atomic::Ordering::SeqCst)
}

/// The plugin library is about to be unmapped: every worker thread of the preparation stops. Waits at most `wait` for them; true when none runs.
pub fn stop_all(wait: Duration) -> bool {
    STOP.store(true, std::sync::atomic::Ordering::SeqCst);
    let t0 = Instant::now();
    while live_threads() > 0 && t0.elapsed() < wait {
        std::thread::sleep(Duration::from_millis(5));
    }
    live_threads() == 0
}

struct LiveGuard(Arc<std::sync::atomic::AtomicBool>);
impl Drop for LiveGuard {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        LIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn stopped() -> bool {
    STOP.load(std::sync::atomic::Ordering::SeqCst)
}

/// A point inside a long job where it looks at the stop sign (between sources, between jobs), so `stop_all` returns in time.
fn checkpoint() -> Result<(), String> {
    if stopped() {
        Err("stopped".into())
    } else {
        Ok(())
    }
}

/// The fault runs of the check (`DS_ARA_PREPARE_FAULT`); a delivery build reads none.
fn fault() -> String {
    crate::test_switch("DS_ARA_PREPARE_FAULT").unwrap_or_default()
}

/// True when the fault run `f` is one of the faults of this run (`DS_ARA_PREPARE_FAULT` may name several, separated by commas).
fn has(f: &str) -> bool {
    fault().split(',').any(|x| x == f)
}

/// The watchdog of this run (`DS_ARA_PREPARE_WAIT_MS` shortens it for the check).
fn limit() -> Duration {
    if has("no-watchdog") {
        return Duration::from_secs(10 * 365 * 86_400);
    }
    watchdog()
}

fn watchdog() -> Duration {
    crate::test_switch("DS_ARA_PREPARE_WAIT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(WATCHDOG, Duration::from_millis)
}

// ---------------------------------------------------------------- what a row is made from

#[derive(Debug, Clone)]
pub struct SrcIn {
    pub key: String,
    /// The uncut events of the source (source time).
    pub events: Vec<Value>,
    pub rate: u32,
    pub wav: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct RowIn {
    pub id: String,
    pub name: String,
    pub clips: Vec<Clip>,
    pub srcs: Vec<SrcIn>,
    pub note: i64,
}

/// The regions of one source inside one row: the clips of THIS row that have the source (a source that is in two rows delivers only
/// the clips of the row it is made for).
#[must_use]
pub fn regions_of(row: &RowIn, src: &str) -> Vec<SongRegion> {
    row.clips
        .iter()
        .filter(|c| c.src == src)
        .map(|c| SongRegion {
            start_mod: c.a,
            dur_mod: c.b - c.a,
            start_play: c.s,
            dur_play: (c.b - c.a) * c.k,
        })
        .collect()
}

/// Every hit of the row in song time (events of `to_song`), by time. A source with no clip of the row gives none.
#[must_use]
pub fn mapped_events(row: &RowIn) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for s in &row.srcs {
        let regions = regions_of(row, &s.key);
        if regions.is_empty() {
            continue;
        }
        out.extend(to_song(&s.events, &regions).0);
    }
    out.sort_by(|a, b| {
        let (x, y) = (a["t"].as_f64().unwrap_or(0.0), b["t"].as_f64().unwrap_or(0.0));
        x.total_cmp(&y)
    });
    out
}

/// The end of the last clip of the row in the song (seconds).
#[must_use]
pub fn row_end(row: &RowIn) -> Option<f64> {
    let all: Vec<SongRegion> = row.srcs.iter().flat_map(|s| regions_of(row, &s.key)).collect();
    song_end(&all)
}

fn midi_events(events: &[Value]) -> Result<Vec<MidiEvent>, String> {
    events
        .iter()
        .map(|e| {
            let t = e
                .get("t")
                .and_then(Value::as_f64)
                .ok_or("every event needs a numeric t")?;
            let v = match e.get("v") {
                None => 100,
                Some(v) => v.as_i64().ok_or("v must be an integer")?,
            };
            Ok(MidiEvent { t, v })
        })
        .collect()
}

/// The first source that has a clip in the row (its rate and file are those of the row's MIDI, spike and audio files).
#[must_use]
pub fn first_used(row: &RowIn) -> Option<&SrcIn> {
    row.srcs
        .iter()
        .find(|s| !regions_of(row, &s.key).is_empty())
        .or_else(|| row.srcs.first())
}

/// The file stem of every row: the DAW track name made safe by `deliver::file_stems` (the numbering of #560 over the rows).
/// `MERGED_STEM` takes part under the id `~merged`, so a row of that name is numbered, not overwritten by it.
#[must_use]
pub fn stems(rows: &[RowIn]) -> HashMap<String, String> {
    let mut names: HashMap<String, String> = rows.iter().map(|r| (r.id.clone(), r.name.clone())).collect();
    names.insert("~merged".into(), MERGED_STEM.into());
    let mut out = crate::deliver::file_stems(&names);
    if has("raw-name") {
        // the fault run: the row name goes into the file name as it is
        for r in rows {
            out.insert(r.id.clone(), r.name.clone());
        }
    }
    out
}

fn file_name(stem: &str, kind: Kind) -> String {
    format!("{stem}{}", kind.suffix())
}

/// The final file name of every set, unique inside its extension (S5): MIDI and the merged file share `.mid`, the spike and the audio spike
/// share `.wav`; two names that equal each other without case (`drumspike_app::export::file_name_key`) number the later one `Stem (2)`.
/// The key of a row set is `(row id, kind)`, the one of the merged file `("", Merged)`.
#[must_use]
pub fn final_names(rows: &[RowIn], stems: &HashMap<String, String>) -> HashMap<(String, Kind), String> {
    let mut out = HashMap::new();
    let mut ids: Vec<&RowIn> = rows.iter().collect();
    ids.sort_by(|a, b| a.id.cmp(&b.id));
    for set in [[Kind::Midi, Kind::Merged], [Kind::Spike, Kind::Audio]] {
        let mut seen: HashSet<String> = HashSet::new();
        for kind in set {
            let entries: Vec<(String, String)> = if kind == Kind::Merged {
                vec![(
                    String::new(),
                    stems.get("~merged").cloned().unwrap_or_else(|| MERGED_STEM.into()),
                )]
            } else {
                ids.iter()
                    .filter_map(|r| stems.get(&r.id).map(|st| (r.id.clone(), st.clone())))
                    .collect()
            };
            for (id, stem) in entries {
                let mut n = 1;
                let mut name = file_name(&stem, kind);
                while !has("no-file-numbering") && seen.contains(&drumspike_app::export::file_name_key(&name)) {
                    n += 1;
                    name = file_name(&format!("{stem} ({n})"), kind);
                }
                seen.insert(drumspike_app::export::file_name_key(&name));
                out.insert((id, kind), name);
            }
        }
    }
    out
}

/// The error of an audio-spike set whose sources have different sample rates (ruling R8), checked before anything is written.
///
/// # Errors
///
/// The text the handle shows.
pub fn check_rates(row: &RowIn) -> Result<(), String> {
    let mut rates = row
        .srcs
        .iter()
        .filter(|s| !regions_of(row, &s.key).is_empty())
        .map(|s| s.rate);
    match rates.next() {
        Some(first) if rates.any(|r| r != first) => Err("clips have different sample rates".into()),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------- the hash of a set

fn h_f(h: &mut impl Hasher, v: f64) {
    v.to_bits().hash(h);
}

/// `fp`: everything the file of `kind` for `row` is made from. A change of any input changes it; an input that the kind does not
/// read (the velocity of a spike) does not.
#[must_use]
pub fn fingerprint(row: &RowIn, kind: Kind, name: &str, tempo_key: &str, bump: u64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (kind.word(), name, bump).hash(&mut h);
    for c in &row.clips {
        (&c.src, c.a.to_bits(), c.b.to_bits(), c.s.to_bits(), c.k.to_bits()).hash(&mut h);
    }
    for s in &row.srcs {
        (&s.key, s.rate, &s.wav).hash(&mut h);
        for e in &s.events {
            h_f(&mut h, e["t"].as_f64().unwrap_or(f64::NAN));
            if kind == Kind::Midi {
                e["v"].as_i64().unwrap_or(100).hash(&mut h);
            }
        }
    }
    if kind == Kind::Midi {
        // #577: the note of the row is an input of the MIDI file (fault `fp-no-note`: it is not, so a Load Preset leaves the old file ready)
        let note = if has("fp-no-note") { 0 } else { row.note };
        (note, tempo_key).hash(&mut h);
    }
    h.finish()
}

// ---------------------------------------------------------------- the files

fn ppq_of(rate: u32, map: &TempoMap) -> u16 {
    export_ppq(rate, map, EXPORT_TEMPO)
}

/// The MIDI file of one row.
///
/// # Errors
///
/// The text of the failure.
pub fn write_midi_row(row: &RowIn, map: &TempoMap, out: &Path) -> Result<(), String> {
    let ev = midi_events(&mapped_events(row))?;
    let rate = first_used(row).map_or(48_000, |s| s.rate);
    export_midi_map(&ev, row.note, out, ppq_of(rate, map), map, EXPORT_TEMPO, rate).map_err(|e| e.to_string())
}

/// The merged MIDI file: all rows with hits on ONE note track, each hit on its row's note.
///
/// # Errors
///
/// The text of the failure.
pub fn write_merged(rows: &[RowIn], map: &TempoMap, out: &Path) -> Result<(), String> {
    checkpoint()?;
    let evs: Vec<(usize, Vec<MidiEvent>)> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| midi_events(&mapped_events(r)).map(|e| (i, e)))
        .collect::<Result<_, _>>()?;
    let tracks: Vec<MidiTrack> = evs
        .iter()
        .filter(|(_, e)| !e.is_empty())
        .map(|(i, e)| MidiTrack {
            name: &rows[*i].name,
            note: rows[*i].note,
            rate: first_used(&rows[*i]).map_or(48_000, |s| s.rate),
            events: e,
        })
        .collect();
    let rate = tracks.iter().map(|t| t.rate).max().unwrap_or(48_000);
    let ppq = ppq_of(rate, map);
    if has("two-tracks") {
        // the fault run: one MIDI track per drum, as the single mode of the dialog
        return export_multi_midi(&tracks, &[], out, ppq, map, EXPORT_TEMPO).map_err(|e| e.to_string());
    }
    export_merged_midi(&tracks, out, ppq, map, EXPORT_TEMPO).map_err(|e| e.to_string())
}

fn home_wav(row: &RowIn) -> Result<&Path, String> {
    first_used(row)
        .and_then(|s| s.wav.as_deref())
        .ok_or_else(|| format!("no audio for {}", row.name))
}

/// The bytes a spike or audio-spike file of the row will take (float 32, one channel, the length of the row in the song): the check of the
/// bound of `.prepared/` runs before anything is written (S8).
#[must_use]
pub fn estimate_bytes(row: &RowIn) -> u64 {
    let rate = first_used(row).map_or(48_000, |s| s.rate);
    let frames = row_end(row).map_or(0.0, |e| (e * f64::from(rate)).ceil().max(0.0));
    (frames as u64).saturating_mul(4)
}

/// The synthetic spike file of one row: the times of every source, the rate and format of the first source.
///
/// # Errors
///
/// The text of the failure.
pub fn write_spike_row(row: &RowIn, out: &Path) -> Result<(), String> {
    check_rates(row)?;
    let times: Vec<f64> = mapped_events(row).iter().filter_map(|e| e["t"].as_f64()).collect();
    write_spike_file_song(home_wav(row)?, out, &times, row_end(row))
        .map(|_| ())
        .map_err(|e| e.message)
}

/// The sum of audio-spike signals (a plain float sum; the snippets of two sources meet only where two clips overlap).
#[must_use]
pub fn sum_signals(parts: &[Vec<f32>]) -> Vec<f32> {
    let n = parts.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = vec![0.0f32; n];
    for p in parts {
        for (o, v) in out.iter_mut().zip(p) {
            *o += *v;
        }
    }
    out
}

/// The audio-spike file of one row. One source: the export call. Several: one temporary file per source, summed (ruling R8).
///
/// # Errors
///
/// The text of the failure.
pub fn write_audio_row(row: &RowIn, out: &Path) -> Result<(), String> {
    check_rates(row)?;
    let end = row_end(row);
    let used: Vec<&SrcIn> = row
        .srcs
        .iter()
        .filter(|s| !regions_of(row, &s.key).is_empty())
        .collect();
    if used.len() <= 1 {
        let s = used.first().copied().or(row.srcs.first()).ok_or("no source")?;
        let wav = s.wav.as_deref().ok_or_else(|| format!("no audio for {}", row.name))?;
        let pairs = song_pairs(&s.events, &regions_of(row, &s.key));
        return write_audio_spike_file_song(wav, out, &pairs, end)
            .map(|_| ())
            .map_err(|e| e.message);
    }
    if (used.len() as u64).saturating_mul(estimate_bytes(row)) > MEM_CAP_BYTES {
        return Err("not enough memory for the audio spikes of this row".into());
    }
    let mut parts = Vec::new();
    let mut rate = 0;
    let mut format = WavFormat::Float32;
    for (i, s) in used.iter().enumerate() {
        checkpoint()?;
        let wav = s.wav.as_deref().ok_or_else(|| format!("no audio for {}", s.key))?;
        let tmp = out.with_extension(format!("part{i}.tmp"));
        let pairs = song_pairs(&s.events, &regions_of(row, &s.key));
        let r = write_audio_spike_file_song(wav, &tmp, &pairs, end).map_err(|e| e.message);
        let read = r.and_then(|(f, _)| {
            format = f.format;
            rate = f.sr;
            read_wav_mono(&tmp, MixMode::Ffmpeg)
                .map(|(v, _)| v)
                .map_err(|e| e.to_string())
        });
        let _ = std::fs::remove_file(&tmp);
        parts.push(read?);
    }
    let spec = WavSpec {
        sample_rate: rate,
        channels: 1,
        format,
        extensible: false,
        channel_mask: 0,
    };
    let bext = Bext {
        description: "Drum Spike audio spikes".into(),
        originator: String::new(),
        originator_reference: String::new(),
        origination_date: String::new(),
        origination_time: String::new(),
        time_reference: 0,
        version: 1,
        coding_history: String::new(),
    };
    write_wav(out, &spec, &sum_signals(&parts), Some(&bext)).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- the state of a handle

/// What a handle shows.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Ready(usize),
    Preparing,
    Error(String),
    /// A wav kind before its first use.
    Off,
    /// The row has no hit.
    None,
}

impl State {
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            State::Ready(n) => json!({"state": "ready", "n": n, "why": ""}),
            State::Preparing => json!({"state": "preparing", "n": 0, "why": ""}),
            State::Error(w) => json!({"state": "error", "n": 0, "why": w}),
            State::Off => json!({"state": "off", "n": 0, "why": "press to prepare"}),
            State::None => json!({"state": "none", "n": 0, "why": "no hits"}),
        }
    }
}

/// The state from what is known: `ready_fp` is the fp of the set on disk, `want` the current fp (`None`: the row has no hit),
/// `failed` the reason of a failed job for `want`, `pending` how long the work for `want` has been due or running.
#[must_use]
pub fn state_of(
    kind: Kind,
    used: bool,
    want: Option<u64>,
    ready: Option<(u64, usize)>,
    failed: Option<&str>,
    pending: Option<Duration>,
    limit: Duration,
) -> State {
    let Some(want) = want else { return State::None };
    if kind.is_wav() && !used {
        return State::Off;
    }
    if let Some((fp, n)) = ready {
        if fp == want {
            return State::Ready(n);
        }
    }
    if let Some(why) = failed {
        return State::Error(why.to_string());
    }
    if pending.is_some_and(|p| p > limit) {
        return State::Error("no answer from the preparation".into());
    }
    State::Preparing
}

// ---------------------------------------------------------------- the worker

type Key = (String, Kind);

struct Ready {
    fp: u64,
    /// The files in `.prepared/` (the worker writes nowhere else).
    prepared: Vec<PathBuf>,
}

/// One failed job: the `fp` it failed for, the reason, when, and how many times in a row.
type Failed = (u64, String, Instant, u32);

/// The wait before a failed job is tried again by itself: 5 s, doubled for each failure, at most 60 s.
#[must_use]
pub fn backoff(tries: u32) -> Duration {
    Duration::from_secs((5u64 << tries.saturating_sub(1).min(4)).min(60))
}

/// True while a failure holds the work back (the same `fp`, the back-off not over).
fn failed_blocks(f: &Failed, fp: u64) -> bool {
    f.0 == fp && f.2.elapsed() < backoff(f.3)
}

#[derive(Default)]
struct St {
    first: Option<Instant>,
    last: Option<Instant>,
    used: [bool; 2],
    want: HashMap<Key, u64>,
    ready: HashMap<Key, Ready>,
    failed: HashMap<Key, Failed>,
    /// The document has no export folder (the service is open but gives none): the handles say so.
    no_folder: bool,
    pending_since: HashMap<Key, Instant>,
    busy: Option<(Key, Instant)>,
    /// (id, name, srcs) of the rows at the last cycle.
    meta: Vec<(String, String, Vec<String>)>,
    notes: HashMap<String, i64>,
    started: bool,
    gen: u64,
    pushed: String,
    cycles: u64,
    jobs: u64,
    bump: u64,
    midi_pending: bool,
    wav_pending: bool,
    /// The tempo as the worker saw it last (a change wakes the worker).
    tempo_seen: Option<String>,
    /// The scenario `late-row`: the row `Snare_audio` shows only after the first take.
    revealed: bool,
    /// Counts the wakes; `clean_seq` is the count that the last pass looked at (a handle shows `preparing` while they differ: something changed since).
    wake_seq: u64,
    clean_seq: u64,
    pushed_seq: u64,
}

pub type FilesFn = Arc<dyn Fn() -> HashMap<String, (PathBuf, u32)> + Send + Sync>;

/// The preparation of one document.
pub struct Prep {
    page: Arc<PageHandle>,
    rows: Arc<dyn Fn() -> Rows + Send + Sync>,
    tempo: Arc<dyn Fn() -> Answer + Send + Sync>,
    files: FilesFn,
    st: Mutex<St>,
    cv: Condvar,
    /// Set by the worker thread when it ends.
    ended: Arc<std::sync::atomic::AtomicBool>,
    /// A command that writes the project ran and no `done` of a job was seen since (see [`Prep::command_done`]).
    job_open: std::sync::atomic::AtomicBool,
    /// M1d4 (#578): the rows that are not done (a handle of such a row is never ready) and the rows that are only in progress.
    gate: Mutex<Option<GateFn>>,
    /// M1c4 (#589): the progress and the cancel of the export of the dialog.
    exp: ExportProgress,
}

/// One file of a planned export: `(row id, kind, file name, DAW track)`.
pub type ExportName = (String, Kind, String, String);

/// The failure of an export: the status, the text the dialog shows, and, when some files were written before a rename failed, which files and which DAW tracks.
#[derive(Debug, Clone)]
pub struct ExportError {
    pub code: u16,
    pub text: String,
    pub written: Vec<String>,
    pub not_written: Vec<String>,
}

impl From<(u16, String)> for ExportError {
    fn from((code, text): (u16, String)) -> Self {
        ExportError {
            code,
            text,
            written: Vec::new(),
            not_written: Vec::new(),
        }
    }
}

/// How a copy ended when it did not.
#[derive(Debug, PartialEq, Eq)]
pub enum CopyErr {
    Cancelled,
    /// The prepared file is gone (the worker made the set again).
    Missing,
    Other(String),
}

type CopyFn = Box<dyn Fn(&Path, &Path, &std::sync::atomic::AtomicBool) -> Result<(), CopyErr>>;
type RenameFn = Box<dyn Fn(&Path, &Path) -> std::io::Result<()>>;

/// The file operations of an export (the tests change them).
pub struct ExportHooks {
    pub copy: CopyFn,
    pub rename: RenameFn,
    /// Called between the last copy and the check of the hits.
    pub after_copy: Box<dyn Fn()>,
}

impl ExportHooks {
    #[must_use]
    pub fn real() -> ExportHooks {
        ExportHooks {
            copy: Box::new(copy_file),
            rename: Box::new(|a, b| std::fs::rename(a, b)),
            after_copy: Box::new(|| {
                // the fault run `slow-copy` (a gate build only) keeps the copy open for a while: the stage changes the hits in that time
                if has("slow-copy") {
                    std::thread::sleep(Duration::from_millis(2500));
                }
            }),
        }
    }
}

/// The temporary files of one export: removed when it ends, however it ends (an error, a cancel, a panic).
struct Temps(Vec<PathBuf>);

impl Drop for Temps {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

static TEMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const TEMP_MARK: &str = ".dsx-";

/// A temporary name of this process that no earlier export used: `.<name>.dsx-<pid>-<n>.tmp`.
fn temp_name(name: &str) -> String {
    let n = TEMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!(".{name}{TEMP_MARK}{}-{n}.tmp", std::process::id())
}

/// The process and number of a temporary name of the plugin (`None` for any other name).
fn temp_parts(name: &str) -> Option<(u32, u64)> {
    let rest = name.strip_prefix('.')?.strip_suffix(".tmp")?;
    let tail = &rest[rest.rfind(TEMP_MARK)? + TEMP_MARK.len()..];
    let (pid, n) = tail.split_once('-')?;
    Some((pid.parse().ok()?, n.parse().ok()?))
}

/// Exports of this process that are between their clean-up and their last rename (all documents, all plugin instances of the DAW).
static COPYING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Counts one export in `COPYING` until it ends, however it ends.
struct CopyGuard;

impl CopyGuard {
    fn new() -> Self {
        COPYING.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        CopyGuard
    }
}

impl Drop for CopyGuard {
    fn drop(&mut self) {
        COPYING.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Removes the temporary files that an earlier export of the plugin left in `dir`: a name of this process with an earlier number, or a name
/// of any process that is older than one hour. `alone` is false while another export of this process copies (another document or plugin
/// instance of the DAW can export into the same folder): then a name of this process can be its live file, and only the age rule applies.
fn clean_old_temps(dir: &Path, alone: bool) {
    let now = TEMP_SEQ.load(std::sync::atomic::Ordering::SeqCst);
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some((pid, n)) = temp_parts(&name) else { continue };
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|d| d > Duration::from_secs(3600));
        if ((alone && pid == std::process::id() && n < now) || old) && e.metadata().is_ok_and(|m| m.is_file()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// True when a file or link is in the way of an export (it is replaced); a folder is not (it is refused).
#[must_use]
pub fn replaces(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| !m.is_dir())
}

/// The export of the dialog in progress: files done of the total, the DAW track being copied, and the cancel flag (read until the first rename).
#[derive(Default)]
struct ExportProgress {
    done: std::sync::atomic::AtomicUsize,
    total: std::sync::atomic::AtomicUsize,
    busy: std::sync::atomic::AtomicBool,
    /// An export (the waiting calls too) is in progress: one at a time.
    running: std::sync::atomic::AtomicBool,
    cancel: std::sync::atomic::AtomicBool,
    track: Mutex<String>,
}

/// M1d4: the ids of the DAW track rows that are not done: `(id, in_progress)`. A failed or offline row is not in progress.
pub type GateFn = Arc<dyn Fn() -> Vec<(String, bool)> + Send + Sync>;

struct Gathered {
    rows: Vec<RowIn>,
    map: TempoMap,
    tempo_key: String,
    stems: HashMap<String, String>,
    /// The final file name of every set.
    names: HashMap<Key, String>,
}

impl Prep {
    #[must_use]
    pub fn new(
        page: Arc<PageHandle>,
        rows: Arc<dyn Fn() -> Rows + Send + Sync>,
        tempo: Arc<dyn Fn() -> Answer + Send + Sync>,
        files: FilesFn,
    ) -> Arc<Prep> {
        Arc::new(Prep {
            page,
            rows,
            tempo,
            files,
            st: Mutex::new(St::default()),
            cv: Condvar::new(),
            ended: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            job_open: std::sync::atomic::AtomicBool::new(false),
            gate: Mutex::new(None),
            exp: ExportProgress::default(),
        })
    }

    /// M1d4: the plugin tells which rows are not done (the states of the first open).
    pub fn set_gate(&self, g: GateFn) {
        *self.gate.lock().unwrap_or_else(|e| e.into_inner()) = Some(g);
    }

    /// The rows that are not done, `(id, in_progress)`; none under the fault run `handle-early`.
    fn blocked(&self) -> Vec<(String, bool)> {
        if crate::test_switch("DS_ARA_FIRSTOPEN_FAULT").as_deref() == Ok("handle-early") {
            return Vec::new();
        }
        let g = self.gate.lock().unwrap_or_else(|e| e.into_inner()).clone();
        g.map(|f| f()).unwrap_or_default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, St> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Something changed (an edit, a round, a row, the tempo): the worker looks again after the quiet time.
    #[track_caller]
    pub fn wake(self: &Arc<Self>) {
        if crate::test_switch("DS_ARA_PREPARE_TRACE").is_ok() {
            let at = std::panic::Location::caller();
            crate::session().log(
                json!({"event": "prepare_wake", "at": format!("{}:{}", at.file(), at.line()), "ms": crate::mono_ms()})
                    .to_string(),
            );
        }
        {
            let mut st = self.lock();
            if stopped() && live_threads() == 0 {
                STOP.store(false, std::sync::atomic::Ordering::SeqCst);
            }
            let now = Instant::now();
            st.first.get_or_insert(now);
            st.last = Some(now);
            st.bump += u64::from(has("all-rows"));
            st.midi_pending = true;
            st.wav_pending = true;
            st.wake_seq += 1;
            if !st.started {
                st.started = true;
                let weak = Arc::downgrade(self);
                let ended = Arc::clone(&self.ended);
                LIVE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let spawned = std::thread::Builder::new()
                    .name("drumspike-ara-prepare".into())
                    .spawn(move || {
                        let _live = LiveGuard(ended);
                        worker(&weak);
                    });
                if spawned.is_err() {
                    LIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }
        self.cv.notify_all();
    }

    /// The page asks for the state: the worker starts once (a poll of the page changes nothing and wakes nothing).
    pub fn ensure_started(self: &Arc<Self>) {
        if !self.lock().started {
            self.wake();
        }
    }

    /// The page's answer to a command, as `changed_after` sees it: a command that writes the project wakes the worker; a poll of the job status that
    /// says `done` wakes it ONCE for the job (the page polls every 2 s: a wake for each answer would restart the quiet time for ever).
    #[track_caller]
    pub fn command_done(self: &Arc<Self>, writes: bool, ended: bool) {
        use std::sync::atomic::Ordering::SeqCst;
        if writes {
            self.job_open.store(true, SeqCst);
            self.wake();
        } else if ended && self.job_open.swap(false, SeqCst) {
            self.wake();
        }
    }

    /// Due at once: the next pass looks and works with no quiet time.
    fn kick(&self) {
        let mut st = self.lock();
        let ago = Instant::now().checked_sub(Duration::from_secs(60));
        st.midi_pending = true;
        st.wav_pending = true;
        st.first = ago;
        st.last = ago;
    }

    /// A press on a handle: the first use of a wav kind (it is kept current from now on), or a new try of a set that failed (a press on an error).
    /// `row` names one row; `None`: every row of the kind. The work is due at once.
    pub fn want_kind(self: &Arc<Self>, kind: Kind, row: Option<&str>) {
        {
            let mut st = self.lock();
            match kind {
                Kind::Spike => st.used[0] = true,
                Kind::Audio => st.used[1] = true,
                _ => {}
            }
            st.failed.retain(|k, _| !(k.1 == kind && row.is_none_or(|r| r == k.0)));
        }
        self.wake();
        self.kick();
    }

    /// The rows of the document (the scenario `late-row` shows one of them late).
    fn rows_now(&self) -> Rows {
        let mut r = (self.rows)();
        if has("late-row") && !self.lock().revealed {
            r.rows.retain(|x| x.name != "Snare_audio");
        }
        r
    }

    /// True when the tempo of the host is another than at the last look: the MIDI files depend on it, so the worker looks again (the process
    /// call of the host gives it with no event of its own).
    fn tempo_changed(&self) -> bool {
        let key = format!("{:?}", (self.tempo)().map);
        let mut st = self.lock();
        let changed = st.tempo_seen.as_ref().is_some_and(|k| *k != key);
        st.tempo_seen = Some(key);
        changed
    }

    fn exports_dir(&self) -> Option<PathBuf> {
        let svc = self.page.service()?;
        let sid = self.page.sid()?;
        svc.drag_dir(&sid).ok().and_then(|d| d.parent().map(Path::to_path_buf))
    }

    fn gather(&self, refresh: bool) -> Option<Gathered> {
        let svc = self.page.service()?;
        let sid = self.page.sid()?;
        let rows = self.rows_now();
        let tempo = (self.tempo)();
        let files = (self.files)();
        // the same follower refresh as the export (R4): `tracks_get` derives a follower whose hits went stale; it also gives the notes
        let tracks = if refresh && !has("no-refresh") {
            svc.tracks_get(&sid).ok()
        } else {
            None
        };
        let mut st_notes = HashMap::new();
        let mut out = Vec::new();
        for r in &rows.rows {
            let mut srcs = Vec::new();
            for key in &r.srcs {
                let ev = svc.events_get(&sid, key).ok();
                let events = ev
                    .as_ref()
                    .and_then(|v| v["events"].as_array())
                    .map(|a| uncut(a))
                    .unwrap_or_default();
                let sr = ev.as_ref().and_then(|v| v["sr"].as_u64()).unwrap_or(48_000) as u32;
                let (wav, rate) = files.get(key).map_or((None, sr), |(p, r)| (Some(p.clone()), *r));
                srcs.push(SrcIn {
                    key: key.clone(),
                    events,
                    rate,
                    wav,
                });
            }
            let home = r.srcs.first().cloned().unwrap_or_default();
            let note = tracks
                .as_ref()
                .and_then(|t| t["tracks"][&home]["note"].as_i64())
                .or_else(|| self.lock().notes.get(&home).copied())
                .unwrap_or(36);
            st_notes.insert(home, note);
            out.push(RowIn {
                id: r.id.clone(),
                name: r.name.clone(),
                clips: r.clips.clone(),
                srcs,
                note,
            });
        }
        self.lock().notes.extend(st_notes);
        if has("per-source") {
            // the fault run: one file per source, as the export of the dialog does
            out = out
                .into_iter()
                .flat_map(|r| {
                    r.srcs
                        .iter()
                        .map(|s| RowIn {
                            id: format!("{}-{}", r.id, s.key),
                            name: format!("{}-{}", r.name, s.key),
                            clips: r.clips.iter().filter(|c| c.src == s.key).cloned().collect(),
                            srcs: vec![s.clone()],
                            note: r.note,
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
        }
        let stems = stems(&out);
        let names = final_names(&out, &stems);
        let map = tempo.map.clone().unwrap_or_default();
        let tempo_key = format!("{:?}", tempo.map);
        Some(Gathered {
            rows: out,
            map,
            tempo_key,
            stems,
            names,
        })
    }

    fn fps(&self, g: &Gathered) -> HashMap<Key, u64> {
        let (used, bump) = {
            let st = self.lock();
            (st.used, st.bump)
        };
        let mut out = HashMap::new();
        let mut merged = std::collections::hash_map::DefaultHasher::new();
        let mut any = false;
        for r in &g.rows {
            if mapped_events(r).is_empty() {
                continue;
            }
            any = true;
            // the final file name is part of `fp` (a renamed set is stale and made again); the fault run leaves it out
            let name = |k: Kind| {
                if has("fp-no-name") {
                    return String::new();
                }
                g.names.get(&(r.id.clone(), k)).cloned().unwrap_or_default()
            };
            let m = fingerprint(r, Kind::Midi, &name(Kind::Midi), &g.tempo_key, bump);
            m.hash(&mut merged);
            out.insert((r.id.clone(), Kind::Midi), m);
            if used[0] {
                out.insert(
                    (r.id.clone(), Kind::Spike),
                    fingerprint(r, Kind::Spike, &name(Kind::Spike), "", bump),
                );
            }
            if used[1] {
                out.insert(
                    (r.id.clone(), Kind::Audio),
                    fingerprint(r, Kind::Audio, &name(Kind::Audio), "", bump),
                );
            }
        }
        if any {
            g.names.get(&(String::new(), Kind::Merged)).hash(&mut merged);
            out.insert((String::new(), Kind::Merged), merged.finish());
        }
        out
    }

    fn state_for(&self, st: &St, key: &Key, has_hits: bool) -> State {
        let used = match key.1 {
            Kind::Spike => st.used[0],
            Kind::Audio => st.used[1],
            _ => true,
        };
        if has_hits && key.1.is_wav() && !used {
            return State::Off;
        }
        let want = if has_hits { st.want.get(key).copied() } else { None };
        let dirty = st.wake_seq != st.clean_seq;
        // before the first cycle, and while something changed since the last one, nothing is known: a row has hits (a row that gets its first
        // hit shows preparing, not "no hits") until a pass says it has none
        let want = if has_hits && want.is_none() && (st.cycles == 0 || dirty) {
            Some(0)
        } else {
            want
        };
        let ready = st.ready.get(key).map(|r| (r.fp, r.prepared.len()));
        let failed = st.failed.get(key).filter(|f| Some(f.0) == want).map(|f| f.1.as_str());
        let mut pending = st.pending_since.get(key).map(|t| t.elapsed());
        if let Some((bk, since)) = &st.busy {
            if bk == key {
                pending = Some(since.elapsed());
            }
        }
        let mut state = state_of(key.1, used, want, ready, failed, pending, limit());
        if st.no_folder && matches!(state, State::Ready(_) | State::Preparing) {
            state = State::Error("no export folder".into());
        }
        // something changed since the last pass looked: a set that is ready for the old state is not ready for the new one
        if st.wake_seq != st.clean_seq && matches!(state, State::Ready(_)) {
            return State::Preparing;
        }
        state
    }

    /// The answer of `plugin_deliver_state` and the payload of the page event `plugin-deliver`.
    #[must_use]
    pub fn state_json(&self) -> Value {
        let rows = self.rows_now();
        let blocked = self.blocked();
        let st = self.lock();
        let dirty = st.wake_seq != st.clean_seq;
        let row_has = |id: &str| st.cycles == 0 || dirty || st.want.contains_key(&(id.to_string(), Kind::Midi));
        let items: Vec<Value> = rows
            .rows
            .iter()
            .map(|r| {
                let has = row_has(&r.id);
                let k = |kind: Kind| self.state_for(&st, &(r.id.clone(), kind), has).to_json();
                // M1d4: a row that is not done has no ready handle (in progress: preparing; failed or offline: none)
                if let Some((_, busy)) = blocked.iter().find(|(id, _)| *id == r.id || id == "*") {
                    let s = if *busy {
                        json!({"state": "preparing", "n": 0, "why": ""})
                    } else {
                        json!({"state": "none", "n": 0, "why": "no hits"})
                    };
                    return json!({"id": r.id, "name": r.name, "srcs": r.srcs, "midi": s, "spike": s, "audio": s});
                }
                json!({"id": r.id, "name": r.name, "srcs": r.srcs, "midi": k(Kind::Midi), "spike": k(Kind::Spike), "audio": k(Kind::Audio)})
            })
            .collect();
        let any = rows.rows.iter().any(|r| row_has(&r.id));
        let mut merged = self.state_for(&st, &(String::new(), Kind::Merged), any).to_json();
        if blocked.iter().any(|(_, busy)| *busy) {
            merged = json!({"state": "preparing", "n": 0, "why": ""});
        }
        json!({"gen": st.gen, "rows": items, "merged": merged, "counters": {"cycles": st.cycles, "jobs": st.jobs},
               "used": {"spike": st.used[0], "audio": st.used[1]},
               "hint": crate::deliver::DROP_HINT})
    }

    fn push(&self) {
        let v = self.state_json();
        let mut st = self.lock();
        let text = {
            let mut c = v.clone();
            c["gen"] = Value::Null;
            c["counters"] = Value::Null;
            c.to_string()
        };
        if st.pushed == text {
            return;
        }
        st.pushed = text;
        st.gen += 1;
        let mut v = v;
        v["gen"] = json!(st.gen);
        drop(st);
        self.page.log.push("plugin-deliver", v);
    }

    /// The files of the items, when every one of them is ready for the CURRENT state of the events: each prepared file is hard-linked into
    /// `Exports/Dragged/` under its final name (the plugin-owned folder; nothing there is ever replaced or deleted) and the paths there are
    /// answered. A set whose file is gone from `.prepared/` is stale. A failed set is tried again.
    ///
    /// # Errors
    ///
    /// `(status, text)`: 409 with the state of the first item that is not ready, 413 over the bound of a drag, 500 when a link fails.
    pub fn take(self: &Arc<Self>, items: &[(String, Kind)]) -> Result<Vec<PathBuf>, (u16, String)> {
        let Some(g) = self.gather(true) else {
            return Err((503, "the document is closed".into()));
        };
        let Some(exports) = self.exports_dir() else {
            return Err((409, "error: no export folder".into()));
        };
        let fps = self.fps(&g);
        let picks: Vec<PathBuf> = self
            .ready_files(&g, &fps, items, true)?
            .into_iter()
            .map(|(_, p)| p)
            .collect();
        if picks.len() > MAX_DRAG_FILES {
            return Err((413, format!("error: more than {MAX_DRAG_FILES} items in one drag")));
        }
        if has("late-row") && items.iter().any(|(_, k)| k.is_wav()) {
            self.lock().revealed = true;
            self.wake();
        }
        let dragged = exports.join(DRAGGED_DIR);
        std::fs::create_dir_all(&dragged).map_err(|e| (500, format!("error: {}: {e}", dragged.display())))?;
        picks
            .iter()
            .map(|p| deliver(p, &dragged).map_err(|e| (500, format!("error: {e}"))))
            .collect()
    }

    /// The prepared file of every item, when each is ready for the CURRENT state of the events (the rule of a take and of the export of the dialog:
    /// one place). Not ready: 409 with the name of the DAW track and the state; a failed set is tried again.
    fn ready_files(
        self: &Arc<Self>,
        g: &Gathered,
        fps: &HashMap<Key, u64>,
        items: &[(String, Kind)],
        wake_idle: bool,
    ) -> Result<Vec<(Key, PathBuf)>, (u16, String)> {
        // M1d4: a row that is not done is never taken (its handle is not ready)
        let blocked = self.blocked();
        if items.iter().any(|(id, _)| {
            blocked.iter().any(|(b, _)| b == id || b == "*") || (id.is_empty() && blocked.iter().any(|(_, busy)| *busy))
        }) {
            return Err((409, "preparing: a track is not ready yet".into()));
        }
        let mut picks: Vec<(Key, PathBuf)> = Vec::new();
        let mut st = self.lock();
        for (id, kind) in items {
            let key = (id.clone(), *kind);
            let files_ok = st
                .ready
                .get(&key)
                .is_some_and(|r| r.prepared.iter().all(|f| f.is_file()));
            if st.ready.contains_key(&key) && !files_ok {
                st.ready.remove(&key); // the file is gone: the set is stale, it is made again
            }
            let ok = match (fps.get(&key), st.ready.get(&key)) {
                (Some(want), Some(r)) => *want == r.fp || has("stale-ok"),
                _ => false,
            };
            if !ok {
                let why = match self.state_for_fps(&st, &key, fps) {
                    State::Error(w) => format!("error: {w} (trying again)"),
                    State::Off => "not prepared yet".into(),
                    State::None => "no hits".into(),
                    _ => "preparing".into(),
                };
                let was_error = st.failed.remove(&key).is_some();
                // a take wakes the worker; the export that waits does it once, at its start: a set that is only `preparing` needs no look
                let wake = wake_idle
                    || was_error
                    || matches!(self.state_for_fps(&st, &key, fps), State::Off | State::Error(_));
                drop(st);
                if wake {
                    self.wake();
                }
                if was_error {
                    self.kick(); // a press on a failed set is a new try, at once
                }
                return Err((409, format!("{}: {why}", self.name_of(g, id))));
            }
            if let Some(r) = st.ready.get(&key) {
                picks.extend(r.prepared.iter().map(|p| (key.clone(), p.clone())));
            }
        }
        Ok(picks)
    }

    /// Check the items that the page names (M1c4, R10): a row id only from the live rows, a kind that exists, the merged file with no row, nothing else in
    /// an item (no name, no path). The rows with no hit are not an error here: they have no set.
    fn check_items(&self, g: &Gathered, items: &[Value]) -> Result<Vec<(String, Kind)>, (u16, String)> {
        if items.is_empty() || items.len() > 2048 {
            return Err((400, "items must list 1 to 2048 entries".into()));
        }
        let mut out = Vec::new();
        for it in items {
            let Some(o) = it.as_object() else {
                return Err((400, "an item is {row, kind}".into()));
            };
            if o.keys().any(|k| k != "row" && k != "kind") && !has("trust-name") {
                return Err((400, "an item has only a row and a kind".into()));
            }
            let kind = o.get("kind").and_then(Value::as_str).and_then(Kind::parse);
            let row = o.get("row").and_then(Value::as_str).unwrap_or("");
            let Some(kind) = kind else {
                return Err((400, "an item needs a kind: midi, spike, audio or merged".into()));
            };
            let live = g.rows.iter().any(|r| r.id == row);
            if (kind == Kind::Merged) != row.is_empty() || (!row.is_empty() && !live && !has("any-row")) {
                return Err((400, "unknown DAW track".into()));
            }
            if !out.contains(&(row.to_string(), kind)) {
                out.push((row.to_string(), kind)); // the same row and kind twice is one file
            }
        }
        Ok(out)
    }

    /// The names that an export of `items` would write (see [`ExportName`]): `(row id, kind, file name, DAW track)` in the order of the items. A row with no hit has none.
    ///
    /// # Errors
    ///
    /// `(status, text)`: 400 for an item that is not a live row and a kind, 503 when the document is closed.
    pub fn export_names(&self, items: &[Value]) -> Result<Vec<ExportName>, (u16, String)> {
        let Some(g) = self.gather(false) else {
            return Err((503, "the document is closed".into()));
        };
        let list = self.check_items(&g, items)?;
        let fps = self.fps(&g);
        Ok(list
            .into_iter()
            .filter(|k| fps.contains_key(k))
            .filter_map(|k| {
                let name = g.names.get(&k)?.clone();
                let track = self.name_of(&g, &k.0);
                Some((k.0, k.1, name, track))
            })
            .collect())
    }

    /// The export of the dialog in a DAW (M1c4, #589): the prepared files of `items`, copied to `dest` under their names. All or nothing: a set that is not
    /// ready for the CURRENT hits answers 409 and nothing is written; `start` is the first call of one export (it starts the wav kinds and wakes the worker
    /// once). Every file is copied to a temporary name first (the cancel is read until the first rename), the hits are checked again, then each file is
    /// renamed over the old file of its name. One export runs at a time: a second call answers 409 and changes nothing of the first. A guard removes the
    /// temporary files on an error, a cancel and a panic. `protected` are the files that no export may replace (`Service::protected_files_everywhere`).
    ///
    /// # Errors
    ///
    /// [`ExportError`]: 400 for a bad item, 409 for a set not ready (the text starts with the DAW track and `preparing`), an export that runs, a folder, a
    /// read-only or protected file in the way, or a cancel; 500 for a failed copy (it names the DAW track) or a rename that failed after others were done
    /// (the error then lists the files written and the DAW tracks not written).
    pub fn export_to(
        self: &Arc<Self>,
        items: &[Value],
        dest: &Path,
        protected: &[PathBuf],
        start: bool,
    ) -> Result<Value, ExportError> {
        self.export_to_with(items, dest, protected, start, &ExportHooks::real())
    }

    /// [`Prep::export_to`] with the file operations given (the tests make a rename or a copy fail, or change the hits between the copy and the check).
    ///
    /// # Errors
    ///
    /// As [`Prep::export_to`].
    pub fn export_to_with(
        self: &Arc<Self>,
        items: &[Value],
        dest: &Path,
        protected: &[PathBuf],
        start: bool,
        hooks: &ExportHooks,
    ) -> Result<Value, ExportError> {
        use std::sync::atomic::Ordering::SeqCst;
        // one export at a time: the flags of a running export are never touched by another call
        if self.exp.running.compare_exchange(false, true, SeqCst, SeqCst).is_err() {
            return Err((409, "an export is running".to_string()).into());
        }
        let _running = BusyGuard(&self.exp.running);
        let Some(g) = self.gather(true) else {
            return Err((503, "the document is closed".to_string()).into());
        };
        let list = self.check_items(&g, items)?;
        if start {
            for kind in [Kind::Spike, Kind::Audio] {
                if list.iter().any(|(_, k)| *k == kind) {
                    self.want_kind(kind, None);
                }
            }
            self.wake(); // once for this export; a poll that waits does not wake the worker again
        }
        let Some(exports) = self.exports_dir() else {
            return Err((409, "error: no export folder".to_string()).into());
        };
        let fps = self.fps(&g);
        let picks = self.ready_files(&g, &fps, &list, false)?;
        let prepared_dir = exports.join(PREPARED_DIR);
        let dest_c = std::fs::canonicalize(dest).map_err(|e| (409, format!("{}: {e}", dest.display())))?;
        for inside in [prepared_dir.clone(), exports.join(DRAGGED_DIR)] {
            if std::fs::canonicalize(&inside).is_ok_and(|c| c == dest_c) {
                return Err((
                    409,
                    "the folder of the prepared files is not an export folder".to_string(),
                )
                    .into());
            }
        }
        // the names: the producer's, one safe component each
        let mut plan: Vec<(Key, PathBuf, PathBuf)> = Vec::new();
        for (i, (key, src)) in picks.into_iter().enumerate() {
            let mut name = src.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            if has("trust-name") {
                // the fault run: a name that the page sends is used as it is
                if let Some(n) = items.get(i).and_then(|it| it["name"].as_str()) {
                    name = n.to_string();
                }
            }
            let ok =
                !name.is_empty() && Path::new(&name).components().count() == 1 && !name.contains('/') && name != "..";
            if !ok && !has("trust-name") {
                return Err((400, "the file name is not allowed".to_string()).into());
            }
            plan.push((key, src, dest_c.join(&name)));
        }
        // everything that can be checked for ALL targets is checked before the first file is written, so that a failure later is rare
        let outs: Vec<PathBuf> = plan.iter().map(|(_, _, o)| o.clone()).collect();
        drumspike_app::export::check_export_plan(&outs, protected).map_err(|e| (409, e.message))?;
        for (key, _, out) in &plan {
            if let Ok(m) = std::fs::symlink_metadata(out) {
                if m.is_dir() {
                    return Err((
                        409,
                        format!("{} is a folder: an export never replaces a folder", out.display()),
                    )
                        .into());
                }
                if m.permissions().readonly() {
                    return Err((
                        409,
                        format!(
                            "{}: {} is read-only: an export does not replace it",
                            self.name_of(&g, &key.0),
                            out.display()
                        ),
                    )
                        .into());
                }
            }
        }
        let replaced = plan.iter().filter(|(_, _, o)| replaces(o)).count();
        self.exp.cancel.store(false, SeqCst);
        self.exp.done.store(0, SeqCst);
        self.exp.total.store(plan.len(), SeqCst);
        self.exp.busy.store(true, SeqCst);
        let _busy = BusyGuard(&self.exp.busy);
        let _copying = CopyGuard::new();
        clean_old_temps(&dest_c, COPYING.load(SeqCst) == 1);
        let mut temps = Temps(Vec::new());
        let mut tmp_of: Vec<PathBuf> = Vec::new();
        for (key, src, out) in &plan {
            let track = self.name_of(&g, &key.0);
            *self.exp.track.lock().unwrap_or_else(|e| e.into_inner()) = track.clone();
            let name = out.file_name().and_then(|n| n.to_str()).unwrap_or("file");
            let tmp = dest_c.join(temp_name(name));
            temps.0.push(tmp.clone());
            tmp_of.push(tmp.clone());
            match (hooks.copy)(src, &tmp, &self.exp.cancel) {
                Ok(()) => {}
                Err(CopyErr::Cancelled) => return Err((409, "cancelled: nothing was written".to_string()).into()),
                Err(CopyErr::Missing) => {
                    // the worker made the set again under another name (a renamed DAW track, for example): it is not ready, the page waits
                    self.lock().ready.remove(key);
                    self.wake();
                    return Err((409, format!("{track}: preparing")).into());
                }
                Err(CopyErr::Other(e)) => return Err((500, format!("{track}: could not copy the file: {e}")).into()),
            }
            self.exp.done.fetch_add(1, SeqCst);
        }
        (hooks.after_copy)();
        // the hits may have changed during the copy: a set that is not ready for the hits of now is not delivered
        if !has("no-late-check") {
            let g2 = self.gather(true);
            let fresh = g2.as_ref().is_some_and(|g2| {
                let f2 = self.fps(g2);
                let st = self.lock();
                list.iter().all(|k| {
                    f2.get(k)
                        .is_some_and(|w| st.ready.get(k).is_some_and(|r| r.fp == *w && r.fp == fps[k]))
                })
            });
            if !fresh {
                self.wake();
                return Err((409, "preparing: the hits changed during the export".to_string()).into());
            }
        }
        if self.exp.cancel.load(SeqCst) {
            return Err((409, "cancelled: nothing was written".to_string()).into());
        }
        let mut written: Vec<String> = Vec::new();
        for (i, (key, _, out)) in plan.iter().enumerate() {
            if let Err(e) = (hooks.rename)(&tmp_of[i], out) {
                let not_written: Vec<String> = plan[i..].iter().map(|(k, _, _)| self.name_of(&g, &k.0)).collect();
                let total = plan.len();
                let text = format!(
                    "{} of {total} files were written: {}. Not written: {}. Press Export again. ({}: could not write the file: {e})",
                    written.len(),
                    if written.is_empty() { "none".to_string() } else { written.join(", ") },
                    not_written.join(", "),
                    self.name_of(&g, &key.0)
                );
                return Err(ExportError {
                    code: 500,
                    text,
                    written,
                    not_written,
                });
            }
            written.push(out.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string());
        }
        let files: Vec<PathBuf> = plan.iter().map(|(_, _, o)| o.clone()).collect();
        let count = |k: Kind| plan.iter().filter(|(key, _, _)| key.1 == k).count();
        let (mut no_region, mut before_start) = (0, 0);
        for r in g
            .rows
            .iter()
            .filter(|r| list.iter().any(|(id, _)| *id == r.id) || list.iter().any(|(_, k)| *k == Kind::Merged))
        {
            let (a, b) = left_out(r);
            no_region += a;
            before_start += b;
        }
        Ok(
            json!({"files": files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>(),
                  "dest": dest_c.display().to_string(),
                  "midi": count(Kind::Midi), "spikes": count(Kind::Spike), "audio": count(Kind::Audio),
                  "merged": count(Kind::Merged), "replaced": replaced,
                  "outside_song": {"no_region": no_region, "before_start": before_start}}),
        )
    }

    /// The progress of the export of the dialog; `cancel` asks it to stop (it stops before the first rename).
    #[must_use]
    pub fn export_progress(&self, cancel: bool) -> Value {
        use std::sync::atomic::Ordering::SeqCst;
        if cancel {
            self.exp.cancel.store(true, SeqCst);
        }
        json!({"busy": self.exp.busy.load(SeqCst), "done": self.exp.done.load(SeqCst), "total": self.exp.total.load(SeqCst),
               "track": self.exp.track.lock().unwrap_or_else(|e| e.into_inner()).clone()})
    }

    fn name_of(&self, g: &Gathered, id: &str) -> String {
        if id.is_empty() {
            return "Merged MIDI".into();
        }
        g.rows
            .iter()
            .find(|r| r.id == id)
            .map_or(id.to_string(), |r| r.name.clone())
    }

    fn state_for_fps(&self, st: &St, key: &Key, fps: &HashMap<Key, u64>) -> State {
        let used = match key.1 {
            Kind::Spike => st.used[0],
            Kind::Audio => st.used[1],
            _ => true,
        };
        let has_hits =
            fps.keys().any(|k| k.0 == key.0 && k.1 == Kind::Midi) || key.1 == Kind::Merged && !fps.is_empty();
        if has_hits && key.1.is_wav() && !used {
            return State::Off;
        }
        let want = fps.get(key).copied();
        let ready = st.ready.get(key).map(|r| (r.fp, r.prepared.len()));
        let failed = st.failed.get(key).filter(|f| Some(f.0) == want).map(|f| f.1.as_str());
        state_of(key.1, used, want, ready, failed, None, limit())
    }

    /// Counters for the check: (cycles, jobs).
    #[must_use]
    pub fn counters(&self) -> (u64, u64) {
        let st = self.lock();
        (st.cycles, st.jobs)
    }

    /// A wake that changed nothing (the same hits, clips, tempo and names) must not leave the handles on `preparing` for the quiet time: one look
    /// with no job; when the wanted state is the one of the last pass the handles are clean again.
    fn settle_dirty(self: &Arc<Self>) {
        let seen = {
            let st = self.lock();
            if st.wake_seq == st.clean_seq || st.cycles == 0 {
                return;
            }
            st.wake_seq
        };
        let Some(g) = self.gather(true) else { return };
        let fps = self.fps(&g);
        {
            let mut st = self.lock();
            if st.want != fps {
                return;
            }
            st.clean_seq = seen;
        }
        self.push();
    }

    // one pass: look at the rows, set the wanted fps, make what is due
    fn cycle(self: &Arc<Self>, midi_due: bool, wav_due: bool) {
        let seen = self.lock().wake_seq;
        let Some(g) = self.gather(true) else { return };
        let fps = self.fps(&g);
        let Some(exports) = self.exports_dir() else {
            self.lock().no_folder = true;
            self.push();
            return;
        };
        let prepared = exports.join(PREPARED_DIR);
        {
            let mut st = self.lock();
            st.no_folder = false;
            st.cycles += 1;
            st.clean_seq = seen;
            st.meta = g
                .rows
                .iter()
                .map(|r| {
                    (
                        r.id.clone(),
                        r.name.clone(),
                        r.srcs.iter().map(|s| s.key.clone()).collect(),
                    )
                })
                .collect();
            update_books(&mut st, &fps);
        }
        self.push();
        let keep: HashSet<PathBuf> = {
            let st = self.lock();
            st.ready.values().flat_map(|r| r.prepared.iter().cloned()).collect()
        };
        if has("user-file") {
            plant_user_file(&exports, &g);
        }
        clean_prepared(&prepared, &keep);
        let mut order: Vec<Key> = Vec::new();
        for kind in [Kind::Midi, Kind::Merged, Kind::Spike, Kind::Audio] {
            for r in std::iter::once(None).chain(g.rows.iter().map(Some)) {
                let key = (r.map_or(String::new(), |r| r.id.clone()), kind);
                if (kind == Kind::Merged) == r.is_none() && fps.contains_key(&key) {
                    order.push(key);
                }
            }
        }
        for key in order {
            let due = if key.1.is_wav() { wav_due } else { midi_due };
            let need = {
                let st = self.lock();
                let fp = fps[&key];
                !st.ready.get(&key).is_some_and(|r| r.fp == fp)
                    && !st.failed.get(&key).is_some_and(|f| failed_blocks(f, fp))
            };
            if !due || !need {
                continue;
            }
            if stopped() {
                return;
            }
            self.job(&g, &key, fps[&key], &exports, &prepared);
        }
        self.push();
    }

    fn job(self: &Arc<Self>, g: &Gathered, key: &Key, fp: u64, exports: &Path, prepared: &Path) {
        {
            let mut st = self.lock();
            // the file name of this set is the file of another set: a fresh one is never overwritten (this job fails and is tried again); a stale one
            // is no longer ready, because its file is about to change (a take must not deliver the content of this set under that name)
            if let Some(out) = g.names.get(key).map(|n| prepared.join(n)) {
                if !has("no-held-check") {
                    match held_by_other(&st, key, &out) {
                        Held::Fresh => {
                            let tries = st.failed.get(key).filter(|f| f.0 == fp).map_or(1, |f| f.3 + 1);
                            st.failed.insert(
                                key.clone(),
                                (fp, "the file name is held by another set".into(), Instant::now(), tries),
                            );
                            drop(st);
                            self.push();
                            return;
                        }
                        Held::Stale(k) => {
                            st.ready.remove(&k);
                        }
                        Held::No => {}
                    }
                }
            }
            st.busy = Some((key.clone(), Instant::now()));
            st.jobs += 1;
        }
        self.push();
        if has("forever") {
            // the fault run: the job never ends; the state must say so after the watchdog
            while Arc::strong_count(self) > 1 && !stopped() {
                std::thread::sleep(Duration::from_millis(100));
            }
            return;
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| make(g, key, exports, prepared)))
            .unwrap_or_else(|_| Err("the preparation panicked".into()));
        let mut st = self.lock();
        st.busy = None;
        match r {
            Ok(prep) => {
                st.failed.remove(key);
                st.pending_since.remove(key);
                st.ready.insert(key.clone(), Ready { fp, prepared: prep });
            }
            Err(why) => {
                let tries = st.failed.get(key).filter(|f| f.0 == fp).map_or(1, |f| f.3 + 1);
                st.failed.insert(key.clone(), (fp, why, Instant::now(), tries));
                st.pending_since.remove(key); // the failure is the state now; the watchdog counts from the next change
            }
        }
        drop(st);
        self.push();
    }
}

struct BusyGuard<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Copy `src` to `dst` in pieces of 1 MiB, looking at the cancel flag and the stop sign between them. `dst` is created new (an existing file of that name is never
/// followed or cut); a failed or cancelled copy removes the `dst` it made.
fn copy_file(src: &Path, dst: &Path, cancel: &std::sync::atomic::AtomicBool) -> Result<(), CopyErr> {
    use std::io::{Read, Write};
    let mut a = std::fs::File::open(src).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CopyErr::Missing
        } else {
            CopyErr::Other(format!("{}: {e}", src.display()))
        }
    })?;
    let mut b = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)
        .map_err(|e| CopyErr::Other(format!("{}: {e}", dst.display())))?;
    let r = (|| -> Result<(), CopyErr> {
        let mut buf = vec![0u8; 1 << 20];
        loop {
            if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(CopyErr::Cancelled);
            }
            checkpoint().map_err(CopyErr::Other)?;
            let n = a.read(&mut buf).map_err(|e| CopyErr::Other(e.to_string()))?;
            if n == 0 {
                break;
            }
            b.write_all(&buf[..n]).map_err(|e| CopyErr::Other(e.to_string()))?;
        }
        b.sync_all().map_err(|e| CopyErr::Other(e.to_string()))
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(dst);
    }
    r
}

/// The hits of the row that the song map leaves out: (in no clip, before song 0). The same `to_song` as the files.
#[must_use]
pub fn left_out(row: &RowIn) -> (usize, usize) {
    row.srcs.iter().fold((0, 0), |(a, b), s| {
        let regions = regions_of(row, &s.key);
        if regions.is_empty() {
            return (a, b);
        }
        let c = to_song(&s.events, &regions).1;
        (a + c.no_region, b + c.before_start)
    })
}

/// True when a new file of `size` bytes would take `.prepared/` (holding `others` bytes besides the file it replaces) over [`CAP_BYTES`].
#[must_use]
pub fn over_cap(others: u64, size: u64) -> bool {
    others.saturating_add(size) > CAP_BYTES
}

fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

/// Remove from `.prepared/` every file that no ready set holds (an old set, a temporary file, a set of a row that is gone).
fn clean_prepared(prepared: &Path, keep: &HashSet<PathBuf>) {
    for e in std::fs::read_dir(prepared).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_file() && !keep.contains(&p) {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// Who holds the prepared path `out` besides `key`: a ready set that is still the wanted state (`Fresh`), one that is stale (`Stale`), or nobody.
enum Held {
    Fresh,
    Stale(Key),
    No,
}

fn held_by_other(st: &St, key: &Key, out: &Path) -> Held {
    for (k, r) in &st.ready {
        if k != key && r.prepared.iter().any(|p| p == out) {
            return if st.want.get(k) == Some(&r.fp) {
                Held::Fresh
            } else {
                Held::Stale(k.clone())
            };
        }
    }
    Held::No
}

/// The books after a pass looked at the rows: `want` is what the files must be made from now. A change of the wanted state starts the watchdog again
/// (S10); sets of rows that are gone or kinds that are off leave the books (their files are removed from `.prepared/` by the pass).
fn update_books(st: &mut St, fps: &HashMap<Key, u64>) {
    for (k, fp) in fps {
        let fresh = st.ready.get(k).is_some_and(|r| r.fp == *fp);
        if fresh {
            st.pending_since.remove(k);
        } else if st.want.get(k) != Some(fp) {
            st.pending_since.insert(k.clone(), Instant::now());
        } else {
            st.pending_since.entry(k.clone()).or_insert_with(Instant::now);
        }
    }
    st.want = fps.clone();
    st.pending_since.retain(|k, _| fps.contains_key(k));
    st.ready.retain(|k, _| fps.contains_key(k));
    st.failed.retain(|k, _| fps.contains_key(k));
}

/// Hard-link the prepared file `src` into `dir` under its name. The same file (the same inode) already there is reused; another file of that
/// name is never replaced: the new name is numbered (`Snare_spike 2.mid`). Nothing in `dir` is ever deleted. No copy fallback: a failed link
/// is an error.
///
/// # Errors
///
/// The text of the failure.
pub fn deliver(src: &Path, dir: &Path) -> Result<PathBuf, String> {
    deliver_with(src, dir, &|a, b| {
        if has("no-hard-link") {
            Err(std::io::Error::other("hard links are not allowed here"))
        } else {
            std::fs::hard_link(a, b)
        }
    })
}

/// True when `a` and `b` hold the same bytes (the size first).
fn same_bytes(a: &Path, b: &Path) -> bool {
    use std::io::Read;
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false;
    };
    if ma.len() != mb.len() {
        return false;
    }
    let (Ok(mut fa), Ok(mut fb)) = (std::fs::File::open(a), std::fs::File::open(b)) else {
        return false;
    };
    let (mut x, mut y) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let (Ok(n), Ok(m)) = (fa.read(&mut x), fb.read(&mut y)) else {
            return false;
        };
        if n != m || x[..n] != y[..m] {
            return false;
        }
        if n == 0 {
            return true;
        }
    }
}

/// [`deliver`] with the way of linking given. When the link fails (a volume without hard links, exFAT or a network disk), the file is COPIED to a
/// temporary name in `dir` and renamed; a name is still never replaced. The same file already there is the same inode or, when inodes cannot
/// match, the same bytes. An error only when the copy fails too.
///
/// # Errors
///
/// The text of the failure.
pub fn deliver_with(
    src: &Path,
    dir: &Path,
    link: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    use std::os::unix::fs::MetadataExt;
    let name = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "the prepared file has no name".to_string())?;
    let (stem, ext) = name.rsplit_once('.').map_or((name, ""), |(a, b)| (a, b));
    let sm = std::fs::metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    for n in 1..10_000u32 {
        let cand = match (n, ext.is_empty()) {
            (1, _) => name.to_string(),
            (_, true) => format!("{stem} {n}"),
            (_, false) => format!("{stem} {n}.{ext}"),
        };
        let p = dir.join(&cand);
        match std::fs::symlink_metadata(&p) {
            Ok(m) => {
                if (m.ino() == sm.ino() && m.dev() == sm.dev()) || same_bytes(src, &p) {
                    return Ok(std::fs::canonicalize(&p).unwrap_or(p));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if link(src, &p).is_ok() {
                    return Ok(std::fs::canonicalize(&p).unwrap_or(p));
                }
                let tmp = dir.join(format!(".{cand}.tmp"));
                let copied = std::fs::copy(src, &tmp).map_err(|e| format!("could not copy to {}: {e}", p.display()));
                if let Err(e) = copied {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(e);
                }
                std::fs::rename(&tmp, &p).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    format!("could not copy to {}: {e}", p.display())
                })?;
                return Ok(std::fs::canonicalize(&p).unwrap_or(p));
            }
            Err(e) => return Err(format!("{}: {e}", p.display())),
        }
    }
    Err("too many files of that name".into())
}

/// The scenario `user-file` of the check: a file of the user, with known bytes, lies in `Exports/` under the name of a row's MIDI file before the
/// first set is made. Nothing of the preparation may change it.
fn plant_user_file(exports: &Path, g: &Gathered) {
    for r in &g.rows {
        let name = format!("{}_spike.mid", r.name);
        let p = exports.join(&name);
        if !p.exists() {
            let _ = std::fs::write(&p, format!("USER FILE {name}").as_bytes());
        }
    }
}

/// Make the set of `key` in `.prepared/` and nowhere else: written under a temporary name, then renamed, so a file that was delivered (linked
/// into `Exports/Dragged/`) keeps its content. Returns the prepared paths.
fn make(g: &Gathered, key: &Key, exports: &Path, prepared: &Path) -> Result<Vec<PathBuf>, String> {
    checkpoint()?;
    std::fs::create_dir_all(prepared).map_err(|e| format!("{}: {e}", prepared.display()))?;
    let row = g.rows.iter().find(|r| r.id == key.0);
    #[cfg(test)]
    if row.is_some_and(|r| r.name.starts_with("SLOWJOB")) {
        // the test of the stop: a long job whose loop looks at the stop sign
        for _ in 0..1000 {
            checkpoint()?;
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let name = g.names.get(key).cloned().ok_or("the row is gone")?;
    // the name is one safe component (`file_stems`); nothing that is not inside the folder is ever written
    let out = prepared.join(&name);
    let tmp = prepared.join(format!("{name}.tmp"));
    if out.parent() != Some(prepared) || tmp.parent() != Some(prepared) {
        return Err("the name is not allowed".into());
    }
    let others = dir_size(prepared).saturating_sub(std::fs::metadata(&out).map_or(0, |m| m.len()));
    if let (true, Some(r)) = (key.1.is_wav(), row) {
        if over_cap(others, estimate_bytes(r)) {
            return Err("not enough room for the prepared files (limit 2 GiB)".into());
        }
    }
    match (key.1, row) {
        (Kind::Midi, Some(r)) => write_midi_row(r, &g.map, &tmp)?,
        (Kind::Spike, Some(r)) => write_spike_row(r, &tmp)?,
        (Kind::Audio, Some(r)) => write_audio_row(r, &tmp)?,
        (Kind::Merged, _) => write_merged(&g.rows, &g.map, &tmp)?,
        _ => return Err("the row is gone".into()),
    }
    let size = std::fs::metadata(&tmp).map_or(0, |m| m.len());
    if over_cap(others, size) {
        let _ = std::fs::remove_file(&tmp);
        return Err("not enough room for the prepared files (limit 2 GiB)".into());
    }
    std::fs::rename(&tmp, &out).map_err(|e| format!("{}: {e}", out.display()))?;
    if has("link-exports") {
        // the fault run: the old behaviour, a link in Exports/ under the final name that replaces a file of the user
        let fin = exports.join(&name);
        let _ = std::fs::remove_file(&fin);
        let _ = std::fs::hard_link(&out, &fin);
    }
    Ok(vec![out])
}

fn worker(weak: &Weak<Prep>) {
    loop {
        std::thread::sleep(Duration::from_millis(150));
        if stopped() {
            return;
        }
        let Some(p) = weak.upgrade() else { return };
        if p.tempo_changed() {
            p.wake();
        }
        {
            // a failed set is tried again after its back-off
            let mut st = p.lock();
            let due = st
                .failed
                .iter()
                .any(|(k, f)| st.want.get(k) == Some(&f.0) && f.2.elapsed() >= backoff(f.3));
            if due {
                st.midi_pending = true;
                st.wav_pending = true;
                let ago = Instant::now().checked_sub(Duration::from_secs(60));
                st.first = ago;
                st.last = ago;
            }
        }
        let need_push = {
            let mut st = p.lock();
            let need = st.wake_seq != st.pushed_seq;
            st.pushed_seq = st.wake_seq;
            need
        };
        if need_push {
            p.push();
        }
        let (midi, wav) = {
            let st = p.lock();
            let (Some(first), Some(last)) = (st.first, st.last) else {
                continue;
            };
            let now = Instant::now();
            let quiet = now.duration_since(last);
            let waited = now.duration_since(first);
            let m = st.midi_pending
                && (quiet >= Duration::from_millis(quiet_ms(false)) || waited >= Duration::from_secs(5));
            let w =
                st.wav_pending && (quiet >= Duration::from_millis(quiet_ms(true)) || waited >= Duration::from_secs(10));
            (m, w)
        };
        if p.page.job_running() {
            continue;
        }
        if !(midi || wav) {
            p.settle_dirty();
            continue;
        }
        {
            let mut st = p.lock();
            // cleared before the pass: a wake during it starts the next one
            if midi {
                st.midi_pending = false;
            }
            if wav {
                st.wav_pending = false;
            }
            if !st.midi_pending && !st.wav_pending {
                st.first = None;
                st.last = None;
            }
        }
        p.cycle(midi, wav);
    }
}

fn quiet_ms(wav: bool) -> u64 {
    if has("no-debounce") {
        return 0;
    }
    let base = crate::test_switch("DS_ARA_PREPARE_QUIET_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    match (base, wav) {
        (Some(b), false) => b,
        (Some(b), true) => b * 3,
        (None, false) => 800,
        (None, true) => 3000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(src: &str, a: f64, b: f64, s: f64, k: f64) -> Clip {
        Clip {
            src: src.into(),
            a,
            b,
            s,
            k,
        }
    }

    fn src(key: &str, ts: &[f64], rate: u32) -> SrcIn {
        SrcIn {
            key: key.into(),
            events: ts.iter().map(|t| json!({"t": t, "v": 90})).collect(),
            rate,
            wav: None,
        }
    }

    fn row(id: &str, name: &str, clips: Vec<Clip>, srcs: Vec<SrcIn>) -> RowIn {
        RowIn {
            id: id.into(),
            name: name.into(),
            clips,
            srcs,
            note: 38,
        }
    }

    #[test]
    fn a_row_with_two_sources_gives_one_list_with_the_hits_of_all_clips_at_their_song_positions() {
        let r = row(
            "q1",
            "Snare",
            vec![clip("a", 0.0, 4.0, 10.0, 1.0), clip("b", 2.0, 6.0, 20.0, 1.0)],
            vec![src("a", &[1.0, 5.0], 48_000), src("b", &[1.0, 3.0], 48_000)],
        );
        let t: Vec<f64> = mapped_events(&r).iter().map(|e| e["t"].as_f64().unwrap()).collect();
        // a: 1 -> 11 (5 is outside the clip); b: 1 is before the slice, 3 -> 21
        assert_eq!(t, vec![11.0, 21.0]);
        assert_eq!(row_end(&r), Some(24.0));
    }

    #[test]
    fn a_source_in_two_rows_delivers_only_the_clips_of_its_own_row() {
        let a = row(
            "q1",
            "A",
            vec![clip("x", 0.0, 2.0, 0.0, 1.0)],
            vec![src("x", &[1.0, 5.0], 48_000)],
        );
        let b = row(
            "q2",
            "B",
            vec![clip("x", 4.0, 6.0, 8.0, 1.0)],
            vec![src("x", &[1.0, 5.0], 48_000)],
        );
        assert_eq!(mapped_events(&a).len(), 1);
        assert_eq!(mapped_events(&b)[0]["t"].as_f64(), Some(9.0));
    }

    #[test]
    fn a_stretched_clip_follows_its_factor() {
        let r = row(
            "q1",
            "A",
            vec![clip("x", 0.0, 2.0, 1.0, 2.0)],
            vec![src("x", &[1.0], 48_000)],
        );
        assert_eq!(mapped_events(&r)[0]["t"].as_f64(), Some(3.0));
    }

    #[test]
    fn hostile_row_names_give_one_safe_stem_each_and_equal_names_are_numbered() {
        let long = "x".repeat(300);
        let names = ["../x", "/abs", "", long.as_str(), "Snare", "snare"];
        let rows: Vec<RowIn> = names
            .iter()
            .enumerate()
            .map(|(i, n)| row(&format!("q{i}"), n, vec![], vec![]))
            .collect();
        let s = stems(&rows);
        for r in &rows {
            let stem = &s[&r.id];
            assert!(
                !stem.contains('/') && !stem.starts_with('.') && !stem.is_empty(),
                "{stem}"
            );
            assert!(stem.len() <= crate::deliver::STEM_BYTES + 8, "{stem}");
            assert_eq!(Path::new(&file_name(stem, Kind::Midi)).components().count(), 1);
        }
        assert_ne!(s["q4"], s["q5"], "Snare and snare differ by a number");
    }

    #[test]
    fn the_merged_name_is_fixed_and_numbered_when_a_row_has_it() {
        let rows = vec![row("q1", MERGED_STEM, vec![], vec![])];
        let s = stems(&rows);
        assert_eq!(s["q1"], MERGED_STEM);
        assert_eq!(s["~merged"], format!("{MERGED_STEM} (2)"));
    }

    #[test]
    fn every_input_of_a_file_changes_its_fp_and_an_input_it_does_not_read_does_not() {
        let base = row(
            "q1",
            "A",
            vec![clip("x", 0.0, 2.0, 0.0, 1.0)],
            vec![src("x", &[0.5], 48_000)],
        );
        let fp = |r: &RowIn, k: Kind, tempo: &str| fingerprint(r, k, "A", tempo, 0);
        let m0 = fp(&base, Kind::Midi, "t");
        let mut moved = base.clone();
        moved.clips[0].s = 1.0;
        assert_ne!(fp(&moved, Kind::Midi, "t"), m0, "a moved clip");
        let mut edited = base.clone();
        edited.srcs[0].events[0]["t"] = json!(0.6);
        assert_ne!(fp(&edited, Kind::Midi, "t"), m0, "a moved hit");
        let mut vel = base.clone();
        vel.srcs[0].events[0]["v"] = json!(10);
        assert_ne!(fp(&vel, Kind::Midi, "t"), m0, "a velocity changes the MIDI file");
        assert_eq!(
            fp(&vel, Kind::Spike, ""),
            fp(&base, Kind::Spike, ""),
            "but not the spike file"
        );
        assert_ne!(fp(&base, Kind::Midi, "u"), m0, "the tempo");
        let mut note = base.clone();
        note.note = 40;
        assert_ne!(fp(&note, Kind::Midi, "t"), m0, "the note");
        assert_ne!(fingerprint(&base, Kind::Midi, "B", "t", 0), m0, "the stem");
    }

    /// #577: a note that a preset or a new default sets makes the MIDI set of the row and the merged set stale; the spike sets do not read it; and
    /// the export settings are no input of any set (the preparation reads none of them).
    #[test]
    fn a_changed_note_makes_the_midi_and_merged_sets_stale_and_the_export_settings_make_nothing() {
        let base = row(
            "q1",
            "A",
            vec![clip("x", 0.0, 2.0, 0.0, 1.0)],
            vec![src("x", &[0.5], 48_000)],
        );
        let mut other = base.clone();
        other.note = 40;
        let fp = |r: &RowIn, k: Kind| fingerprint(r, k, "A", "t", 0);
        assert_ne!(
            fp(&other, Kind::Midi),
            fp(&base, Kind::Midi),
            "the MIDI set of the row is stale"
        );
        assert_eq!(
            fp(&other, Kind::Spike),
            fp(&base, Kind::Spike),
            "the spike set does not read the note"
        );
        assert_eq!(fp(&other, Kind::Audio), fp(&base, Kind::Audio));
        // the merged set hashes the MIDI fingerprint of every row (see `fps`)
        let merged = |rows: &[&RowIn]| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for r in rows {
                fp(r, Kind::Midi).hash(&mut h);
            }
            h.finish()
        };
        assert_ne!(merged(&[&other]), merged(&[&base]), "the merged set is stale");
        // the export settings: no source of the preparation names them
        let src_text = include_str!("prepare.rs");
        for needle in [
            concat!("export", "_last"),
            concat!("export", "_compensate"),
            concat!("midi", "_mode"),
        ] {
            assert_eq!(src_text.matches(needle).count(), 0, "the preparation names {needle}");
        }
    }

    #[test]
    fn sources_of_different_rates_in_one_row_give_the_audio_error() {
        let r = row(
            "q1",
            "A",
            vec![clip("x", 0.0, 2.0, 0.0, 1.0), clip("y", 0.0, 2.0, 3.0, 1.0)],
            vec![src("x", &[1.0], 48_000), src("y", &[1.0], 44_100)],
        );
        assert_eq!(check_rates(&r), Err("clips have different sample rates".into()));
        let ok = row(
            "q1",
            "A",
            vec![clip("x", 0.0, 2.0, 0.0, 1.0)],
            vec![src("x", &[1.0], 48_000), src("y", &[1.0], 44_100)],
        );
        assert_eq!(
            check_rates(&ok),
            Ok(()),
            "a source with no clip of the row does not count"
        );
    }

    #[test]
    fn the_sum_of_two_signals_adds_sample_by_sample_and_keeps_the_longer_length() {
        assert_eq!(sum_signals(&[vec![1.0, 0.0, 0.5], vec![0.0, 2.0]]), vec![1.0, 2.0, 0.5]);
    }

    #[test]
    fn the_state_of_a_handle_follows_the_fp_and_never_stays_preparing() {
        let lim = Duration::from_secs(30);
        let st =
            |used, want, ready, failed: Option<&str>, pend| state_of(Kind::Spike, used, want, ready, failed, pend, lim);
        assert_eq!(st(true, None, None, None, None), State::None);
        assert_eq!(st(false, Some(1), None, None, None), State::Off);
        assert_eq!(st(true, Some(1), Some((1, 1)), None, None), State::Ready(1));
        assert_eq!(
            st(true, Some(2), Some((1, 1)), None, None),
            State::Preparing,
            "an older set is not ready"
        );
        assert_eq!(
            st(true, Some(2), Some((1, 1)), Some("no space"), None),
            State::Error("no space".into())
        );
        assert!(
            matches!(
                st(true, Some(2), None, None, Some(Duration::from_secs(31))),
                State::Error(_)
            ),
            "the watchdog"
        );
        assert_eq!(
            st(true, Some(2), None, None, Some(Duration::from_secs(5))),
            State::Preparing
        );
    }

    #[test]
    fn a_drag_of_more_than_the_bound_is_refused_with_a_reason() {
        assert_eq!(MAX_DRAG_FILES, 512);
        assert!(crate::deliver::drag_count_ok(512));
        assert!(!crate::deliver::drag_count_ok(513));
        assert!(!crate::deliver::drag_count_ok(0));
    }

    #[test]
    fn the_prepared_folder_is_bounded_at_two_gibibytes() {
        assert!(!over_cap(CAP_BYTES - 10, 10));
        assert!(over_cap(CAP_BYTES - 10, 11));
        assert!(over_cap(u64::MAX, 1), "no overflow");
    }

    #[test]
    fn the_prepared_folder_is_cleaned_of_files_no_set_holds() {
        let d = std::env::temp_dir().join(format!("ds-prep-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (keep, old) = (d.join("a_spike.mid"), d.join("gone_spike.mid"));
        std::fs::write(&keep, b"x").unwrap();
        std::fs::write(&old, b"x").unwrap();
        std::fs::write(d.join(".t.tmp"), b"x").unwrap();
        clean_prepared(&d, &HashSet::from([keep.clone()]));
        assert!(keep.exists() && !old.exists() && !d.join(".t.tmp").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// R4: a follower whose hits went stale while its audio was offline is derived again by the refresh of the preparation, so the file
    /// of its row holds the hits of the leader's edit; without the refresh (the fault run) the stored, stale hits are read.
    #[test]
    fn a_follower_row_carries_the_refreshed_hits_after_an_edit_of_its_leader() {
        use drumspike_app::{Service, ServiceConfig};
        use drumspike_core::io::{encode_wav, WavFormat, WavSpec};
        use drumspike_core::project::{add_track, build_source, new_project, save_project};
        let root = std::env::temp_dir().join(format!("ds-prep-follow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sr = 48_000u32;
        let kick = |onsets: &[f64]| -> Vec<f32> {
            let mut x = vec![0.0f64; (3.5 * f64::from(sr)) as usize];
            for &t0 in onsets {
                let i0 = (t0 * f64::from(sr)).round() as usize;
                let mut cum = 0.0;
                for k in 0..((0.4 * f64::from(sr)) as usize) {
                    if i0 + k >= x.len() {
                        break;
                    }
                    let t = k as f64 / f64::from(sr);
                    cum += 55.0 + 70.0 * (-t / 0.03).exp();
                    x[i0 + k] += (2.0 * std::f64::consts::PI * cum / f64::from(sr)).sin()
                        * (t / 0.0005).min(1.0)
                        * (-t / 0.12).exp()
                        * 0.7;
                }
            }
            x.iter().map(|&v| v as f32).collect()
        };
        let mut p = new_project(&root, "Follow").unwrap();
        let all = [0.25, 0.75, 1.25, 1.75, 2.25, 2.75];
        for (name, onsets) in [("lead", all.to_vec()), ("sub", all.iter().map(|t| t - 0.01).collect())] {
            let wav = root.join("daw").join(format!("{name}.wav"));
            std::fs::create_dir_all(wav.parent().unwrap()).unwrap();
            let spec = WavSpec {
                sample_rate: sr,
                channels: 1,
                format: WavFormat::Float32,
                extensible: false,
                channel_mask: 0,
            };
            let x = kick(&onsets);
            std::fs::write(&wav, encode_wav(&spec, &x, None)).unwrap();
            let rec = build_source(&wav, &p.dir, "referenced").unwrap();
            let tr = add_track(&mut p, name, Some(rec), "kick", 36, Some("average")).unwrap();
            tr.sr = Some(i64::from(sr));
            tr.n_samples = Some(x.len() as i64);
            tr.duration = Some(x.len() as f64 / f64::from(sr));
            tr.color = Some("#e05555".into());
        }
        save_project(&mut p).unwrap();
        let svc = Service::new(ServiceConfig::default());
        svc.set_home(root.join("home"));
        let sid = svc.project_open(&p.dir.display().to_string()).unwrap()["sid"]
            .as_str()
            .unwrap()
            .to_string();
        svc.events_set(&sid, "lead", &json!([{"t": 0.25, "v": 100}, {"t": 1.25, "v": 100}]))
            .unwrap();
        svc.track_set_follow(&sid, "sub", Some("lead")).unwrap();
        let wav = root.join("daw/sub.wav");
        let away = wav.with_extension("away");
        std::fs::rename(&wav, &away).unwrap();
        svc.project_get(&sid).unwrap();
        svc.events_set(
            &sid,
            "lead",
            &json!([{"t": 0.25, "v": 100}, {"t": 1.25, "v": 100}, {"t": 2.25, "v": 100}]),
        )
        .unwrap();
        std::fs::rename(&away, &wav).unwrap();
        svc.project_get(&sid).unwrap();
        let page = Arc::new(PageHandle::default());
        page.set(Some(svc.clone()));
        page.set_sid(Some(sid));
        let rows = Rows {
            rows: ["lead", "sub"]
                .iter()
                .map(|k| crate::rows::Row {
                    id: format!("q-{k}"),
                    name: (*k).into(),
                    srcs: vec![(*k).into()],
                    clips: vec![clip(k, 0.0, 3.5, 0.0, 1.0)],
                })
                .collect(),
            ..Rows::default()
        };
        let prep = Prep::new(
            page,
            Arc::new(move || rows.clone()),
            Arc::new(|| crate::tempo_ara::choose(None, "test", None)),
            Arc::new(HashMap::new),
        );
        let hits =
            |g: &Gathered, id: &str| -> usize { g.rows.iter().find(|r| r.id == id).unwrap().srcs[0].events.len() };
        let stale = prep.gather(false).unwrap();
        assert_eq!(
            hits(&stale, "q-sub"),
            2,
            "the stored follower still holds the old hits (the fault run reads them)"
        );
        let fresh = prep.gather(true).unwrap();
        assert_eq!(hits(&fresh, "q-lead"), 3);
        assert_eq!(
            hits(&fresh, "q-sub"),
            3,
            "the refresh derived the follower again: its row has the hit of the leader's edit"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------ round 2

    /// A document with two tracks `lead` and `sub` (kick-like bursts), no follow; the service, the sid and the folder of the files.
    fn two_tracks(tag: &str) -> (drumspike_app::Service, String, PathBuf) {
        use drumspike_app::{Service, ServiceConfig};
        use drumspike_core::io::{encode_wav, WavFormat, WavSpec};
        use drumspike_core::project::{add_track, build_source, new_project, save_project};
        let root = std::env::temp_dir().join(format!("ds-prep-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sr = 48_000u32;
        let mut p = new_project(&root, "Two").unwrap();
        for name in ["lead", "sub"] {
            let wav = root.join("daw").join(format!("{name}.wav"));
            std::fs::create_dir_all(wav.parent().unwrap()).unwrap();
            let spec = WavSpec {
                sample_rate: sr,
                channels: 1,
                format: WavFormat::Float32,
                extensible: false,
                channel_mask: 0,
            };
            let x = vec![0.1f32; (3.5 * f64::from(sr)) as usize];
            std::fs::write(&wav, encode_wav(&spec, &x, None)).unwrap();
            let rec = build_source(&wav, &p.dir, "referenced").unwrap();
            let tr = add_track(&mut p, name, Some(rec), "kick", 36, Some("average")).unwrap();
            tr.sr = Some(i64::from(sr));
            tr.n_samples = Some(x.len() as i64);
            tr.duration = Some(x.len() as f64 / f64::from(sr));
            tr.color = Some("#e05555".into());
        }
        save_project(&mut p).unwrap();
        let svc = Service::new(ServiceConfig::default());
        svc.set_home(root.join("home"));
        let sid = svc.project_open(&p.dir.display().to_string()).unwrap()["sid"]
            .as_str()
            .unwrap()
            .to_string();
        (svc, sid, root)
    }

    fn prep_of(svc: &drumspike_app::Service, sid: &str, root: &Path) -> Arc<Prep> {
        let page = Arc::new(PageHandle::default());
        page.set(Some(svc.clone()));
        page.set_sid(Some(sid.to_string()));
        let rows = Rows {
            rows: ["lead", "sub"]
                .iter()
                .map(|k| crate::rows::Row {
                    id: format!("q-{k}"),
                    name: (*k).into(),
                    srcs: vec![(*k).into()],
                    clips: vec![clip(k, 0.0, 3.5, 0.0, 1.0)],
                })
                .collect(),
            ..Rows::default()
        };
        let wavs: HashMap<String, (PathBuf, u32)> = ["lead", "sub"]
            .iter()
            .map(|k| ((*k).to_string(), (root.join("daw").join(format!("{k}.wav")), 48_000)))
            .collect();
        Prep::new(
            page,
            Arc::new(move || rows.clone()),
            Arc::new(|| crate::tempo_ara::choose(None, "test", None)),
            Arc::new(move || wavs.clone()),
        )
    }

    /// The stop sign of the workers is one flag for the whole process: the tests that set it and the tests that copy files run one after the other.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static L: Mutex<()> = Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn it(row: &str, kind: &str) -> Value {
        json!({"row": row, "kind": kind})
    }

    /// M1c4 (#589): the export of the dialog copies the prepared files, byte for byte what a take delivers, under the same names; it replaces a file of the
    /// same name, never a folder or a protected file, writes nothing for a stale set and leaves no temporary file.
    #[test]
    fn the_export_of_the_dialog_copies_the_prepared_files_and_never_a_stale_set_a_folder_or_a_protected_file() {
        let _serial = serial();
        let (svc, sid, root) = two_tracks("export");
        svc.events_set(&sid, "lead", &json!([{"t": 0.25, "v": 100}, {"t": 1.25, "v": 100}]))
            .unwrap();
        svc.events_set(&sid, "sub", &json!([{"t": 0.5, "v": 100}])).unwrap();
        let prep = prep_of(&svc, &sid, &root);
        prep.cycle(true, true);
        let dest = root.join("out");
        std::fs::create_dir_all(&dest).unwrap();
        let items = [it("q-lead", "midi"), it("q-sub", "midi")];
        // a file of the same name is replaced, and the count says so; the plan names the files
        std::fs::write(dest.join("lead_spike.mid"), b"OLD").unwrap();
        let names = prep.export_names(&items).unwrap();
        assert_eq!(
            names.iter().map(|n| n.2.as_str()).collect::<Vec<_>>(),
            ["lead_spike.mid", "sub_spike.mid"]
        );
        let got = prep.export_to(&items, &dest, &[], true).unwrap();
        assert_eq!((got["midi"].as_u64(), got["replaced"].as_u64()), (Some(2), Some(1)));
        let taken = prep
            .take(&[("q-lead".to_string(), Kind::Midi), ("q-sub".to_string(), Kind::Midi)])
            .unwrap();
        for (n, t) in ["lead_spike.mid", "sub_spike.mid"].iter().zip(&taken) {
            assert_eq!(
                std::fs::read(dest.join(n)).unwrap(),
                std::fs::read(t).unwrap(),
                "{n}: the bytes of the handle"
            );
        }
        assert!(
            std::fs::read_dir(&dest)
                .unwrap()
                .flatten()
                .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")),
            "no temporary file is left"
        );
        // a wav kind that was never used: the first call starts it and the answer is 409 (the page asks again); nothing is written
        let wav = [it("q-lead", "spike")];
        let e = prep.export_to(&wav, &dest, &[], true).unwrap_err();
        assert_eq!(e.code, 409, "{e:?}");
        assert!(!dest.join("lead_spike.wav").exists());
        let t = Instant::now();
        loop {
            prep.cycle(true, true);
            match prep.export_to(&wav, &dest, &[], false) {
                Ok(_) => break,
                Err(e) => assert!(e.code == 409 && t.elapsed() < Duration::from_secs(20), "{e:?}"),
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(dest.join("lead_spike.wav").is_file());
        // a stale set: the hits changed, the pass has not run: 409 and the old file in the folder is not touched
        svc.events_set(&sid, "lead", &json!([{"t": 0.25, "v": 100}, {"t": 2.25, "v": 100}]))
            .unwrap();
        let before = std::fs::read(dest.join("lead_spike.mid")).unwrap();
        let e = prep.export_to(&items, &dest, &[], false).unwrap_err();
        assert_eq!(e.code, 409, "{e:?}");
        assert_eq!(std::fs::read(dest.join("lead_spike.mid")).unwrap(), before);
        prep.cycle(true, true);
        prep.export_to(&items, &dest, &[], false).unwrap();
        assert_ne!(
            std::fs::read(dest.join("lead_spike.mid")).unwrap(),
            before,
            "the new hit is in the file"
        );
        // a folder with the name of a file is never replaced; a protected file is never replaced; the prepared folder is no destination
        let dir_dest = root.join("out2");
        std::fs::create_dir_all(dir_dest.join("sub_spike.mid")).unwrap();
        assert_eq!(prep.export_to(&items, &dir_dest, &[], false).unwrap_err().code, 409);
        assert!(
            dir_dest.join("sub_spike.mid").is_dir() && !dir_dest.join("lead_spike.mid").exists(),
            "all or nothing"
        );
        let guarded = dest.join("lead_spike.mid");
        assert_eq!(prep.export_to(&items, &dest, &[guarded], false).unwrap_err().code, 409);
        let exports = prep.exports_dir().unwrap();
        assert_eq!(
            prep.export_to(&items, &exports.join(PREPARED_DIR), &[], false)
                .unwrap_err()
                .code,
            409
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M1c4 (R10): the page names a row id and a kind and nothing else; an id that is not a live row, a name or a path in an item, a bad kind: 400, nothing written.
    #[test]
    fn a_forged_row_id_or_name_in_an_item_is_refused_and_nothing_is_written() {
        let _serial = serial();
        let (svc, sid, root) = two_tracks("forged");
        svc.events_set(&sid, "lead", &json!([{"t": 0.25, "v": 100}])).unwrap();
        let prep = prep_of(&svc, &sid, &root);
        prep.cycle(true, true);
        let dest = root.join("out");
        std::fs::create_dir_all(&dest).unwrap();
        for bad in [
            it("../etc", "midi"),
            it("q-nothing", "midi"),
            it("", "midi"),
            it("q-lead", "merged"),
            it("q-lead", "exe"),
            json!({"row": "q-lead", "kind": "midi", "name": "../escape.mid"}),
            json!({"row": "q-lead", "kind": "midi", "path": "/tmp/x"}),
            json!("q-lead"),
        ] {
            assert_eq!(
                prep.export_to(std::slice::from_ref(&bad), &dest, &[], false)
                    .unwrap_err()
                    .code,
                400,
                "{bad}"
            );
            assert_eq!(
                prep.export_names(std::slice::from_ref(&bad)).unwrap_err().0,
                400,
                "{bad}"
            );
        }
        assert!(prep.export_to(&[], &dest, &[], false).is_err());
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
        assert!(!root.join("escape.mid").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M1c4 (R8): a cancelled or failed copy leaves no file, not even a temporary one; the hits that the song map leaves out are counted.
    #[test]
    fn a_cancelled_copy_leaves_no_file_and_the_hits_left_out_are_counted() {
        let _serial = serial();
        let d = std::env::temp_dir().join(format!("ds-prep-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.bin"), vec![7u8; 3 << 20]).unwrap();
        let flag = std::sync::atomic::AtomicBool::new(false);
        copy_file(&d.join("a.bin"), &d.join("b.bin"), &flag).unwrap();
        assert_eq!(std::fs::read(d.join("b.bin")).unwrap().len(), 3 << 20);
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            copy_file(&d.join("a.bin"), &d.join("c.bin"), &flag).unwrap_err(),
            CopyErr::Cancelled
        );
        assert!(!d.join("c.bin").exists());
        assert_eq!(
            copy_file(&d.join("missing"), &d.join("e.bin"), &flag).unwrap_err(),
            CopyErr::Missing
        );
        assert!(!d.join("e.bin").exists());
        // an existing file at the destination name is never cut or followed
        std::fs::write(d.join("keep.bin"), b"KEEP").unwrap();
        flag.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            copy_file(&d.join("a.bin"), &d.join("keep.bin"), &flag),
            Err(CopyErr::Other(_))
        ));
        assert_eq!(std::fs::read(d.join("keep.bin")).unwrap(), b"KEEP");
        let r = row(
            "r1",
            "Row",
            vec![clip("a", 1.0, 2.0, 0.0, 1.0)],
            vec![src("a", &[0.5, 1.5, 1.6, 2.5], 48_000)],
        );
        assert_eq!(left_out(&r), (2, 0), "two hits outside the clip");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A document with `n` tracks `t0`..`t{n-1}` that all read ONE audio file of `secs` seconds (float 32, mono, 48 kHz), each with `hits` hits.
    fn many_tracks(tag: &str, n: usize, secs: f64, hits: usize) -> (drumspike_app::Service, String, PathBuf) {
        many_tracks_named(tag, n, secs, hits, "all.wav")
    }

    fn many_tracks_named(
        tag: &str,
        n: usize,
        secs: f64,
        hits: usize,
        wav_name: &str,
    ) -> (drumspike_app::Service, String, PathBuf) {
        use drumspike_app::{Service, ServiceConfig};
        use drumspike_core::io::{encode_wav, WavFormat, WavSpec};
        use drumspike_core::project::{add_track, build_source, new_project, save_project};
        let root = std::env::temp_dir().join(format!("ds-prep-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sr = 48_000u32;
        let mut p = new_project(&root, "Many").unwrap();
        let wav = root.join("daw").join(wav_name);
        std::fs::create_dir_all(wav.parent().unwrap()).unwrap();
        let spec = WavSpec {
            sample_rate: sr,
            channels: 1,
            format: WavFormat::Float32,
            extensible: false,
            channel_mask: 0,
        };
        let x = vec![0.1f32; (secs * f64::from(sr)) as usize];
        std::fs::write(&wav, encode_wav(&spec, &x, None)).unwrap();
        for k in 0..n {
            let rec = build_source(&wav, &p.dir, "referenced").unwrap();
            let tr = add_track(&mut p, &format!("t{k}"), Some(rec), "kick", 36, Some("average")).unwrap();
            tr.sr = Some(i64::from(sr));
            tr.n_samples = Some(x.len() as i64);
            tr.duration = Some(secs);
            tr.color = Some("#e05555".into());
        }
        save_project(&mut p).unwrap();
        let svc = Service::new(ServiceConfig::default());
        svc.set_home(root.join("home"));
        let sid = svc.project_open(&p.dir.display().to_string()).unwrap()["sid"]
            .as_str()
            .unwrap()
            .to_string();
        for k in 0..n {
            let ev: Vec<Value> = (0..hits)
                .map(|i| json!({"t": 0.1 + (secs - 0.3) * i as f64 / hits as f64, "v": 100}))
                .collect();
            svc.events_set(&sid, &format!("t{k}"), &json!(ev)).unwrap();
        }
        (svc, sid, root)
    }

    fn prep_many(svc: &drumspike_app::Service, sid: &str, root: &Path, n: usize, secs: f64) -> Arc<Prep> {
        prep_many_named(svc, sid, root, n, secs, "all.wav")
    }

    fn prep_many_named(
        svc: &drumspike_app::Service,
        sid: &str,
        root: &Path,
        n: usize,
        secs: f64,
        wav_name: &str,
    ) -> Arc<Prep> {
        let page = Arc::new(PageHandle::default());
        page.set(Some(svc.clone()));
        page.set_sid(Some(sid.to_string()));
        let rows = Rows {
            rows: (0..n)
                .map(|k| crate::rows::Row {
                    id: format!("q{k}"),
                    name: format!("t{k}"),
                    srcs: vec![format!("t{k}")],
                    clips: vec![clip(&format!("t{k}"), 0.0, secs, 0.0, 1.0)],
                })
                .collect(),
            ..Rows::default()
        };
        let wav = root.join("daw").join(wav_name);
        let wavs: HashMap<String, (PathBuf, u32)> = (0..n).map(|k| (format!("t{k}"), (wav.clone(), 48_000))).collect();
        Prep::new(
            page,
            Arc::new(move || rows.clone()),
            Arc::new(|| crate::tempo_ara::choose(None, "test", None)),
            Arc::new(move || wavs.clone()),
        )
    }

    fn cpu_ms() -> u128 {
        // the CPU time of this process, from `ps` (10 ms resolution)
        let out = std::process::Command::new("/bin/ps")
            .args(["-o", "cputime=", "-p", &std::process::id().to_string()])
            .output()
            .unwrap();
        let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let (m, rest) = t.split_once(':').unwrap_or(("0", "0"));
        let secs: f64 = rest.parse().unwrap_or(0.0);
        (m.parse::<f64>().unwrap_or(0.0) * 60_000.0 + secs * 1000.0) as u128
    }

    /// M1c4 (#589), the two measures of the ticket. Run: `cargo test -p drumspike-ara --lib measure_ -- --ignored --nocapture` (about 3 GB of scratch
    /// disk for the first one). It prints the numbers; the check does not run it (the times depend on the machine).
    #[test]
    #[ignore = "a measure: prints numbers, writes about 3 GB"]
    fn measure_the_spike_files_of_a_ten_minute_song_with_twelve_tracks() {
        let (n, secs) = (12usize, 600.0f64);
        let (svc, sid, root) = many_tracks("measure12", n, secs, 700);
        let prep = prep_many(&svc, &sid, &root, n, secs);
        let t = Instant::now();
        prep.cycle(true, true);
        let midi_ms = t.elapsed().as_millis();
        prep.lock().used = [true, false];
        let t = Instant::now();
        prep.cycle(true, true);
        let spike_ms = t.elapsed().as_millis();
        let exports = prep.exports_dir().unwrap();
        let bytes: u64 = std::fs::read_dir(exports.join(PREPARED_DIR))
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "wav"))
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        let dest = root.join("out");
        std::fs::create_dir_all(&dest).unwrap();
        let mut items: Vec<Value> = (0..n).map(|k| it(&format!("q{k}"), "midi")).collect();
        items.extend((0..n).map(|k| it(&format!("q{k}"), "spike")));
        let t = Instant::now();
        let got = prep.export_to(&items, &dest, &[], false).unwrap();
        let copy_ms = t.elapsed().as_millis();
        println!(
            "MEASURE ten minutes, 12 tracks, 700 hits each: MIDI sets (12 + merged) {midi_ms} ms; spike sets (12) {spike_ms} ms ({} MB written, {} ms for each file on average); \
             the dialog's copy of {} files into a folder {copy_ms} ms",
            bytes / 1_000_000,
            spike_ms / n as u128,
            got["files"].as_array().map_or(0, Vec::len)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "a measure: prints numbers"]
    fn measure_the_idle_look_with_forty_rows() {
        let (n, secs) = (40usize, 60.0f64);
        let (svc, sid, root) = many_tracks("measure40", n, secs, 300);
        let prep = prep_many(&svc, &sid, &root, n, secs);
        prep.cycle(true, true);
        let reps = 20u32;
        let t = Instant::now();
        for _ in 0..reps {
            let g = prep.gather(true).unwrap();
            let _ = prep.fps(&g);
        }
        let look = t.elapsed().as_secs_f64() * 1000.0 / f64::from(reps);
        let t = Instant::now();
        for _ in 0..reps {
            let _ = prep.state_json();
        }
        let state = t.elapsed().as_secs_f64() * 1000.0 / f64::from(reps);
        let payload = prep.state_json().to_string().len();
        // the worker with nothing to do: CPU time and passes over 5 s
        prep.wake();
        std::thread::sleep(Duration::from_millis(1500));
        let (c0, _) = prep.counters();
        let cpu0 = cpu_ms();
        std::thread::sleep(Duration::from_secs(5));
        let (c1, _) = prep.counters();
        println!(
            "MEASURE idle look, 40 rows, 300 hits each: one look (gather with the follower refresh, then the fingerprints) {look:.2} ms; state_json {state:.2} ms, payload {payload} bytes; \
             over 5 s with no change: {} passes, {} ms CPU of this process",
            c1 - c0,
            cpu_ms() - cpu0
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    fn real_hooks_with(rename: RenameFn, copy: CopyFn, after_copy: Box<dyn Fn()>) -> ExportHooks {
        ExportHooks {
            copy,
            rename,
            after_copy,
        }
    }

    /// Five prepared MIDI sets, a folder holding an old file for each of them, and the items of the export.
    fn five_rows(tag: &str) -> (drumspike_app::Service, String, PathBuf, Arc<Prep>, PathBuf, Vec<Value>) {
        let (svc, sid, root) = many_tracks(tag, 5, 3.5, 5);
        let prep = prep_many(&svc, &sid, &root, 5, 3.5);
        prep.cycle(true, true);
        let dest = root.join("out");
        std::fs::create_dir_all(&dest).unwrap();
        for k in 0..5 {
            std::fs::write(dest.join(format!("t{k}_spike.mid")), b"OLD").unwrap();
        }
        let items: Vec<Value> = (0..5).map(|k| it(&format!("q{k}"), "midi")).collect();
        (svc, sid, root, prep, dest, items)
    }

    fn temps_in(d: &Path) -> Vec<String> {
        std::fs::read_dir(d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect()
    }

    /// M1c4 round 2 (M1): a rename that fails at the third of five files: the answer lists the two files written and the three DAW tracks not written, no
    /// temporary file is left and the three old files are untouched.
    #[test]
    fn a_rename_that_fails_in_the_middle_lists_what_was_written_and_what_was_not() {
        let _serial = serial();
        let (_svc, _sid, root, prep, dest, items) = five_rows("renamefail");
        let n = std::cell::Cell::new(0);
        let hooks = real_hooks_with(
            Box::new(move |a, b| {
                n.set(n.get() + 1);
                if n.get() == 3 {
                    Err(std::io::Error::other("injected"))
                } else {
                    std::fs::rename(a, b)
                }
            }),
            Box::new(copy_file),
            Box::new(|| {}),
        );
        let e = prep.export_to_with(&items, &dest, &[], false, &hooks).unwrap_err();
        assert_eq!(e.code, 500);
        assert_eq!(e.written, ["t0_spike.mid", "t1_spike.mid"]);
        assert_eq!(e.not_written, ["t2", "t3", "t4"]);
        assert!(
            e.text.contains("2 of 5 files were written")
                && e.text.contains("Not written: t2, t3, t4")
                && e.text.contains("Press Export again"),
            "{}",
            e.text
        );
        assert!(temps_in(&dest).is_empty(), "no temporary file is left");
        for k in 0..2 {
            assert_ne!(std::fs::read(dest.join(format!("t{k}_spike.mid"))).unwrap(), b"OLD");
        }
        for k in 2..5 {
            assert_eq!(
                std::fs::read(dest.join(format!("t{k}_spike.mid"))).unwrap(),
                b"OLD",
                "t{k} is untouched"
            );
        }
        // a read-only file in the way is found BEFORE the first file is written
        let ro = dest.join("t4_spike.mid");
        let mut perm = std::fs::metadata(&ro).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&ro, perm).unwrap();
        let before = std::fs::read(dest.join("t0_spike.mid")).unwrap();
        std::fs::write(dest.join("t0_spike.mid"), b"OLD").unwrap();
        let e = prep.export_to(&items, &dest, &[], false).unwrap_err();
        assert_eq!(e.code, 409, "{}", e.text);
        assert_eq!(
            std::fs::read(dest.join("t0_spike.mid")).unwrap(),
            b"OLD",
            "nothing was written"
        );
        let _ = before;
        let mut perm = std::fs::metadata(&ro).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(&ro, perm).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M1c4 round 2 (S2): one export at a time; a second call changes nothing of the first; the same row and kind twice is one file.
    #[test]
    fn a_second_export_is_refused_and_touches_nothing_of_the_first_and_a_double_item_is_one_file() {
        let _serial = serial();
        use std::sync::atomic::Ordering::SeqCst;
        let (_svc, _sid, root, prep, dest, items) = five_rows("one-at-a-time");
        prep.exp.running.store(true, SeqCst);
        prep.exp.done.store(7, SeqCst);
        prep.exp.total.store(9, SeqCst);
        prep.exp.busy.store(true, SeqCst);
        let e = prep.export_to(&items, &dest, &[], true).unwrap_err();
        assert_eq!((e.code, e.text.as_str()), (409, "an export is running"));
        assert_eq!(prep.exp.done.load(SeqCst), 7);
        assert_eq!(prep.exp.total.load(SeqCst), 9);
        assert!(
            prep.exp.busy.load(SeqCst) && prep.exp.running.load(SeqCst) && !prep.exp.cancel.load(SeqCst),
            "the flags of the first export are as they were"
        );
        prep.exp.running.store(false, SeqCst);
        prep.exp.busy.store(false, SeqCst);
        let twice = vec![it("q0", "midi"), it("q0", "midi"), it("q1", "midi")];
        let got = prep.export_to(&twice, &dest, &[], false).unwrap();
        assert_eq!(got["files"].as_array().unwrap().len(), 2, "q0 twice is one file");
        assert_eq!(prep.export_names(&twice).unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M1c4 round 2 (S3): the temporary files go on a panic inside the copy too; the guard flags are given back.
    #[test]
    fn a_panic_in_the_copy_leaves_no_temporary_file_and_the_next_export_works() {
        let _serial = serial();
        let (_svc, _sid, root, prep, dest, items) = five_rows("panic");
        let n = std::cell::Cell::new(0);
        let hooks = real_hooks_with(
            Box::new(|a, b| std::fs::rename(a, b)),
            Box::new(move |a, b, c| {
                n.set(n.get() + 1);
                assert!(n.get() < 3, "injected panic");
                copy_file(a, b, c)
            }),
            Box::new(|| {}),
        );
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            prep.export_to_with(&items, &dest, &[], false, &hooks)
        }));
        assert!(r.is_err(), "the panic went through");
        assert!(temps_in(&dest).is_empty(), "no temporary file after a panic");
        for k in 0..5 {
            assert_eq!(std::fs::read(dest.join(format!("t{k}_spike.mid"))).unwrap(), b"OLD");
        }
        assert!(
            prep.export_to(&items, &dest, &[], false).is_ok(),
            "the flags were given back"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M1c4 round 2 (S5 a): the hits change between the copy and the check: 409, nothing renamed, the temporary files are gone.
    #[test]
    fn hits_that_change_during_the_copy_stop_the_export_before_the_first_rename() {
        let _serial = serial();
        let (svc, sid, root, prep, dest, items) = five_rows("late");
        let (svc2, sid2) = (svc.clone(), sid.clone());
        let hooks = real_hooks_with(
            Box::new(|a, b| std::fs::rename(a, b)),
            Box::new(copy_file),
            Box::new(move || {
                svc2.events_set(&sid2, "t0", &json!([{"t": 0.5, "v": 100}, {"t": 2.0, "v": 100}]))
                    .unwrap();
            }),
        );
        let e = prep.export_to_with(&items, &dest, &[], false, &hooks).unwrap_err();
        assert_eq!(e.code, 409);
        assert!(
            e.text.starts_with("preparing") && e.text.contains("changed"),
            "{}",
            e.text
        );
        assert!(temps_in(&dest).is_empty());
        for k in 0..5 {
            assert_eq!(
                std::fs::read(dest.join(format!("t{k}_spike.mid"))).unwrap(),
                b"OLD",
                "nothing was renamed"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M1c4 round 2 (S5 c, S6): a source that cannot be read is a 500 that names the DAW track, with no temporary file and the old files untouched; a prepared file
    /// that is gone when the copy opens it is a 409 "preparing" (the page waits), not a 500.
    #[test]
    fn an_unreadable_source_is_a_named_failure_and_a_vanished_one_is_preparing() {
        let _serial = serial();
        use std::os::unix::fs::PermissionsExt;
        let (_svc, _sid, root, prep, dest, items) = five_rows("copyfail");
        let bad = prep.exports_dir().unwrap().join(PREPARED_DIR).join("t1_spike.mid");
        std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o000)).unwrap();
        let e = prep.export_to(&items, &dest, &[], false).unwrap_err();
        std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(e.code, 500);
        assert!(e.text.starts_with("t1: could not copy the file"), "{}", e.text);
        assert!(temps_in(&dest).is_empty());
        for k in 0..5 {
            assert_eq!(std::fs::read(dest.join(format!("t{k}_spike.mid"))).unwrap(), b"OLD");
        }
        let prepared = prep.exports_dir().unwrap().join(PREPARED_DIR);
        let hooks = real_hooks_with(
            Box::new(|a, b| std::fs::rename(a, b)),
            Box::new(move |a, b, c| {
                let _ = std::fs::remove_file(prepared.join("t2_spike.mid"));
                copy_file(a, b, c)
            }),
            Box::new(|| {}),
        );
        let e = prep.export_to_with(&items, &dest, &[], false, &hooks).unwrap_err();
        assert_eq!(e.code, 409, "{}", e.text);
        assert_eq!(e.text, "t2: preparing");
        assert!(temps_in(&dest).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M1c4 round 2 (S5 d): a file that the service protects (the audio file of a track) is never replaced, through the real list of the service.
    #[test]
    fn an_output_that_names_the_audio_file_of_a_track_is_refused_with_the_real_protected_list() {
        let _serial = serial();
        let (svc, sid, root) = many_tracks_named("protected", 1, 3.5, 5, "t0_spike.wav");
        let prep = prep_many_named(&svc, &sid, &root, 1, 3.5, "t0_spike.wav");
        prep.lock().used = [true, false];
        prep.cycle(true, true);
        let before = std::fs::read(root.join("daw/t0_spike.wav")).unwrap();
        let protected = svc.protected_files_everywhere();
        assert!(!protected.is_empty());
        let e = prep
            .export_to(&[it("q0", "spike")], &root.join("daw"), &protected, false)
            .unwrap_err();
        assert_eq!(e.code, 409);
        assert!(e.text.contains("protected"), "{}", e.text);
        assert_eq!(std::fs::read(root.join("daw/t0_spike.wav")).unwrap(), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M1c4 round 2 (S4): a poll that waits starts no look of the worker: 5 s of polls with 40 rows and a set that is not ready make at most 2 looks.
    #[test]
    fn five_seconds_of_waiting_with_forty_rows_start_at_most_two_looks() {
        let _serial = serial();
        let (svc, sid, root) = many_tracks("waitcost", 40, 3.5, 20);
        let prep = prep_many(&svc, &sid, &root, 40, 3.5);
        let dest = root.join("out");
        std::fs::create_dir_all(&dest).unwrap();
        let items: Vec<Value> = (0..40).map(|k| it(&format!("q{k}"), "midi")).collect();
        let t = Instant::now();
        let mut polls = 0;
        while t.elapsed() < Duration::from_secs(5) {
            let e = prep.export_to(&items, &dest, &[], false).unwrap_err();
            assert_eq!(e.code, 409, "{}", e.text);
            polls += 1;
            std::thread::sleep(Duration::from_millis(300));
        }
        assert!(polls >= 10);
        assert!(prep.counters().0 <= 2, "{} looks for {polls} polls", prep.counters().0);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M1c4 round 2 (S3): the old temporary files of the plugin go on the next export, any other file stays; `replaces` is the one rule for a file in the way.
    #[test]
    fn old_temporary_files_of_the_plugin_are_removed_and_other_files_stay() {
        let d = std::env::temp_dir().join(format!("ds-prep-temps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let _ = temp_name("x");
        let pid = std::process::id();
        let mine_old = format!(".a.mid.dsx-{pid}-0.tmp");
        let mine_future = format!(".b.mid.dsx-{pid}-999999999.tmp");
        let other_new = format!(".c.mid.dsx-{}-0.tmp", pid + 1);
        let other_old = format!(".d.mid.dsx-{}-0.tmp", pid + 1);
        for n in [
            &mine_old,
            &mine_future,
            &other_new,
            &other_old,
            &".keep.tmp".to_string(),
            &"notes.txt".to_string(),
            &".e.tmp.dsx-x-1.tmp".to_string(),
        ] {
            std::fs::write(d.join(n), b"x").unwrap();
        }
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(d.join(&other_old))
            .unwrap();
        f.set_modified(std::time::SystemTime::now() - Duration::from_secs(7200))
            .unwrap();
        // another export of this process copies: a name of this process can be its live file and stays; the age rule still applies
        clean_old_temps(&d, false);
        assert!(
            d.join(&mine_old).is_file() && !d.join(&other_old).exists(),
            "not alone: only the age rule"
        );
        std::fs::write(d.join(&other_old), b"x").unwrap();
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(d.join(&other_old))
            .unwrap();
        f.set_modified(std::time::SystemTime::now() - Duration::from_secs(7200))
            .unwrap();
        drop(f);
        clean_old_temps(&d, true);
        let left: HashSet<String> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!left.contains(&mine_old) && !left.contains(&other_old), "{left:?}");
        for keep in [
            &mine_future,
            &other_new,
            &".keep.tmp".to_string(),
            &"notes.txt".to_string(),
            &".e.tmp.dsx-x-1.tmp".to_string(),
        ] {
            assert!(left.contains(keep), "{keep} stays");
        }
        std::fs::create_dir_all(d.join("folder")).unwrap();
        assert!(replaces(&d.join("notes.txt")) && !replaces(&d.join("folder")) && !replaces(&d.join("nothing")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_delivered_file_keeps_its_bytes_and_the_next_take_delivers_a_numbered_new_one() {
        let (svc, sid, root) = two_tracks("deliver");
        svc.events_set(&sid, "lead", &json!([{"t": 0.25, "v": 100}, {"t": 1.25, "v": 100}]))
            .unwrap();
        svc.events_set(&sid, "sub", &json!([{"t": 0.5, "v": 100}])).unwrap();
        let prep = prep_of(&svc, &sid, &root);
        prep.lock().used = [true, false];
        prep.cycle(true, true);
        let exports = prep.exports_dir().unwrap();
        // the user's own file of the same name in Exports/ is never touched by the preparation
        let user = exports.join("lead_spike.mid");
        std::fs::write(&user, b"USER").unwrap();
        prep.cycle(true, true);
        let item = [("q-lead".to_string(), Kind::Midi)];
        let first = prep.take(&item).unwrap();
        assert_eq!(first[0].parent().unwrap().file_name().unwrap(), DRAGGED_DIR);
        assert_eq!(first[0].file_name().unwrap(), "lead_spike.mid");
        let old_bytes = std::fs::read(&first[0]).unwrap();
        assert_eq!(
            prep.take(&item).unwrap(),
            first,
            "the same set again: the same delivered file"
        );
        // an edit: the set in .prepared/ is replaced by a new file; the delivered one keeps its content
        svc.events_set(
            &sid,
            "lead",
            &json!([{"t": 0.25, "v": 100}, {"t": 1.25, "v": 100}, {"t": 2.25, "v": 100}]),
        )
        .unwrap();
        assert!(prep.take(&item).is_err(), "stale until the pass made it again");
        prep.cycle(true, true);
        let second = prep.take(&item).unwrap();
        assert_eq!(second[0].file_name().unwrap(), "lead_spike 2.mid");
        assert_eq!(
            std::fs::read(&first[0]).unwrap(),
            old_bytes,
            "the delivered file has the old bytes"
        );
        assert_ne!(std::fs::read(&second[0]).unwrap(), old_bytes);
        assert_eq!(
            std::fs::read(&user).unwrap(),
            b"USER",
            "a file of the user in Exports/ is not changed"
        );
        // a prepared file that is gone makes the set stale: not ready, made again
        let gone = exports.join(PREPARED_DIR).join("sub_spike.mid");
        assert!(prep.take(&[("q-sub".to_string(), Kind::Midi)]).is_ok());
        std::fs::remove_file(&gone).unwrap();
        assert_eq!(prep.take(&[("q-sub".to_string(), Kind::Midi)]).unwrap_err().0, 409);
        prep.cycle(true, true);
        assert!(gone.is_file(), "made again");
        assert!(prep.take(&[("q-sub".to_string(), Kind::Midi)]).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_name_that_exists_in_the_delivery_folder_with_other_content_is_numbered_never_replaced_and_a_failed_link_is_an_error(
    ) {
        let d = std::env::temp_dir().join(format!("ds-prep-del-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("p")).unwrap();
        std::fs::create_dir_all(d.join("o")).unwrap();
        std::fs::write(d.join("p/Snare_spike.mid"), b"NEW").unwrap();
        std::fs::write(d.join("o/Snare_spike.mid"), b"OTHER").unwrap();
        let got = deliver(&d.join("p/Snare_spike.mid"), &d.join("o")).unwrap();
        assert_eq!(got.file_name().unwrap(), "Snare_spike 2.mid");
        assert_eq!(std::fs::read(d.join("o/Snare_spike.mid")).unwrap(), b"OTHER");
        assert_eq!(std::fs::read(&got).unwrap(), b"NEW");
        assert_eq!(
            deliver(&d.join("p/Snare_spike.mid"), &d.join("o")).unwrap(),
            got,
            "the same inode: reused"
        );
        assert!(
            deliver(&d.join("p/none.mid"), &d.join("o")).is_err(),
            "no source: an error, no copy"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn file_names_are_unique_per_kind_set_without_case() {
        let rows = vec![
            row("q1", "Snare", vec![], vec![]),
            row("q2", "Snare_audio", vec![], vec![]),
        ];
        let stems: HashMap<String, String> = [("q1", "Snare"), ("q2", "Snare_audio"), ("~merged", MERGED_STEM)]
            .iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect();
        let n = final_names(&rows, &stems);
        assert_eq!(n[&("q2".to_string(), Kind::Spike)], "Snare_audio_spike.wav");
        assert_eq!(
            n[&("q1".to_string(), Kind::Audio)],
            "Snare (2)_audio_spike.wav",
            "the later one is numbered"
        );
        let low: HashSet<String> = n.values().map(|v| v.to_lowercase()).collect();
        assert_eq!(low.len(), n.len());
        // case only
        let rows = vec![
            row("q1", "snare_audio", vec![], vec![]),
            row("q2", "Snare", vec![], vec![]),
        ];
        let stems: HashMap<String, String> = [("q1", "snare_audio"), ("q2", "Snare"), ("~merged", MERGED_STEM)]
            .iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect();
        let n = final_names(&rows, &stems);
        let low: HashSet<String> = n.values().map(|v| v.to_lowercase()).collect();
        assert_eq!(low.len(), n.len(), "{n:?}");
    }

    #[test]
    fn a_failure_is_tried_again_on_a_press_a_take_and_after_the_back_off() {
        assert_eq!(
            [1, 2, 3, 4, 5, 6, 9].map(|t| backoff(t).as_secs()),
            [5, 10, 20, 40, 60, 60, 60]
        );
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (svc, sid, root) = two_tracks("retry");
        let prep = prep_of(&svc, &sid, &root);
        let key = ("q-lead".to_string(), Kind::Midi);
        prep.lock()
            .failed
            .insert(key.clone(), (1, "x".into(), Instant::now(), 1));
        assert!(
            failed_blocks(&prep.lock().failed[&key], 1),
            "held back inside the back-off"
        );
        assert!(
            !failed_blocks(&(1, "x".into(), Instant::now() - Duration::from_secs(6), 1), 1),
            "free after it"
        );
        assert!(
            !failed_blocks(&(1, "x".into(), Instant::now(), 1), 2),
            "a new state is not held back"
        );
        prep.want_kind(Kind::Midi, Some("q-lead"));
        assert!(prep.lock().failed.is_empty(), "a press on the error handle");
        prep.lock()
            .failed
            .insert(key.clone(), (1, "x".into(), Instant::now(), 1));
        let _ = prep.take(std::slice::from_ref(&key));
        assert!(prep.lock().failed.is_empty(), "a take");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_change_of_the_hits_starts_the_watchdog_again_after_an_old_failure() {
        let (svc, sid, root) = two_tracks("watch");
        let prep = prep_of(&svc, &sid, &root);
        let key = ("q-lead".to_string(), Kind::Midi);
        let mut st = prep.lock();
        st.cycles = 1;
        st.want.insert(key.clone(), 1);
        st.failed.insert(key.clone(), (1, "old".into(), Instant::now(), 1));
        st.pending_since
            .insert(key.clone(), Instant::now() - Duration::from_secs(120));
        assert_eq!(prep.state_for(&st, &key, true), State::Error("old".into()));
        // an edit: the wanted state changes, the old failure no longer counts and the old time is gone
        update_books(&mut st, &HashMap::from([(key.clone(), 2)]));
        assert_eq!(prep.state_for(&st, &key, true), State::Preparing);
        assert!(st.pending_since[&key].elapsed() < Duration::from_secs(5));
        drop(st);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_row_with_its_first_hit_shows_preparing_and_no_export_folder_is_an_error() {
        let (svc, sid, root) = two_tracks("states");
        let prep = prep_of(&svc, &sid, &root);
        let key = ("q-lead".to_string(), Kind::Midi);
        let mut st = prep.lock();
        st.cycles = 3;
        assert_eq!(prep.state_for(&st, &key, false), State::None, "no hits");
        st.wake_seq = 5; // something changed since the last pass: the row may have its first hit now
        assert_eq!(prep.state_for(&st, &key, true), State::Preparing);
        st.clean_seq = 5;
        st.no_folder = true;
        st.want.insert(key.clone(), 1);
        assert_eq!(prep.state_for(&st, &key, true), State::Error("no export folder".into()));
        drop(st);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_spike_set_needs_one_rate_and_a_file_for_the_first_used_source_like_the_audio_set() {
        let mixed = row(
            "q1",
            "A",
            vec![clip("x", 0.0, 2.0, 0.0, 1.0), clip("y", 0.0, 2.0, 3.0, 1.0)],
            vec![src("x", &[1.0], 48_000), src("y", &[1.0], 44_100)],
        );
        let out = std::env::temp_dir().join(format!("ds-prep-s16-{}.wav", std::process::id()));
        assert_eq!(
            write_spike_row(&mixed, &out),
            Err("clips have different sample rates".into())
        );
        assert_eq!(
            write_audio_row(&mixed, &out),
            Err("clips have different sample rates".into())
        );
        let nofile = row(
            "q1",
            "A",
            vec![clip("y", 0.0, 2.0, 0.0, 1.0)],
            vec![src("x", &[1.0], 48_000), src("y", &[1.0], 48_000)],
        );
        // the first source has no clip in this row: the first USED source (`y`) decides; it has no file
        assert_eq!(write_spike_row(&nofile, &out), Err("no audio for A".into()));
        assert!(!out.exists());
    }

    #[test]
    fn the_size_is_estimated_before_anything_is_written_and_a_set_over_the_bound_is_refused() {
        let long = row(
            "q1",
            "Long",
            vec![clip("x", 0.0, 600.0, 0.0, 1.0)],
            vec![src("x", &[1.0], 48_000)],
        );
        assert_eq!(estimate_bytes(&long), 600 * 48_000 * 4);
        let huge = row(
            "q1",
            "Huge",
            vec![clip("x", 0.0, 100_000.0, 0.0, 1.0)],
            vec![src("x", &[1.0], 48_000)],
        );
        let d = std::env::temp_dir().join(format!("ds-prep-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let key = ("q1".to_string(), Kind::Spike);
        let g = Gathered {
            rows: vec![huge],
            map: TempoMap::default(),
            tempo_key: String::new(),
            stems: HashMap::from([
                ("q1".to_string(), "Huge".to_string()),
                ("~merged".to_string(), MERGED_STEM.to_string()),
            ]),
            names: HashMap::from([(key.clone(), "Huge_spike.wav".to_string())]),
        };
        let r = make(&g, &key, &d, &d.join(PREPARED_DIR));
        assert_eq!(
            r,
            Err("not enough room for the prepared files (limit 2 GiB)".to_string())
        );
        assert!(
            !d.join(PREPARED_DIR).join("Huge_spike.wav.tmp").exists(),
            "nothing was written"
        );
        // the sum of several audio parts is bounded in memory too
        let parts = row(
            "q1",
            "P",
            vec![clip("x", 0.0, 100_000.0, 0.0, 1.0), clip("y", 0.0, 100_000.0, 0.0, 1.0)],
            vec![src("x", &[1.0], 48_000), src("y", &[1.0], 48_000)],
        );
        assert_eq!(
            write_audio_row(&parts, &d.join("o.wav")),
            Err("not enough memory for the audio spikes of this row".into())
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_hostile_name_never_reaches_a_path_outside_the_prepared_folder() {
        let d = std::env::temp_dir().join(format!("ds-prep-host-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let r = row(
            "q1",
            "x",
            vec![clip("x", 0.0, 1.0, 0.0, 1.0)],
            vec![src("x", &[0.5], 48_000)],
        );
        for bad in ["../x_spike.mid", "/abs_spike.mid", "a/b_spike.mid"] {
            let key = ("q1".to_string(), Kind::Midi);
            let g = Gathered {
                rows: vec![r.clone()],
                map: TempoMap::default(),
                tempo_key: String::new(),
                stems: HashMap::new(),
                names: HashMap::from([(key.clone(), bad.to_string())]),
            };
            let out = make(&g, &key, &d, &d.join(PREPARED_DIR));
            assert_eq!(out, Err("the name is not allowed".to_string()), "{bad}");
        }
        assert!(!d.join("x_spike.mid").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_closures_of_the_preparation_do_not_keep_the_document_alive_and_the_worker_ends_with_it() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        struct Doc;
        let doc = Arc::new(Doc);
        let (w1, w2, w3) = (Arc::downgrade(&doc), Arc::downgrade(&doc), Arc::downgrade(&doc));
        let page = Arc::new(PageHandle::default());
        let prep = Prep::new(
            page,
            Arc::new(move || w1.upgrade().map(|_| Rows::default()).unwrap_or_default()),
            Arc::new(move || {
                w2.upgrade().map(|_| ()).map_or_else(
                    || crate::tempo_ara::choose(None, "closed", None),
                    |_| crate::tempo_ara::choose(None, "x", None),
                )
            }),
            Arc::new(move || w3.upgrade().map(|_| HashMap::new()).unwrap_or_default()),
        );
        let weak_doc = Arc::downgrade(&doc);
        let weak_prep = Arc::downgrade(&prep);
        prep.wake();
        let ended = Arc::clone(&prep.ended);
        assert!(!ended.load(std::sync::atomic::Ordering::SeqCst), "the worker runs");
        drop(doc);
        assert!(
            weak_doc.upgrade().is_none(),
            "the document is freed although the preparation lives"
        );
        drop(prep);
        let t0 = Instant::now();
        while !ended.load(std::sync::atomic::Ordering::SeqCst) && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(weak_prep.upgrade().is_none(), "the preparation is freed");
        assert!(
            ended.load(std::sync::atomic::Ordering::SeqCst),
            "the worker thread has ended"
        );
    }

    #[test]
    fn a_poll_that_says_done_wakes_the_worker_once_for_a_job_not_at_every_answer() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (svc, sid, root) = two_tracks("poll");
        let prep = prep_of(&svc, &sid, &root);
        let seq = |p: &Arc<Prep>| p.lock().wake_seq;
        prep.command_done(true, false); // a command that writes the project
        let s1 = seq(&prep);
        prep.command_done(false, true); // the first `done` of the job
        let s2 = seq(&prep);
        assert_eq!(s2, s1 + 1);
        for _ in 0..5 {
            prep.command_done(false, true); // the page polls every 2 s: no wake, or the quiet time would restart for ever
        }
        assert_eq!(seq(&prep), s2);
        prep.command_done(false, false);
        assert_eq!(seq(&prep), s2);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------ round 3

    /// Tests that start a worker thread and read or set the global stop sign run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    type RowSpec = Arc<Mutex<Vec<(String, String, String)>>>;

    /// A preparation whose rows (id, name, source) the test changes while it runs.
    fn prep_rows(svc: &drumspike_app::Service, sid: &str, root: &Path, specs: &RowSpec) -> Arc<Prep> {
        let page = Arc::new(PageHandle::default());
        page.set(Some(svc.clone()));
        page.set_sid(Some(sid.to_string()));
        let sp = Arc::clone(specs);
        let wavs: HashMap<String, (PathBuf, u32)> = ["lead", "sub"]
            .iter()
            .map(|k| ((*k).to_string(), (root.join("daw").join(format!("{k}.wav")), 48_000)))
            .collect();
        Prep::new(
            page,
            Arc::new(move || Rows {
                rows: sp
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|(id, name, src)| crate::rows::Row {
                        id: id.clone(),
                        name: name.clone(),
                        srcs: vec![src.clone()],
                        clips: vec![clip(src, 0.0, 3.5, 0.0, 1.0)],
                    })
                    .collect(),
                ..Rows::default()
            }),
            Arc::new(|| crate::tempo_ara::choose(None, "test", None)),
            Arc::new(move || wavs.clone()),
        )
    }

    fn spec(id: &str, name: &str, src: &str) -> (String, String, String) {
        (id.into(), name.into(), src.into())
    }

    #[test]
    fn a_new_row_whose_spike_file_has_the_name_of_another_rows_audio_file_never_changes_that_audio_set() {
        let (svc, sid, root) = two_tracks("p1");
        svc.events_set(&sid, "lead", &json!([{"t": 0.25, "v": 100}, {"t": 1.25, "v": 100}]))
            .unwrap();
        svc.events_set(&sid, "sub", &json!([{"t": 0.5, "v": 100}])).unwrap();
        let specs: RowSpec = Arc::new(Mutex::new(vec![spec("q-lead", "Snare", "lead")]));
        let prep = prep_rows(&svc, &sid, &root, &specs);
        prep.lock().used = [true, true];
        prep.cycle(true, true);
        let audio = [("q-lead".to_string(), Kind::Audio)];
        let first = prep.take(&audio).unwrap();
        assert_eq!(first[0].file_name().unwrap(), "Snare_audio_spike.wav");
        let a = std::fs::read(&first[0]).unwrap();
        // a new row "Snare_audio" with the spike kind: its file name is the file name of the audio set of "Snare"
        specs.lock().unwrap().push(spec("q-sub", "Snare_audio", "sub"));
        prep.cycle(true, true);
        let again = prep.take(&audio).unwrap();
        assert_eq!(
            again[0].file_name().unwrap(),
            "Snare (2)_audio_spike.wav",
            "the audio set is renamed: stale, made again"
        );
        assert_eq!(
            std::fs::read(&again[0]).unwrap(),
            a,
            "its content is still the audio, not the spike"
        );
        let spike = prep.take(&[("q-sub".to_string(), Kind::Spike)]).unwrap();
        assert_eq!(
            spike[0].file_name().unwrap(),
            "Snare_audio_spike 2.wav",
            "the delivered name was taken: numbered"
        );
        assert_ne!(std::fs::read(&spike[0]).unwrap(), a);
        assert_eq!(
            std::fs::read(&first[0]).unwrap(),
            a,
            "what was delivered keeps its bytes"
        );
        // the rule itself: a fresh holder is never overwritten, a stale one is dropped
        let mut st = prep.lock();
        let (k, other) = (("q-x".to_string(), Kind::Spike), ("q-y".to_string(), Kind::Audio));
        let out = PathBuf::from("/p/Snare_audio_spike.wav");
        st.ready.insert(
            other.clone(),
            Ready {
                fp: 7,
                prepared: vec![out.clone()],
            },
        );
        st.want.insert(other.clone(), 7);
        assert!(matches!(held_by_other(&st, &k, &out), Held::Fresh));
        st.want.insert(other.clone(), 8);
        assert!(matches!(held_by_other(&st, &k, &out), Held::Stale(_)));
        assert!(
            matches!(held_by_other(&st, &other, &out), Held::No),
            "a set does not hold against itself"
        );
        drop(st);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn when_a_link_fails_the_file_is_copied_never_over_a_file_and_the_same_bytes_are_reused() {
        let d = std::env::temp_dir().join(format!("ds-prep-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("p")).unwrap();
        std::fs::create_dir_all(d.join("o")).unwrap();
        let no_link =
            |_: &Path, _: &Path| -> std::io::Result<()> { Err(std::io::Error::other("no hard links on this volume")) };
        std::fs::write(d.join("p/A_spike.mid"), b"ONE").unwrap();
        let first = deliver_with(&d.join("p/A_spike.mid"), &d.join("o"), &no_link).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), b"ONE");
        assert_eq!(first.file_name().unwrap(), "A_spike.mid");
        // the same bytes again (no inode in common): reused, no second file
        assert_eq!(
            deliver_with(&d.join("p/A_spike.mid"), &d.join("o"), &no_link).unwrap(),
            first
        );
        // new content under the name: a numbered copy, the first copy is not replaced
        std::fs::write(d.join("p/tmp"), b"TWO").unwrap();
        std::fs::rename(d.join("p/tmp"), d.join("p/A_spike.mid")).unwrap();
        let second = deliver_with(&d.join("p/A_spike.mid"), &d.join("o"), &no_link).unwrap();
        assert_eq!(second.file_name().unwrap(), "A_spike 2.mid");
        assert_eq!(std::fs::read(&first).unwrap(), b"ONE");
        assert_eq!(std::fs::read(&second).unwrap(), b"TWO");
        let left: Vec<String> = std::fs::read_dir(d.join("o"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into())
            .collect();
        assert_eq!(left.len(), 2, "no temporary file is left: {left:?}");
        // the copy fails too (the folder cannot be written): an error
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(d.join("ro")).unwrap();
        std::fs::set_permissions(d.join("ro"), std::fs::Permissions::from_mode(0o555)).unwrap();
        let r = deliver_with(&d.join("p/A_spike.mid"), &d.join("ro"), &no_link);
        std::fs::set_permissions(d.join("ro"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(r.is_err(), "{r:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_change_of_the_tempo_wakes_the_worker_and_the_handles_stop_saying_ready() {
        let (svc, sid, root) = two_tracks("tempo");
        let page = Arc::new(PageHandle::default());
        page.set(Some(svc.clone()));
        page.set_sid(Some(sid));
        let bpm = Arc::new(Mutex::new(120.0f64));
        let b = Arc::clone(&bpm);
        let prep = Prep::new(
            page,
            Arc::new(Rows::default),
            Arc::new(move || {
                let mut a = crate::tempo_ara::choose(None, "test", None);
                a.map = Some(TempoMap {
                    bpm: *b.lock().unwrap(),
                    ..TempoMap::default()
                });
                a
            }),
            Arc::new(HashMap::new),
        );
        assert!(!prep.tempo_changed(), "the first look only remembers");
        assert!(!prep.tempo_changed(), "the same tempo");
        *bpm.lock().unwrap() = 97.3;
        assert!(prep.tempo_changed(), "another tempo");
        assert!(!prep.tempo_changed(), "once for a change");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_stop_during_a_long_job_ends_the_worker_within_the_unload_wait() {
        let _serial = serial();
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (svc, sid, root) = two_tracks("stop");
        svc.events_set(&sid, "lead", &json!([{"t": 0.25, "v": 100}])).unwrap();
        let specs: RowSpec = Arc::new(Mutex::new(vec![spec("q-lead", "SLOWJOB", "lead")]));
        let prep = prep_rows(&svc, &sid, &root, &specs);
        let ended = Arc::clone(&prep.ended);
        prep.wake();
        prep.kick();
        let t0 = Instant::now();
        while prep.lock().busy.is_none() && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(prep.lock().busy.is_some(), "the long job runs (it would take 10 s)");
        let t1 = Instant::now();
        let stopped_in_time = stop_all(Duration::from_millis(1500));
        let took = t1.elapsed();
        STOP.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(
            stopped_in_time && took < Duration::from_millis(1500),
            "stop_all returned after {took:?}"
        );
        assert!(ended.load(std::sync::atomic::Ordering::SeqCst), "the worker has ended");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn with_no_export_folder_the_state_the_page_gets_says_so_and_the_memory_cap_is_the_one_of_the_audio_sum() {
        let (svc, sid, root) = two_tracks("nofolder");
        let specs: RowSpec = Arc::new(Mutex::new(vec![spec("q-lead", "Kick", "lead")]));
        let prep = prep_rows(&svc, &sid, &root, &specs);
        {
            let mut st = prep.lock();
            st.cycles = 2;
            st.clean_seq = st.wake_seq;
            st.want.insert(("q-lead".into(), Kind::Midi), 1);
            st.no_folder = true;
        }
        // state_json is what the page gets (the shell command and the event)
        let v = prep.state_json();
        assert_eq!(v["rows"][0]["midi"]["state"], "error");
        assert_eq!(v["rows"][0]["midi"]["why"], "no export folder");
        assert_eq!(
            MEM_CAP_BYTES,
            1024 * 1024 * 1024,
            "used by write_audio_row; tested in the_size_is_estimated_before_anything_is_written..."
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_midi_and_merged_files_of_a_row_are_written_and_the_merged_one_has_one_note_track() {
        let d = std::env::temp_dir().join(format!("ds-prep-mid-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let a = row(
            "q1",
            "Kick",
            vec![clip("x", 0.0, 4.0, 1.0, 1.0)],
            vec![src("x", &[0.5, 1.5], 48_000)],
        );
        let mut b = row(
            "q2",
            "Snare",
            vec![clip("y", 0.0, 4.0, 0.0, 1.0)],
            vec![src("y", &[1.0], 48_000)],
        );
        b.note = 40;
        write_midi_row(&a, &TempoMap::default(), &d.join("a.mid")).unwrap();
        write_merged(&[a, b], &TempoMap::default(), &d.join("m.mid")).unwrap();
        let bytes = std::fs::read(d.join("m.mid")).unwrap();
        assert_eq!(&bytes[..4], b"MThd");
        assert_eq!(
            u16::from_be_bytes([bytes[10], bytes[11]]),
            2,
            "the tempo track and one note track"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
