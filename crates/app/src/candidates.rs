//! A4: the model-free detector's candidate list of a track, kept after an analysis in
//! `Analysis/<track>/candidates.bin`, so the Sensitivity slider and `track_repick` pick
//! again without the model and without a new detection (the model path keeps
//! the activation dump (model branch only) in the same folder, the same lifetime, the same reset on re-analysis).
//!
//! Layout of `candidates.bin` (little endian): the magic `DSCND\0\0\0`, `u32` version 1,
//! `u64` count, then per candidate: `u64` frame, `f64` time, `f32` flux, `f32` `rel_db`,
//! `f32` similarity, `f32` attack, `u8` kept, `u8` gate. Version 2 (A6) appends two `f32`
//! per record, the cross-mic lead (ms) and the band level ratio (dB) against the loudest
//! coinciding hit of a kick or snare track, NaN when there is none (the cross-track rule ran
//! only on tracks with one). `candidates.json` next to it holds
//! the key (as `activation.json` does, with the detector's model null) plus the detection's
//! [`OnsetParams`] and its weak-flux limit, which the re-pick needs to re-judge the gates.
//! The key is written last, so a half-written pair never looks current.

use std::fs;
use std::io;
use std::path::Path;

use drumspike_core::detect::onset::{Candidate, Gate, OnsetParams};
use drumspike_core::project::CacheKey;
use serde_json::{Value, json};

use crate::jobs::CHAIN_KIND;

/// The candidates file inside `Analysis/<track>/`.
pub const CANDIDATES_BIN: &str = "candidates.bin";
/// The key file next to it.
pub const CANDIDATES_KEY: &str = "candidates.json";
/// The magic bytes at the start of [`CANDIDATES_BIN`].
pub const MAGIC: [u8; 8] = *b"DSCND\0\0\0";
/// The format version (2: A6 appends the cross-mic lead and ratio per record, 3: A1 the
/// soft-busy flag, 4: A1 the second-template similarity).
pub const VERSION: u32 = 4;
/// Header: magic + version + count.
const HEADER: usize = 20;
/// The bytes of one record (version 1: without the trailing two `f32`; version 3: the
/// trailing soft-busy byte; version 4: the trailing second-template similarity).
const RECORD: usize = 8 + 8 + 4 + 4 + 4 + 4 + 1 + 1 + 4 + 4 + 1 + 4;
/// The bytes of a version 1 record.
const RECORD_V1: usize = 8 + 8 + 4 + 4 + 4 + 4 + 1 + 1;

/// The key of the candidates of a source, so a relinked source, another downmix, another
/// build or host makes the file stale. The model data of an old analysis is dropped: main
/// is model-free (#281 D16), so the DSP candidates are current whatever an old record says.
pub fn candidates_key(source: &CacheKey, mix: &str, _model: &Value) -> Value {
    json!({
        "source": source,
        "mix": mix,
        "chain": CHAIN_KIND.name(),
    })
}

/// Write the candidates, their detection params, the weak-flux limit and the key into `dir`
/// (made when missing). The key goes last.
///
/// # Errors
///
/// The [`io::Error`] of creating `dir` or of writing `candidates.bin` or the key file atomically (for example
/// `dir` is not writable or is a file).
pub fn write_candidates(
    dir: &Path,
    cands: &[Candidate],
    params: &OnsetParams,
    weak_limit: f32,
    key: &Value,
) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let _ = fs::remove_file(dir.join(CANDIDATES_KEY));
    let mut buf = Vec::with_capacity(HEADER + cands.len() * RECORD);
    buf.extend_from_slice(&MAGIC);
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&(cands.len() as u64).to_le_bytes());
    for c in cands {
        buf.extend_from_slice(&(c.frame as u64).to_le_bytes());
        buf.extend_from_slice(&c.time.to_le_bytes());
        buf.extend_from_slice(&c.flux.to_le_bytes());
        buf.extend_from_slice(&c.rel_db.to_le_bytes());
        buf.extend_from_slice(&c.similarity.to_le_bytes());
        buf.extend_from_slice(&c.attack.to_le_bytes());
        buf.push(u8::from(c.kept));
        buf.push(gate_byte(c.gate));
        buf.extend_from_slice(&c.cross_lead_ms.unwrap_or(f32::NAN).to_le_bytes());
        buf.extend_from_slice(&c.cross_ratio_db.unwrap_or(f32::NAN).to_le_bytes());
        buf.push(u8::from(c.soft_busy));
        buf.extend_from_slice(&c.similarity_soft.to_le_bytes());
    }
    // the shared atomic helper (R10): unique temporary files, guard-owned cleanup
    drumspike_core::io::write_atomic(&dir.join(CANDIDATES_BIN), &buf)?;
    let meta = json!({"key": key, "params": params, "weak_limit": weak_limit});
    drumspike_core::io::write_atomic(&dir.join(CANDIDATES_KEY), meta.to_string().as_bytes())
}

