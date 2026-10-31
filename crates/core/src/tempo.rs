//! P4-8 stage A: the tempo and meter map of a project.
//!
//! The map is optional in the document (`tempo_map`, format stays 1): a start tempo, tempo steps
//! at a time in seconds (steps only, no ramps), a start meter, meter changes at a bar, and the
//! time of bar 1 beat 1. The default map (120 BPM, 4/4, bar 1 at 0 s) is not stored.
//!
//! [`TempoMap`] is the model with its JSON form and its checks. [`TickMap`] is the map as a
//! MIDI file holds it: tempo events at whole ticks with whole microseconds per quarter note.
//! The MIDI export computes every note through the `TickMap`, so a note sits within half a
//! tick of its hit through the file's own events. [`import_midi`] reads the events of a MIDI
//! file back into a map.
//!
//! Rules of the file (decided in P4-8 stage A):
//! * tick 0 of the file is time 0 s of the audio; a note at or before 0 s is a note at tick 0;
//! * a tempo step becomes a `set_tempo` at the tick nearest to its time; the tempo is rounded to
//!   whole microseconds per quarter note (the drift of that rounding is part of the file's map,
//!   so the note ticks follow it);
//! * the start meter is a time signature at tick 0; a meter change is a time signature at the
//!   tick of its bar line, counted from the tick of bar 1;
//! * bar 1 at a time after 0 s is a marker event `Bar 1` in the tempo track at its tick (no
//!   marker when bar 1 is at 0 s). A bar 1 before 0 s is not supported yet.

use serde_json::{Map, Value, json};

use crate::project::Document;
use crate::pyops::py_round;

/// Key of the map in the document.
pub const DOC_KEY: &str = "tempo_map";
/// Smallest and largest tempo in BPM (the largest gives a `set_tempo` of 60,000 microseconds).
pub const BPM_RANGE: (f64, f64) = (4.0, 1000.0);
/// Closest two tempo steps may be, in seconds.
pub const MIN_STEP_GAP_S: f64 = 0.001;
/// Most steps or meter changes a map holds.
pub const MAX_ENTRIES: usize = 100_000;
/// Text of the marker event that holds the position of bar 1.
pub const BAR1_MARKER: &str = "Bar 1";

/// A tempo step: from `t` seconds on the tempo is `bpm`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoStep {
    /// Time in seconds of the audio (greater than 0).
    pub t: f64,
    /// Beats (quarter notes) per minute.
    pub bpm: f64,
}

/// A meter change: from bar `bar` (2 or more, bar 1 holds the start meter) on the meter is `num/den`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeterChange {
    /// Bar number, counted from bar 1.
    pub bar: u32,
    /// Beats per bar.
    pub num: u8,
    /// Note value of a beat: 1, 2, 4, 8, 16, 32 or 64.
    pub den: u8,
}

/// The tempo and meter map. The default is 120 BPM, 4/4, bar 1 at 0 s.
#[derive(Debug, Clone, PartialEq)]
pub struct TempoMap {
    /// Start tempo in BPM, from time 0 on.
    pub bpm: f64,
    /// Tempo steps, strictly increasing in time.
    pub steps: Vec<TempoStep>,
    /// Start meter `(num, den)`.
    pub meter: (u8, u8),
    /// Meter changes, strictly increasing in bars.
    pub changes: Vec<MeterChange>,
    /// Time in seconds of bar 1 beat 1 (0 or more).
    pub bar1_s: f64,
}

impl Default for TempoMap {
    fn default() -> Self {
        TempoMap {
            bpm: 120.0,
            steps: Vec::new(),
            meter: (4, 4),
            changes: Vec::new(),
            bar1_s: 0.0,
        }
    }
}

fn valid_meter(num: u8, den: u8) -> bool {
    (1..=64).contains(&num) && matches!(den, 1 | 2 | 4 | 8 | 16 | 32 | 64)
}

fn check_bpm(bpm: f64, what: &str) -> Result<(), String> {
    if bpm.is_finite() && bpm >= BPM_RANGE.0 && bpm <= BPM_RANGE.1 {
        Ok(())
    } else {
        Err(format!(
            "{what}: bpm must be {} to {}, not {bpm}",
            BPM_RANGE.0, BPM_RANGE.1
        ))
    }
}

