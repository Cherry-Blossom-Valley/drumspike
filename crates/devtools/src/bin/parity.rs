//! Parity harness: decode one WAV with the core and print what the Python side compares.
//!
//! ```text
//! parity <wav> [--mix ffmpeg|average] [--t0-file list.json] [--pcm-out samples.f32]
//!        [--tile level,from,to]... [--samples from,to]...
//! ```
//!
//! Prints one JSON object: `info` (the parsed header), `peaks` (the pyramid, keyed by
//! bucket size like `engine.compute_peaks`), `velocities` (`engine.velocity_map` at the
//! times of `--t0-file`), `tiles` and `samples` (the tile API at the requested windows).
//! `--pcm-out` writes the decoded mono signal as raw little-endian f32 so the Python side
//! can compare every sample with `engine.load_f32`. Used by `tools/parity/check_parity.py`.
//!
//! ```text
//! parity refine <job.json>
//! ```
//!
//! Refinement job (P1-4, `check_refine.py`): `{"wav": path, "mix": "ffmpeg"|"average",
//! "hits": [{"t0": s, "neighbors": [s, ...]}, ...], "window": [lo_ms, hi_ms]}` (`window`
//! optional, default `RefineWindow::DEFAULT`, P2-9). Prints `{"sr", "refined": [{"t", "rise"}],
//! "velocities": [...]}` with `engine._refine_sr` per hit and `engine.velocity_map` at the
//! refined times. P4-12: `"rule": 2` runs the reach rule (`refine_reach`) instead, and each hit
//! carries its `step` (`kept`, `reached`, `tail`, `unresolved`); `1` or no `rule` is the reference.
//! P4-12 stage 2: `"rule": 3` adds the first-motion back-track, each hit with its `back` (`moved`,
//! `kept`, `busy`, `separate`).
//!
//! ```text
//! parity midi <job.json> <out.mid>
//! parity spike <job.json> <out.wav>
//! ```
//!
//! MIDI job (P1-5, `check_midi.py`): `{"events": [{"t": s, "v": int}], "note": 36, "ppq":
//! 24000, "tempo": 120.0}` (`v` defaults to 100 like `engine.export_midi`); writes the file
//! and prints `{"notes": [{"tick", "note", "velocity", "duration"}]}`. Spike job: `{"source`:
//! wav, "times": [s, ...], "format": "pcm16"|"pcm24"|"float32"}`; writes the spike file at the
//! source's rate and length with its `bext` and prints `{"sr", "n_samples", "time_reference"}`.
//!
//! ```text
//! parity resample <in.wav> <out.wav> [--mix ffmpeg|average] [--mono-out mono.wav]
//! ```
//!
//! Resampler (P2-2a, `check_resample.py`): reads the source as mono `f32` with the crate's
//! reader, resamples it to 44.1 kHz with `detect::resample` (48 and 96 kHz; 44.1 kHz passes
//! through) and writes a 44.1 kHz float32 WAV. `--mono-out` also writes the unresampled mono
//! signal at the native rate so the Python side can feed soxr the same samples. Prints
//! `{"sr_in", "n_in", "sr_out", "n_out", "taps_per_phase", "seconds"}` (`seconds` is the
//! resampling alone, not the I/O).
//!
//! ```text
//! parity features <wav44|sig.npy> <chain> <out.npy> [--mix ffmpeg|average] [--repeat N]
//! ```
//!
//! Features (P2-2b, `check_features.py`): reads a 44.1 kHz signal (a WAV through the crate's
//! reader, default mix ffmpeg, or a 1-D float32 `.npy` such as the reference `sig44.npy`),
//! computes `detect::features` for the chain (`cli`, `madmom` or `mix`; `mix` = the cli
//! features) and writes the `(frames, 84)` float32 `.npy`. A WAV at 48 or 96 kHz is resampled
//! with `detect::resample` first. `--repeat` times the feature step N times. Prints
//! `{"chain", "sr", "n_samples", "frames", "seconds": [..], "min_s"}`.
//!
//! ```text
//! parity onsets <wav> <role> <out.json> [--mix ffmpeg|average] [--set key=value]...
//!               [--odf-out odf.f32]
//! ```
//!
//! DSP onset fallback (P2-5, `check_onsets.py`): reads the source as mono `f32` (default
//! mix ffmpeg, like the detector input), runs `detect::onset` for the role (`kick`, `snare`,
//! `tom`, `hat`, `cymbal`, `other`) and writes `{"wav", "role", "sr", "n_samples",
//! "frames", "params", "seconds": {"read","resample" | "onsets" | "features", "resample", "spectrogram"}, "hits": [{"frame", "time", "rise_db",
//! "confidence", "flux", "rel_db", "similarity"}]}` to `<out.json>`. `--set` overrides one
//! field of the role's `OnsetParams` for tuning (every field by its name; `gate_db` and
//! `sim_min` also take `none`, `shape` takes `flux` or `spectrum`). `detect` is the whole
//! `detect_onsets_with` call; `resample` and `spectrogram` time those two stages again
//! separately for the breakdown. `--odf-out <file>` also writes the SuperFlux onset function as
//! raw little-endian f32 (one value per frame). Prints a one-line summary.
//!
//! ```text
//! parity project <folder-or-document> <out_dir>
//! parity legacy <session_dir>
//! ```
//!
//! Project round trip (P1-6; `check_project.py` until P3-12): `project` loads a document with the Rust
//! layer (migrating an older format in memory), saves it with `save_project` into
//! `<out_dir>/<folder name>/` and prints `{"loaded": <document>, "saved": <document path>}`;
//! `legacy` reads a format-0 session folder (`meta.json` + `events.json` + `source.wav`) and
//! prints the migrated format-1 document, so the Python side can compare both with
//! `project.load_project`, `project.legacy_session_document` and `project.migrate_forward`.
//!
//! ```text
//! parity attribute <session-manifest.json> <out.json>
//! ```
//!
//! Attribution flags (P2-6, `check_attribution.py`): the manifest is `{"params": {..},
//! "tracks": [{"id", "role", "instrument", "path", "mix", "hits": [{"sample", "rise"}]}]`.
//! `params` overrides fields of `AttributionParams::enabled()` by name (roles by app name;
//! both flags are on unless the manifest turns them off). Every track is decoded with the
//! crate's reader (all at one rate). Writes `{"sr", "seconds", "params", "transfers":
//! [{"source", "target", "gain_db", "hits", "quiet_only"}], "tracks": [{"id", "role",
//! "hits": [{"sample", "rise", "flags", "confidence", "bleed", "resonance"}]}}` and prints a
//! summary (`seconds` is the attribution alone, not the decode).
//!
//! ```text
//! parity tom-bleed <session-manifest.json> <out.json>
//! parity tom-pairs <manifest.json> <out.json>
//! parity flam <manifest.json> <out.json>
//! ```
//!
//! The tom bleed rule (P4-14 stage 2, `tools/tom_bleed_cd.py` `rule_cd`, `tests/test_rust_tom_bleed.py`): the
//! manifest is `{"threshold_db"?, "mics": [{"id", "drum"?, "path", "hits": [sample, ...]}]}` (mono files at one
//! rate, decoded as `average`; `drum` defaults to the id). Writes `{"sr", "off": null | {"kind": "silence" |
//! "played", "mics": [id]}, "mics": [{"id", "strong", "level_db", "level_ok", "louder_share", "played",
//! "silent_share", "silent", "excess": [..], "bleed": [..]}], "transfers": [{"source", "target", "gain_db", "hits",
//! "quiet_only"}], "merged": [[id, id]]}`. Version 2 (P4-17 stage 2, `tools/tom_bleed_sk.py`): the manifest's optional
//! `"sources": [{"id", "kind": "snare" | "kick", "path", "hits"}]` (and `snare_db`, `kick_db`) add per mic
//! `"source_excess": [[snare, kick], ..]` and `"sources": [{"tom", "source", "gain_db", "back_db", "silent_share",
//! "unused": null | "no gain" | "silent" | "shared"}]`.
//!
//! The tom pair rule (P4-21, version 4 with the #182 keep guard, `tools/tom_pairs.py` `pair_marks`,
//! `tests/test_rust_tom_pairs.py`): the manifest
//! is `{"window_ms"?, "margin_db"?, "mics": [{"id", "drum"?, "path", "times": [s, ...], "output"?: [bool, ...],
//! "target"?: [bool, ...], "guard"?: bool}]}` (`output` and `target` default to all true, `guard` to true). Writes
//! `{"sr", "levels": [[dB]], "rises": [[dB | null]], "marks": [[bool]]}`
//!
//! The flam rule (P4-13 stage B, `tools/flam_pairs.py` `second_onsets` as `tools/flam_split.py` runs it,
//! `tests/test_rust_flam.py`): the manifest is `{"path", "hits": [s, ...], "sources"?: [{"id", "path"}], "rise_db"?,
//! "margin_db"?, "min_gap_s"?}` (decoded as `average`; a source at another rate is left out). Writes `{"sr", "seconds",
//! "found": [{"t", "first", "gap_ms", "rise_db", "level_db", "bleed"}], "gains": [{"id", "gain_db"}], "added": [s],
//! "late_peak_db", "late_share"}` (the late peak of the mic over `hits`, `null` without a value: the app's gate under -3 dB).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use drumspike_core::detect::attribution::{
    attribute, AttributionParams, FilterKind, HitInput, Role as AttrRole, TrackInput,
};
use drumspike_core::detect::features::{features, num_frames, N_BANDS};
use drumspike_core::detect::flam::zero_phase;
use drumspike_core::detect::onset::{band_range, detect_onsets_with, superflux, OnsetParams, Role, Shape};
use drumspike_core::detect::resample::{resample_to_44100, Resampler, TARGET_RATE};
use drumspike_core::detect::tom_bleed::{
    pair_level_db, pair_marks, stroke_rise_db, tom_bleed_with, Off as BleedOff, PairMic, SourceKind, SourceMic,
    TomBleedParams, TomMic, Unused, PAIR_HP_HZ,
};
use drumspike_core::detect::Chain;
use drumspike_core::io::{decode_many, read_wav_mono, write_wav, MappedWav, MixMode, WavFormat, WavInfo, WavSpec};
use drumspike_core::midi::{
    export_midi, note_events, MidiEvent, NoteEvent, DEFAULT_PPQ, DEFAULT_TEMPO, DEFAULT_VELOCITY,
};
use drumspike_core::peaks::{PeakLevel, Track};
use drumspike_core::project::{self, Project};
use drumspike_core::refine::{refine_rule_peak, refine_sr, BackStep, ReachStep, RefineRule, RefineWindow, FM_PEAK_DB};
use drumspike_core::spike::export_spikes;
use drumspike_core::velocity::{velocity_map, VelocityMap};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct MidiJobEvent {
    t: f64,
    v: Option<i64>,
}

