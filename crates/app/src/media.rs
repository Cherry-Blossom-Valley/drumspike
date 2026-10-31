//! Audio access of the commands: decode a source to mono f32 (with a small cache for
//! `velocities_recompute`), the sample windows of `samples_window` (P3-15d: a positioned read
//! of the window only, no whole decode) and the peak cache `Analysis/<track>/peaks.bin`
//! (P3-2b), built on first use.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use drumspike_core::cache::{PEAKS_BIN, PeakCache, build_cache, cache_is_current};
use drumspike_core::io::{Plan, WavFormat, encode_samples};
use drumspike_core::layer::OneShot;
use drumspike_core::project::{ANALYSIS_DIR, CacheKey, cache_key};
use drumspike_core::{MappedWav, MixMode, WavError, WavInfo, read_wav_mono};

use crate::error::AppError;
use crate::registry::{OpenProject, lock};

/// How many decoded tracks the cache keeps at most (a 5-minute 96 kHz track is 115 MB).
pub const DECODE_CACHE_LEN: usize = 3;

/// The byte budget of the decode cache: the held samples of all cached decodes together
/// stay below it (LRU evicts until they do). 512 MiB: two 10-minute 96 kHz tracks (about
/// 230 MB of f32 each) fit next to one shorter track, so the cache still spares the hot
/// files while the memory of the process is bounded. Overridable with the env var
/// `DRUMSPIKE_DECODE_CACHE_BUDGET` (bytes) for tests and tuning. A decode larger than the
/// budget is returned but not kept.
pub const DECODE_CACHE_BUDGET_BYTES: usize = 512 * 1024 * 1024;

/// The env var that overrides [`DECODE_CACHE_BUDGET_BYTES`].
pub const DECODE_CACHE_BUDGET_ENV: &str = "DRUMSPIKE_DECODE_CACHE_BUDGET";

/// How many opened sources the window reader keeps: header, downmix plan and the open file,
/// no samples (P3-15d).
pub const WINDOW_SOURCES_LEN: usize = 64;

/// The most threads one peak build uses (a 5-minute 96 kHz track is 115 MB of samples in memory).
pub const PEAK_WORKERS: usize = 6;

/// Run `work(i)` for every `i` in `0..n` on up to [`PEAK_WORKERS`] threads (fewer when the
/// host has fewer cores or `n` is smaller); returns when every call has returned. Each index
/// is taken once, in increasing order of the start. The peak builds of `tracks_add` and of
/// the lane job (`peaks_build_start`) use it.
pub fn on_peak_workers(n: usize, work: impl Fn(usize) + Sync) {
    let workers = std::thread::available_parallelism()
        .map_or(2, std::num::NonZero::get)
        .clamp(1, PEAK_WORKERS)
        .min(n);
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= n {
                        break;
                    }
                    work(i);
                }
            });
        }
    });
}

/// A decoded track: the mono signal at the file's own rate.
#[derive(Debug)]
pub struct Decoded {
    /// Native sample rate.
    pub sr: u32,
    /// Mono samples.
    pub samples: Vec<f32>,
}

/// `ffmpeg` (the default of an older document) or `average`.
pub fn mix_mode(name: &str) -> MixMode {
    if name == "average" {
        MixMode::Average
    } else {
        MixMode::Ffmpeg
    }
}

/// The error of a source read, with the file name in front (the text of [`decode`]).
fn named(path: &Path, e: WavError) -> AppError {
    let e = AppError::from(e);
    AppError {
        code: e.code,
        message: format!("{}: {}", path.display(), e.message),
    }
}

/// Read and downmix a WAV file (the source is only read).
///
/// # Errors
///
/// Returns an internal error (with the path in front) when the file cannot be opened or read, and a 400 error
/// when it is not a supported WAV format.
pub fn decode(path: &Path, mode: MixMode) -> Result<Decoded, AppError> {
    let (samples, info) = read_wav_mono(path, mode).map_err(|e| named(path, e))?;
    Ok(Decoded {
        sr: info.sample_rate,
        samples,
    })
}

/// What the peak cache of one track needs, taken from the project while its lock is held,
/// so the (slow) build runs without the lock.
#[derive(Debug, Clone)]
pub struct PeakRequest {
    /// `Analysis/<track>/`.
    pub dir: PathBuf,
    /// The key the cache must carry (the source record's identity).
    pub key: CacheKey,
    /// The source file, or the 409 of an offline track.
    pub source: Result<PathBuf, AppError>,
    /// The track's downmix.
    pub mix: MixMode,
}