impl TempoMap {
    /// True for the default map (the one the document does not store).
    pub fn is_default(&self) -> bool {
        *self == TempoMap::default()
    }

    /// Check every field; the message names the first fault.
    ///
    /// # Errors
    ///
    /// A message when `bpm` or a step's bpm is outside the BPM range or not finite, the meter is
    /// not 1 to 64 beats over a power-of-two note value, `bar1_s` is outside 0 to 1,000,000 s,
    /// there are more than `MAX_ENTRIES` steps or changes, a step is not at least the minimum gap
    /// after the one before, or a meter change is invalid or not above the bar before it.
    pub fn validate(&self) -> Result<(), String> {
        check_bpm(self.bpm, "bpm")?;
        if !valid_meter(self.meter.0, self.meter.1) {
            return Err(format!(
                "meter {}/{}: beats 1 to 64, note value 1, 2, 4, 8, 16, 32 or 64",
                self.meter.0, self.meter.1
            ));
        }
        if !self.bar1_s.is_finite() || self.bar1_s < 0.0 || self.bar1_s > 1.0e6 {
            return Err(format!(
                "bar1_s must be 0 to 1000000 s, not {} (bar 1 before the audio starts is not supported yet)",
                self.bar1_s
            ));
        }
        if self.steps.len() > MAX_ENTRIES || self.changes.len() > MAX_ENTRIES {
            return Err(format!("at most {MAX_ENTRIES} tempo steps and meter changes"));
        }
        let mut last = 0.0;
        for (i, s) in self.steps.iter().enumerate() {
            check_bpm(s.bpm, &format!("steps[{i}]"))?;
            if !s.t.is_finite() || s.t < last + MIN_STEP_GAP_S {
                return Err(format!(
                    "steps[{i}]: t must be at least {MIN_STEP_GAP_S} s after the one before (and after 0), not {}",
                    s.t
                ));
            }
            last = s.t;
        }
        let mut bar = 1u32;
        for (i, c) in self.changes.iter().enumerate() {
            if !valid_meter(c.num, c.den) {
                return Err(format!("changes[{i}]: meter {}/{} is not valid", c.num, c.den));
            }
            if c.bar <= bar {
                return Err(format!(
                    "changes[{i}]: bar must be above {bar} (bar 1 holds the start meter)"
                ));
            }
            bar = c.bar;
        }
        Ok(())
    }

    /// The map as JSON: `{bpm, steps: [{t, bpm}], meter: [num, den], changes: [{bar, meter: [num, den]}], bar1_s}`.
    pub fn to_json(&self) -> Value {
        json!({
            "bpm": self.bpm,
            "steps": self.steps.iter().map(|s| json!({"t": s.t, "bpm": s.bpm})).collect::<Vec<_>>(),
            "meter": [self.meter.0, self.meter.1],
            "changes": self.changes.iter().map(|c| json!({"bar": c.bar, "meter": [c.num, c.den]})).collect::<Vec<_>>(),
            "bar1_s": self.bar1_s,
        })
    }

    /// Read a map from JSON. Every key is optional (a missing key is the default; `{}` is the
    /// default map); an unknown key, a value of the wrong type or a map that fails
    /// [`TempoMap::validate`] is an error with a message.
    ///
    /// # Errors
    ///
    /// A message naming the first fault: `v` or a nested value of the wrong type, an unknown key,
    /// a missing `t`, `bpm`, `bar` or `meter` in a step or change, or a map that fails
    /// [`TempoMap::validate`].
    pub fn from_json(v: &Value) -> Result<TempoMap, String> {
        Self::parse(v, true)
    }

    /// Read a map a document stores: like [`TempoMap::from_json`], but an unknown key (a newer
    /// build wrote it) is ignored, so an older build still exports MIDI. The key is not kept
    /// when the map is stored again.
    ///
    /// # Errors
    ///
    /// The same as [`TempoMap::from_json`], except that an unknown key is not an error.
    pub fn from_json_lenient(v: &Value) -> Result<TempoMap, String> {
        Self::parse(v, false)
    }

