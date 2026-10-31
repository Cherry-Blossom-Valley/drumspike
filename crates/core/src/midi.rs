//! MIDI export: `engine.export_midi` reproduced byte for byte.
//!
//! The engine builds a `pretty_midi.PrettyMIDI(resolution=ppq, initial_tempo=tempo)` with
//! one drum instrument named `spikes` and one 50 ms note per event, and writes it through
//! mido. This module produces the same file:
//!
//! * format 1, two tracks, `ppq` ticks per quarter note;
//! * track 0: `set_tempo` and `time_signature` 4/4 at tick 0 (in that order, `pretty_midi`
//!   sorts by event type), `end_of_track` one tick later;
//! * track 1: `track_name` "spikes", `program_change` 0 on channel 10, per event a
//!   `note_on` with the velocity and a `note_on` with velocity 0 at `t + 0.05 s`, all sorted
//!   by tick then type, note and velocity (a note off sorts before a note on at the same
//!   tick), `end_of_track` one tick after the last event; running status as mido writes it.
//!
//! Ticks follow `PrettyMIDI.time_to_tick` on a fresh object: `tick_scale = 60 / (tempo * ppq)`,
//! `tick = round(t / tick_scale)` with Python's ties-to-even, 0 for `t <= 0`. At 24,000 PPQ
//! and 120 BPM one tick is one sample at 48 kHz.
//!
//! A standard MIDI file stores the time between two events as a variable-length quantity
//! of at most 28 bits (`midly`'s `u28`); mido writes longer quantities without a limit. A
//! file whose every delta fits (any gap shorter than about 93 minutes at 24,000 PPQ) is
//! written with `midly`; a file with a longer gap is written by `write_smf`, a small
//! writer that encodes the same events with mido's unbounded quantity, so Rust and Python
//! write the same bytes in both cases (the unit tests prove that both writers agree).

use std::path::Path;

use midly::num::{u4, u7, u15, u24, u28};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use serde::{Deserialize, Serialize};

use crate::pyops::py_round;
use crate::tempo::{BAR1_MARKER, TempoMap, TickMap, tempo_us};

/// Default ticks per quarter note (`engine.export_midi(ppq=24000)`).
pub const DEFAULT_PPQ: u16 = 24_000;
/// Default tempo in BPM (`engine.export_midi(tempo=120.0)`).
pub const DEFAULT_TEMPO: f64 = 120.0;
/// Note length in seconds (`end = t + 0.05`).
pub const NOTE_LENGTH_S: f64 = 0.05;
/// Instrument name of the single track.
pub const TRACK_NAME: &str = "spikes";
/// MIDI channel of a drum instrument (channel 10, zero-based 9).
pub const DRUM_CHANNEL: u8 = 9;
/// Default velocity when an event has none (`e.get("v", 100)`).
pub const DEFAULT_VELOCITY: i64 = 100;
/// Largest delta a standard MIDI file can hold (28-bit variable-length quantity): the
/// limit of the `midly` writer. A longer delta goes through `write_smf`.
pub const MAX_DELTA: u64 = (1 << 28) - 1;
/// Largest tick an export accepts (`2^62`): far beyond any session, it keeps the tick
/// arithmetic inside `i64`. Python has no limit; a time this large is not audio.
pub const MAX_TICK: i64 = 1 << 62;
/// Largest time a map export accepts in seconds (a day): keeps the tick arithmetic exact.
pub const MAX_SECONDS: f64 = 86_400.0;

/// One event to export: time in seconds and velocity (`{"t": s, "v": int}`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MidiEvent {
    /// Onset in seconds.
    pub t: f64,
    /// Velocity 0..=127 (mido rejects anything else).
    pub v: i64,
}

/// A note as the file stores it: the tuple invariant I4 compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteEvent {
    /// Absolute tick of the note on.
    pub tick: u64,
    /// Note number.
    pub note: u8,
    /// Velocity.
    pub velocity: u8,
    /// Ticks until the note off.
    pub duration: u64,
}