#[derive(Deserialize)]
struct MidiJob {
    events: Vec<MidiJobEvent>,
    #[serde(default = "default_note")]
    note: i64,
    #[serde(default = "default_ppq")]
    ppq: u16,
    #[serde(default = "default_tempo")]
    tempo: f64,
}

fn default_note() -> i64 {
    36
}
fn default_ppq() -> u16 {
    DEFAULT_PPQ
}
fn default_tempo() -> f64 {
    DEFAULT_TEMPO
}

#[derive(Serialize)]
struct MidiOut {
    notes: Vec<NoteEvent>,
}

fn run_midi(job_path: &Path, out: &Path) -> Result<(), String> {
    let s = std::fs::read_to_string(job_path).map_err(|e| format!("read {}: {e}", job_path.display()))?;
    let job: MidiJob = serde_json::from_str(&s).map_err(|e| format!("job json: {e}"))?;
    let events: Vec<MidiEvent> = job
        .events
        .iter()
        .map(|e| MidiEvent {
            t: e.t,
            v: e.v.unwrap_or(DEFAULT_VELOCITY),
        })
        .collect();
    let notes = note_events(&events, job.note, job.ppq, job.tempo).map_err(|e| e.to_string())?;
    export_midi(&events, job.note, out, job.ppq, job.tempo).map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string(&MidiOut { notes }).expect("json"));
    Ok(())
}

#[derive(Deserialize)]
struct SpikeJob {
    source: PathBuf,
    times: Vec<f64>,
    #[serde(default)]
    format: Option<String>,
}

#[derive(Serialize)]
struct SpikeOut {
    sr: u32,
    n_samples: usize,
    time_reference: Option<u64>,
}

fn run_spike(job_path: &Path, out: &Path) -> Result<(), String> {
    let s = std::fs::read_to_string(job_path).map_err(|e| format!("read {}: {e}", job_path.display()))?;
    let job: SpikeJob = serde_json::from_str(&s).map_err(|e| format!("job json: {e}"))?;
    let format = match job.format.as_deref().unwrap_or("pcm16") {
        "pcm16" => WavFormat::Pcm16,
        "pcm24" => WavFormat::Pcm24,
        "pcm32" => WavFormat::Pcm32,
        "float32" => WavFormat::Float32,
        f => return Err(format!("unknown format {f}")),
    };
    let src = MappedWav::open(&job.source).map_err(|e| e.to_string())?;
    let info = &src.info;
    export_spikes(
        out,
        info.sample_rate,
        info.n_frames,
        &job.times,
        info.bext.as_ref(),
        format,
    )
    .map_err(|e| e.to_string())?;
    let o = SpikeOut {
        sr: info.sample_rate,
        n_samples: info.n_frames,
        time_reference: info.time_reference(),
    };
    println!("{}", serde_json::to_string(&o).expect("json"));
    Ok(())
}

#[derive(Deserialize)]
struct RefineHit {
    t0: f64,
    #[serde(default)]
    neighbors: Vec<f64>,
}

#[derive(Deserialize)]
struct RefineJob {
    wav: PathBuf,
    #[serde(default)]
    mix: Option<String>,
    hits: Vec<RefineHit>,
    #[serde(default)]
    window: Option<(f64, f64)>,
    #[serde(default)]
    rule: Option<u64>,
    /// P4-31: the back-track peak margin in dB (default: the shipped one); `tests/test_rust_kick_walk.py`.
    #[serde(default)]
    peak_db: Option<f64>,
}

#[derive(Serialize)]
struct Refined {
    t: f64,
    rise: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    step: Option<&'static str>,
    /// P4-12 stage 2: what the back-track did (rule 3 only).
    #[serde(skip_serializing_if = "Option::is_none")]
    back: Option<&'static str>,
}

#[derive(Serialize)]
struct RefineOut {
    sr: u32,
    refined: Vec<Refined>,
    velocities: Vec<i64>,
}

fn parse_mix(s: Option<&str>) -> Option<MixMode> {
    match s.unwrap_or("ffmpeg") {
        "ffmpeg" => Some(MixMode::Ffmpeg),
        "average" => Some(MixMode::Average),
        _ => None,
    }
}

