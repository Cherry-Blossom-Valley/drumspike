//! Binary peak cache of the app: `Analysis/<track>/peaks.bin` (P3-2b).
//!
//! The server keeps `peaks.json`; the app reads tiles from this file instead, without
//! loading it whole. Layout (all numbers little endian):
//!
//! | Offset | Type | Content |
//! |---|---|---|
//! | 0 | 8 bytes | magic `DSPEAKS\0` |
//! | 8 | u32 | version, 1 |
//! | 12 | u32 | sample rate |
//! | 16 | u64 | sample count of the track |
//! | 24 | u32 | level count `L` |
//! | 28 | `L` x (u32, u32) | block size and bin count of every level |
//! | after the header | f32 | per level: `min[bins]`, `max[bins]`, `rms[bins]`, levels in header order |
//!
//! The values are the f64 values of [`PeakLevel`] stored as f32. They were rounded to
//! 5 decimals in float32 first, so the f32 to f64 round trip is exact and a level read back
//! equals [`compute_peaks`] bit for bit. A level shorter than one block holds one zero bin.
//!
//! `key.json` next to the file holds the [`CacheKey`] the server writes (`size`, `mtime`,
//! `head_sha1`). The key is written last, so a half-built folder never looks current.
//!
//! The reader refuses any header its writer could not have written (R20): a level count
//! over 64, a block of 0, a bin count that is not `max(1, n_samples / block)`, and any
//! length the file does not hold. A malformed file is a clear error — never a panic and
//! never an allocation the file cannot hold.

use std::fs::{self, File};
use std::io;
use std::path::Path;

use crate::peaks::{PeakLevel, clamp_window, compute_peaks};
use crate::project::CacheKey;

/// File name of the binary cache inside `Analysis/<track>/`.
pub const PEAKS_BIN: &str = "peaks.bin";
/// The magic bytes at the start of the file.
pub const MAGIC: [u8; 8] = *b"DSPEAKS\0";
/// The format version this module reads and writes.
pub const VERSION: u32 = 1;
/// Bytes of the fixed header part (before the level table).
const FIXED_HEADER: usize = 28;

/// A cache problem.
#[derive(Debug)]
pub enum CacheError {
    /// A file system error.
    Io(io::Error),
    /// The file is not a valid `peaks.bin` (the text says why).
    Format(String),
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CacheError::Io(e) => write!(f, "peak cache: {e}"),
            CacheError::Format(m) => write!(f, "peak cache: {m}"),
        }
    }
}

impl std::error::Error for CacheError {}

impl From<io::Error> for CacheError {
    fn from(e: io::Error) -> Self {
        CacheError::Io(e)
    }
}

/// Header entry of one level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelInfo {
    /// Bucket size in samples.
    pub block: usize,
    /// Bin count.
    pub bins: usize,
}

/// A tile read from the file: bins `[start, start + min.len())` of one level.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheTile {
    /// Bucket size of the level.
    pub level: usize,
    /// Index of the first bin.
    pub start: usize,
    /// Minimum per bin.
    pub min: Vec<f32>,
    /// Maximum per bin.
    pub max: Vec<f32>,
    /// RMS per bin.
    pub rms: Vec<f32>,
}

/// Encode `peaks.bin` for a track of `n_samples` samples at `sr`.
pub fn encode_peaks_bin(sr: u32, n_samples: u64, levels: &[(usize, PeakLevel)]) -> Vec<u8> {
    let total: usize = levels.iter().map(|(_, l)| l.n * 3 * 4).sum();
    let mut out = Vec::with_capacity(FIXED_HEADER + levels.len() * 8 + total);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&n_samples.to_le_bytes());
    out.extend_from_slice(&(levels.len() as u32).to_le_bytes());
    for (bs, l) in levels {
        out.extend_from_slice(&(*bs as u32).to_le_bytes());
        out.extend_from_slice(&(l.n as u32).to_le_bytes());
    }
    for (_, l) in levels {
        for v in [&l.min, &l.max, &l.rms] {
            for &x in v {
                out.extend_from_slice(&(x as f32).to_le_bytes());
            }
        }
    }
    out
}