    fn parse(v: &Value, strict: bool) -> Result<TempoMap, String> {
        let o = v.as_object().ok_or("the tempo map must be an object")?;
        for k in o.keys() {
            if strict && !["bpm", "steps", "meter", "changes", "bar1_s"].contains(&k.as_str()) {
                return Err(format!("unknown key `{k}` in the tempo map"));
            }
        }
        let mut m = TempoMap::default();
        let num = |v: &Value, what: &str| v.as_f64().ok_or_else(|| format!("{what} must be a number"));
        let meter = |v: &Value, what: &str| -> Result<(u8, u8), String> {
            let a = v
                .as_array()
                .filter(|a| a.len() == 2)
                .ok_or_else(|| format!("{what} must be [beats, note value]"))?;
            let n = |x: &Value| {
                x.as_u64()
                    .and_then(|x| u8::try_from(x).ok())
                    .ok_or_else(|| format!("{what} must hold small whole numbers"))
            };
            Ok((n(&a[0])?, n(&a[1])?))
        };
        if let Some(x) = o.get("bpm") {
            m.bpm = num(x, "bpm")?;
        }
        if let Some(x) = o.get("bar1_s") {
            m.bar1_s = num(x, "bar1_s")?;
        }
        if let Some(x) = o.get("meter") {
            m.meter = meter(x, "meter")?;
        }
        if let Some(x) = o.get("steps") {
            for (i, s) in x.as_array().ok_or("steps must be a list")?.iter().enumerate() {
                let so = s.as_object().ok_or_else(|| format!("steps[{i}] must be {{t, bpm}}"))?;
                let get = |k: &str| {
                    so.get(k)
                        .ok_or_else(|| format!("steps[{i}] needs `{k}`"))
                        .and_then(|v| num(v, &format!("steps[{i}].{k}")))
                };
                if let Some(k) = so.keys().find(|k| strict && !["t", "bpm"].contains(&k.as_str())) {
                    return Err(format!("steps[{i}]: unknown key `{k}`"));
                }
                m.steps.push(TempoStep {
                    t: get("t")?,
                    bpm: get("bpm")?,
                });
            }
        }
        if let Some(x) = o.get("changes") {
            for (i, c) in x.as_array().ok_or("changes must be a list")?.iter().enumerate() {
                let co = c
                    .as_object()
                    .ok_or_else(|| format!("changes[{i}] must be {{bar, meter}}"))?;
                if let Some(k) = co.keys().find(|k| strict && !["bar", "meter"].contains(&k.as_str())) {
                    return Err(format!("changes[{i}]: unknown key `{k}`"));
                }
                let bar = co
                    .get("bar")
                    .and_then(Value::as_u64)
                    .and_then(|b| u32::try_from(b).ok())
                    .ok_or_else(|| format!("changes[{i}] needs a whole `bar`"))?;
                let (n, d) = meter(
                    co.get("meter").ok_or_else(|| format!("changes[{i}] needs `meter`"))?,
                    &format!("changes[{i}].meter"),
                )?;
                m.changes.push(MeterChange { bar, num: n, den: d });
            }
        }
        m.validate()?;
        Ok(m)
    }
}

/// One tempo segment of a [`TickMap`].
#[derive(Debug, Clone, Copy, PartialEq)]
struct Seg {
    tick: i64,
    start_s: f64,
    /// The whole microseconds per quarter the file writes (`set_tempo`).
    us: u32,
    /// #466: the exact microseconds per quarter of the map's tempo (6e7 / bpm). The ticks are
    /// computed from this, so a DAW running the user's exact tempo sees every note within the
    /// tick rounding; a DAW that applies the file's whole-microsecond `set_tempo` instead sees
    /// the relative mismatch |us - `exact_us`| / `exact_us`, which no SMF can avoid for a tempo
    /// whose quarter is not a whole number of microseconds (about 1.3e-7, or 2 samples over
    /// 5 minutes at 48 kHz). Equal to `us` for a whole-microsecond tempo and for every map
    /// read from a file ([`TickMap::from_events`]), so those grids are unchanged.
    exact_us: f64,
}

