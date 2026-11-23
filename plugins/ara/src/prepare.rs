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
            Kind::Midi | Kind::Merged => ".mid",
            Kind::Spike => "_spike.wav",
            Kind::Audio => "_audio_spike.wav",
        }
    }
    fn is_wav(self) -> bool {
        matches!(self, Kind::Spike | Kind::Audio)
    }
}

/// The fault runs of the check (`DS_ARA_PREPARE_FAULT`); a delivery build reads none.
fn fault() -> String {
    crate::test_switch("DS_ARA_PREPARE_FAULT").unwrap_or_default()
}

/// The watchdog of this run (`DS_ARA_PREPARE_WAIT_MS` shortens it for the check).
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

/// The file stem of every row: the DAW track name made safe by `deliver::file_stems` (the numbering of #560 over the rows).
/// `MERGED_STEM` takes part under the id `~merged`, so a row of that name is numbered, not overwritten by it.
#[must_use]
pub fn stems(rows: &[RowIn]) -> HashMap<String, String> {
    let mut names: HashMap<String, String> = rows.iter().map(|r| (r.id.clone(), r.name.clone())).collect();
    names.insert("~merged".into(), MERGED_STEM.into());
    let mut out = crate::deliver::file_stems(&names);
    if fault() == "raw-name" {
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
pub fn fingerprint(row: &RowIn, kind: Kind, stem: &str, tempo_key: &str, bump: u64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (kind.word(), stem, bump).hash(&mut h);
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
        (row.note, tempo_key).hash(&mut h);
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
    let rate = row.srcs.first().map_or(48_000, |s| s.rate);
    export_midi_map(&ev, row.note, out, ppq_of(rate, map), map, EXPORT_TEMPO, rate).map_err(|e| e.to_string())
}

/// The merged MIDI file: all rows with hits on ONE note track, each hit on its row's note.
///
/// # Errors
///
/// The text of the failure.
pub fn write_merged(rows: &[RowIn], map: &TempoMap, out: &Path) -> Result<(), String> {
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
            rate: rows[*i].srcs.first().map_or(48_000, |s| s.rate),
            events: e,
        })
        .collect();
    let rate = tracks.iter().map(|t| t.rate).max().unwrap_or(48_000);
    let ppq = ppq_of(rate, map);
    if fault() == "two-tracks" {
        // the fault run: one MIDI track per drum, as the single mode of the dialog
        return export_multi_midi(&tracks, &[], out, ppq, map, EXPORT_TEMPO).map_err(|e| e.to_string());
    }
    export_merged_midi(&tracks, out, ppq, map, EXPORT_TEMPO).map_err(|e| e.to_string())
}

fn home_wav(row: &RowIn) -> Result<&Path, String> {
    row.srcs
        .first()
        .and_then(|s| s.wav.as_deref())
        .ok_or_else(|| format!("no audio file for {}", row.name))
}