fn gate_byte(g: Gate) -> u8 {
    match g {
        Gate::Level => 0,
        Gate::WeakFlux => 1,
        Gate::WeakAttack => 2,
        Gate::Shape => 3,
        Gate::Gap => 4,
        Gate::CrossBleed => 5,
        Gate::Soft => 6,
    }
}

fn byte_gate(b: u8) -> Option<Gate> {
    match b {
        0 => Some(Gate::Level),
        1 => Some(Gate::WeakFlux),
        2 => Some(Gate::WeakAttack),
        3 => Some(Gate::Shape),
        4 => Some(Gate::Gap),
        5 => Some(Gate::CrossBleed),
        6 => Some(Gate::Soft),
        _ => None,
    }
}

/// The stored record of `dir` without a key check (the rule re-writes it with the same key):
/// the detection's parameters and the weak-flux limit, and the key itself.
pub fn read_candidates_meta(dir: &Path) -> Option<(OnsetParams, f32, Value)> {
    let meta: Value = serde_json::from_str(&fs::read_to_string(dir.join(CANDIDATES_KEY)).ok()?).ok()?;
    let params: OnsetParams = serde_json::from_value(meta.get("params")?.clone()).ok()?;
    let weak_limit = meta.get("weak_limit")?.as_f64()? as f32;
    Some((params, weak_limit, meta.get("key")?.clone()))
}

/// The stored record of `dir` when its key equals `key` and the file is whole; else `None`.
/// Returns the candidates, the detection's parameters and its weak-flux limit.
///
/// # Panics
///
/// Panics (slice indexing, `expect` on fixed-size `try_into`) only if a record lies outside the file; the
/// check that the file length equals header plus `n` records of the version's size rules that out for any
/// file content.
pub fn read_candidates(dir: &Path, key: &Value) -> Option<(Vec<Candidate>, OnsetParams, f32)> {
    let meta: Value = serde_json::from_str(&fs::read_to_string(dir.join(CANDIDATES_KEY)).ok()?).ok()?;
    if meta.get("key")? != key {
        return None;
    }
    let params: OnsetParams = serde_json::from_value(meta.get("params")?.clone()).ok()?;
    let weak_limit = meta.get("weak_limit")?.as_f64()? as f32;
    let b = fs::read(dir.join(CANDIDATES_BIN)).ok()?;
    if b.len() < HEADER || b[..8] != MAGIC {
        return None;
    }
    let n = u64::from_le_bytes(b[12..20].try_into().expect("8 bytes"));
    let version = u32_le(&b, 8);
    let record = match version {
        1 => RECORD_V1,
        2..=3 => RECORD - 4,
        4 => RECORD,
        _ => return None,
    };
    let expected = n
        .checked_mul(record as u64)
        .and_then(|payload| payload.checked_add(HEADER as u64))?;
    if b.len() as u64 != expected {
        return None;
    }
    let n = n as usize;
    let mut cands = Vec::with_capacity(n);
    for i in 0..n {
        let o = HEADER + i * record;
        let f32_at = |k: usize| f32::from_le_bytes(b[o + k..o + k + 4].try_into().expect("4 bytes"));
        let gate = byte_gate(b[o + 33])?;
        let (lead, ratio) = if version >= 2 {
            let l = f32_at(34);
            let r = f32_at(38);
            (
                if l.is_nan() { None } else { Some(l) },
                if r.is_nan() { None } else { Some(r) },
            )
        } else {
            (None, None)
        };
        let soft_busy = version >= 3 && o + 42 < b.len() && b[o + 42] == 1;
        let similarity_soft = if version >= 4 { f32_at(43) } else { f32::NAN };
        cands.push(Candidate {
            frame: u64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes")) as usize,
            time: f64::from_le_bytes(b[o + 8..o + 16].try_into().expect("8 bytes")),
            flux: f32_at(16),
            rel_db: f32_at(20),
            similarity: f32_at(24),
            attack: f32_at(28),
            kept: b[o + 32] == 1,
            gate,
            cross_lead_ms: lead,
            cross_ratio_db: ratio,
            soft_busy,
            similarity_soft,
        });
    }
    Some((cands, params, weak_limit))
}

