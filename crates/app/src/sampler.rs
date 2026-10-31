//! P4-7c: the sampler calibration of a track and the MIDI placement that uses it.
//!
//! The engineer's sampler (Slate Trigger, Superior Drummer 3, Drumforge Saviour) plays its transient
//! a little after the MIDI note. Three steps tell the app how much:
//!
//! 1. `sampler_calibration_midi {sid, name, dest?}` writes `<name>_calibration.mid` with ONE note (the
//!    track's note, velocity 100) on the start of bar 2 of the project's tempo map (2.000 s with the
//!    default map). The engineer imports it at the session start and bounces the sampler's kit piece.
//! 2. `track_sampler_set {sid, name, path, one_shot?}` loads the bounce: the delay from the note-on time
//!    to the bounce's transient (the refinement rule of the one-shots) and the bounce's polarity against
//!    the close mic (its attack correlated with the median shape of the track's hits) go into the
//!    track's `sampler` object. An empty `path` removes it. `one_shot: true` also sets the bounce as the
//!    track's layer one-shot (P4-7a), with its polarity.
//! 3. Every MIDI export (`export_midi`, `export_all`, the multi-track file, the drag files of the Export
//!    dialog) puts each note at the aligned attack (the hit, plus the per-hit lag of the layer's alignment
//!    when the track's layer has Align on) minus the offset, rounded to the sample; a note that would
//!    fall before the start goes to 0 s and is counted in the answer (`sampler.<track>.clamped`). The
//!    option `compensate` (default true) turns this off. A track without a calibration is unchanged.
//!    The sub-sample part of the alignment exists only in the render.
//!
//! # The sampler object
//!
//! The track's `sampler` key (optional; the format stays 1): `offset_samples` (f64, samples of the
//! bounce's rate `sr`; negative when the sampler plays early), `sr`, `polarity` (`normal`, `inverted`, or
//! null when the mic or its hits were not there), `match` (0 to 1, the height of the polarity
//! correlation, or null), `measured_at` (seconds since the epoch) and `file` (the bounce).

use std::path::PathBuf;

use drumspike_core::io::MappedWav;
use drumspike_core::layer::{AlignParams, one_shot_onset, read_one_shot};
use drumspike_core::midi::MidiEvent;
use drumspike_core::pyops::py_round;
use drumspike_core::sampler::{bounce_polarity, calibration_midi, calibration_time, compensate, measure_offset};
use serde_json::{Map, Value, json};

use crate::Service;
use crate::error::AppError;
use crate::export::{EXPORT_PPQ, EXPORT_TEMPO, export_dir};
use crate::layer::{LayerHit, Polarity, applied_lag, layer_job, opt_path, protected_files, save_target};
use crate::media::mix_mode;
use crate::registry::lock;

/// The commands of the sampler calibration.
pub const SAMPLER_COMMANDS: [&str; 3] = ["sampler_calibration_midi", "track_sampler_get", "track_sampler_set"];

/// The key of a track's sampler object.
pub const SAMPLER_KEY: &str = "sampler";

/// The sampler calibration of a track (the `sampler` object, see the module docs).
#[derive(Debug, Clone, PartialEq)]
pub struct Sampler {
    /// The delay of the bounce's transient after the note, in samples of `sr`.
    pub offset_samples: f64,
    /// The bounce's rate.
    pub sr: u32,
    /// +1 normal, -1 inverted, `None` unknown.
    pub polarity: Option<i8>,
    /// The height of the polarity correlation.
    pub match_r: Option<f64>,
    /// Seconds since the epoch.
    pub measured_at: f64,
    /// The bounce file.
    pub file: String,
}