/// Why an export is refused: the same values mido rejects, plus a time that is not finite
/// or beyond [`MAX_TICK`].
#[derive(Debug, Clone, PartialEq)]
pub enum MidiError {
    /// Velocity outside 0..=127.
    Velocity(i64),
    /// Note number outside 0..=127.
    Note(i64),
    /// A time that gives no tick (`t` not finite or beyond `MAX_TICK` ticks).
    Tick(f64),
    /// `ppq` must be 1..=32767 (15 bits) and `tempo` positive and finite.
    Timing {
        /// The pulses per quarter note asked for.
        ppq: u16,
        /// The tempo asked for, in beats per minute.
        tempo: f64,
    },
    /// The tempo map cannot be written (P4-8): the message names the fault.
    Map(String),
    /// Writing the file failed: the error kind (so the binding can raise the matching
    /// `OSError` subclass) and the message.
    Io(std::io::ErrorKind, String),
}

impl std::fmt::Display for MidiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MidiError::Velocity(v) => write!(f, "velocity {v} is not in 0..=127"),
            MidiError::Note(n) => write!(f, "note {n} is not in 0..=127"),
            MidiError::Tick(t) => write!(f, "time {t} s does not fit a MIDI file ({MAX_TICK} ticks)"),
            MidiError::Timing { ppq, tempo } => write!(f, "bad timing: ppq {ppq}, tempo {tempo}"),
            MidiError::Map(e) => write!(f, "tempo map: {e}"),
            MidiError::Io(_, e) => write!(f, "write failed: {e}"),
        }
    }
}

impl std::error::Error for MidiError {}

impl From<std::io::Error> for MidiError {
    fn from(e: std::io::Error) -> Self {
        MidiError::Io(e.kind(), e.to_string())
    }
}

/// Seconds per tick, as `PrettyMIDI.__init__` stores it: `60.0 / (initial_tempo * resolution)`.
pub fn tick_scale(ppq: u16, tempo: f64) -> f64 {
    60.0 / (tempo * f64::from(ppq))
}

/// `PrettyMIDI.time_to_tick(t)` on a fresh object: 0 for `t <= 0`, else `round(t / tick_scale)`
/// with ties to even.
pub fn time_to_tick(t: f64, tick_scale: f64) -> i64 {
    if t <= 0.0 { 0 } else { py_round(t / tick_scale) }
}

/// The `set_tempo` value `pretty_midi` writes: `int(6e7 / (60 / (tick_scale * ppq)))` in
/// microseconds per quarter note, with the same floating-point steps.
pub fn tempo_meta(ppq: u16, tempo: f64) -> u32 {
    let ts = tick_scale(ppq, tempo);
    let bpm = 60.0 / (ts * f64::from(ppq));
    (6e7 / bpm) as u32
}

fn check_timing(ppq: u16, tempo: f64) -> Result<(), MidiError> {
    if ppq == 0 || ppq > 0x7FFF || tempo <= 0.0 || tempo.is_nan() || !tempo.is_finite() {
        return Err(MidiError::Timing { ppq, tempo });
    }
    Ok(())
}

/// The note tuples `(tick, note, velocity, duration)` of an export, one per event in input
/// order: what `engine.export_midi` writes and what `pretty_midi` reads back.
pub fn note_events(events: &[MidiEvent], note: i64, ppq: u16, tempo: f64) -> Result<Vec<NoteEvent>, MidiError> {
    check_timing(ppq, tempo)?;
    let key = u8::try_from(note)
        .ok()
        .filter(|k| *k <= 127)
        .ok_or(MidiError::Note(note))?;
    let ts = tick_scale(ppq, tempo);
    events
        .iter()
        .map(|e| {
            let vel = u8::try_from(e.v)
                .ok()
                .filter(|v| *v <= 127)
                .ok_or(MidiError::Velocity(e.v))?;
            let on = time_to_tick(e.t, ts);
            let off = time_to_tick(e.t + NOTE_LENGTH_S, ts);
            // a NaN or infinite t, or a tick that saturates i64, has no place in the file
            if !e.t.is_finite() || off > MAX_TICK || on > MAX_TICK {
                return Err(MidiError::Tick(e.t));
            }
            Ok(NoteEvent {
                tick: on as u64,
                note: key,
                velocity: vel,
                duration: (off - on) as u64,
            })
        })
        .collect()
}