/// The map as a MIDI file holds it: whole ticks and whole microseconds per quarter note.
#[derive(Debug, Clone, PartialEq)]
pub struct TickMap {
    /// Ticks per quarter note.
    pub ppq: u16,
    segs: Vec<Seg>,
}

/// Microseconds per quarter note of a tempo, rounded to a whole number that fits a `set_tempo`.
///
/// # Errors
///
/// A message when `bpm` is not finite or outside the BPM range.
pub fn tempo_us(bpm: f64) -> Result<u32, String> {
    check_bpm(bpm, "tempo")?;
    Ok((6e7 / bpm).round() as u32)
}

impl TickMap {
    /// The file's form of `map`: step ticks are the ticks nearest to the step times through the
    /// segments before them. Errors when two steps fall on one tick or the map is not valid.
    ///
    /// # Errors
    ///
    /// A message when `map` fails [`TempoMap::validate`], `ppq` is 0 or above 32767, or a step
    /// rounds to the same tick as the step (or start) before it.
    ///
    /// # Panics
    ///
    /// Panics through `expect("one segment")` only if the segment list were empty; it always
    /// starts with the tempo segment at tick 0, so this cannot happen.
    pub fn new(ppq: u16, map: &TempoMap) -> Result<TickMap, String> {
        map.validate()?;
        if ppq == 0 || ppq > 0x7FFF {
            return Err(format!("ppq must be 1 to 32767, not {ppq}"));
        }
        let mut tm = TickMap {
            ppq,
            segs: vec![Seg {
                tick: 0,
                start_s: 0.0,
                us: tempo_us(map.bpm)?,
                exact_us: 6e7 / map.bpm,
            }],
        };
        for (i, s) in map.steps.iter().enumerate() {
            let tick = tm.tick(s.t);
            let last = *tm.segs.last().expect("one segment");
            if tick <= last.tick {
                return Err(format!("steps[{i}] falls on the tick of the step before it"));
            }
            tm.segs.push(Seg {
                tick,
                start_s: tm.time(tick),
                us: tempo_us(s.bpm)?,
                exact_us: 6e7 / s.bpm,
            });
        }
        Ok(tm)
    }

    /// The map from the tempo events of a file: `(tick, microseconds per quarter)`, sorted by
    /// tick; none at tick 0 means 500,000 (the MIDI default of 120 BPM).
    ///
    /// # Panics
    ///
    /// Panics through `expect("one segment")` only if the segment list were empty; it always
    /// starts with the default segment at tick 0, so this cannot happen.
    pub fn from_events(ppq: u16, events: &[(i64, u32)]) -> TickMap {
        let mut tm = TickMap {
            ppq,
            segs: vec![Seg {
                tick: 0,
                start_s: 0.0,
                us: 500_000,
                exact_us: 500_000.0,
            }],
        };
        for &(tick, us) in events {
            let last = *tm.segs.last().expect("one segment");
            if tick <= last.tick {
                let last_mut = tm.segs.last_mut().expect("one segment");
                last_mut.us = us; // the last event at a tick wins
                last_mut.exact_us = us as f64;
                continue;
            }
            let start_s = tm.time(tick);
            tm.segs.push(Seg {
                tick,
                start_s,
                us,
                exact_us: us as f64,
            });
        }
        tm
    }

    /// The tempo events of the file: `(tick, microseconds per quarter)`, the first at tick 0.
    pub fn tempo_events(&self) -> Vec<(i64, u32)> {
        self.segs.iter().map(|s| (s.tick, s.us)).collect()
    }

    fn seg_at_tick(&self, tick: i64) -> &Seg {
        let i = self.segs.partition_point(|s| s.tick <= tick);
        &self.segs[i.saturating_sub(1)]
    }

    /// Time in seconds of a tick.
    pub fn time(&self, tick: i64) -> f64 {
        let s = self.seg_at_tick(tick);
        s.start_s + (tick - s.tick) as f64 * s.exact_us / (self.ppq as f64 * 1e6)
    }