impl Sampler {
    /// The sampler of a JSON object; `None` when it is not one.
    pub fn from_json(v: &Value) -> Option<Sampler> {
        let m = v.as_object()?;
        Some(Sampler {
            offset_samples: m.get("offset_samples")?.as_f64().filter(|x| x.is_finite())?,
            sr: u32::try_from(m.get("sr")?.as_u64()?).ok().filter(|r| *r > 0)?,
            polarity: match m.get("polarity").and_then(Value::as_str) {
                Some("normal") => Some(1),
                Some("inverted") => Some(-1),
                _ => None,
            },
            match_r: m.get("match").and_then(Value::as_f64),
            measured_at: m.get("measured_at").and_then(Value::as_f64).unwrap_or(0.0),
            file: m.get("file").and_then(Value::as_str).unwrap_or("").to_string(),
        })
    }

    /// The JSON object of the sampler.
    pub fn to_json(&self) -> Value {
        json!({
            "offset_samples": self.offset_samples,
            "sr": self.sr,
            "polarity": self.polarity_name(),
            "match": self.match_r.map(|r| (r * 1000.0).round() / 1000.0),
            "measured_at": self.measured_at,
            "file": self.file,
        })
    }

    /// `normal`, `inverted` or `None`.
    pub fn polarity_name(&self) -> Option<&'static str> {
        self.polarity.map(|s| if s < 0 { "inverted" } else { "normal" })
    }

    /// The offset in samples of a track at `sr`.
    pub fn offset_at(&self, sr: u32) -> f64 {
        self.offset_samples * sr as f64 / self.sr as f64
    }

    /// The offset in milliseconds.
    pub fn offset_ms(&self) -> f64 {
        self.offset_samples / self.sr as f64 * 1000.0
    }
}

/// The sampler of a track's extra keys, `None` when absent or not one.
pub fn sampler_of(extra: &Map<String, Value>) -> Option<Sampler> {
    extra.get(SAMPLER_KEY).and_then(Sampler::from_json)
}

/// How a track's MIDI notes move: the offset and the per-hit lags in samples of the track's rate.
#[derive(Debug, Clone, PartialEq)]
pub struct Shift {
    /// The offset in samples at `sr`.
    pub offset: f64,
    /// The rate the samples are of.
    pub sr: u32,
    /// Per event (the order of the list the notes are made from) the lag of the layer's alignment, 0 without one.
    pub lags: Vec<f64>,
    /// The per-hit alignment was used.
    pub aligned: bool,
}

/// What an export did to one track's notes (the answer's `sampler.<track>`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShiftInfo {
    /// The offset in milliseconds.
    pub offset_ms: f64,
    /// The per-hit alignment was used.
    pub aligned: bool,
    /// Notes written.
    pub notes: usize,
    /// Notes that fell before the start and were put at 0 s.
    pub clamped: usize,
}

impl ShiftInfo {
    /// The JSON of the answer.
    pub fn to_json(&self) -> Value {
        json!({"offset_ms": (self.offset_ms * 1e3).round() / 1e3, "aligned": self.aligned,
               "notes": self.notes, "clamped": self.clamped})
    }

    /// The warning text of a track with clamped notes.
    pub fn warning(&self, name: &str) -> Option<String> {
        (self.clamped > 0).then(|| {
            format!(
                "{name}: {} note{} would fall before the start with the sampler offset: put at 0 s",
                self.clamped,
                if self.clamped == 1 { "" } else { "s" }
            )
        })
    }
}

/// The MIDI events of a track and what the sampler shift did to them (`None` without a shift).
pub type MidiPlan = (Vec<MidiEvent>, Option<ShiftInfo>);

/// The MIDI events of `events` (`t` and `v`, a missing `v` is 100) with the sampler shift applied:
/// each note at the aligned attack minus the offset, on a whole sample. Without a shift the events are
/// the plain ones.
///
/// # Errors
/// 400 when an entry of `events` has no valid `t` (the error of `export::midi_events`).
pub fn midi_events_shifted(events: &[Value], shift: Option<&Shift>) -> Result<MidiPlan, AppError> {
    let mut ev = crate::export::midi_events(events)?;
    let Some(s) = shift else { return Ok((ev, None)) };
    let mut clamped = 0;
    for (k, e) in ev.iter_mut().enumerate() {
        let lag = s.lags.get(k).copied().unwrap_or(0.0);
        let (t, c) = compensate(e.t, lag, s.offset, s.sr);
        e.t = t;
        clamped += usize::from(c);
    }
    let info = ShiftInfo {
        offset_ms: s.offset / s.sr as f64 * 1000.0,
        aligned: s.aligned,
        notes: ev.len(),
        clamped,
    };
    Ok((ev, Some(info)))
}