/// A channel message of track 1 before the delta encoding, with `pretty_midi`'s sort key.
#[derive(Debug, Clone, Copy)]
struct Msg {
    tick: u64,
    /// `event_compare` secondary score: track name 0 (stable, first), program change
    /// `6 * 65536`, note on `10 * 65536 + note * 256 + velocity`.
    score: i64,
    kind: Kind,
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    TrackName,
    Program,
    NoteOn { key: u8, vel: u8 },
}

/// An event of the tempo track (track 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Meta0 {
    /// `set_tempo`: microseconds per quarter note.
    Tempo(u32),
    /// `time_signature`: beats and the power of two of the note value (24 clocks, 8 32nds).
    Signature(u8, u8),
    /// `marker` text.
    Marker(String),
}

impl Meta0 {
    /// `pretty_midi`'s order of events at one tick: tempo, then time signature (then the marker).
    fn rank(&self) -> u8 {
        match self {
            Meta0::Tempo(_) => 0,
            Meta0::Signature(..) => 1,
            Meta0::Marker(_) => 2,
        }
    }
}

/// The tempo track of every export before P4-8: `set_tempo` and 4/4 at tick 0.
fn default_track0(tempo_us: u32) -> Vec<(u64, Meta0)> {
    vec![(0, Meta0::Tempo(tempo_us)), (0, Meta0::Signature(4, 2))]
}

/// The file's form of a single tempo (the default map, #434): a one-segment [`TickMap`] through
/// the whole microseconds per quarter note the file writes. Since #466 the ticks follow the
/// exact tempo of the map (the user's number) while `set_tempo` stays the nearest whole
/// microsecond: a DAW running the user's exact tempo sees every note within the tick rounding,
/// and a DAW that applies the file's whole-microsecond `set_tempo` sees at most the relative
/// mismatch of that rounding (about 1.3e-7, 2 samples over 5 minutes at 48 kHz), which no SMF
/// can avoid for a tempo whose quarter is not a whole number of microseconds. The old engine
/// computed the ticks from a truncated written tempo, which drifted the notes 16 ticks over 6
/// minutes at 118 BPM; the #434 integer anchoring put the same drift on the DAW side instead.
fn single_tick_map(ppq: u16, tempo: f64) -> Result<TickMap, MidiError> {
    check_timing(ppq, tempo)?;
    let map = TempoMap {
        bpm: tempo,
        ..TempoMap::default()
    };
    TickMap::new(ppq, &map).map_err(MidiError::Map)
}

/// The bytes `engine.export_midi(events, note, path, ppq, tempo)` writes.
pub fn midi_bytes(events: &[MidiEvent], note: i64, ppq: u16, tempo: f64) -> Result<Vec<u8>, MidiError> {
    let notes = note_events(events, note, ppq, tempo)?;
    let tempo_us = u24::try_from(tempo_meta(ppq, tempo)).ok_or(MidiError::Timing { ppq, tempo })?;
    write_file(&track1_messages(&notes), ppq, &default_track0(tempo_us.as_int()))
}

/// Write the two tracks: `midly` when every delta fits 28 bits, else `write_smf`.
fn write_file(msgs: &[Msg], ppq: u16, track0: &[(u64, Meta0)]) -> Result<Vec<u8>, MidiError> {
    let mut last = 0u64;
    let fits = msgs.iter().map(|m| m.tick).all(|t| {
        let d = t - last;
        last = t;
        d <= MAX_DELTA
    }) && {
        let mut last = 0u64;
        track0.iter().all(|(t, _)| {
            let d = t - last;
            last = *t;
            d <= MAX_DELTA
        })
    };
    if fits {
        midly_bytes(msgs, ppq, track0)
    } else {
        Ok(write_smf(msgs, ppq, track0))
    }
}