    /// The tick nearest to a time (ties to even); 0 for a time at or before 0.
    pub fn tick(&self, t: f64) -> i64 {
        if t <= 0.0 {
            return 0;
        }
        let i = self.segs.partition_point(|s| s.start_s <= t);
        let s = &self.segs[i.saturating_sub(1)];
        s.tick + py_round((t - s.start_s) * (self.ppq as f64 * 1e6) / s.exact_us)
    }

    /// Ticks of a bar of `num/den`: `ppq * 4 * num / den`; `None` when that is not a whole number.
    pub fn bar_ticks(&self, num: u8, den: u8) -> Option<i64> {
        let x = self.ppq as i64 * 4 * num as i64;
        (x % den as i64 == 0).then_some(x / den as i64)
    }

    /// The tick of bar 1 for a map: `bar1_s` as a tick.
    pub fn bar1_tick(&self, map: &TempoMap) -> i64 {
        self.tick(map.bar1_s)
    }

    /// The time signatures of the file, `(tick, num, den)`: the start meter at tick 0, then a
    /// change at the tick of each bar line counted from bar 1.
    ///
    /// # Errors
    ///
    /// A message when the meter in force before a change has a bar length that is not a whole
    /// number of ticks at this map's `ppq`.
    pub fn meter_events(&self, map: &TempoMap) -> Result<Vec<(i64, u8, u8)>, String> {
        let mut out = vec![(0, map.meter.0, map.meter.1)];
        let (mut tick, mut bar, mut cur) = (self.bar1_tick(map), 1u32, map.meter);
        for c in &map.changes {
            let len = self.bar_ticks(cur.0, cur.1).ok_or_else(|| {
                format!(
                    "a bar of {}/{} does not fit {} ticks per quarter",
                    cur.0, cur.1, self.ppq
                )
            })?;
            tick += (c.bar - bar) as i64 * len;
            bar = c.bar;
            cur = (c.num, c.den);
            out.push((tick, c.num, c.den));
        }
        Ok(out)
    }
}

/// What [`import_midi`] found.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// The map the file holds.
    pub map: TempoMap,
    /// Ticks per quarter note of the file.
    pub ppq: u16,
    /// Tempo events read (before dropping the ones that repeat the tempo).
    pub tempo_events: usize,
    /// Time signature events read.
    pub meter_events: usize,
    /// Time signature events that were not on a bar line: they took effect at the next bar line.
    pub snapped: usize,
}