/// The synthetic spike file of one row: the times of every source, the rate and format of the first source.
///
/// # Errors
///
/// The text of the failure.
pub fn write_spike_row(row: &RowIn, out: &Path) -> Result<(), String> {
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
    let used: Vec<&SrcIn> = row.srcs.iter().filter(|s| !regions_of(row, &s.key).is_empty()).collect();
    if used.len() <= 1 {
        let s = used.first().copied().or(row.srcs.first()).ok_or("no source")?;
        let wav = s.wav.as_deref().ok_or_else(|| format!("no audio file for {}", row.name))?;
        let pairs = song_pairs(&s.events, &regions_of(row, &s.key));
        return write_audio_spike_file_song(wav, out, &pairs, end)
            .map(|_| ())
            .map_err(|e| e.message);
    }
    let mut parts = Vec::new();
    let mut rate = 0;
    let mut format = WavFormat::Float32;
    for (i, s) in used.iter().enumerate() {
        let wav = s.wav.as_deref().ok_or_else(|| format!("no audio file for {}", s.key))?;
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
    /// The files in `Exports/` (the paths a drag takes).
    files: Vec<PathBuf>,
    /// The files in `.prepared/`.
    prepared: Vec<PathBuf>,
}

#[derive(Default)]
struct St {
    first: Option<Instant>,
    last: Option<Instant>,
    used: [bool; 2],
    want: HashMap<Key, u64>,
    ready: HashMap<Key, Ready>,
    failed: HashMap<Key, (u64, String)>,
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
}

struct Gathered {
    rows: Vec<RowIn>,
    map: TempoMap,
    tempo_key: String,
    stems: HashMap<String, String>,
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
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, St> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Something changed (an edit, a round, a row, the tempo): the worker looks again after the quiet time.
    pub fn wake(self: &Arc<Self>) {
        {
            let mut st = self.lock();
            let now = Instant::now();
            st.first.get_or_insert(now);
            st.last = Some(now);
            st.bump += u64::from(fault() == "all-rows");
            st.midi_pending = true;
            st.wav_pending = true;
            if !st.started {
                st.started = true;
                let weak = Arc::downgrade(self);
                let _ = std::thread::Builder::new()
                    .name("drumspike-ara-prepare".into())
                    .spawn(move || worker(&weak));
            }
        }
        self.cv.notify_all();
    }

    /// A press on the handle of a wav kind: the first use. The work is due at once.
    pub fn want_kind(self: &Arc<Self>, kind: Kind) {
        {
            let mut st = self.lock();
            match kind {
                Kind::Spike => st.used[0] = true,
                Kind::Audio => st.used[1] = true,
                _ => {}
            }
            st.failed.retain(|k, _| k.1 != kind);
        }
        self.wake();
        let mut st = self.lock();
        let ago = Instant::now().checked_sub(Duration::from_secs(60));
        st.first = ago;
        st.last = ago;
    }

    fn exports_dir(&self) -> Option<PathBuf> {
        let svc = self.page.service()?;
        let sid = self.page.sid()?;
        svc.drag_dir(&sid).ok().and_then(|d| d.parent().map(Path::to_path_buf))
    }

    fn gather(&self) -> Option<Gathered> {
        let svc = self.page.service()?;
        let sid = self.page.sid()?;
        let rows = (self.rows)();
        let tempo = (self.tempo)();
        let files = (self.files)();
        // the same follower refresh as the export (R4): `tracks_get` derives a follower whose hits went stale; it also gives the notes
        let tracks = if fault() == "no-refresh" {
            None
        } else {
            svc.tracks_get(&sid).ok()
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
        if fault() == "per-source" {
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
        let map = tempo.map.clone().unwrap_or_default();
        let tempo_key = format!("{:?}", tempo.map);
        Some(Gathered {
            rows: out,
            map,
            tempo_key,
            stems,
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
            let stem = &g.stems[&r.id];
            let m = fingerprint(r, Kind::Midi, stem, &g.tempo_key, bump);
            m.hash(&mut merged);
            out.insert((r.id.clone(), Kind::Midi), m);
            if used[0] {
                out.insert((r.id.clone(), Kind::Spike), fingerprint(r, Kind::Spike, stem, "", bump));
            }
            if used[1] {
                out.insert((r.id.clone(), Kind::Audio), fingerprint(r, Kind::Audio, stem, "", bump));
            }
        }
        if any {
            g.stems["~merged"].hash(&mut merged);
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
        let want = if has_hits { st.want.get(key).copied() } else { None };
        // before the first cycle nothing is known: a row has hits until a cycle says it has none
        let want = if has_hits && want.is_none() && st.cycles == 0 { Some(0) } else { want };
        let ready = st.ready.get(key).map(|r| (r.fp, r.files.len()));
        let failed = st.failed.get(key).filter(|(fp, _)| Some(*fp) == want).map(|(_, w)| w.as_str());
        let mut pending = st.pending_since.get(key).map(|t| t.elapsed());
        if let Some((bk, since)) = &st.busy {
            if bk == key {
                pending = Some(since.elapsed());
            }
        }
        state_of(key.1, used, want, ready, failed, pending, watchdog())
    }

    /// The answer of `plugin_deliver_state` and the payload of the page event `plugin-deliver`.
    #[must_use]
    pub fn state_json(&self) -> Value {
        let rows = (self.rows)();
        let st = self.lock();
        let row_has = |id: &str| st.cycles == 0 || st.want.contains_key(&(id.to_string(), Kind::Midi));
        let items: Vec<Value> = rows
            .rows
            .iter()
            .map(|r| {
                let has = row_has(&r.id);
                let k = |kind: Kind| self.state_for(&st, &(r.id.clone(), kind), has).to_json();
                json!({"id": r.id, "name": r.name, "srcs": r.srcs, "midi": k(Kind::Midi), "spike": k(Kind::Spike), "audio": k(Kind::Audio)})
            })
            .collect();
        let any = rows.rows.iter().any(|r| row_has(&r.id));
        let merged = self.state_for(&st, &(String::new(), Kind::Merged), any).to_json();
        json!({"gen": st.gen, "rows": items, "merged": merged,
               "used": {"spike": st.used[0], "audio": st.used[1]},
               "hint": "Drop at the start of the session"})
    }

    fn push(&self) {
        let v = self.state_json();
        let mut st = self.lock();
        let text = {
            let mut c = v.clone();
            c["gen"] = Value::Null;
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

    /// The files of the items, when every one of them is ready for the CURRENT state of the events.
    ///
    /// # Errors
    ///
    /// `(status, text)`: 409 with the state of the first item that is not ready, 413 over the bound of a drag.
    pub fn take(self: &Arc<Self>, items: &[(String, Kind)]) -> Result<Vec<PathBuf>, (u16, String)> {
        let Some(g) = self.gather() else {
            return Err((503, "the document is closed".into()));
        };
        let fps = self.fps(&g);
        let mut out: Vec<PathBuf> = Vec::new();
        {
            let st = self.lock();
            for (id, kind) in items {
                let key = (id.clone(), *kind);
                let ok = match (fps.get(&key), st.ready.get(&key)) {
                    (Some(want), Some(r)) => *want == r.fp || fault() == "stale-ok",
                    _ => false,
                };
                if !ok {
                    let why = match self.state_for_fps(&st, &key, &fps) {
                        State::Error(w) => format!("error: {w}"),
                        State::Off => "not prepared yet".into(),
                        State::None => "no hits".into(),
                        _ => "preparing".into(),
                    };
                    drop(st);
                    self.wake();
                    return Err((409, format!("{}: {why}", self.name_of(&g, id))));
                }
                out.extend(st.ready[&key].files.iter().cloned());
            }
        }
        if out.len() > MAX_DRAG_FILES {
            return Err((413, format!("error: more than {MAX_DRAG_FILES} files in one drag")));
        }
        Ok(out)
    }

    fn name_of(&self, g: &Gathered, id: &str) -> String {
        if id.is_empty() {
            return "Merged MIDI".into();
        }
        g.rows.iter().find(|r| r.id == id).map_or(id.to_string(), |r| r.name.clone())
    }

    fn state_for_fps(&self, st: &St, key: &Key, fps: &HashMap<Key, u64>) -> State {
        let used = match key.1 {
            Kind::Spike => st.used[0],
            Kind::Audio => st.used[1],
            _ => true,
        };
        let want = fps.get(key).copied();
        let ready = st.ready.get(key).map(|r| (r.fp, r.files.len()));
        let failed = st.failed.get(key).filter(|(fp, _)| Some(*fp) == want).map(|(_, w)| w.as_str());
        state_of(key.1, used, want, ready, failed, None, watchdog())
    }

    /// Counters for the check: (cycles, jobs).
    #[must_use]
    pub fn counters(&self) -> (u64, u64) {
        let st = self.lock();
        (st.cycles, st.jobs)
    }

    // one pass: look at the rows, set the wanted fps, make what is due
    fn cycle(self: &Arc<Self>, midi_due: bool, wav_due: bool) {
        let Some(g) = self.gather() else { return };
        let fps = self.fps(&g);
        let Some(exports) = self.exports_dir() else { return };
        let prepared = exports.join(PREPARED_DIR);
        {
            let mut st = self.lock();
            st.cycles += 1;
            st.meta = g.rows.iter().map(|r| (r.id.clone(), r.name.clone(), r.srcs.iter().map(|s| s.key.clone()).collect())).collect();
            for (k, fp) in &fps {
                let fresh = st.ready.get(k).is_some_and(|r| r.fp == *fp);
                if fresh {
                    st.pending_since.remove(k);
                } else if st.want.get(k) != Some(fp) || !st.pending_since.contains_key(k) {
                    st.pending_since.entry(k.clone()).or_insert_with(Instant::now);
                }
            }
            st.want = fps.clone();
            st.pending_since.retain(|k, _| fps.contains_key(k));
            // sets of rows that are gone or kinds that are off: removed from the books, their files below
            st.ready.retain(|k, _| fps.contains_key(k));
            st.failed.retain(|k, _| fps.contains_key(k));
        }
        self.push();
        let keep: HashSet<PathBuf> = {
            let st = self.lock();
            st.ready.values().flat_map(|r| r.prepared.iter().cloned()).collect()
        };
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
                !st.ready.get(&key).is_some_and(|r| r.fp == fp) && !st.failed.get(&key).is_some_and(|(f, _)| *f == fp)
            };
            if !due || !need {
                continue;
            }
            self.job(&g, &key, fps[&key], &exports, &prepared);
        }
        self.push();
    }

    fn job(self: &Arc<Self>, g: &Gathered, key: &Key, fp: u64, exports: &Path, prepared: &Path) {
        {
            let mut st = self.lock();
            st.busy = Some((key.clone(), Instant::now()));
            st.jobs += 1;
        }
        self.push();
        if fault() == "forever" {
            // the fault run: the job never ends; the state must say so after the watchdog
            while Arc::strong_count(self) > 1 {
                std::thread::sleep(Duration::from_millis(100));
            }
            return;
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| make(g, key, exports, prepared)))
            .unwrap_or_else(|_| Err("the preparation panicked".into()));
        let mut st = self.lock();
        st.busy = None;
        match r {
            Ok((prep, fin)) => {
                st.failed.remove(key);
                st.pending_since.remove(key);
                st.ready.insert(
                    key.clone(),
                    Ready {
                        fp,
                        files: fin,
                        prepared: prep,
                    },
                );
            }
            Err(why) => {
                st.failed.insert(key.clone(), (fp, why));
            }
        }
        drop(st);
        self.push();
    }
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

/// Make the set of `key`: write it to `.prepared/` (temporary name, then a rename), then link it into `Exports/` under the same
/// name. Returns the prepared and the final paths.
fn make(g: &Gathered, key: &Key, exports: &Path, prepared: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    std::fs::create_dir_all(prepared).map_err(|e| format!("{}: {e}", prepared.display()))?;
    let row = g.rows.iter().find(|r| r.id == key.0);
    let stem = g.stems.get(&key.0).cloned().or_else(|| (key.1 == Kind::Merged).then(|| g.stems["~merged"].clone()));
    let stem = stem.ok_or("the row is gone")?;
    let name = file_name(&stem, key.1);
    // the name is one safe component (`file_stems`): nothing of the row name can leave the folder
    let out = prepared.join(&name);
    let tmp = prepared.join(format!(".{name}.tmp"));
    match (key.1, row) {
        (Kind::Midi, Some(r)) => write_midi_row(r, &g.map, &tmp)?,
        (Kind::Spike, Some(r)) => write_spike_row(r, &tmp)?,
        (Kind::Audio, Some(r)) => write_audio_row(r, &tmp)?,
        (Kind::Merged, _) => write_merged(&g.rows, &g.map, &tmp)?,
        _ => return Err("the row is gone".into()),
    }
    let size = std::fs::metadata(&tmp).map_or(0, |m| m.len());
    let others = dir_size(prepared).saturating_sub(std::fs::metadata(&out).map_or(0, |m| m.len()));
    if others + size > CAP_BYTES {
        let _ = std::fs::remove_file(&tmp);
        return Err("prepared files exceed 2 GB".into());
    }
    std::fs::rename(&tmp, &out).map_err(|e| format!("{}: {e}", out.display()))?;
    let fin = exports.join(&name);
    if fin.parent() != Some(exports) {
        return Err("the file name leaves the folder".into());
    }
    let _ = std::fs::remove_file(&fin);
    if std::fs::hard_link(&out, &fin).is_err() {
        std::fs::copy(&out, &fin).map_err(|e| format!("{}: {e}", fin.display()))?;
    }
    let fin = std::fs::canonicalize(&fin).unwrap_or(fin);
    Ok((vec![out], vec![fin]))
}

fn worker(weak: &Weak<Prep>) {
    loop {
        std::thread::sleep(Duration::from_millis(150));
        let Some(p) = weak.upgrade() else { return };
        let (midi, wav) = {
            let st = p.lock();
            let (Some(first), Some(last)) = (st.first, st.last) else { continue };
            let now = Instant::now();
            let quiet = now.duration_since(last);
            let waited = now.duration_since(first);
            let m = st.midi_pending && (quiet >= Duration::from_millis(quiet_ms(false)) || waited >= Duration::from_secs(5));
            let w = st.wav_pending && (quiet >= Duration::from_millis(quiet_ms(true)) || waited >= Duration::from_secs(10));
            (m, w)
        };
        if !(midi || wav) || p.page.job_running() {
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
        }
        p.cycle(midi, wav);
    }
}

fn quiet_ms(wav: bool) -> u64 {
    let base = crate::test_switch("DS_ARA_PREPARE_QUIET_MS").ok().and_then(|v| v.parse::<u64>().ok());
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
        let a = row("q1", "A", vec![clip("x", 0.0, 2.0, 0.0, 1.0)], vec![src("x", &[1.0, 5.0], 48_000)]);
        let b = row("q2", "B", vec![clip("x", 4.0, 6.0, 8.0, 1.0)], vec![src("x", &[1.0, 5.0], 48_000)]);
        assert_eq!(mapped_events(&a).len(), 1);
        assert_eq!(mapped_events(&b)[0]["t"].as_f64(), Some(9.0));
    }

    #[test]
    fn a_stretched_clip_follows_its_factor() {
        let r = row("q1", "A", vec![clip("x", 0.0, 2.0, 1.0, 2.0)], vec![src("x", &[1.0], 48_000)]);
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
            assert!(!stem.contains('/') && !stem.starts_with('.') && !stem.is_empty(), "{stem}");
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
        let base = row("q1", "A", vec![clip("x", 0.0, 2.0, 0.0, 1.0)], vec![src("x", &[0.5], 48_000)]);
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
        assert_eq!(fp(&vel, Kind::Spike, ""), fp(&base, Kind::Spike, ""), "but not the spike file");
        assert_ne!(fp(&base, Kind::Midi, "u"), m0, "the tempo");
        let mut note = base.clone();
        note.note = 40;
        assert_ne!(fp(&note, Kind::Midi, "t"), m0, "the note");
        assert_ne!(fingerprint(&base, Kind::Midi, "B", "t", 0), m0, "the stem");
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
        assert_eq!(check_rates(&ok), Ok(()), "a source with no clip of the row does not count");
    }

    #[test]
    fn the_sum_of_two_signals_adds_sample_by_sample_and_keeps_the_longer_length() {
        assert_eq!(sum_signals(&[vec![1.0, 0.0, 0.5], vec![0.0, 2.0]]), vec![1.0, 2.0, 0.5]);
    }

    #[test]
    fn the_state_of_a_handle_follows_the_fp_and_never_stays_preparing() {
        let lim = Duration::from_secs(30);
        let st = |used, want, ready, failed: Option<&str>, pend| state_of(Kind::Spike, used, want, ready, failed, pend, lim);
        assert_eq!(st(true, None, None, None, None), State::None);
        assert_eq!(st(false, Some(1), None, None, None), State::Off);
        assert_eq!(st(true, Some(1), Some((1, 1)), None, None), State::Ready(1));
        assert_eq!(st(true, Some(2), Some((1, 1)), None, None), State::Preparing, "an older set is not ready");
        assert_eq!(st(true, Some(2), Some((1, 1)), Some("no space"), None), State::Error("no space".into()));
        assert!(matches!(st(true, Some(2), None, None, Some(Duration::from_secs(31))), State::Error(_)), "the watchdog");
        assert_eq!(st(true, Some(2), None, None, Some(Duration::from_secs(5))), State::Preparing);
    }

    #[test]
    fn a_drag_of_more_than_the_bound_is_refused_with_a_reason() {
        assert_eq!(MAX_DRAG_FILES, 512);
        assert!(crate::deliver::drag_count_ok(512));
        assert!(!crate::deliver::drag_count_ok(513));
        assert!(!crate::deliver::drag_count_ok(0));
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

    #[test]
    fn the_midi_and_merged_files_of_a_row_are_written_and_the_merged_one_has_one_note_track() {
        let d = std::env::temp_dir().join(format!("ds-prep-mid-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let a = row("q1", "Kick", vec![clip("x", 0.0, 4.0, 1.0, 1.0)], vec![src("x", &[0.5, 1.5], 48_000)]);
        let mut b = row("q2", "Snare", vec![clip("y", 0.0, 4.0, 0.0, 1.0)], vec![src("y", &[1.0], 48_000)]);
        b.note = 40;
        write_midi_row(&a, &TempoMap::default(), &d.join("a.mid")).unwrap();
        write_merged(&[a, b], &TempoMap::default(), &d.join("m.mid")).unwrap();
        let bytes = std::fs::read(d.join("m.mid")).unwrap();
        assert_eq!(&bytes[..4], b"MThd");
        assert_eq!(u16::from_be_bytes([bytes[10], bytes[11]]), 2, "the tempo track and one note track");
        let _ = std::fs::remove_dir_all(&d);
    }
}