/// The note tuples of an export through a tempo map: like [`note_events`], with the ticks of
/// `tm` (the file's own tempo events) instead of one tempo.
pub fn note_events_tm(events: &[MidiEvent], note: i64, tm: &TickMap) -> Result<Vec<NoteEvent>, MidiError> {
    let key = u8::try_from(note)
        .ok()
        .filter(|k| *k <= 127)
        .ok_or(MidiError::Note(note))?;
    events
        .iter()
        .map(|e| {
            let vel = u8::try_from(e.v)
                .ok()
                .filter(|v| *v <= 127)
                .ok_or(MidiError::Velocity(e.v))?;
            // a NaN or infinite t, or a tick that saturates i64, has no place in the file
            let on = tm.tick(e.t);
            let off = tm.tick(e.t + NOTE_LENGTH_S);
            if !e.t.is_finite() || off > MAX_TICK || on > MAX_TICK {
                return Err(MidiError::Tick(e.t));
            }
            Ok(NoteEvent {
                tick: on as u64,
                note: key,
                velocity: vel,
                duration: (off - on) as u64,
            })
        })
        .collect()
}

/// The tempo track of a map: tempo events at their ticks, the time signatures, and the marker
/// `Bar 1` when bar 1 is after tick 0; sorted by tick, then tempo before signature before marker.
fn map_track0(tm: &TickMap, map: &TempoMap) -> Result<Vec<(u64, Meta0)>, MidiError> {
    let mut out: Vec<(u64, Meta0)> = tm
        .tempo_events()
        .into_iter()
        .map(|(t, us)| (t as u64, Meta0::Tempo(us)))
        .collect();
    for (t, n, d) in tm.meter_events(map).map_err(MidiError::Map)? {
        out.push((t as u64, Meta0::Signature(n, d.trailing_zeros() as u8)));
    }
    let t1 = tm.bar1_tick(map);
    if t1 > 0 {
        out.push((t1 as u64, Meta0::Marker(BAR1_MARKER.to_string())));
    }
    out.sort_by_key(|(t, m)| (*t, m.rank()));
    Ok(out)
}

/// The bytes of the export through a tempo map (P4-8). The default map writes exactly the bytes
/// of [`midi_bytes`] at `legacy_tempo` (the golden and frozen files); any other map writes the
/// tempo, the time signatures and the `Bar 1` marker in the tempo track and each note at its
/// time through the map ([`TickMap`]); then `legacy_tempo` is not used.
pub fn midi_bytes_map(
    events: &[MidiEvent],
    note: i64,
    ppq: u16,
    map: &TempoMap,
    legacy_tempo: f64,
) -> Result<Vec<u8>, MidiError> {
    if map.is_default() {
        let tm = single_tick_map(ppq, legacy_tempo)?;
        let notes = note_events_tm(events, note, &tm)?;
        let us = tempo_us(legacy_tempo).map_err(MidiError::Map)?;
        return write_file(&track1_messages(&notes), ppq, &default_track0(us));
    }
    let tm = TickMap::new(ppq, map).map_err(MidiError::Map)?;
    let notes = note_events_tm(events, note, &tm)?;
    let track0 = map_track0(&tm, map)?;
    write_file(&track1_messages(&notes), ppq, &track0)
}

/// [`midi_bytes_map`] written to `path`.
pub fn export_midi_map(
    events: &[MidiEvent],
    note: i64,
    path: &Path,
    ppq: u16,
    map: &TempoMap,
    legacy_tempo: f64,
) -> Result<(), MidiError> {
    crate::io::write_atomic(path, &midi_bytes_map(events, note, ppq, map, legacy_tempo)?)?;
    Ok(())
}

/// One drum track of a multi-track file (P4-30): its name, its note and its hits.
#[derive(Debug, Clone, Copy)]
pub struct MidiTrack<'a> {
    /// The MIDI track name (the drum track's name).
    pub name: &'a str,
    /// The note of every hit of this track.
    pub note: i64,
    /// The hits.
    pub events: &'a [MidiEvent],
}

/// A review marker of the export (P4-40g): its time in seconds and the optional note text.
#[derive(Debug, Clone, Copy)]
pub struct MidiMarker<'a> {
    /// The time in seconds (through the export's tempo, as the notes are).
    pub t: f64,
    /// The note as the marker text; `None` writes `Marker N` (N from 1, in the order given).
    pub note: Option<&'a str>,
}