fn run_refine(job_path: &Path) -> Result<(), String> {
    let s = std::fs::read_to_string(job_path).map_err(|e| format!("read {}: {e}", job_path.display()))?;
    let job: RefineJob = serde_json::from_str(&s).map_err(|e| format!("job json: {e}"))?;
    let mix = parse_mix(job.mix.as_deref()).ok_or_else(|| format!("unknown mix mode {:?}", job.mix))?;
    let (x, info) = read_wav_mono(&job.wav, mix).map_err(|e| e.to_string())?;
    let sr = info.sample_rate;
    let win = job
        .window
        .map_or(RefineWindow::DEFAULT, |(lo_ms, hi_ms)| RefineWindow { lo_ms, hi_ms });
    let rule = match job.rule {
        None => RefineRule::Reference,
        Some(v) => RefineRule::from_version(v).ok_or_else(|| format!("unknown rule {v}"))?,
    };
    let refined: Vec<Refined> = job
        .hits
        .iter()
        .map(|h| match rule {
            RefineRule::Reference => {
                let (t, rise) = refine_sr(&x, h.t0, sr, &h.neighbors, win);
                Refined {
                    t,
                    rise,
                    step: None,
                    back: None,
                }
            }
            RefineRule::Reach | RefineRule::FirstMotion | RefineRule::Foot | RefineRule::FootCapped => {
                let r = refine_rule_peak(&x, h.t0, sr, &h.neighbors, win, rule, job.peak_db.unwrap_or(FM_PEAK_DB));
                let step = match r.step {
                    ReachStep::Kept => "kept",
                    ReachStep::Reached => "reached",
                    ReachStep::Tail => "tail",
                    ReachStep::Unresolved => "unresolved",
                };
                let back = match r.back {
                    BackStep::Off => None,
                    BackStep::Moved => Some("moved"),
                    BackStep::Kept => Some("kept"),
                    BackStep::Busy => Some("busy"),
                    BackStep::Separate => Some("separate"),
                };
                Refined {
                    t: r.t,
                    rise: r.rise,
                    step: Some(step),
                    back,
                }
            }
        })
        .collect();
    let times: Vec<f64> = refined.iter().map(|r| r.t).collect();
    let velocities = velocity_map(&x, sr, &times, &VelocityMap::default());
    let out = RefineOut {
        sr,
        refined,
        velocities,
    };
    println!("{}", serde_json::to_string(&out).expect("json"));
    Ok(())
}

#[derive(Serialize)]
struct Tile {
    level: usize,
    from: i64,
    to: i64,
    start: usize,
    min: Vec<f64>,
    max: Vec<f64>,
    rms: Vec<f64>,
}

#[derive(Serialize)]
struct Window {
    from: i64,
    to: i64,
    start: usize,
    data: Vec<f32>,
}

#[derive(Serialize)]
struct Out {
    info: WavInfo,
    n_samples: usize,
    peaks: BTreeMap<String, PeakLevel>,
    velocities: Vec<i64>,
    tiles: Vec<Option<Tile>>,
    samples: Vec<Window>,
}

fn run_project(input: &Path, out_dir: &Path) -> Result<(), String> {
    let loaded = project::load_project(input).map_err(|e| e.to_string())?;
    let loaded_json: serde_json::Value = serde_json::to_value(&loaded.doc).map_err(|e| e.to_string())?;
    let pdir = out_dir.join(project::basename(&loaded.dir));
    project::ensure_layout(&pdir).map_err(|e| e.to_string())?;
    let mut saved = Project::new(&pdir, loaded.doc, None);
    let path = project::save_project(&mut saved).map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::json!({"loaded": loaded_json, "saved": path.to_string_lossy()})
    );
    Ok(())
}

fn run_legacy(session_dir: &Path) -> Result<(), String> {
    let doc0 = project::legacy_session_document(session_dir).map_err(|e| e.to_string())?;
    let doc = project::migrate_forward(doc0).map_err(|e| e.to_string())?;
    println!("{}", doc.to_json());
    Ok(())
}

#[derive(Serialize)]
struct ResampleOut {
    sr_in: u32,
    n_in: usize,
    sr_out: u32,
    n_out: usize,
    taps_per_phase: usize,
    seconds: f64,
}

fn run_resample(args: &[String]) -> Result<(), String> {
    let (Some(input), Some(out)) = (args.first(), args.get(1)) else {
        return Err("usage: parity resample <in.wav> <out.wav> [--mix ffmpeg|average] [--mono-out mono.wav]".into());
    };
    let mut mix = MixMode::Ffmpeg;
    let mut mono_out: Option<PathBuf> = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--mix" => {
                mix = parse_mix(args.get(i + 1).map(String::as_str))
                    .ok_or_else(|| format!("unknown mix mode {:?}", args.get(i + 1)))?;
                i += 2;
            }
            "--mono-out" => {
                mono_out = Some(PathBuf::from(args.get(i + 1).ok_or("--mono-out needs a path")?));
                i += 2;
            }
            a => return Err(format!("unknown argument {a}")),
        }
    }
    let (x, info) = read_wav_mono(Path::new(input), mix).map_err(|e| e.to_string())?;
    let spec = |sample_rate| WavSpec {
        sample_rate,
        channels: 1,
        format: WavFormat::Float32,
        extensible: false,
        channel_mask: 0,
    };
    if let Some(p) = mono_out {
        write_wav(&p, &spec(info.sample_rate), &x, None).map_err(|e| format!("write {}: {e}", p.display()))?;
    }
    let resampler = Resampler::new(info.sample_rate).map_err(|e| e.to_string())?;
    let t = std::time::Instant::now();
    let y = resampler.process(&x);
    let seconds = t.elapsed().as_secs_f64();
    write_wav(Path::new(out), &spec(TARGET_RATE), &y, None).map_err(|e| format!("write {out}: {e}"))?;
    let o = ResampleOut {
        sr_in: info.sample_rate,
        n_in: x.len(),
        sr_out: TARGET_RATE,
        n_out: y.len(),
        taps_per_phase: resampler.taps_per_phase(),
        seconds,
    };
    println!("{}", serde_json::to_string(&o).expect("json"));
    Ok(())
}

/// Read a little-endian float32 C-order `.npy` (format 1.0, 2.0 or 3.0): `(shape, data)`.
fn read_npy_f32(path: &Path) -> Result<(Vec<usize>, Vec<f32>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if bytes.len() < 10 || &bytes[..6] != b"\x93NUMPY" {
        return Err(format!("{}: not a .npy file", path.display()));
    }
    let (header_len, start) = match bytes[6] {
        1 => (u16::from_le_bytes([bytes[8], bytes[9]]) as usize, 10),
        2 | 3 if bytes.len() >= 12 => (
            u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize,
            12,
        ),
        v => return Err(format!("{}: unsupported .npy version {v}", path.display())),
    };
    let header = std::str::from_utf8(bytes.get(start..start + header_len).ok_or("short .npy header")?)
        .map_err(|e| format!("npy header: {e}"))?;
    if !header.contains("'descr': '<f4'") || !header.contains("'fortran_order': False") {
        return Err(format!("{}: need '<f4' in C order, header {header}", path.display()));
    }
    let shape = header
        .split("'shape': (")
        .nth(1)
        .and_then(|r| r.split(')').next())
        .ok_or_else(|| format!("npy header without shape: {header}"))?;
    let dims: Vec<usize> = shape
        .split(',')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(|d| d.parse::<usize>().map_err(|e| format!("npy shape {shape}: {e}")))
        .collect::<Result<_, _>>()?;
    let body = &bytes[start + header_len..];
    let count: usize = dims.iter().product();
    if body.len() != count * 4 {
        return Err(format!(
            "{}: {} data bytes for shape ({shape})",
            path.display(),
            body.len()
        ));
    }
    let data = body.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
    Ok((dims, data))
}

/// Read a 1-D float32 `.npy`.
fn read_npy_f32_1d(path: &Path) -> Result<Vec<f32>, String> {
    let (dims, data) = read_npy_f32(path)?;
    if dims.len() != 1 {
        return Err(format!("{}: shape {dims:?} is not 1-D", path.display()));
    }
    Ok(data)
}