/// Read the tempo and time-signature events of a MIDI file into a map (the notes are not read).
/// A tempo event that repeats the tempo before it adds no step. `Bar 1` markers set the time of
/// bar 1 (the first one counts); a file without one has bar 1 at 0 s. Errors for a file that is
/// not a standard MIDI file or that counts time in frames.
///
/// # Errors
///
/// A message when `bytes` is not a parseable MIDI file, the file counts time in frames, a time
/// signature is not supported, a tempo is outside the BPM range, a bar length does not fit a
/// whole number of ticks, or the resulting map fails [`TempoMap::validate`].
pub fn import_midi(bytes: &[u8]) -> Result<Imported, String> {
    use midly::{MetaMessage, Smf, Timing, TrackEventKind};
    let smf = Smf::parse(bytes).map_err(|e| format!("not a MIDI file: {e}"))?;
    let ppq = match smf.header.timing {
        Timing::Metrical(p) if p.as_int() > 0 => p.as_int(),
        _ => return Err("the MIDI file counts time in frames, not in ticks per quarter note".into()),
    };
    let mut tempos: Vec<(i64, u32)> = Vec::new();
    let mut sigs: Vec<(i64, u8, u8)> = Vec::new();
    let mut bar1: Option<i64> = None;
    for track in &smf.tracks {
        let mut tick = 0i64;
        for e in track {
            tick += e.delta.as_int() as i64;
            if let TrackEventKind::Meta(m) = &e.kind {
                match m {
                    MetaMessage::Tempo(us) => tempos.push((tick, us.as_int())),
                    MetaMessage::TimeSignature(n, dp, _, _) => {
                        let den = 1u32.checked_shl(*dp as u32).filter(|d| *d <= 64);
                        match den {
                            Some(d) if valid_meter(*n, d as u8) => sigs.push((tick, *n, d as u8)),
                            _ => return Err(format!("time signature {n}/2^{dp} is not supported")),
                        }
                    }
                    MetaMessage::Marker(t) if bar1.is_none() && t.eq_ignore_ascii_case(BAR1_MARKER.as_bytes()) => {
                        bar1 = Some(tick);
                    }
                    _ => {}
                }
            }
        }
    }
    tempos.sort_by_key(|e| e.0); // stable: the last event at a tick wins
    sigs.sort_by_key(|e| e.0);
    let (n_tempo, n_sig) = (tempos.len(), sigs.len());
    for &(_, us) in &tempos {
        let bpm = 6e7 / us.max(1) as f64;
        if !(BPM_RANGE.0..=BPM_RANGE.1).contains(&bpm) {
            return Err(format!(
                "a tempo of {us} microseconds per quarter is outside {} to {} BPM",
                BPM_RANGE.0, BPM_RANGE.1
            ));
        }
    }
    let tm = TickMap::from_events(ppq, &tempos);
    let mut map = TempoMap {
        bpm: 6e7 / tm.segs[0].us as f64,
        ..TempoMap::default()
    };
    for s in &tm.segs[1..] {
        let bpm = 6e7 / s.us as f64;
        let prev = map.steps.last().map_or(map.bpm, |p| p.bpm);
        if bpm != prev {
            map.steps.push(TempoStep { t: s.start_s, bpm });
        }
    }
    let t1 = bar1.unwrap_or(0);
    map.bar1_s = tm.time(t1);
    // the start meter: the last signature at or before bar 1; the rest are changes at bar lines
    let mut cur = (4u8, 4u8);
    let mut rest = Vec::new();
    for &(tick, n, d) in &sigs {
        if tick <= t1 {
            cur = (n, d);
        } else {
            rest.push((tick, n, d));
        }
    }
    map.meter = cur;
    let (mut at, mut bar, mut snapped) = (t1, 1u32, 0usize);
    for (tick, n, d) in rest {
        let len = tm
            .bar_ticks(cur.0, cur.1)
            .ok_or_else(|| format!("a bar of {}/{} does not fit {ppq} ticks per quarter", cur.0, cur.1))?;
        let bars = (tick - at + len - 1) / len; // the next bar line at or after the event
        if (tick - at) % len != 0 {
            snapped += 1;
        }
        at += bars * len;
        bar += bars as u32;
        if (n, d) == cur {
            continue;
        }
        cur = (n, d);
        match map.changes.last_mut() {
            Some(last) if last.bar == bar => {
                last.num = n;
                last.den = d;
            }
            _ => map.changes.push(MeterChange { bar, num: n, den: d }),
        }
    }
    map.validate()?;
    Ok(Imported {
        map,
        ppq,
        tempo_events: n_tempo,
        meter_events: n_sig,
        snapped,
    })
}

impl Document {
    /// The tempo map of the document: the stored `tempo_map`, the default map when none is
    /// stored. An error with a message when the stored value is not a valid map.
    ///
    /// # Errors
    ///
    /// The message of [`TempoMap::from_json_lenient`] when the stored value is not a valid map.
    pub fn tempo_map(&self) -> Result<TempoMap, String> {
        match self.extra.get(DOC_KEY) {
            None => Ok(TempoMap::default()),
            Some(v) => TempoMap::from_json_lenient(v),
        }
    }