impl Service {
    /// The shift of a track's MIDI notes for `events` (the uncut hits in the order the notes are made from):
    /// `None` when the track has no calibration. The lags come from the layer's alignment when the track's
    /// layer has Align on and its file and the track's audio are there (an alignment that fails leaves the
    /// notes on the hits).
    pub(crate) fn sampler_shift(&self, sid: &str, name: &str, events: &[Value]) -> Result<Option<Shift>, AppError> {
        let op = self.inner.registry.get(sid)?;
        let (sampler, job, source) = {
            let g = lock(&op);
            let tr = g.track(name)?;
            let Some(s) = sampler_of(&tr.extra) else {
                return Ok(None);
            };
            (s, layer_job(&g, name)?, g.source(name).ok())
        };
        let sr = source
            .and_then(|p| MappedWav::open(&p).ok())
            .map_or(sampler.sr, |w| w.info.sample_rate);
        let mut lags = vec![0.0; events.len()];
        let mut aligned = false;
        if let Some(job) = job.filter(|j| j.layer.align) {
            let hits: Vec<LayerHit> = events
                .iter()
                .map(|e| LayerHit {
                    t: e.get("t").and_then(Value::as_f64).unwrap_or(0.0),
                    v: e.get("v").and_then(Value::as_i64).unwrap_or(100),
                })
                .collect();
            if let Ok(al) = self.layer_alignment(sid, name, &job, &hits) {
                let sign = al.sign_for(job.layer.polarity);
                let scale = sr as f64 / al.sr as f64;
                for (l, x) in lags.iter_mut().zip(&al.hits) {
                    // an edge or low-match lag is not applied by the render, nor by the MIDI file
                    *l = x.map_or(0.0, |x| applied_lag(&x, sign) * scale);
                }
                aligned = true;
            }
        }
        Ok(Some(Shift {
            offset: sampler.offset_at(sr),
            sr,
            lags,
            aligned,
        }))
    }

    /// `sampler_calibration_midi {sid, name, dest?}`: write `<name>_calibration.mid` into `dest` (else
    /// `Exports/`): one note (the track's note, velocity 100) on the start of bar 2 of the tempo map. Answers
    /// `{path, size, note, t, tick, ppq, text}`; `t` is the note-on time in seconds, `text` says what to do next.
    ///
    /// # Errors
    /// The same as [`Service::sampler_calibration_midi_to`] without a `path`.
    pub fn sampler_calibration_midi(&self, sid: &str, name: &str, dest: Option<&str>) -> Result<Value, AppError> {
        self.sampler_calibration_midi_to(sid, name, dest, None)
    }