/// The text of marker `i`: the note, or `Marker N` (N from 1).
fn marker_text(m: &MidiMarker, i: usize) -> String {
    m.note.map_or_else(|| format!("Marker {}", i + 1), str::to_string)
}

/// The review markers for an export through a tempo map: one `marker` per marker at the tick of
/// `tm`. A time that is not finite or beyond [`MAX_SECONDS`] is refused.
fn marker_events_tm(markers: &[MidiMarker], tm: &TickMap) -> Result<Vec<(u64, Meta0)>, MidiError> {
    markers
        .iter()
        .enumerate()
        .map(|(i, m)| {
            if !m.t.is_finite() || m.t > MAX_SECONDS {
                return Err(MidiError::Tick(m.t));
            }
            Ok((tm.tick(m.t) as u64, Meta0::Marker(marker_text(m, i))))
        })
        .collect()
}

/// P4-30: the bytes of one type 1 file with a tempo track (tempo, meter and `Bar 1` marker from `map`; the
/// default map writes `legacy_tempo` and 4/4 as every other export) and one MIDI track per drum track,
/// each on its own note, channel 10, with the same note length and ticks as the per-track files.
/// P4-40g: one `marker` event per review marker in the tempo track, at its time through the map.
/// A time distance beyond 28 bits (about 93 minutes at 24,000 PPQ) is refused with [`MidiError::Tick`].
pub fn multi_midi_bytes(
    tracks: &[MidiTrack],
    markers: &[MidiMarker],
    ppq: u16,
    map: &TempoMap,
    legacy_tempo: f64,
) -> Result<Vec<u8>, MidiError> {
    let (per_track, mut track0) = if map.is_default() {
        let tm = single_tick_map(ppq, legacy_tempo)?;
        let us = tempo_us(legacy_tempo).map_err(MidiError::Map)?;
        let notes = tracks
            .iter()
            .map(|t| note_events_tm(t.events, t.note, &tm))
            .collect::<Result<Vec<_>, _>>()?;
        let mut t0 = default_track0(us);
        t0.extend(marker_events_tm(markers, &tm)?);
        (notes, t0)
    } else {
        let tm = TickMap::new(ppq, map).map_err(MidiError::Map)?;
        let notes = tracks
            .iter()
            .map(|t| note_events_tm(t.events, t.note, &tm))
            .collect::<Result<Vec<_>, _>>()?;
        let mut t0 = map_track0(&tm, map)?;
        t0.extend(marker_events_tm(markers, &tm)?);
        (notes, t0)
    };
    track0.sort_by_key(|(t, m)| (*t, m.rank()));
    let mut smf = Smf::new(Header::new(Format::Parallel, Timing::Metrical(u15::new(ppq))));
    smf.tracks.push(meta_track(&track0)?);
    let channel = u4::new(DRUM_CHANNEL);
    for (t, notes) in tracks.iter().zip(&per_track) {
        let mut out = Vec::new();
        let mut last = 0u64;
        for m in track1_messages(notes) {
            let delta = u28::try_from(u32::try_from(m.tick - last).unwrap_or(u32::MAX))
                .ok_or(MidiError::Tick(m.tick as f64))?;
            last = m.tick;
            let kind = match m.kind {
                Kind::TrackName => TrackEventKind::Meta(MetaMessage::TrackName(t.name.as_bytes())),
                Kind::Program => TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::ProgramChange { program: u7::new(0) },
                },
                Kind::NoteOn { key, vel } => TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOn {
                        key: u7::new(key),
                        vel: u7::new(vel),
                    },
                },
            };
            out.push(TrackEvent { delta, kind });
        }
        out.push(TrackEvent {
            delta: u28::new(1),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        });
        smf.tracks.push(out);
    }
    let mut bytes = Vec::new();
    smf.write_std(&mut bytes)?;
    Ok(bytes)
}