/// A 44.1 kHz mono signal from a 1-D float32 `.npy` or a WAV (resampled to 44.1 kHz when it
/// is at 48 or 96 kHz): `(signal, source rate)`.
fn read_signal_44k(path: &Path, mix: MixMode) -> Result<(Vec<f32>, u32), String> {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("npy")) {
        return Ok((read_npy_f32_1d(path)?, TARGET_RATE));
    }
    let (x, info) = read_wav_mono(path, mix).map_err(|e| e.to_string())?;
    let y = resample_to_44100(&x, info.sample_rate).map_err(|e| e.to_string())?;
    Ok((y, info.sample_rate))
}

/// Parse `--mix` and `--repeat` (the shared options of `features` and `detect`).
fn parse_mix_repeat(args: &[String]) -> Result<(MixMode, usize), String> {
    let mut mix = MixMode::Ffmpeg;
    let mut repeat = 1usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mix" => {
                mix = parse_mix(args.get(i + 1).map(String::as_str))
                    .ok_or_else(|| format!("unknown mix mode {:?}", args.get(i + 1)))?;
            }
            "--repeat" => {
                repeat = args
                    .get(i + 1)
                    .ok_or("--repeat needs a number")?
                    .parse::<usize>()
                    .map_err(|e| format!("--repeat: {e}"))?
                    .max(1);
            }
            a => return Err(format!("unknown argument {a}")),
        }
        i += 2;
    }
    Ok((mix, repeat))
}

fn parse_chain(s: &str) -> Result<Chain, String> {
    Chain::parse(s).ok_or_else(|| format!("unknown chain {s} (cli, madmom or mix)"))
}

fn run_features(args: &[String]) -> Result<(), String> {
    let (Some(input), Some(chain), Some(out)) = (args.first(), args.get(1), args.get(2)) else {
        return Err(
            "usage: parity features <wav44|sig.npy> <chain> <out.npy> [--mix ffmpeg|average] [--repeat N]".into(),
        );
    };
    let chain = parse_chain(chain)?;
    let (mix, repeat) = parse_mix_repeat(&args[3..])?;
    let (x, sr) = read_signal_44k(Path::new(input), mix)?;
    let mut seconds = Vec::with_capacity(repeat);
    let mut f = Vec::new();
    for _ in 0..repeat {
        let t = std::time::Instant::now();
        f = features(&x, chain);
        seconds.push(t.elapsed().as_secs_f64());
    }
    let frames = num_frames(x.len(), chain);
    write_npy_f32_2d(Path::new(out), frames, N_BANDS, &f)?;
    let doc = serde_json::json!({
        "chain": chain.name(),
        "sr": sr,
        "n_samples": x.len(),
        "frames": frames,
        "min_s": seconds.iter().cloned().fold(f64::INFINITY, f64::min),
        "seconds": seconds,
    });
    println!("{doc}");
    Ok(())
}

/// Write a 2-D little-endian float32 C-order `.npy` (format 1.0, header padded to 64 bytes).
fn write_npy_f32_2d(path: &Path, rows: usize, cols: usize, data: &[f32]) -> Result<(), String> {
    assert_eq!(data.len(), rows * cols);
    let mut header = format!("{{'descr': '<f4', 'fortran_order': False, 'shape': ({rows}, {cols}), }}");
    while (10 + header.len() + 1) % 64 != 0 {
        header.push(' ');
    }
    header.push('\n');
    let mut buf = Vec::with_capacity(10 + header.len() + data.len() * 4);
    buf.extend_from_slice(b"\x93NUMPY\x01\x00");
    buf.extend_from_slice(&(header.len() as u16).to_le_bytes());
    buf.extend_from_slice(header.as_bytes());
    for v in data {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, buf).map_err(|e| format!("write {}: {e}", path.display()))
}
#[derive(Deserialize)]
struct AttrHit {
    sample: i64,
    #[serde(default)]
    rise: Option<f64>,
}

#[derive(Deserialize)]
struct AttrTrack {
    id: String,
    role: String,
    #[serde(default)]
    instrument: Option<String>,
    path: PathBuf,
    #[serde(default)]
    mix: Option<String>,
    hits: Vec<AttrHit>,
}

#[derive(Deserialize, Default)]
struct AttrParams {
    bleed_enabled: Option<bool>,
    resonance_enabled: Option<bool>,
    bleed_roles: Option<Vec<String>>,
    resonance_roles: Option<Vec<String>>,
    window_ms: Option<f64>,
    lag_tol_ms: Option<f64>,
    source_rise_db: Option<f64>,
    source_match_ms: Option<f64>,
    margin_db: Option<f64>,
    dominance_db: Option<f64>,
    own_window_ms: Option<(f64, f64)>,
    pair_window_ms: Option<(f64, f64)>,
    preroll_ms: Option<f64>,
    silence_db: Option<f64>,
    calib_strong_db: Option<f64>,
    calib_quiet_ms: Option<f64>,
    calib_percentile: Option<f64>,
    calib_min_hits: Option<usize>,
    resonance_db: Option<f64>,
}

#[derive(Deserialize)]
struct AttrJob {
    #[serde(default)]
    params: AttrParams,
    tracks: Vec<AttrTrack>,
}

fn roles(names: &[String]) -> Result<Vec<AttrRole>, String> {
    names
        .iter()
        .map(|n| AttrRole::from_name(n).ok_or_else(|| format!("unknown role {n}")))
        .collect()
}

fn attr_params(o: &AttrParams) -> Result<AttributionParams, String> {
    let mut p = AttributionParams::enabled();
    macro_rules! set {
        ($($f:ident),*) => { $( if let Some(v) = o.$f { p.$f = v; } )* };
    }
    set!(
        bleed_enabled,
        resonance_enabled,
        window_ms,
        lag_tol_ms,
        source_rise_db,
        source_match_ms,
        margin_db,
        dominance_db,
        own_window_ms,
        pair_window_ms,
        preroll_ms,
        silence_db,
        calib_strong_db,
        calib_quiet_ms,
        calib_percentile,
        calib_min_hits,
        resonance_db
    );
    if let Some(r) = &o.bleed_roles {
        p.bleed_roles = roles(r)?;
    }
    if let Some(r) = &o.resonance_roles {
        p.resonance_roles = roles(r)?;
    }
    Ok(p)
}