/// Write `peaks.bin` atomically (temporary file, then rename).
pub fn write_peaks_bin(path: &Path, sr: u32, n_samples: u64, levels: &[(usize, PeakLevel)]) -> Result<(), CacheError> {
    let bytes = encode_peaks_bin(sr, n_samples, levels);
    let tmp = path.with_extension("bin.tmp");
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Reads tiles from a `peaks.bin` with positioned reads; the file is never loaded whole.
#[derive(Debug)]
pub struct PeakCache {
    file: File,
    sr: u32,
    n_samples: u64,
    levels: Vec<LevelInfo>,
    /// Byte offset of the first value of every level.
    offsets: Vec<u64>,
}

fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
    }
    #[cfg(windows)]
    {
        let mut done = 0;
        while done < buf.len() {
            let n = std::os::windows::fs::FileExt::seek_read(file, &mut buf[done..], offset + done as u64)?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            done += n;
        }
        Ok(())
    }
}

/// The `N` bytes at `i` as a little-endian unsigned integer, `Err` when the slice is
/// shorter (the module's doc: a malformed file is a clear error, never a panic).
fn uint_at<const N: usize>(b: &[u8], i: usize) -> io::Result<u64> {
    let s = b
        .get(i..i + N)
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "header cut off"))?;
    // `get(i..i+N)` is `Some` only when the whole range is in bounds, so `s` holds
    // exactly N bytes and the loop below cannot overflow or miss an element.
    let mut v = 0u64;
    for (k, byte) in s.iter().enumerate() {
        v |= u64::from(*byte) << (8 * k);
    }
    Ok(v)
}

fn u32_at(b: &[u8], i: usize) -> io::Result<u32> {
    Ok(uint_at::<4>(b, i)? as u32)
}

impl PeakCache {
    /// Open `peaks.bin`: read and check the header and the file length.
    pub fn open(path: &Path) -> Result<PeakCache, CacheError> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        if len < FIXED_HEADER as u64 {
            return Err(CacheError::Format(format!("{len} bytes, shorter than the header")));
        }
        let mut head = [0u8; FIXED_HEADER];
        read_at(&file, &mut head, 0)?;
        if head[..8] != MAGIC {
            return Err(CacheError::Format("bad magic".into()));
        }
        let version = u32_at(&head, 8)?;
        if version != VERSION {
            return Err(CacheError::Format(format!("version {version}, expected {VERSION}")));
        }
        let sr = u32_at(&head, 12)?;
        let n_samples = uint_at::<8>(&head, 16)?;
        let count = u32_at(&head, 24)? as usize;
        if count > 64 {
            return Err(CacheError::Format(format!("{count} levels")));
        }
        // Checked before the allocation, so the table is never longer than the file.
        if len < FIXED_HEADER as u64 + count as u64 * 8 {
            return Err(CacheError::Format("level table cut off".into()));
        }
        let mut table = vec![0u8; count * 8];
        read_at(&file, &mut table, FIXED_HEADER as u64)?;
        let mut levels = Vec::with_capacity(count);
        let mut offsets = Vec::with_capacity(count);
        let mut pos = (FIXED_HEADER + table.len()) as u64;
        for i in 0..count {
            let block = u32_at(&table, i * 8)? as usize;
            let bins = u32_at(&table, i * 8 + 4)? as usize;
            if block == 0 {
                return Err(CacheError::Format(format!("level {i}: block 0")));
            }
            // The writer stores max(1, n_samples / block) bins per level: full blocks
            // only, the trailing partial block dropped, one zero bin for a level
            // shorter than the block (compute_peaks, the aggregated levels included).
            // Anything else is a length the file cannot hold: a header that says
            // "4 billion frames" is rejected here, before any allocation.
            if bins as u64 != (n_samples / block as u64).max(1) {
                return Err(CacheError::Format(format!(
                    "level {i}: {bins} bins cannot hold {n_samples} samples of block {block}"
                )));
            }
            let info = LevelInfo { block, bins };
            offsets.push(pos);
            pos += info.bins as u64 * 12;
            levels.push(info);
        }
        if pos != len {
            return Err(CacheError::Format(format!("{len} bytes, the header says {pos}")));
        }
        Ok(PeakCache {
            file,
            sr,
            n_samples,
            levels,
            offsets,
        })
    }

    /// Sample rate of the track.
    pub fn sr(&self) -> u32 {
        self.sr
    }

    /// Sample count of the track.
    pub fn n_samples(&self) -> u64 {
        self.n_samples
    }

    /// Block size and bin count of every level, in file order.
    pub fn levels(&self) -> &[LevelInfo] {
        &self.levels
    }

    fn index(&self, level: usize) -> Option<usize> {
        self.levels.iter().position(|l| l.block == level)
    }

    /// Bins `[from, to)` of the level with block size `level`, clamped like
    /// [`crate::Track::peaks`]; `None` when the file has no such level.
    pub fn tile(&self, level: usize, from: i64, to: i64) -> Result<Option<CacheTile>, CacheError> {
        let Some(i) = self.index(level) else { return Ok(None) };
        let info = self.levels[i];
        let (s, e) = clamp_window(info.bins, from, to);
        let n = e - s;
        let mut cols: [Vec<f32>; 3] = Default::default();
        let mut buf = vec![0u8; n * 4];
        for (c, col) in cols.iter_mut().enumerate() {
            read_at(&self.file, &mut buf, self.offsets[i] + ((c * info.bins + s) * 4) as u64)?;
            *col = buf.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        }
        let [min, max, rms] = cols;
        Ok(Some(CacheTile {
            level,
            start: s,
            min,
            max,
            rms,
        }))
    }

    /// One whole level as a [`PeakLevel`] (for tests and tools; the app reads tiles).
    pub fn read_level(&self, level: usize) -> Result<Option<PeakLevel>, CacheError> {
        let Some(info) = self.index(level).map(|i| self.levels[i]) else {
            return Ok(None);
        };
        let Some(t) = self.tile(level, 0, info.bins as i64)? else {
            // unreachable: `tile` answers `Some` for every level `index` knows
            return Err(CacheError::Format(format!("level {level} vanished")));
        };
        let up = |v: Vec<f32>| v.into_iter().map(f64::from).collect::<Vec<f64>>();
        Ok(Some(PeakLevel {
            min: up(t.min),
            max: up(t.max),
            rms: up(t.rms),
            n: info.bins,
        }))
    }
}