    /// Store `map`; the default map removes the key (the document is then as it was before the
    /// feature). The caller marks the project dirty.
    pub fn set_tempo_map(&mut self, map: &TempoMap) {
        if map.is_default() {
            self.extra.shift_remove(DOC_KEY);
        } else {
            let o: Map<String, Value> = match map.to_json() {
                Value::Object(o) => o,
                _ => unreachable!("to_json is an object"),
            };
            self.extra.insert(DOC_KEY.into(), Value::Object(o));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three_steps() -> TempoMap {
        TempoMap {
            bpm: 120.0,
            steps: vec![
                TempoStep { t: 8.0, bpm: 100.0 },
                TempoStep { t: 16.0, bpm: 150.0 },
                TempoStep { t: 24.0, bpm: 80.0 },
            ],
            meter: (4, 4),
            changes: vec![
                MeterChange { bar: 5, num: 3, den: 4 },
                MeterChange { bar: 9, num: 6, den: 8 },
            ],
            bar1_s: 2.0,
        }
    }

    #[test]
    fn the_default_map_is_default_and_is_not_stored() {
        let m = TempoMap::default();
        assert!(m.is_default() && m.validate().is_ok());
        let mut doc = Document::from_json(r#"{"format": 1, "x": 1}"#).unwrap();
        assert_eq!(doc.tempo_map().unwrap(), m);
        doc.set_tempo_map(&three_steps());
        assert!(doc.to_json().contains("tempo_map"));
        assert_eq!(doc.tempo_map().unwrap(), three_steps());
        doc.set_tempo_map(&m);
        assert_eq!(doc.to_json(), r#"{"format":1,"x":1}"#);
    }

    #[test]
    fn a_stored_map_with_an_unknown_key_reads_but_a_caller_map_is_refused() {
        // P4-8 fix round 1: a newer build wrote `swing` (and a step key); this build ignores them.
        let v = json!({"bpm": 90.0, "swing": 0.6, "steps": [{"t": 4.0, "bpm": 100.0, "ramp": true}],
                       "changes": [{"bar": 3, "meter": [3, 4], "label": "x"}]});
        let doc = Document::from_json(&json!({"format": 1, "tempo_map": v}).to_string()).unwrap();
        let m = doc.tempo_map().unwrap();
        assert_eq!((m.bpm, m.steps.len(), m.changes.len()), (90.0, 1, 1));
        assert!(TempoMap::from_json(&v).is_err());
        assert!(TempoMap::from_json(&json!({"steps": [{"t": 4.0, "bpm": 100.0, "ramp": 1}]})).is_err());
        // a wrong value is still an error when stored
        let bad = Document::from_json(&json!({"format": 1, "tempo_map": {"bpm": 0}}).to_string()).unwrap();
        assert!(bad.tempo_map().is_err());
    }

    #[test]
    fn json_round_trips_and_every_key_is_optional() {
        let m = three_steps();
        assert_eq!(TempoMap::from_json(&m.to_json()).unwrap(), m);
        assert_eq!(TempoMap::from_json(&json!({})).unwrap(), TempoMap::default());
        let only = TempoMap::from_json(&json!({"bpm": 90.0})).unwrap();
        assert_eq!((only.bpm, only.meter, only.bar1_s), (90.0, (4, 4), 0.0));
    }

    #[test]
    fn a_bad_map_is_refused_with_a_message() {
        for bad in [
            json!(5),
            json!({"tempo": 90}),
            json!({"bpm": 3.0}),
            json!({"bpm": "fast"}),
            json!({"meter": [4, 3]}),
            json!({"meter": [0, 4]}),
            json!({"meter": [4]}),
            json!({"bar1_s": -1.0}),
            json!({"steps": [{"t": 5.0, "bpm": 100.0}, {"t": 5.0, "bpm": 90.0}]}),
            json!({"steps": [{"t": 0.0, "bpm": 100.0}]}),
            json!({"steps": [{"t": 5.0}]}),
            json!({"steps": [{"t": 5.0, "bpm": 100.0, "x": 1}]}),
            json!({"changes": [{"bar": 1, "meter": [3, 4]}]}),
            json!({"changes": [{"bar": 4, "meter": [3, 4]}, {"bar": 4, "meter": [5, 4]}]}),
            json!({"changes": [{"bar": 4}]}),
        ] {
            assert!(TempoMap::from_json(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_tick_map_follows_the_steps_exactly_where_ticks_are_exact() {
        let tm = TickMap::new(24_000, &three_steps()).unwrap();
        // 120 BPM: 48,000 ticks per second; 100 BPM: 40,000; 150 BPM: 60,000
        assert_eq!(
            tm.tempo_events(),
            vec![
                (0, 500_000),
                (384_000, 600_000),
                (704_000, 400_000),
                (1_184_000, 750_000)
            ]
        );
        assert_eq!(tm.tick(8.0), 384_000);
        assert_eq!(tm.tick(12.0), 384_000 + 160_000);
        assert_eq!(tm.time(704_000), 16.0);
        assert_eq!(tm.tick(0.0), 0);
        assert_eq!(tm.tick(-3.0), 0);
        assert_eq!(tm.bar_ticks(3, 4), Some(72_000));
        assert_eq!(tm.bar_ticks(6, 8), Some(72_000));
        assert_eq!(tm.bar1_tick(&three_steps()), 96_000);
        // bar 1 at 2 s, 4 bars of 4/4 at 100/120 BPM: bar 5 is at tick 96,000 + 4 * 96,000
        let ev = tm.meter_events(&three_steps()).unwrap();
        assert_eq!(ev, vec![(0, 4, 4), (480_000, 3, 4), (480_000 + 4 * 72_000, 6, 8)]);
    }

    #[test]
    fn a_step_off_the_tick_grid_lands_within_half_a_tick() {
        let m = TempoMap {
            steps: vec![TempoStep {
                t: 1.000_001,
                bpm: 133.7,
            }],
            ..TempoMap::default()
        };
        let tm = TickMap::new(24_000, &m).unwrap();
        let t = tm.tempo_events()[1].0;
        assert!((tm.time(t) - 1.000_001).abs() <= 0.5 / 48_000.0 + 1e-12);
        // two steps on one tick are refused
        let m2 = TempoMap {
            steps: vec![TempoStep { t: 1.0, bpm: 100.0 }, TempoStep { t: 1.001, bpm: 90.0 }],
            ..TempoMap::default()
        };
        assert!(TickMap::new(24_000, &m2).is_ok());
        assert!(TickMap::new(0, &m2).is_err());
    }

    #[test]
    fn tempo_us_rounds_and_checks_the_range() {
        assert_eq!(tempo_us(120.0), Ok(500_000));
        assert_eq!(tempo_us(133.7), Ok(448_766));
        assert!(tempo_us(3.9).is_err() && tempo_us(1000.1).is_err() && tempo_us(f64::NAN).is_err());
    }

    #[test]
    fn the_ticks_follow_the_exact_tempo_not_the_rounded_set_tempo() {
        // #466: 92.000 BPM is 652173.913 us per quarter; the file writes 652174. A DAW at the
        // user's 92.000 must find the note of a hit at 300 s on its sample: the tick of the
        // exact tempo (24000 * 92 / 60 * 300 = 11,040,000), not the integer-tempo tick
        // (11,039,996, which a DAW at 92.000 plays 1.8 samples early).
        let m = TempoMap {
            bpm: 92.0,
            ..TempoMap::default()
        };
        let tm = TickMap::new(24_000, &m).unwrap();
        assert_eq!(tm.tempo_events(), vec![(0, 652_174)]);
        assert_eq!(tm.tick(300.0), 11_040_000);
        assert!((tm.time(11_040_000) - 300.0).abs() < 1e-9);
        // and the ticks still come back through the file's own (integer) tempo within the
        // documented integer-set_tempo limit: 300 s * 0.087/652173.9 = 40 us = 1.9 samples
        let back = 11_040_000_f64 * 652_174.0 / (24_000.0 * 1e6);
        assert!((back - 300.0).abs() < 2.0 / 48_000.0, "{}", (back - 300.0) * 48_000.0);
        // a whole-microsecond tempo keeps its grid: 120 BPM is exactly 500,000
        let tm = TickMap::new(24_000, &TempoMap::default()).unwrap();
        assert_eq!(tm.tick(300.0), 14_400_000);
        assert_eq!(tm.time(14_400_000), 300.0);
    }

    #[test]
    fn from_events_builds_the_segments_of_a_file() {
        // no event at tick 0: the MIDI default of 500,000; two events at one tick: the last wins
        let tm = TickMap::from_events(480, &[(480, 1_000_000), (480, 600_000), (960, 400_000)]);
        assert_eq!(tm.tempo_events(), vec![(0, 500_000), (480, 600_000), (960, 400_000)]);
        assert_eq!(tm.time(480), 0.5);
        assert_eq!(tm.time(960), 0.5 + 0.6);
        assert_eq!(tm.bar1_tick(&TempoMap::default()), 0);
    }
}