fn run_attribute(job_path: &Path, out: &Path) -> Result<(), String> {
    let s = std::fs::read_to_string(job_path).map_err(|e| format!("read {}: {e}", job_path.display()))?;
    let job: AttrJob = serde_json::from_str(&s).map_err(|e| format!("manifest json: {e}"))?;
    let params = attr_params(&job.params)?;
    let mut mix = None;
    for t in &job.tracks {
        let m = parse_mix(t.mix.as_deref()).ok_or_else(|| format!("unknown mix mode {:?}", t.mix))?;
        if mix.is_some_and(|x| x != m) {
            return Err("all tracks of a manifest must use one mix mode".into());
        }
        mix = Some(m);
    }
    let paths: Vec<&PathBuf> = job.tracks.iter().map(|t| &t.path).collect();
    let decoded = decode_many(&paths, mix.unwrap_or(MixMode::Ffmpeg));
    let mut signals = Vec::with_capacity(decoded.len());
    let mut sr = None;
    for (d, t) in decoded.into_iter().zip(&job.tracks) {
        let (x, info) = d.map_err(|e| format!("{}: {e}", t.path.display()))?;
        if sr.is_some_and(|r| r != info.sample_rate) {
            return Err(format!(
                "{}: rate {} differs from the session",
                t.path.display(),
                info.sample_rate
            ));
        }
        sr = Some(info.sample_rate);
        signals.push(x);
    }
    let sr = sr.ok_or("manifest has no tracks")?;
    let mut inputs = Vec::with_capacity(job.tracks.len());
    for (t, x) in job.tracks.iter().zip(&signals) {
        inputs.push(TrackInput {
            id: t.id.clone(),
            role: AttrRole::from_name(&t.role).ok_or_else(|| format!("unknown role {}", t.role))?,
            instrument: t.instrument.clone().unwrap_or_else(|| t.id.clone()),
            signal: x,
            hits: t
                .hits
                .iter()
                .map(|h| HitInput {
                    sample: h.sample,
                    rise: h.rise,
                })
                .collect(),
        });
    }
    let t0 = std::time::Instant::now();
    let res = attribute(&inputs, sr, &params).map_err(|e| e.to_string())?;
    let seconds = t0.elapsed().as_secs_f64();
    let id = |i: usize| serde_json::Value::String(inputs[i].id.clone());
    let tracks: Vec<serde_json::Value> = inputs
        .iter()
        .zip(&res.flags)
        .map(|(t, fl)| {
            let hits: Vec<serde_json::Value> = t
                .hits
                .iter()
                .zip(fl)
                .map(|(h, f)| {
                    serde_json::json!({
                        "sample": h.sample,
                        "rise": h.rise,
                        "flags": f.names(),
                        "confidence": f.confidence(),
                        "bleed": f.bleed.map(|b| serde_json::json!({
                            "source": id(b.source), "lag_samples": b.lag_samples, "excess_db": b.excess_db,
                            "dominance_db": b.dominance_db, "confidence": b.confidence})),
                        "resonance": f.resonance.map(|r| serde_json::json!({
                            "rise_db": r.rise_db, "median_db": r.median_db, "confidence": r.confidence})),
                    })
                })
                .collect();
            serde_json::json!({"id": t.id, "role": t.role.name(), "hits": hits})
        })
        .collect();
    let transfers: Vec<serde_json::Value> = res
        .transfers
        .iter()
        .map(|g| {
            serde_json::json!({"source": id(g.source), "target": id(g.target), "gain_db": g.gain_db,
                               "hits": g.hits, "quiet_only": g.quiet_only})
        })
        .collect();
    let role_names = |v: &[AttrRole]| v.iter().map(|r| r.name()).collect::<Vec<_>>();
    let doc = serde_json::json!({
        "sr": sr,
        "seconds": seconds,
        "params": {
            "bleed_enabled": params.bleed_enabled, "resonance_enabled": params.resonance_enabled,
            "bleed_roles": role_names(&params.bleed_roles), "resonance_roles": role_names(&params.resonance_roles),
            "window_ms": params.window_ms, "lag_tol_ms": params.lag_tol_ms, "source_rise_db": params.source_rise_db,
            "source_match_ms": params.source_match_ms, "margin_db": params.margin_db,
            "dominance_db": params.dominance_db, "own_window_ms": params.own_window_ms,
            "pair_window_ms": params.pair_window_ms, "preroll_ms": params.preroll_ms,
            "silence_db": params.silence_db,
            "calib_strong_db": params.calib_strong_db, "calib_quiet_ms": params.calib_quiet_ms,
            "calib_percentile": params.calib_percentile, "calib_min_hits": params.calib_min_hits,
            "resonance_db": params.resonance_db,
        },
        "transfers": transfers,
        "tracks": tracks,
    });
    std::fs::write(out, serde_json::to_string(&doc).expect("json"))
        .map_err(|e| format!("write {}: {e}", out.display()))?;
    let n_hits: usize = res.flags.iter().map(Vec::len).sum();
    let n_bleed = res.flags.iter().flatten().filter(|f| f.bleed.is_some()).count();
    let n_res = res.flags.iter().flatten().filter(|f| f.resonance.is_some()).count();
    println!(
        "{}",
        serde_json::json!({"sr": sr, "tracks": inputs.len(), "hits": n_hits, "bleed": n_bleed,
                           "resonance": n_res, "seconds": seconds})
    );
    Ok(())
}

fn set_param(p: &mut OnsetParams, kv: &str) -> Result<(), String> {
    let (k, v) = kv
        .split_once('=')
        .ok_or_else(|| format!("--set needs key=value, got {kv}"))?;
    let f = || v.parse::<f64>().map_err(|e| format!("--set {k}: {e}"));
    let u = || v.parse::<usize>().map_err(|e| format!("--set {k}: {e}"));
    match k {
        "band_lo_hz" => p.band_lo_hz = f()?,
        "band_hi_hz" => p.band_hi_hz = f()?,
        "delta" => p.delta = f()? as f32,
        "pre_avg" => p.pre_avg = u()?,
        "post_avg" => p.post_avg = u()?,
        "pre_max" => p.pre_max = u()?,
        "post_max" => p.post_max = u()?,
        "ref_quantile" => p.ref_quantile = f()? as f32,
        "gate_db" => p.gate_db = if v == "none" { None } else { Some(f()? as f32) },
        "template_db" => p.template_db = f()? as f32,
        "shape" => {
            p.shape = match v {
                "flux" => Shape::Flux,
                "spectrum" => Shape::Spectrum,
                _ => return Err(format!("--set shape: flux or spectrum, got {v}")),
            }
        }
        "sim_min" => p.sim_min = if v == "none" { None } else { Some(f()? as f32) },
        "sim_gate_db" => p.sim_gate_db = f()? as f32,
        "weak_ratio" => p.weak_ratio = f()? as f32,
        "attack_db" => p.attack_db = f()? as f32,
        "min_gap_s" => p.min_gap_s = f()?,
        _ => return Err(format!("--set: unknown parameter {k}")),
    }
    Ok(())
}

fn run_onsets(args: &[String]) -> Result<(), String> {
    let (Some(input), Some(role), Some(out)) = (args.first(), args.get(1), args.get(2)) else {
        return Err("usage: parity onsets <wav> <role> <out.json> [--mix ffmpeg|average] [--set key=value]...".into());
    };
    let role = Role::parse(role).ok_or_else(|| format!("unknown role {role}"))?;
    let mut params = OnsetParams::for_role(role);
    let mut mix = MixMode::Ffmpeg;
    let mut odf_out: Option<PathBuf> = None;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--mix" => {
                mix = parse_mix(args.get(i + 1).map(String::as_str))
                    .ok_or_else(|| format!("unknown mix mode {:?}", args.get(i + 1)))?;
                i += 2;
            }
            "--set" => {
                set_param(&mut params, args.get(i + 1).ok_or("--set needs key=value")?)?;
                i += 2;
            }
            "--odf-out" => {
                odf_out = Some(PathBuf::from(args.get(i + 1).ok_or("--odf-out needs a path")?));
                i += 2;
            }
            a => return Err(format!("unknown argument {a}")),
        }
    }
    let t = std::time::Instant::now();
    let (x, info) = read_wav_mono(Path::new(input), mix).map_err(|e| e.to_string())?;
    let read_s = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    let hits = detect_onsets_with(&x, info.sample_rate, &params).map_err(|e| e.to_string())?;
    let detect_s = t.elapsed().as_secs_f64();
    let n44 = Resampler::shared(info.sample_rate)
        .map_err(|e| e.to_string())?
        .output_len(x.len());
    // The two front stages again, one by one, for the timing breakdown (the hits above are
    // the result).
    let t = std::time::Instant::now();
    let y = resample_to_44100(&x, info.sample_rate).map_err(|e| e.to_string())?;
    let resample_s = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    let spec = features(&y, Chain::Cli);
    let spectrogram_s = t.elapsed().as_secs_f64();
    if let Some(p) = odf_out {
        let flux = superflux(&spec, band_range(params.band_lo_hz, params.band_hi_hz));
        let bytes: Vec<u8> = flux.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(&p, bytes).map_err(|e| format!("write {}: {e}", p.display()))?;
    }
    let doc = serde_json::json!({
        "wav": input,
        "role": role.name(),
        "sr": info.sample_rate,
        "n_samples": x.len(),
        "frames": num_frames(n44, Chain::Cli),
        "params": {
            "band_lo_hz": params.band_lo_hz, "band_hi_hz": params.band_hi_hz, "delta": params.delta,
            "pre_avg": params.pre_avg, "post_avg": params.post_avg,
            "pre_max": params.pre_max, "post_max": params.post_max,
            "ref_quantile": params.ref_quantile, "gate_db": params.gate_db, "template_db": params.template_db,
            "shape": format!("{:?}", params.shape).to_lowercase(),
            "sim_min": params.sim_min, "sim_gate_db": params.sim_gate_db,
            "weak_ratio": params.weak_ratio, "attack_db": params.attack_db, "min_gap_s": params.min_gap_s,
        },
        "seconds": {"read": read_s, "detect": detect_s, "resample": resample_s, "spectrogram": spectrogram_s},
        "hits": hits.iter().map(|h| serde_json::json!({
            "frame": h.frame, "time": h.time, "rise_db": h.rise_db,
            "confidence": h.confidence, "flux": h.flux, "rel_db": h.rel_db, "similarity": h.similarity,
        })).collect::<Vec<_>>(),
    });
    std::fs::write(out, serde_json::to_string(&doc).expect("json")).map_err(|e| format!("write {out}: {e}"))?;
    println!("{} hits, detect {:.3} s", hits.len(), detect_s);
    Ok(())
}