    /// [`Service::sampler_calibration_midi`] into the file `path` when given (P4-7d: the save dialog's file; it
    /// replaces `dest` and the name `<name>_calibration.mid`; see `save_target` in `layer.rs`).
    ///
    /// # Errors
    /// 404 when `sid` is not an open project or `name` is not a track; 409 when the project is
    /// open read-only; an error from `save_target` or `export_dir` for a bad `path` or `dest`; 400
    /// when the stored tempo map is not valid or has no usable bar 2; 500 when the file cannot be
    /// written.
    pub fn sampler_calibration_midi_to(
        &self,
        sid: &str,
        name: &str,
        dest: Option<&str>,
        path: Option<&str>,
    ) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let (note, map, out) = {
            let g = lock(&op);
            let note = g.track(name)?.note.unwrap_or(36);
            let out = match path {
                Some(p) => save_target(p, &protected_files(&g))?,
                None => export_dir(&g, dest)?.join(format!("{name}_calibration.mid")),
            };
            (note, crate::tempo::doc_map(&g.project.doc)?, out)
        };
        let (bytes, t) = calibration_midi(note, &map, EXPORT_PPQ, EXPORT_TEMPO)?;
        let (tick, _) = calibration_time(&map, EXPORT_PPQ).map_err(AppError::bad)?;
        let path = out;
        drumspike_core::io::write_atomic(&path, &bytes)
            .map_err(|e| AppError::internal(format!("{}: {e}", path.display())))?;
        Ok(json!({
            "path": path.display().to_string(),
            "size": std::fs::metadata(&path).map_or(0, |m| m.len()),
            "note": note,
            "t": t,
            "tick": tick,
            "ppq": EXPORT_PPQ,
            "text": format!(
                "Import {} at the session start, bounce the sampler's kit piece for note {note} from the session start, then load the bounce here.",
                path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
            ),
        }))
    }

    /// `track_sampler_get {sid, name}`: `{sampler, offset_ms, polarity, file}`; all null for a track without
    /// a calibration.
    ///
    /// # Errors
    /// 404 when `sid` is not an open project or `name` is not a track.
    pub fn track_sampler_get(&self, sid: &str, name: &str) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        let g = lock(&op);
        let s = sampler_of(&g.track(name)?.extra);
        Ok(get_json(s.as_ref()))
    }

    /// `track_sampler_set {sid, name, path?, one_shot?}`: load a bounce (WAV or AIFF, at most 20 s from the
    /// session start, the calibration note at [`calibration_time`]) and keep the measured offset and
    /// polarity in the track's `sampler` object; a `path` that is empty, null or missing removes it.
    /// `one_shot: true` also sets the bounce as the track's layer one-shot (its polarity, when measured, goes
    /// to the layer). Answers `{sampler, offset_ms, polarity, file, layer}` (`layer`: the answer of
    /// `track_layer_get` after a `one_shot`, else null). 400 for a bounce the app cannot read or a silent one.
    ///
    /// # Errors
    /// 404 when `sid` is not an open project or `name` is not a track; 409 when the project is
    /// open read-only; 400 when `path` or `one_shot` has the wrong type, the stored tempo map is
    /// not valid, or the bounce cannot be read or measured; any error of `track_layer_set` after a
    /// `one_shot`.
    pub fn track_sampler_set(&self, sid: &str, name: &str, args: &Value) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let path = match args.get("path") {
            None | Some(Value::Null) => None,
            Some(Value::String(p)) if p.trim().is_empty() => None,
            Some(Value::String(p)) => Some(PathBuf::from(p)),
            Some(_) => return Err(AppError::bad("`path` must be a file path or null")),
        };
        let one_shot = match args.get("one_shot") {
            None | Some(Value::Null) => false,
            Some(v) => v
                .as_bool()
                .ok_or_else(|| AppError::bad("`one_shot` must be true or false"))?,
        };
        let Some(path) = path else {
            let mut g = lock(&op);
            let tr = g.track_mut(name)?;
            if tr.extra.shift_remove(SAMPLER_KEY).is_some() {
                g.mark_dirty();
                g.emit_layer(name);
            }
            return Ok(get_json(None));
        };
        let (note_on, mic, mode, hits) = {
            let g = lock(&op);
            g.track(name)?;
            let map = crate::tempo::doc_map(&g.project.doc)?;
            let (_, t) = calibration_time(&map, EXPORT_PPQ).map_err(AppError::bad)?;
            let tr = g.track(name)?;
            let hits: Vec<f64> = tr
                .events()
                .iter()
                .filter(|e| !crate::histogram::event_cut(e))
                .map(|e| e.t)
                .collect();
            (t, g.source(name).ok(), mix_mode(tr.mix_mode_or_default()), hits)
        };
        let shot = read_one_shot(&path).map_err(AppError::bad)?;
        let mono = shot.mono();
        let m = measure_offset(&mono, shot.sr, note_on).map_err(AppError::bad)?;
        // the polarity against the close mic, when its audio and hits are there
        let mut polarity = None;
        if let (Some(mic), false) = (mic, hits.is_empty())
            && let Ok(d) = self.inner.media.decode_cached(&mic, mode)
        {
            let at = shot.resampled(d.sr).mono();
            let onset = one_shot_onset(&at, d.sr);
            let idx: Vec<i64> = hits.iter().map(|t| py_round(t * d.sr as f64)).collect();
            polarity = bounce_polarity(&at, onset, &d.samples, &idx, d.sr, &AlignParams::default());
        }
        let s = Sampler {
            offset_samples: m.offset_samples,
            sr: m.sr,
            polarity: polarity.map(|p| p.0),
            match_r: polarity.map(|p| p.1),
            measured_at: drumspike_core::project::now(),
            file: path.display().to_string(),
        };
        {
            let mut g = lock(&op);
            g.track_mut(name)?.extra.insert(SAMPLER_KEY.into(), s.to_json());
            g.mark_dirty();
            g.emit_layer(name);
        }
        let mut out = get_json(Some(&s));
        if one_shot {
            let mut a = json!({"path": path.display().to_string()});
            if let Some(p) = s.polarity {
                a["polarity"] = json!(if p < 0 { Polarity::Invert } else { Polarity::Normal }.as_str());
            }
            out["layer"] = self.track_layer_set(sid, name, &a)?;
        }
        Ok(out)
    }

    /// `track_sampler_wave {sid, name, start?, end?}` (P4-40e): the mono samples of the calibration
    /// bounce `[start, end)` in its own samples — the Layer view draws it with the sampler offset
    /// line. The bytes of `samples_window` (see [`crate::media::wave_bytes`]). 409 without a
    /// calibration or its file.
    ///
    /// # Errors
    /// 404 when `sid` is not an open project or `name` is not a track; 409 when the track has no
    /// sampler calibration; otherwise whatever reading the calibration file as a one-shot returns.
    pub fn track_sampler_wave(&self, sid: &str, name: &str, start: i64, end: i64) -> Result<Vec<u8>, AppError> {
        let op = self.inner.registry.get(sid)?;
        let file = {
            let g = lock(&op);
            let tr = g.track(name)?;
            sampler_of(&tr.extra)
                .map(|s| s.file)
                .ok_or_else(|| AppError::conflict(format!("track {name} has no sampler calibration")))?
        };
        let path = PathBuf::from(&file);
        let shot = self.layer_shot(&path, None)?;
        Ok(crate::media::wave_bytes(&shot, start, end))
    }
}