/// The tempo track as `midly` events, ending one tick after the last event.
fn meta_track(track0: &[(u64, Meta0)]) -> Result<Vec<TrackEvent<'_>>, MidiError> {
    let mut t0 = Vec::with_capacity(track0.len() + 1);
    let mut last = 0u64;
    for (tick, m) in track0 {
        let delta =
            u28::try_from(u32::try_from(tick - last).unwrap_or(u32::MAX)).ok_or(MidiError::Tick(*tick as f64))?;
        last = *tick;
        let meta = match m {
            Meta0::Tempo(us) => MetaMessage::Tempo(u24::try_from(*us).ok_or(MidiError::Map(format!("tempo {us}")))?),
            Meta0::Signature(n, dp) => MetaMessage::TimeSignature(*n, *dp, 24, 8),
            Meta0::Marker(text) => MetaMessage::Marker(text.as_bytes()),
        };
        t0.push(TrackEvent {
            delta,
            kind: TrackEventKind::Meta(meta),
        });
    }
    t0.push(TrackEvent {
        delta: u28::new(1),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    Ok(t0)
}

/// [`multi_midi_bytes`] written to `path`.
pub fn export_multi_midi(
    tracks: &[MidiTrack],
    markers: &[MidiMarker],
    path: &Path,
    ppq: u16,
    map: &TempoMap,
    legacy_tempo: f64,
) -> Result<(), MidiError> {
    crate::io::write_atomic(path, &multi_midi_bytes(tracks, markers, ppq, map, legacy_tempo)?)?;
    Ok(())
}

/// Track 1 before the delta encoding, in `pretty_midi`'s order.
fn track1_messages(notes: &[NoteEvent]) -> Vec<Msg> {
    let mut msgs: Vec<Msg> = Vec::with_capacity(2 * notes.len() + 2);
    msgs.push(Msg {
        tick: 0,
        score: 0,
        kind: Kind::TrackName,
    });
    msgs.push(Msg {
        tick: 0,
        score: 6 * 65536,
        kind: Kind::Program,
    });
    for n in notes {
        for (tick, vel) in [(n.tick, n.velocity), (n.tick + n.duration, 0)] {
            msgs.push(Msg {
                tick,
                score: 10 * 65536 + i64::from(n.note) * 256 + i64::from(vel),
                kind: Kind::NoteOn { key: n.note, vel },
            });
        }
    }
    // pretty_midi sorts with a comparator that orders by tick, then by the score; the track
    // name compares equal to everything at tick 0 and Python's sort is stable, so it stays first.
    msgs.sort_by_key(|m| (m.tick, m.score));
    msgs
}

/// The file through `midly` (every delta fits 28 bits).
fn midly_bytes(msgs: &[Msg], ppq: u16, track0: &[(u64, Meta0)]) -> Result<Vec<u8>, MidiError> {
    let header = Header::new(Format::Parallel, Timing::Metrical(u15::new(ppq)));
    let mut smf = Smf::new(header);

    // track 0: tempo, time signature (sorted by type), end of track one tick after the last event
    let mut t0 = Vec::with_capacity(track0.len() + 1);
    let mut last = 0u64;
    for (tick, m) in track0 {
        let delta =
            u28::try_from(u32::try_from(tick - last).unwrap_or(u32::MAX)).ok_or(MidiError::Tick(*tick as f64))?;
        last = *tick;
        let meta = match m {
            Meta0::Tempo(us) => MetaMessage::Tempo(u24::try_from(*us).ok_or(MidiError::Map(format!("tempo {us}")))?),
            Meta0::Signature(n, dp) => MetaMessage::TimeSignature(*n, *dp, 24, 8),
            Meta0::Marker(text) => MetaMessage::Marker(text.as_bytes()),
        };
        t0.push(TrackEvent {
            delta,
            kind: TrackEventKind::Meta(meta),
        });
    }
    t0.push(TrackEvent {
        delta: u28::new(1),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    smf.tracks.push(t0);

    // track 1: name, program change, note on / note off pairs
    let channel = u4::new(DRUM_CHANNEL);
    let mut track = Vec::with_capacity(msgs.len() + 1);
    let mut last = 0u64;
    for m in msgs {
        let delta =
            u28::try_from(u32::try_from(m.tick - last).unwrap_or(u32::MAX)).ok_or(MidiError::Tick(m.tick as f64))?;
        last = m.tick;
        let kind = match m.kind {
            Kind::TrackName => TrackEventKind::Meta(MetaMessage::TrackName(TRACK_NAME.as_bytes())),
            Kind::Program => TrackEventKind::Midi {
                channel,
                message: MidiMessage::ProgramChange { program: u7::new(0) },
            },
            Kind::NoteOn { key, vel } => TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOn {
                    key: u7::new(key),
                    vel: u7::new(vel),
                },
            },
        };
        track.push(TrackEvent { delta, kind });
    }
    track.push(TrackEvent {
        delta: u28::new(1),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    smf.tracks.push(track);

    let mut out = Vec::with_capacity(64 + 8 * msgs.len());
    smf.write_std(&mut out)?;
    Ok(out)
}

/// mido's `encode_variable_int`: 7 bits per byte, most significant first, the high bit set
/// on every byte but the last, no length limit; 0 is one zero byte.
pub fn push_vlq(out: &mut Vec<u8>, mut v: u64) {
    let mut b = vec![(v & 0x7F) as u8];
    v >>= 7;
    while v > 0 {
        b.push((v & 0x7F) as u8 | 0x80);
        v >>= 7;
    }
    out.extend(b.iter().rev());
}

fn push_chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
}

/// The file as mido writes it, for a track 1 with a delta beyond 28 bits: the same header,
/// track 0 and events as the `midly` path, the deltas as unbounded variable-length
/// quantities, running status for the channel messages (a meta event clears it, as in
/// mido's `write_track`).
fn write_smf(msgs: &[Msg], ppq: u16, track0: &[(u64, Meta0)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + 8 * msgs.len());
    let mut hdr = Vec::with_capacity(6);
    hdr.extend_from_slice(&1u16.to_be_bytes());
    hdr.extend_from_slice(&2u16.to_be_bytes());
    hdr.extend_from_slice(&ppq.to_be_bytes());
    push_chunk(&mut out, b"MThd", &hdr);

    let mut t0 = Vec::new();
    let mut last = 0u64;
    for (tick, m) in track0 {
        push_vlq(&mut t0, tick - last);
        last = *tick;
        match m {
            Meta0::Tempo(us) => {
                t0.extend_from_slice(&[0xFF, 0x51, 0x03]);
                t0.extend_from_slice(&us.to_be_bytes()[1..]);
            }
            Meta0::Signature(n, dp) => t0.extend_from_slice(&[0xFF, 0x58, 0x04, *n, *dp, 0x18, 0x08]),
            Meta0::Marker(text) => {
                t0.extend_from_slice(&[0xFF, 0x06]);
                push_vlq(&mut t0, text.len() as u64);
                t0.extend_from_slice(text.as_bytes());
            }
        }
    }
    t0.extend_from_slice(&[0x01, 0xFF, 0x2F, 0x00]);
    push_chunk(&mut out, b"MTrk", &t0);

    let mut t1 = Vec::with_capacity(8 * msgs.len());
    let mut last = 0u64;
    let mut running: Option<u8> = None;
    for m in msgs {
        push_vlq(&mut t1, m.tick - last);
        last = m.tick;
        let (status, data): (u8, Vec<u8>) = match m.kind {
            Kind::TrackName => {
                t1.extend_from_slice(&[0xFF, 0x03, TRACK_NAME.len() as u8]);
                t1.extend_from_slice(TRACK_NAME.as_bytes());
                running = None;
                continue;
            }
            Kind::Program => (0xC0 | DRUM_CHANNEL, vec![0]),
            Kind::NoteOn { key, vel } => (0x90 | DRUM_CHANNEL, vec![key, vel]),
        };
        if running != Some(status) {
            t1.push(status);
        }
        t1.extend_from_slice(&data);
        running = Some(status);
    }
    t1.extend_from_slice(&[0x01, 0xFF, 0x2F, 0x00]);
    push_chunk(&mut out, b"MTrk", &t1);
    out
}

/// `engine.export_midi`: write the single-note drum MIDI file for `events` to `path`.
pub fn export_midi(events: &[MidiEvent], note: i64, path: &Path, ppq: u16, tempo: f64) -> Result<(), MidiError> {
    let bytes = midi_bytes(events, note, ppq, tempo)?;
    crate::io::write_atomic(path, &bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests;