#[derive(Deserialize)]
struct BleedMic {
    id: String,
    #[serde(default)]
    drum: Option<String>,
    path: PathBuf,
    hits: Vec<i64>,
}

#[derive(Deserialize)]
struct BleedSource {
    id: String,
    kind: String,
    path: PathBuf,
    hits: Vec<i64>,
}

#[derive(Deserialize)]
struct BleedJob {
    #[serde(default)]
    threshold_db: Option<f64>,
    #[serde(default)]
    snare_db: Option<f64>,
    #[serde(default)]
    kick_db: Option<f64>,
    mics: Vec<BleedMic>,
    #[serde(default)]
    sources: Vec<BleedSource>,
}

fn run_tom_bleed(job_path: &Path, out: &Path) -> Result<(), String> {
    let s = std::fs::read_to_string(job_path).map_err(|e| format!("read {}: {e}", job_path.display()))?;
    let job: BleedJob = serde_json::from_str(&s).map_err(|e| format!("manifest json: {e}"))?;
    let paths: Vec<&PathBuf> = job
        .mics
        .iter()
        .map(|m| &m.path)
        .chain(job.sources.iter().map(|s| &s.path))
        .collect();
    let mut signals = Vec::with_capacity(paths.len());
    let mut sr = None;
    for (d, path) in decode_many(&paths, MixMode::Average).into_iter().zip(&paths) {
        let (x, info) = d.map_err(|e| format!("{}: {e}", path.display()))?;
        if sr.is_some_and(|r| r != info.sample_rate) {
            return Err(format!(
                "{}: rate {} differs from the session",
                path.display(),
                info.sample_rate
            ));
        }
        sr = Some(info.sample_rate);
        signals.push(x);
    }
    let sr = sr.unwrap_or(48_000);
    let mics: Vec<TomMic> = job
        .mics
        .iter()
        .zip(&signals)
        .map(|(m, x)| TomMic {
            drum: m.drum.clone().unwrap_or_else(|| m.id.clone()),
            signal: x,
            hits: m.hits.clone(),
        })
        .collect();
    let mut params = TomBleedParams::default();
    if let Some(t) = job.threshold_db {
        params.threshold_db = t;
    }
    if let Some(t) = job.snare_db {
        params.snare_threshold_db = t;
    }
    if let Some(t) = job.kick_db {
        params.kick_threshold_db = t;
    }
    let mut sources = Vec::with_capacity(job.sources.len());
    for (s, x) in job.sources.iter().zip(&signals[job.mics.len()..]) {
        let kind = match s.kind.as_str() {
            "snare" => SourceKind::Snare,
            "kick" => SourceKind::Kick,
            k => return Err(format!("{}: kind {k} is not snare or kick", s.id)),
        };
        sources.push(SourceMic {
            kind,
            signal: x,
            hits: s.hits.clone(),
        });
    }
    let r = tom_bleed_with(&mics, &sources, sr, &params).map_err(|e| e.to_string())?;
    let id = |i: usize| serde_json::Value::String(job.mics[i].id.clone());
    let off = match &r.off {
        None => serde_json::Value::Null,
        Some(BleedOff::DigitalSilence { mics }) => {
            serde_json::json!({"kind": "silence", "mics": mics.iter().map(|&i| id(i)).collect::<Vec<_>>()})
        }
        Some(BleedOff::FewerThanTwoPlayed { played }) => {
            serde_json::json!({"kind": "played", "mics": played.iter().map(|&i| id(i)).collect::<Vec<_>>()})
        }
    };
    let mic_rows: Vec<serde_json::Value> = r
        .mics
        .iter()
        .enumerate()
        .map(|(i, m)| {
            serde_json::json!({"id": id(i), "strong": m.strong, "level_db": m.level_db, "level_ok": m.level_ok,
                "louder_share": m.louder_share, "played": m.played, "silent_share": m.silent_share,
                "silent": m.silent, "excess": r.excess[i], "bleed": r.bleed[i],
                "source_excess": r.source_excess.get(i)})
        })
        .collect();
    let transfers: Vec<serde_json::Value> = r
        .transfers
        .iter()
        .map(|g| {
            serde_json::json!({"source": id(g.source), "target": id(g.target), "gain_db": g.gain_db,
                               "hits": g.hits, "quiet_only": g.quiet_only})
        })
        .collect();
    let merged: Vec<serde_json::Value> = r
        .merged
        .iter()
        .map(|&(a, b)| serde_json::json!([id(a), id(b)]))
        .collect();
    let uses: Vec<serde_json::Value> = r
        .sources
        .iter()
        .map(|u| {
            let unused = u.unused.map(|w| match w {
                Unused::NoGain => "no gain",
                Unused::Silent => "silent",
                Unused::Shared => "shared",
            });
            serde_json::json!({"tom": id(u.tom), "source": job.sources[u.source].id, "gain_db": u.gain_db,
                               "back_db": u.back_db, "silent_share": u.silent_share, "unused": unused})
        })
        .collect();
    let doc = serde_json::json!({"sr": sr, "off": off, "mics": mic_rows, "transfers": transfers, "merged": merged,
                                 "sources": uses});
    std::fs::write(out, serde_json::to_string(&doc).expect("json"))
        .map_err(|e| format!("write {}: {e}", out.display()))?;
    println!("{} mics, {} hits marked, off: {:?}", job.mics.len(), r.n_bleed(), r.off);
    Ok(())
}