/// The answer of `track_sampler_get` and `track_sampler_set`.
fn get_json(s: Option<&Sampler>) -> Value {
    match s {
        None => json!({"sampler": Value::Null, "offset_ms": Value::Null, "polarity": Value::Null, "file": Value::Null,
                       "layer": Value::Null}),
        Some(s) => json!({
            "sampler": s.to_json(),
            "offset_ms": (s.offset_ms() * 1e3).round() / 1e3,
            "polarity": s.polarity_name(),
            "file": PathBuf::from(&s.file).file_name().map(|f| f.to_string_lossy().into_owned()),
            "layer": Value::Null,
        }),
    }
}

/// Run a command of the sampler for the bridge, the command line and the MCP server; `None` when `cmd`
/// is not one of them.
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    if !SAMPLER_COMMANDS.contains(&cmd) {
        return None;
    }
    let run = || -> Result<Value, AppError> {
        let (sid, name) = (
            crate::command_policy::arg_str(args, "sid")?,
            crate::command_policy::arg_str(args, "name")?,
        );
        match cmd {
            "sampler_calibration_midi" => {
                let dest = match args.get("dest") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(d)) => Some(d.as_str()),
                    Some(_) => return Err(AppError::bad("`dest` must be a string")),
                };
                svc.sampler_calibration_midi_to(sid, name, dest, opt_path(args, "path")?)
            }
            "track_sampler_get" => svc.track_sampler_get(sid, name),
            "track_sampler_set" => svc.track_sampler_set(sid, name, args),
            _ => Err(AppError::not_found(format!("unknown command {cmd}"))),
        }
    };
    Some(run())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Sampler {
        Sampler {
            offset_samples: 264.5,
            sr: 48_000,
            polarity: Some(-1),
            match_r: Some(0.91234),
            measured_at: 1.5,
            file: "/x/kick bounce.wav".into(),
        }
    }

    #[test]
    fn the_sampler_object_round_trips_and_rejects_a_bad_one() {
        let a = s();
        let back = Sampler::from_json(&a.to_json()).unwrap();
        assert_eq!((back.offset_samples, back.sr, back.polarity), (264.5, 48_000, Some(-1)));
        assert_eq!(back.match_r, Some(0.912));
        assert_eq!(a.polarity_name(), Some("inverted"));
        assert!((a.offset_ms() - 5.510_416).abs() < 1e-5);
        assert!((a.offset_at(96_000) - 529.0).abs() < 1e-9);
        let mut m = Map::new();
        m.insert(SAMPLER_KEY.into(), a.to_json());
        assert!(sampler_of(&m).is_some());
        assert!(sampler_of(&Map::new()).is_none());
        assert!(Sampler::from_json(&json!({"offset_samples": 1.0})).is_none(), "no rate");
        assert!(Sampler::from_json(&json!({"offset_samples": 1.0, "sr": 0})).is_none());
        assert!(Sampler::from_json(&json!({"sr": 48000})).is_none(), "no offset");
        let unknown = Sampler::from_json(&json!({"offset_samples": -3.0, "sr": 44100, "polarity": null})).unwrap();
        assert_eq!((unknown.polarity, unknown.polarity_name()), (None, None));
    }

    #[test]
    fn midi_events_move_by_the_offset_and_the_lag_and_clamp_before_zero() {
        let ev = [
            json!({"t": 0.001, "v": 90}),
            json!({"t": 1.0}),
            json!({"t": 2.0, "v": 127}),
        ];
        let (plain, info) = midi_events_shifted(&ev, None).unwrap();
        assert!(info.is_none());
        assert_eq!(plain.iter().map(|e| e.t).collect::<Vec<_>>(), vec![0.001, 1.0, 2.0]);
        let shift = Shift {
            offset: 100.0,
            sr: 48_000,
            lags: vec![0.0, -4.4, 0.0],
            aligned: true,
        };
        let (m, info) = midi_events_shifted(&ev, Some(&shift)).unwrap();
        // 48 - 100 < 0: clamped; 48,000 - 4.4 - 100 = 47,895.6 -> 47,896; 96,000 - 100
        assert_eq!(m[0].t, 0.0);
        assert_eq!((m[1].t * 48_000.0).round(), 47_896.0);
        assert!((m[1].t * 48_000.0 - 47_896.0).abs() < 1e-6);
        assert_eq!((m[2].t * 48_000.0).round(), 95_900.0);
        assert_eq!(m.iter().map(|e| e.v).collect::<Vec<_>>(), vec![90, 100, 127]);
        let info = info.unwrap();
        assert_eq!((info.notes, info.clamped, info.aligned), (3, 1, true));
        assert!((info.offset_ms - 100.0 / 48.0).abs() < 1e-9);
        assert!(
            info.warning("k")
                .unwrap()
                .contains("1 note would fall before the start")
        );
        let ok = ShiftInfo { clamped: 0, ..info };
        assert!(ok.warning("k").is_none());
        assert_eq!(ok.to_json()["clamped"], 0);
    }

    #[test]
    fn a_bad_event_is_a_400_with_or_without_a_shift() {
        let bad = [json!({"v": 100})];
        assert_eq!(midi_events_shifted(&bad, None).unwrap_err().code, 400);
        let shift = Shift {
            offset: 1.0,
            sr: 48_000,
            lags: vec![],
            aligned: false,
        };
        assert_eq!(midi_events_shifted(&bad, Some(&shift)).unwrap_err().code, 400);
    }
}