fn u32_le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().expect("4 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src() -> CacheKey {
        CacheKey {
            size: Some(10),
            mtime: Some(20),
            head_sha1: Some("ab".into()),
        }
    }

    fn cands() -> Vec<Candidate> {
        vec![
            Candidate {
                frame: 20,
                time: 0.2,
                flux: 1.5,
                rel_db: -0.5,
                similarity: 0.99,
                attack: 30.0,
                kept: true,
                gate: Gate::Level,
                cross_lead_ms: None,
                cross_ratio_db: None,
                soft_busy: false,
                similarity_soft: 0.75,
            },
            Candidate {
                frame: 90,
                time: 0.9,
                flux: 0.2,
                rel_db: -20.0,
                similarity: 0.4,
                attack: 2.0,
                kept: false,
                gate: Gate::Level,
                cross_lead_ms: None,
                cross_ratio_db: None,
                soft_busy: false,
                similarity_soft: f32::NAN,
            },
            Candidate {
                frame: 91,
                time: 0.91,
                flux: 0.3,
                rel_db: -1.0,
                similarity: 0.5,
                attack: 25.0,
                kept: false,
                gate: Gate::Gap,
                cross_lead_ms: None,
                cross_ratio_db: None,
                soft_busy: false,
                similarity_soft: f32::NAN,
            },
        ]
    }

    #[test]
    fn write_and_read_round_trip_under_the_same_key() {
        let d = std::env::temp_dir().join(format!("drumspike-cand-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let dir = d.join("Analysis").join("k in");
        let key = candidates_key(&src(), "ffmpeg", &Value::Null);
        assert_eq!(key["chain"], "mix");
        let cs = cands();
        let p = OnsetParams::for_role(drumspike_core::detect::onset::Role::Kick);
        write_candidates(&dir, &cs, &p, 0.25, &key).unwrap();
        let (got, gp, wl) = read_candidates(&dir, &key).unwrap();
        // a field-wise compare: the NaN second-template similarities never compare equal
        assert!(got.iter().zip(&cs).all(|(a, b)| a.frame == b.frame
            && a.time == b.time
            && a.flux == b.flux
            && a.rel_db == b.rel_db
            && a.similarity == b.similarity
            && a.attack == b.attack
            && a.kept == b.kept
            && a.gate == b.gate
            && a.cross_lead_ms == b.cross_lead_ms
            && a.cross_ratio_db == b.cross_ratio_db
            && a.soft_busy == b.soft_busy
            && (a.similarity_soft == b.similarity_soft || (a.similarity_soft.is_nan() && b.similarity_soft.is_nan()))));
        assert_eq!(gp, p);
        assert!((wl - 0.25).abs() < 1e-6);
        // another source state, downmix or model: stale
        let mut other = src();
        other.mtime = Some(21);
        assert!(read_candidates(&dir, &candidates_key(&other, "ffmpeg", &Value::Null)).is_none());
        assert!(read_candidates(&dir, &candidates_key(&src(), "average", &Value::Null)).is_none());
        // a cut file is not read
        let b = fs::read(dir.join(CANDIDATES_BIN)).unwrap();
        fs::write(dir.join(CANDIDATES_BIN), &b[..b.len() - 4]).unwrap();
        assert!(read_candidates(&dir, &key).is_none());
        assert!(read_candidates(&d.join("none"), &key).is_none());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn params_written_before_the_role_key_read_as_other() {
        // a candidates file written before the role was stored has no `role` key in its params
        let p = OnsetParams::for_role(drumspike_core::detect::onset::Role::Kick);
        let mut v = serde_json::to_value(&p).unwrap();
        v.as_object_mut().unwrap().remove("role");
        let old: OnsetParams = serde_json::from_value(v).unwrap();
        assert_eq!(old.role, drumspike_core::detect::onset::Role::Other);
        assert_eq!(OnsetParams { role: p.role, ..old }, p);
    }
}