#[derive(Deserialize)]
struct PairJobMic {
    id: String,
    #[serde(default)]
    drum: Option<String>,
    path: PathBuf,
    times: Vec<f64>,
    #[serde(default)]
    output: Option<Vec<bool>>,
    #[serde(default)]
    target: Option<Vec<bool>>,
    /// Version 4: the mic's hits keep the own-mic attack rise guard (the default: the current rule).
    #[serde(default = "default_true")]
    guard: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct PairJob {
    #[serde(default)]
    window_ms: Option<f64>,
    #[serde(default)]
    margin_db: Option<f64>,
    #[serde(default)]
    floor_db: Option<f64>,
    mics: Vec<PairJobMic>,
}

fn run_tom_pairs(job_path: &Path, out: &Path) -> Result<(), String> {
    let s = std::fs::read_to_string(job_path).map_err(|e| format!("read {}: {e}", job_path.display()))?;
    let job: PairJob = serde_json::from_str(&s).map_err(|e| format!("manifest json: {e}"))?;
    let paths: Vec<&PathBuf> = job.mics.iter().map(|m| &m.path).collect();
    let mut signals = Vec::with_capacity(paths.len());
    let mut sr = None;
    for (d, path) in decode_many(&paths, MixMode::Average).into_iter().zip(&paths) {
        let (x, info) = d.map_err(|e| format!("{}: {e}", path.display()))?;
        if sr.is_some_and(|r| r != info.sample_rate) {
            return Err(format!(
                "{}: rate {} differs from the session",
                path.display(),
                info.sample_rate
            ));
        }
        sr = Some(info.sample_rate);
        signals.push(x);
    }
    let sr = sr.unwrap_or(48_000);
    let mut params = TomBleedParams::default();
    if let Some(w) = job.window_ms {
        params.winner_window_ms = w;
    }
    if let Some(m) = job.margin_db {
        params.winner_margin_db = m;
    }
    if let Some(f) = job.floor_db {
        params.winner_floor_db = f;
    }
    let ones = |n: usize| vec![true; n];
    let outputs: Vec<Vec<bool>> = job
        .mics
        .iter()
        .map(|m| m.output.clone().unwrap_or_else(|| ones(m.times.len())))
        .collect();
    let targets: Vec<Vec<bool>> = job
        .mics
        .iter()
        .map(|m| m.target.clone().unwrap_or_else(|| ones(m.times.len())))
        .collect();
    let drums: Vec<String> = job
        .mics
        .iter()
        .map(|m| m.drum.clone().unwrap_or_else(|| m.id.clone()))
        .collect();
    let mics: Vec<PairMic> = (0..job.mics.len())
        .map(|i| PairMic {
            drum: &drums[i],
            signal: &signals[i],
            times: &job.mics[i].times,
            output: &outputs[i],
            target: &targets[i],
            guarded: job.mics[i].guard,
        })
        .collect();
    let levels: Vec<Vec<f64>> = mics
        .iter()
        .map(|m| m.times.iter().map(|&t| pair_level_db(m.signal, sr, t)).collect())
        .collect();
    let rises: Vec<Option<Vec<f64>>> = mics
        .iter()
        .map(|m| {
            if !m.guarded {
                return None;
            }
            let hp = zero_phase(m.signal, sr, &[(FilterKind::HighPass, PAIR_HP_HZ)]);
            Some(m.times.iter().map(|&t| stroke_rise_db(m.signal, &hp, sr, t)).collect())
        })
        .collect();
    let marks = pair_marks(&mics, &levels, &rises, &params);
    let doc = serde_json::json!({"sr": sr, "levels": levels, "rises": rises, "marks": marks});
    std::fs::write(out, serde_json::to_string(&doc).expect("json"))
        .map_err(|e| format!("write {}: {e}", out.display()))?;
    println!(
        "{} mics, {} hits marked",
        job.mics.len(),
        marks.iter().flatten().filter(|&&b| b).count()
    );
    Ok(())
}

#[derive(Deserialize)]
struct FlamJobSource {
    id: String,
    path: PathBuf,
}

#[derive(Deserialize)]
struct FlamJob {
    path: PathBuf,
    hits: Vec<f64>,
    #[serde(default)]
    sources: Vec<FlamJobSource>,
    #[serde(default)]
    rise_db: Option<f64>,
    #[serde(default)]
    margin_db: Option<f64>,
    #[serde(default)]
    min_gap_s: Option<f64>,
}

fn run_flam(job_path: &Path, out: &Path) -> Result<(), String> {
    use drumspike_core::detect::flam::{added, attack_env, flam_split, late_peak_db, late_share, FlamParams};
    let s = std::fs::read_to_string(job_path).map_err(|e| format!("read {}: {e}", job_path.display()))?;
    let job: FlamJob = serde_json::from_str(&s).map_err(|e| format!("manifest json: {e}"))?;
    let mut paths: Vec<&PathBuf> = vec![&job.path];
    paths.extend(job.sources.iter().map(|m| &m.path));
    let mut decoded = Vec::with_capacity(paths.len());
    for (d, path) in decode_many(&paths, MixMode::Average).into_iter().zip(&paths) {
        decoded.push(d.map_err(|e| format!("{}: {e}", path.display()))?);
    }
    let sr = decoded[0].1.sample_rate;
    // a source with another rate is left out (the app and `tools/flam_split.py` do the same)
    let used: Vec<usize> = (1..decoded.len()).filter(|&i| decoded[i].1.sample_rate == sr).collect();
    let srcs: Vec<&[f32]> = used.iter().map(|&i| decoded[i].0.as_slice()).collect();
    let mut p = FlamParams::default();
    p.rise_db = job.rise_db.unwrap_or(p.rise_db);
    p.margin_db = job.margin_db.unwrap_or(p.margin_db);
    p.min_gap_s = job.min_gap_s.unwrap_or(p.min_gap_s);
    let t = std::time::Instant::now();
    let (found, gains) = flam_split(&decoded[0].0, sr, &job.hits, &srcs, &p);
    let seconds = t.elapsed().as_secs_f64();
    let (env, hop) = attack_env(&decoded[0].0, sr, &p);
    let share = late_share(&env, &job.hits, sr, hop, &p);
    let late = late_peak_db(&env, &job.hits, sr, hop, &p); // the app's late-peak gate (`tools/flam_pairs.py` `late_peak_db`)
    let rows: Vec<serde_json::Value> = found
        .iter()
        .map(|f| {
            serde_json::json!({"t": f.t, "first": f.first, "gap_ms": f.gap_ms, "rise_db": f.rise_db,
                               "level_db": f.level_db, "bleed": f.bleed})
        })
        .collect();
    let gains: Vec<serde_json::Value> = used
        .iter()
        .zip(&gains)
        .map(|(&i, g)| serde_json::json!({"id": job.sources[i - 1].id, "gain_db": g}))
        .collect();
    let add = added(&found);
    let doc = serde_json::json!({"sr": sr, "seconds": seconds, "found": rows, "gains": gains, "added": add,
                               "late_peak_db": late, "late_share": share});
    std::fs::write(out, serde_json::to_string(&doc).expect("json"))
        .map_err(|e| format!("write {}: {e}", out.display()))?;
    println!(
        "{} hits, {} second strokes, {} added",
        job.hits.len(),
        found.len(),
        add.len()
    );
    Ok(())
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: parity <wav> [--mix ffmpeg|average] [--t0-file list.json] [--pcm-out f] [--tile level,from,to]... [--samples from,to]...\n       parity refine <job.json>\n       parity midi <job.json> <out.mid>\n       parity spike <job.json> <out.wav>\n       parity project <folder-or-document> <out_dir>\n       parity legacy <session_dir>\n       parity resample <in.wav> <out.wav> [--mix ffmpeg|average] [--mono-out mono.wav]\n       parity features <wav44|sig.npy> <chain> <out.npy> [--mix ffmpeg|average] [--repeat N]\n       parity onsets <wav> <role> <out.json> [--mix ffmpeg|average] [--set key=value]...\n       parity attribute <session-manifest.json> <out.json>\n       parity tom-bleed <session-manifest.json> <out.json>\n       parity tom-pairs <manifest.json> <out.json>\n       parity flam <manifest.json> <out.json>"
    );
    ExitCode::from(2)
}