impl PeakRequest {
    /// The request for `name`; 400 for an unknown track.
    ///
    /// # Errors
    ///
    /// Returns a 400 error when the project has no track of this name. An offline track is not an error here: its
    /// 409 is kept in the `source` field.
    pub fn of(op: &OpenProject, name: &str) -> Result<PeakRequest, AppError> {
        let tr = op.track(name)?;
        Ok(PeakRequest {
            dir: op.project.subdir(ANALYSIS_DIR).join(name),
            key: cache_key(tr),
            source: op.source(name),
            mix: mix_mode(tr.mix_mode_or_default()),
        })
    }
}

type DecodeKey = (PathBuf, bool, u64, Option<SystemTime>);

/// The identity of a source for the window reader: the decode key and the inode, so a file
/// that was replaced is opened again.
type WindowKey = (DecodeKey, u64);

/// An opened source of the window reader: the parsed header, the downmix plan and the file
/// (read with positioned reads, so parallel windows share it).
struct WindowSource {
    info: WavInfo,
    plan: Plan,
    file: File,
}

/// One entry of the window reader: filled once, under its own lock, so parallel first
/// requests for one file open and parse it once.
type WindowSlot = Arc<Mutex<Option<Arc<WindowSource>>>>;

/// Read `buf.len()` bytes at `offset`.
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
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
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            done += n;
        }
        Ok(())
    }
}

fn inode(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        std::os::unix::fs::MetadataExt::ino(meta)
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0
    }
}

/// The bytes of one cached decode: the mono samples (`Decoded` holds nothing else that
/// scales with the file).
fn decode_bytes(d: &Decoded) -> usize {
    d.samples.len() * std::mem::size_of::<f32>()
}

/// Removes the inflight key when the request that owns the decode ends, success or
/// error, so a failed decode leaves no slot behind.
struct InflightGuard<'a> {
    map: &'a Mutex<HashMap<DecodeKey, InflightSlot>>,
    key: DecodeKey,
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        lock(self.map).remove(&self.key);
    }
}

/// One wait slot for one decode key that is being decoded right now (single-flight): the
/// first request fills it, the others wait on its lock and share the result.
type InflightSlot = Arc<Mutex<Option<Arc<Decoded>>>>;

/// The caches shared by the commands.
#[derive(Default)]
pub struct Media {
    decoded: Mutex<Vec<(DecodeKey, Arc<Decoded>)>>,
    locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
    windows: Mutex<Vec<(WindowKey, WindowSlot)>>,
    /// How many times the window reader opened a source (for the tests).
    window_opens: AtomicUsize,
    /// One slot per decode key that is being decoded right now (single-flight).
    inflight: Mutex<HashMap<DecodeKey, InflightSlot>>,
    /// How many decodes ran to the end (for the tests).
    decode_runs: AtomicUsize,
    /// The byte budget of the decode cache; `None` reads the env var, else the const.
    budget: Option<usize>,
}

impl Media {
    /// Empty caches.
    pub fn new() -> Media {
        Media {
            budget: std::env::var(DECODE_CACHE_BUDGET_ENV).ok().and_then(|v| v.parse().ok()),
            ..Media::default()
        }
    }

    /// Empty caches with an explicit byte budget (the tests use a small one).
    pub fn with_budget(budget: usize) -> Media {
        Media {
            budget: Some(budget),
            ..Media::default()
        }
    }

    fn budget(&self) -> usize {
        self.budget.unwrap_or(DECODE_CACHE_BUDGET_BYTES)
    }