/// True when `dir` holds a `peaks.bin` and a `key.json` equal to `key`.
pub fn cache_is_current(dir: &Path, key: &CacheKey) -> bool {
    dir.join(PEAKS_BIN).is_file() && key.matches_file(dir)
}

/// Build `dir/peaks.bin` and `dir/key.json` from decoded mono samples. The folder is
/// created when missing; other files in it (the server's `peaks.json`) stay. `key.json`
/// comes last.
pub fn build_cache(dir: &Path, sr: u32, samples: &[f32], key: &CacheKey) -> Result<(), CacheError> {
    fs::create_dir_all(dir)?;
    let levels = compute_peaks(samples);
    write_peaks_bin(&dir.join(PEAKS_BIN), sr, samples.len() as u64, &levels)?;
    let text = serde_json::to_string(key).map_err(|e| CacheError::Format(e.to_string()))?;
    let tmp = dir.join("key.json.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, dir.join("key.json"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Track;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("drumspike-core-cache-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn signal(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| ((i as f32) * 0.37).sin() * (((i / 700) % 5) as f32 * 0.2))
            .collect()
    }

    fn key() -> CacheKey {
        CacheKey {
            size: Some(10),
            mtime: Some(20),
            head_sha1: Some("ab".into()),
        }
    }

    #[test]
    fn round_trip_is_bitwise_equal() {
        for n in [0usize, 100, 5000, 70_000, 300_001] {
            let d = tmp(&format!("rt{n}"));
            let x = signal(n);
            build_cache(&d, 48_000, &x, &key()).unwrap();
            let c = PeakCache::open(&d.join(PEAKS_BIN)).unwrap();
            assert_eq!((c.sr(), c.n_samples()), (48_000, n as u64));
            let want = compute_peaks(&x);
            assert_eq!(c.levels().len(), 3);
            for (bs, lv) in &want {
                let got = c.read_level(*bs).unwrap().unwrap();
                assert_eq!(got.n, lv.n);
                for (a, b) in [(&got.min, &lv.min), (&got.max, &lv.max), (&got.rms, &lv.rms)] {
                    let a: Vec<u64> = a.iter().map(|v| v.to_bits()).collect();
                    let b: Vec<u64> = b.iter().map(|v| v.to_bits()).collect();
                    assert_eq!(a, b, "n={n} level {bs}");
                }
            }
            fs::remove_dir_all(&d).unwrap();
        }
    }

    #[test]
    fn tiles_equal_track_peaks() {
        let d = tmp("tiles");
        let x = signal(300_001);
        build_cache(&d, 96_000, &x, &key()).unwrap();
        let c = PeakCache::open(&d.join(PEAKS_BIN)).unwrap();
        let t = Track::new(96_000, x);
        for &lv in &[256usize, 4096, 65536] {
            for &(f, to) in &[
                (0i64, 10i64),
                (-5, 3),
                (7, 500),
                (100, 90),
                (1000, 2000),
                (4, 1 << 30),
                (1 << 30, 1 << 31),
            ] {
                let want = t.peaks(lv, f, to).unwrap();
                let got = c.tile(lv, f, to).unwrap().unwrap();
                assert_eq!(got.start, want.start);
                let up = |v: &[f32]| v.iter().map(|&x| f64::from(x)).collect::<Vec<f64>>();
                assert_eq!(up(&got.min), want.min);
                assert_eq!(up(&got.max), want.max);
                assert_eq!(up(&got.rms), want.rms);
            }
        }
        assert!(c.tile(999, 0, 10).unwrap().is_none());
        assert!(c.read_level(999).unwrap().is_none());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn header_and_key() {
        let d = tmp("key");
        let x = signal(9000);
        assert!(!cache_is_current(&d, &key()));
        build_cache(&d, 44_100, &x, &key()).unwrap();
        assert!(cache_is_current(&d, &key()));
        let other = CacheKey {
            size: Some(11),
            ..key()
        };
        assert!(!cache_is_current(&d, &other));
        let c = PeakCache::open(&d.join(PEAKS_BIN)).unwrap();
        let infos: Vec<(usize, usize)> = c.levels().iter().map(|l| (l.block, l.bins)).collect();
        assert_eq!(infos, vec![(256, 35), (4096, 2), (65536, 1)]);
        let bytes = fs::read(d.join(PEAKS_BIN)).unwrap();
        assert_eq!(&bytes[..8], b"DSPEAKS\0");
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 1);
        assert_eq!(bytes, encode_peaks_bin(44_100, 9000, &compute_peaks(&x)));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn bad_files_are_errors() {
        let d = tmp("bad");
        let p = d.join(PEAKS_BIN);
        assert!(matches!(PeakCache::open(&p), Err(CacheError::Io(_))));
        fs::write(&p, b"short").unwrap();
        assert!(matches!(PeakCache::open(&p), Err(CacheError::Format(_))));
        let good = encode_peaks_bin(48_000, 9000, &compute_peaks(&signal(9000)));
        let mut bad = good.clone();
        bad[0] = b'X';
        fs::write(&p, &bad).unwrap();
        assert!(PeakCache::open(&p).unwrap_err().to_string().contains("magic"));
        let mut bad = good.clone();
        bad[8] = 2;
        fs::write(&p, &bad).unwrap();
        assert!(PeakCache::open(&p).unwrap_err().to_string().contains("version"));
        fs::write(&p, &good[..good.len() - 4]).unwrap();
        assert!(PeakCache::open(&p).unwrap_err().to_string().contains("header says"));
        let mut bad = good.clone();
        bad[24] = 200;
        fs::write(&p, &bad).unwrap();
        assert!(PeakCache::open(&p).unwrap_err().to_string().contains("levels"));
        fs::write(&p, &good[..30]).unwrap();
        assert!(PeakCache::open(&p).unwrap_err().to_string().contains("cut off"));
        fs::remove_dir_all(&d).unwrap();
    }

    /// R20: every malformed input is a miss or a clear error — never a panic and never
    /// an allocation the file cannot hold. The table covers the header reader (`open`):
    /// empty, truncated, wrong magic, wrong version, length fields at 0, at `u32::MAX` and
    /// at `u64::MAX`, and a length larger than the file.
    #[test]
    fn malformed_inputs_are_a_miss_or_an_error() {
        let d = tmp("malformed");
        let p = d.join(PEAKS_BIN);
        let good = encode_peaks_bin(48_000, 9000, &compute_peaks(&signal(9000)));
        // Offsets in the good file: n_samples 16..24, level count 24..28, the last
        // level's block 44..48 and bins 48..52 (levels 256/35, 4096/2, 65536/1).
        let cases: Vec<(&str, Vec<u8>, &str)> = vec![
            ("empty", Vec::new(), "shorter than the header"),
            ("truncated after the header", good[..28].to_vec(), "cut off"),
            (
                "wrong magic",
                {
                    let mut b = good.clone();
                    b[0] = b'X';
                    b
                },
                "magic",
            ),
            (
                "wrong version",
                {
                    let mut b = good.clone();
                    b[8] = 2;
                    b
                },
                "version",
            ),
            (
                "level count 0",
                {
                    let mut b = good.clone();
                    b[24..28].copy_from_slice(&0u32.to_le_bytes());
                    b
                },
                "header says",
            ),
            (
                "level count u32::MAX",
                {
                    let mut b = good.clone();
                    b[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
                    b
                },
                "levels",
            ),
            (
                "block 0",
                {
                    let mut b = good.clone();
                    b[44..48].copy_from_slice(&0u32.to_le_bytes());
                    b
                },
                "block 0",
            ),
            (
                "bins 0",
                {
                    let mut b = good.clone();
                    b[48..52].copy_from_slice(&0u32.to_le_bytes());
                    b
                },
                "bins",
            ),
            (
                "bins u32::MAX",
                {
                    let mut b = good.clone();
                    b[48..52].copy_from_slice(&u32::MAX.to_le_bytes());
                    b
                },
                "bins",
            ),
            (
                "n_samples 0",
                {
                    let mut b = good.clone();
                    b[16..24].copy_from_slice(&0u64.to_le_bytes());
                    b
                },
                "bins",
            ),
            (
                "n_samples u64::MAX",
                {
                    let mut b = good.clone();
                    b[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
                    b
                },
                "bins",
            ),
            (
                "length larger than the file",
                good[..good.len() - 12].to_vec(),
                "header says",
            ),
        ];
        // Every case must be refused with a clear format error; the list names the ones
        // that were not, so one run shows every accepted malformed input.
        let mut bad: Vec<String> = Vec::new();
        for (name, bytes, want) in &cases {
            fs::write(&p, bytes).unwrap();
            match PeakCache::open(&p) {
                Ok(_) => bad.push(format!("{name}: opened")),
                Err(e) => {
                    let text = e.to_string();
                    if !matches!(e, CacheError::Format(_)) {
                        bad.push(format!("{name}: not a format error: {text}"));
                    } else if !text.contains(want) {
                        bad.push(format!("{name}: expected \"{want}\" in \"{text}\""));
                    }
                }
            }
        }
        assert!(
            bad.is_empty(),
            "malformed inputs that were not an error:\n{}",
            bad.join("\n")
        );
        // The control: a valid cache file still reads (no format change).
        fs::write(&p, &good).unwrap();
        let c = PeakCache::open(&p).unwrap();
        let infos: Vec<(usize, usize)> = c.levels().iter().map(|l| (l.block, l.bins)).collect();
        assert_eq!(infos, vec![(256, 35), (4096, 2), (65536, 1)]);
        assert!(c.read_level(256).unwrap().is_some());
        fs::remove_dir_all(&d).unwrap();
    }

    /// R20: the tile reader bounds every read and allocation by the level's bins, for
    /// hostile windows too; a length larger than the file is an error, never a panic.
    #[test]
    fn tile_reader_stays_bounded_by_the_level() {
        let d = tmp("tilebounds");
        let p = d.join(PEAKS_BIN);
        fs::write(&p, encode_peaks_bin(48_000, 9000, &compute_peaks(&signal(9000)))).unwrap();
        let c = PeakCache::open(&p).unwrap();
        let bins_of = |block: usize| c.levels().iter().find(|l| l.block == block).unwrap().bins;
        for &(from, to) in &[
            (0i64, 1i64 << 62),
            (-(1i64 << 62), -5),
            (i64::MIN, i64::MAX),
            (1 << 40, 1 << 41),
        ] {
            for &bs in &[256usize, 4096, 65536] {
                let t = c.tile(bs, from, to).unwrap().unwrap();
                let bins = bins_of(bs);
                assert!(t.start <= bins, "start {} past {bins} bins", t.start);
                assert!(t.min.len() <= bins, "{} mins for {bins} bins", t.min.len());
                assert_eq!(t.min.len(), t.max.len());
                assert_eq!(t.min.len(), t.rms.len());
            }
        }
        // A window past the level is an empty tile (a miss), not a huge read.
        let t = c.tile(256, 1 << 40, 1 << 41).unwrap().unwrap();
        assert_eq!((t.start, t.min.len()), (35, 0));
        // The file shrank under the open cache: the read fails with an error.
        let bytes = fs::read(&p).unwrap();
        fs::write(&p, &bytes[..10]).unwrap();
        assert!(matches!(c.tile(256, 0, 10), Err(CacheError::Io(_))));
        assert!(matches!(c.read_level(256), Err(CacheError::Io(_))));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn write_leaves_no_temporary_file() {
        let d = tmp("tmpfile");
        build_cache(&d, 48_000, &signal(1000), &key()).unwrap();
        let mut names: Vec<String> = fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into())
            .collect();
        names.sort();
        assert_eq!(names, vec!["key.json", "peaks.bin"]);
        fs::remove_dir_all(&d).unwrap();
    }
}