fn parse_ints(s: &str) -> Vec<i64> {
    s.split(',')
        .map(|v| v.trim().parse::<i64>().expect("integer list"))
        .collect()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return usage();
    }
    if matches!(
        args[0].as_str(),
        "resample" | "infer" | "onsets" | "features" | "pick" | "roll" | "detect"
    ) {
        let result = match args[0].as_str() {
            "resample" => run_resample(&args[1..]),
            "features" => run_features(&args[1..]),
            _ => run_onsets(&args[1..]),
        };
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        };
    }
    if matches!(args[0].as_str(), "attribute" | "tom-bleed" | "tom-pairs" | "flam") {
        let (Some(job), Some(out)) = (args.get(1), args.get(2)) else {
            return usage();
        };
        let run = match args[0].as_str() {
            "attribute" => run_attribute,
            "tom-bleed" => run_tom_bleed,
            "flam" => run_flam,
            _ => run_tom_pairs,
        };
        return match run(Path::new(job), Path::new(out)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        };
    }
    if matches!(args[0].as_str(), "refine" | "midi" | "spike" | "project" | "legacy") {
        let Some(job) = args.get(1) else {
            return usage();
        };
        let job = Path::new(job);
        let result = match args[0].as_str() {
            "refine" => run_refine(job),
            "legacy" => run_legacy(job),
            "project" => match args.get(2) {
                Some(out) => run_project(job, Path::new(out)),
                None => return usage(),
            },
            cmd => {
                let Some(out) = args.get(2) else {
                    return usage();
                };
                if cmd == "midi" {
                    run_midi(job, Path::new(out))
                } else {
                    run_spike(job, Path::new(out))
                }
            }
        };
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        };
    }
    let path = PathBuf::from(&args[0]);
    let mut t0: Vec<f64> = Vec::new();
    let mut mix = MixMode::Ffmpeg;
    let mut pcm_out: Option<PathBuf> = None;
    let mut tiles: Vec<(usize, i64, i64)> = Vec::new();
    let mut windows: Vec<(i64, i64)> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let val = |k: usize| args.get(k).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--t0-file" => {
                let s = std::fs::read_to_string(val(i + 1)).expect("read t0 file");
                t0.extend(serde_json::from_str::<Vec<f64>>(&s).expect("t0 json array"));
                i += 2;
            }
            "--mix" => {
                mix = match val(i + 1).as_str() {
                    "ffmpeg" => MixMode::Ffmpeg,
                    "average" => MixMode::Average,
                    m => {
                        eprintln!("unknown mix mode {m}");
                        return ExitCode::from(2);
                    }
                };
                i += 2;
            }
            "--pcm-out" => {
                pcm_out = Some(PathBuf::from(val(i + 1)));
                i += 2;
            }
            "--tile" => {
                let v = parse_ints(&val(i + 1));
                if v.len() != 3 {
                    return usage();
                }
                tiles.push((v[0] as usize, v[1], v[2]));
                i += 2;
            }
            "--samples" => {
                let v = parse_ints(&val(i + 1));
                if v.len() != 2 {
                    return usage();
                }
                windows.push((v[0], v[1]));
                i += 2;
            }
            _ => return usage(),
        }
    }
    let (x, info) = match read_wav_mono(&path, mix) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    if let Some(p) = pcm_out {
        let mut buf = Vec::with_capacity(x.len() * 4);
        for v in &x {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        if let Err(e) = std::fs::write(&p, buf) {
            eprintln!("error: write {}: {e}", p.display());
            return ExitCode::from(1);
        }
    }
    let sr = info.sample_rate;
    let velocities = velocity_map(&x, sr, &t0, &VelocityMap::default());
    let track = Track::new(sr, x);
    let peaks = track
        .levels()
        .iter()
        .map(|(bs, l)| (bs.to_string(), l.clone()))
        .collect();
    let tiles = tiles
        .iter()
        .map(|&(level, from, to)| {
            track.peaks(level, from, to).map(|t| Tile {
                level,
                from,
                to,
                start: t.start,
                min: t.min.to_vec(),
                max: t.max.to_vec(),
                rms: t.rms.to_vec(),
            })
        })
        .collect();
    let samples = windows
        .iter()
        .map(|&(from, to)| {
            let w = track.samples(from, to);
            Window {
                from,
                to,
                start: w.start,
                data: w.data.to_vec(),
            }
        })
        .collect();
    let out = Out {
        n_samples: track.len(),
        info,
        peaks,
        velocities,
        tiles,
        samples,
    };
    println!("{}", serde_json::to_string(&out).expect("json"));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use drumspike_core::detect::peaks::Onset;

    /// Read a 2-D float32 `.npy`: `(rows, cols, data)`.
    fn read_npy_f32_2d(path: &Path) -> Result<(usize, usize, Vec<f32>), String> {
        let (dims, data) = read_npy_f32(path)?;
        let [rows, cols] = dims[..] else {
            return Err(format!("{}: shape {dims:?} is not 2-D", path.display()));
        };
        Ok((rows, cols, data))
    }

    fn onsets_json(onsets: &[Onset]) -> serde_json::Value {
        let mut counts = [0usize; 5];
        for o in onsets {
            counts[o.class] += 1;
        }
        serde_json::json!({
            "counts": counts,
            "onsets": onsets.iter().map(|o| serde_json::json!([o.frame, o.class, o.time, o.activation])).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn npy_round_trip() {
        let dir = std::env::temp_dir().join(format!("drumspike-npy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.npy");
        let data: Vec<f32> = (0..15).map(|i| i as f32 * 0.25 - 1.0).collect();
        write_npy_f32_2d(&p, 3, 5, &data).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        let header_end = 10 + u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
        assert_eq!(header_end % 64, 0);
        assert_eq!(read_npy_f32_2d(&p).unwrap(), (3, 5, data));
        assert!(read_npy_f32_1d(&p).is_err());
        assert!(
            read_signal_44k(&p, MixMode::Ffmpeg).is_err(),
            "a 2-D .npy is not a signal"
        );
        std::fs::write(&p, b"not npy").unwrap();
        assert!(read_npy_f32_2d(&p).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn npy_1d_signal_and_chain_option() {
        let dir = std::env::temp_dir().join(format!("drumspike-npy1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("sig.npy");
        let data: Vec<f32> = (0..7).map(|i| i as f32 * 0.5).collect();
        let mut header = "{'descr': '<f4', 'fortran_order': False, 'shape': (7,), }".to_string();
        while !(10 + header.len() + 1).is_multiple_of(64) {
            header.push(' ');
        }
        header.push('\n');
        let mut buf = b"\x93NUMPY\x01\x00".to_vec();
        buf.extend_from_slice(&(header.len() as u16).to_le_bytes());
        buf.extend_from_slice(header.as_bytes());
        data.iter().for_each(|v| buf.extend_from_slice(&v.to_le_bytes()));
        std::fs::write(&p, buf).unwrap();
        assert_eq!(read_signal_44k(&p, MixMode::Ffmpeg).unwrap(), (data, TARGET_RATE));
        assert!(read_npy_f32_2d(&p).is_err());
        assert_eq!(parse_chain("madmom").unwrap(), Chain::Madmom);
        assert!(parse_chain("x").is_err());
        let o = [Onset {
            frame: 3,
            class: 2,
            time: 0.03,
            activation: 0.5,
        }];
        let j = onsets_json(&o);
        assert_eq!(j["counts"], serde_json::json!([0, 0, 1, 0, 0]));
        assert_eq!(j["onsets"][0], serde_json::json!([3, 2, 0.03, 0.5]));
        let a: Vec<String> = ["--mix", "average", "--repeat", "3"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(matches!(parse_mix_repeat(&a).unwrap(), (MixMode::Average, 3)));
        std::fs::remove_dir_all(&dir).ok();
    }
}