    /// The decode of `path` from the cache, else read it. Concurrent requests for one
    /// source identity (path, mode, size, time) wait for one decode and share its `Arc`
    /// (single-flight). A file that changed (size or time) is read again. The cache keeps
    /// the last [`DECODE_CACHE_LEN`] decodes within the byte budget
    /// ([`DECODE_CACHE_BUDGET_BYTES`], LRU); a decode larger than the budget is returned
    /// but not kept.
    ///
    /// # Errors
    ///
    /// Returns an internal error when the file's metadata or contents cannot be read, and a 400 error when it
    /// is not a supported WAV format.
    pub fn decode_cached(&self, path: &Path, mode: MixMode) -> Result<Arc<Decoded>, AppError> {
        let meta = std::fs::metadata(path).map_err(|e| AppError::internal(format!("{}: {e}", path.display())))?;
        let key: DecodeKey = (
            path.to_path_buf(),
            mode == MixMode::Average,
            meta.len(),
            meta.modified().ok(),
        );
        if let Some(d) = self.decoded_hit(&key) {
            return Ok(d);
        }
        // single-flight: the first request for the key decodes, the others wait on the
        // slot and share the result; a failed decode leaves no slot behind (the waiters
        // decode themselves).
        let held = lock(&self.inflight).get(&key).map(Arc::clone);
        let slot = if let Some(slot) = held {
            slot
        } else {
            // The owner of a finished decode removes its key only after the cache
            // insert, so no slot here means the decode is in the cache already (a
            // failed or an oversized decode is not). This second lookup closes the
            // window between the two lookups of this request: without it, a request
            // that lost that race decoded again, and the eight-thread test saw two
            // decodes once under load (#58).
            if let Some(d) = self.decoded_hit(&key) {
                return Ok(d);
            }
            // a parallel first request may have made the slot in between
            let mut map = lock(&self.inflight);
            Arc::clone(map.entry(key.clone()).or_default())
        };
        let mut g = lock(&slot);
        if let Some(d) = g.as_ref() {
            return Ok(Arc::clone(d));
        }
        // every exit of the decode removes the key again (also the `?` of a failed
        // decode), so the map keeps no empty slot
        let guard = InflightGuard {
            map: &self.inflight,
            key: key.clone(),
        };
        let d = Arc::new(decode(path, mode)?);
        self.decode_runs.fetch_add(1, Ordering::Relaxed);
        *g = Some(Arc::clone(&d));
        drop(g);
        // the cache entry goes in before the inflight key is removed: a request that
        // arrives in between hits the cache instead of finding no slot and decoding
        // again (for an oversized decode the slot stays valid until the remove)
        self.decoded_insert(key, Arc::clone(&d));
        drop(guard);
        Ok(d)
    }

    /// The decode for `key` from the cache, moved to the back (most recently used).
    fn decoded_hit(&self, key: &DecodeKey) -> Option<Arc<Decoded>> {
        let mut cache = lock(&self.decoded);
        let i = cache.iter().position(|(k, _)| k == key)?;
        let (_, d) = cache.remove(i);
        let d = Arc::clone(&d);
        cache.push((key.clone(), d.clone()));
        Some(d)
    }

    /// Keep `d` under `key`, evicting least-recently-used decodes until the held bytes
    /// are within the budget. A decode larger than the budget is not kept.
    fn decoded_insert(&self, key: DecodeKey, d: Arc<Decoded>) {
        let budget = self.budget();
        if decode_bytes(&d) > budget {
            return;
        }
        let mut cache = lock(&self.decoded);
        cache.retain(|(k, _)| *k != key);
        cache.push((key, d));
        let mut held: usize = cache.iter().map(|(_, d)| decode_bytes(d)).sum();
        while (held > budget || cache.len() > DECODE_CACHE_LEN) && cache.len() > 1 {
            let (_, old) = cache.remove(0);
            held -= decode_bytes(&old);
        }
    }

    /// How many decodes ran to the end since the start (for the tests).
    pub fn decode_runs(&self) -> usize {
        self.decode_runs.load(Ordering::Relaxed)
    }

    /// The opened source of `path` in `mode` for the window reader: from the cache when the
    /// file is the same (path, mode, size, time, inode), else opened, parsed and kept (the
    /// last [`WINDOW_SOURCES_LEN`]). Errors are not kept.
    fn window_source(&self, path: &Path, mode: MixMode) -> Result<Arc<WindowSource>, AppError> {
        let meta = std::fs::metadata(path).map_err(|e| AppError::internal(format!("{}: {e}", path.display())))?;
        let key: WindowKey = (
            (
                path.to_path_buf(),
                mode == MixMode::Average,
                meta.len(),
                meta.modified().ok(),
            ),
            inode(&meta),
        );
        let slot = {
            let mut list = lock(&self.windows);
            let slot = match list.iter().position(|(k, _)| *k == key) {
                Some(i) => list.remove(i).1,
                None => Arc::default(),
            };
            // an older state of the same file in the same mode is not needed again
            list.retain(|(k, _)| !(k.0.0 == key.0.0 && k.0.1 == key.0.1));
            list.push((key, Arc::clone(&slot)));
            if list.len() > WINDOW_SOURCES_LEN {
                list.remove(0);
            }
            slot
        };
        let mut g = lock(&slot);
        if let Some(src) = g.as_ref() {
            return Ok(Arc::clone(src));
        }
        let map = MappedWav::open(path).map_err(|e| named(path, e))?;
        let info = map.info.clone();
        drop(map);
        let plan = info.mix_plan(mode).map_err(|e| named(path, e))?;
        let file = File::open(path).map_err(|e| named(path, WavError::Io(e)))?;
        self.window_opens.fetch_add(1, Ordering::Relaxed);
        let src = Arc::new(WindowSource { info, plan, file });
        *g = Some(Arc::clone(&src));
        Ok(src)
    }

    /// The bytes of `samples_window` for `path` in `mode`: `u32 start`, `u32 sample rate`,
    /// then the mono samples `[start, end)` as f32, little endian. `start` below 0 counts
    /// as 0, the window holds at most `max` samples and ends at the end of the track. Only
    /// the frames of the window are read (one positioned read) and folded with the decoder of
    /// the full decode, so the values equal the same samples of [`decode`] bit for bit.
    ///
    /// # Errors
    ///
    /// Returns an internal error when the file's metadata or the window's frames cannot be read, and a 400 error
    /// when the file is not a supported WAV format.
    pub fn window_bytes(
        &self,
        path: &Path,
        mode: MixMode,
        start: i64,
        end: i64,
        max: i64,
    ) -> Result<Vec<u8>, AppError> {
        let src = self.window_source(path, mode)?;
        let info = &src.info;
        let len = info.n_frames as i64;
        let start = start.max(0);
        let count = (end - start).clamp(0, max.max(0));
        let (s, e) = (start.min(len) as usize, (start + count).min(len) as usize);
        let n = e - s;
        let mut raw = vec![0u8; n * info.block_align];
        read_at(&src.file, &mut raw, (info.data_offset + s * info.block_align) as u64)
            .map_err(|e| named(path, WavError::Io(e)))?;
        let mut x = vec![0.0f32; n];
        let got = info.decode_frames_into(&src.plan, &raw, &mut x);
        debug_assert_eq!(got, n);
        let mut out = Vec::with_capacity(8 + 4 * n);
        out.extend_from_slice(&(s as u32).to_le_bytes());
        out.extend_from_slice(&info.sample_rate.to_le_bytes());
        out.extend_from_slice(&encode_samples(WavFormat::Float32, &x));
        Ok(out)
    }

    /// How many times the window reader opened a source since the start.
    pub fn window_opens(&self) -> usize {
        self.window_opens.load(Ordering::Relaxed)
    }

    /// One lock per analysis folder: two requests for the same track build `peaks.bin` once.
    fn folder_lock(&self, dir: &Path) -> Arc<Mutex<()>> {
        Arc::clone(lock(&self.locks).entry(dir.to_path_buf()).or_default())
    }

    /// The open `peaks.bin` of a track. Missing or stale (`key.json` differs from the
    /// source record): decode the source and build it. A current cache needs no source, so
    /// an offline track with a cache still shows its lanes. The decode is not kept (P3-15d:
    /// the sample windows read the file themselves; six parallel builds would otherwise
    /// leave three tracks of up to 115 MB in memory).
    ///
    /// # Errors
    ///
    /// Returns the 409 error of an offline track when the cache is stale and the source is missing, the errors of
    /// [`decode`] for the source, and an internal error when `peaks.bin` cannot be built or opened.
    pub fn peak_cache(&self, req: &PeakRequest) -> Result<PeakCache, AppError> {
        let guard = self.folder_lock(&req.dir);
        let _g = lock(&guard);
        if !cache_is_current(&req.dir, &req.key) {
            let src = req.source.clone()?;
            let d = decode(&src, req.mix)?;
            build_cache(&req.dir, d.sr, &d.samples, &req.key)?;
        }
        Ok(PeakCache::open(&req.dir.join(PEAKS_BIN))?)
    }
}

/// The bytes of a one-shot window (P4-40e, the Layer view's waveform): `u32 start`, `u32 rate`,
/// then the mono samples `[start, end)` of `shot` as f32 — the shape of `samples_window`. The
/// window is clamped to the file and capped at [`crate::commands::MAX_WINDOW_SAMPLES`].
pub fn wave_bytes(shot: &OneShot, start: i64, end: i64) -> Vec<u8> {
    let mono = shot.mono();
    let n = mono.len() as i64;
    let start = start.clamp(0, n);
    let count = (end - start)
        .clamp(0, crate::commands::MAX_WINDOW_SAMPLES)
        .min(n - start);
    let x = &mono[start as usize..(start + count) as usize];
    let mut out = Vec::with_capacity(8 + 4 * x.len());
    out.extend_from_slice(&(start as u32).to_le_bytes());
    out.extend_from_slice(&shot.sr.to_le_bytes());
    out.extend_from_slice(&encode_samples(WavFormat::Float32, x));
    out
}

#[cfg(test)]
mod tests;
