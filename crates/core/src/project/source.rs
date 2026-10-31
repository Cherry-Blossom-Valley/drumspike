//! Source records: identity (`size`, `mtime`, `head_sha1`, `bext_origination`), the
//! resolution order, byte-identical copies into `Audio Files/` and the `Analysis/` cache key.
//! The port of the `source records` section of `project.py`. Source files are only read.

use std::fs::{File, FileTimes};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use super::document::{SOURCE_MODES, Source, Track};
use super::{AUDIO_DIR, Project, ProjectError, abspath, basename, io_err, normpath, relpath};

/// `head_sha1` covers the first MiB of a file.
pub const HEAD_BYTES: usize = 1024 * 1024;

/// Result of [`resolve_source`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceStatus {
    /// A candidate with the record's size and head hash exists.
    Ok,
    /// A candidate exists but size or head hash differ.
    Changed,
    /// No candidate exists.
    Missing,
}

impl SourceStatus {
    /// `ok`, `changed` or `missing`, as the server reports it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            SourceStatus::Ok => "ok",
            SourceStatus::Changed => "changed",
            SourceStatus::Missing => "missing",
        }
    }
}

fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let mut buf = Vec::with_capacity(HEAD_BYTES.min(f.metadata().map_or(HEAD_BYTES, |m| m.len() as usize)));
    f.by_ref().take(HEAD_BYTES as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

/// `project.head_sha1`: lower-case hex SHA-1 of the first MiB. P3-15c: the hash of a file
/// that did not change is kept in a cache of the process (see [`HeadCacheCounts`]), so an
/// open, `project_get` and `tracks_get` read each unchanged source head once.
pub fn head_sha1(path: &Path) -> Result<String, ProjectError> {
    head_sha1_settled(path, HEAD_CACHE_SETTLE).map(|(h, _)| h)
}

/// Read and hash the first MiB (no cache).
fn hash_head_now(path: &Path) -> Result<String, ProjectError> {
    let head = read_head(path).map_err(io_err(format!("read {}", path.display())))?;
    let digest = Sha1::digest(&head);
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// How long a file must be unchanged (its `mtime` and `ctime` before now) before its head hash
/// is kept. A file system with coarse times (HFS+: 1 s, FAT: 2 s) could otherwise change a
/// file twice within one time step and keep its key; after this time a change gives a new
/// `ctime`. A newer file is hashed every time.
pub const HEAD_CACHE_SETTLE: std::time::Duration = std::time::Duration::from_secs(3);
/// The most entries the head-hash cache keeps; it is emptied when it is full.
pub const HEAD_CACHE_MAX: usize = 4096;

/// What identifies an unchanged file: device, inode, size, `mtime` and `ctime` (ns).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct HeadKey {
    dev: u64,
    ino: u64,
    size: u64,
    mtime_ns: i128,
    ctime_ns: i128,
}

/// The cache key of `path`, or `None` when the file cannot be read or changed less than
/// `settle` ago (the key is not trusted then).
#[cfg(unix)]
fn head_key(path: &Path, settle: std::time::Duration) -> Option<HeadKey> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    let ns = |s: i64, n: i64| i128::from(s) * 1_000_000_000 + i128::from(n);
    let key = HeadKey {
        dev: m.dev(),
        ino: m.ino(),
        size: m.size(),
        mtime_ns: ns(m.mtime(), m.mtime_nsec()),
        ctime_ns: ns(m.ctime(), m.ctime_nsec()),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    let limit = now.as_nanos() as i128 - settle.as_nanos() as i128;
    (key.mtime_ns <= limit && key.ctime_ns <= limit).then_some(key)
}

#[cfg(not(unix))]
fn head_key(_path: &Path, _settle: std::time::Duration) -> Option<HeadKey> {
    None
}

/// Counters of the head-hash cache since the start of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeadCacheCounts {
    /// Hashes answered from the cache.
    pub hits: u64,
    /// Heads read and hashed.
    pub hashed: u64,
}

struct HeadCache {
    map: std::collections::HashMap<HeadKey, String>,
    counts: HeadCacheCounts,
}

static HEAD_CACHE: std::sync::Mutex<Option<HeadCache>> = std::sync::Mutex::new(None);

fn with_head_cache<T>(f: impl FnOnce(&mut HeadCache) -> T) -> T {
    let mut g = HEAD_CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let c = g.get_or_insert_with(|| HeadCache {
        map: std::collections::HashMap::new(),
        counts: HeadCacheCounts::default(),
    });
    f(c)
}

/// The counters of the head-hash cache (tests and measurements).
#[must_use]
pub fn head_cache_counts() -> HeadCacheCounts {
    with_head_cache(|c| c.counts)
}

/// [`head_sha1`] with the settle time `settle` (tests use 0). The lock of the cache is not held
/// while the file is read; the hash is kept only when the key is the same before and after
/// the read. The flag is true when the hash came from the cache.
fn head_sha1_settled(path: &Path, settle: std::time::Duration) -> Result<(String, bool), ProjectError> {
    let key = head_key(path, settle);
    if let Some(k) = key
        && let Some(h) = with_head_cache(|c| {
            let h = c.map.get(&k).cloned();
            if h.is_some() {
                c.counts.hits += 1;
            }
            h
        })
    {
        return Ok((h, true));
    }
    let h = hash_head_now(path)?;
    let still = key.filter(|k| head_key(path, settle) == Some(*k));
    with_head_cache(|c| {
        c.counts.hashed += 1;
        if let Some(k) = still {
            if c.map.len() >= HEAD_CACHE_MAX {
                c.map.clear();
            }
            c.map.insert(k, h.clone());
        }
    });
    Ok((h, false))
}

/// `project.bext_origination`: `OriginationDate` + `OriginationTime` of a Broadcast Wave
/// `bext` chunk found in the first MiB, as `YYYY-MM-DDThh:mm:ssZ`; `None` for a file that
/// is not RIFF/WAVE, has no `bext`, a `bext` shorter than 338 bytes, an empty date or
/// cannot be read.
#[must_use]
pub fn bext_origination(path: &Path) -> Option<String> {
    let head = read_head(path).ok()?;
    if head.len() < 12 || &head[..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12usize;
    while pos + 8 <= head.len() {
        let cid = &head[pos..pos + 4];
        let size = u32::from_le_bytes([head[pos + 4], head[pos + 5], head[pos + 6], head[pos + 7]]) as usize;
        if cid == b"bext" {
            let body = &head[(pos + 8).min(head.len())..(pos + 8 + size).min(head.len())];
            if body.len() < 338 {
                return None;
            }
            let date = ascii_field(&body[320..330]);
            let tm = ascii_field(&body[330..338]);
            return if date.is_empty() {
                None
            } else {
                Some(format!("{date}T{tm}Z"))
            };
        }
        pos += 8 + size + (size & 1);
    }
    None
}

/// `bytes.split(b"\0")[0].decode("ascii", "replace")`.
fn ascii_field(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    b[..end]
        .iter()
        .map(|&c| if c < 128 { c as char } else { '\u{FFFD}' })
        .collect()
}

/// `int(st.st_mtime)`: whole seconds, truncated towards zero.
fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    match meta.modified() {
        Ok(t) => match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => -(e.duration().as_secs_f64() as i64),
        },
        Err(_) => 0,
    }
}

/// `project.build_source`: the record for an audio file (absolute path, project-relative
/// posix path, size, whole-second mtime, head hash, `bext_origination` when present).
pub fn build_source(path: &Path, project_dir: &Path, mode: &str) -> Result<Source, ProjectError> {
    if !SOURCE_MODES.contains(&mode) {
        return Err(ProjectError::Invalid(format!("bad source mode: '{mode}'")));
    }
    let path = abspath(path);
    let st = std::fs::metadata(&path).map_err(io_err(format!("stat {}", path.display())))?;
    Ok(Source {
        mode: Some(mode.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        relative: Some(relpath(&path, &abspath(project_dir))),
        size: Some(st.len() as i64),
        mtime: Some(mtime_secs(&st)),
        head_sha1: Some(head_sha1(&path)?),
        bext_origination: bext_origination(&path),
        extra: Default::default(),
    })
}

/// `project.source_matches`: size and head hash of `path` equal the record's (false when
/// the file cannot be read).
#[must_use]
pub fn source_matches(record: &Source, path: &Path) -> bool {
    let size = match std::fs::metadata(path) {
        Ok(m) => m.len() as i64,
        Err(_) => return false,
    };
    if record.size != Some(size) {
        return false;
    }
    match head_sha1(path) {
        Ok(h) => record.head_sha1.as_deref() == Some(h.as_str()),
        Err(_) => false,
    }
}

/// `project.source_candidates`: the resolution order, duplicates removed: the absolute
/// `path`, `<project>/<relative>` normalised, `<project>/Audio Files/<basename>`.
#[must_use]
pub fn source_candidates(record: &Source, project_dir: &Path) -> Vec<PathBuf> {
    let project_dir = abspath(project_dir);
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(p) = record.path.as_deref().filter(|p| !p.is_empty()) {
        out.push(PathBuf::from(p));
    }
    if let Some(r) = record.relative.as_deref().filter(|r| !r.is_empty()) {
        out.push(normpath(&project_dir.join(r)));
    }
    let base = basename(Path::new(record.path_or_relative())).to_string();
    if !base.is_empty() {
        out.push(project_dir.join(AUDIO_DIR).join(base));
    }
    let mut uniq: Vec<PathBuf> = Vec::new();
    for c in out {
        if !uniq.contains(&c) {
            uniq.push(c);
        }
    }
    uniq
}

/// `project.resolve_source`: the first candidate with matching size and head hash
/// (`Ok`), else the first existing candidate (`Changed`), else `(None, Missing)`.
#[must_use]
pub fn resolve_source(record: Option<&Source>, project_dir: &Path) -> (Option<PathBuf>, SourceStatus) {
    let Some(record) = record else {
        return (None, SourceStatus::Missing);
    };
    let mut existing = None;
    for c in source_candidates(record, project_dir) {
        if c.is_file() {
            if source_matches(record, &c) {
                return (Some(c), SourceStatus::Ok);
            }
            if existing.is_none() {
                existing = Some(c);
            }
        }
    }
    match existing {
        Some(c) => (Some(c), SourceStatus::Changed),
        None => (None, SourceStatus::Missing),
    }
}

/// `shutil.copy2`: the bytes plus the access and modification times.
fn copy2(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::copy(src, dst)?;
    let meta = std::fs::metadata(src)?;
    let mut times = FileTimes::new();
    if let Ok(m) = meta.modified() {
        times = times.set_modified(m);
    }
    if let Ok(a) = meta.accessed() {
        times = times.set_accessed(a);
    }
    let _ = File::options().write(true).open(dst).and_then(|f| f.set_times(times));
    Ok(())
}

/// `project.copy_into_project`: copy the record's file into `Audio Files/` (byte-identical)
/// and return a `copied` record.
///
/// The copy keeps the source's file name unless `name` says otherwise (the importer names a
/// session's `source.wav` after its track). A file that already sits in `Audio Files/` with
/// matching identity is reused, not copied twice: the source itself when it lives there, or
/// `Audio Files/<name>` (or a `(2)`, `(3)` name in that sequence) when that file has the
/// record's size and head hash. The source file is never modified.
pub fn copy_into_project(record: &Source, project_dir: &Path, name: Option<&str>) -> Result<Source, ProjectError> {
    copy_into_project_with(record, project_dir, name, copy2)
}

/// [`copy_into_project`] with the copy step as a parameter: the tests pass a copy that
/// truncates or corrupts the file to reach the two checks that protect `Audio Files/`
/// (the Python tests patch `shutil.copy2` for the same purpose).
pub(crate) fn copy_into_project_with(
    record: &Source,
    project_dir: &Path,
    name: Option<&str>,
    copy: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<Source, ProjectError> {
    let (src, status) = resolve_source(Some(record), project_dir);
    let src = match (src, status) {
        (Some(s), SourceStatus::Ok) => s,
        (_, status) => {
            return Err(ProjectError::CopyFailed {
                reason: status.as_str().to_string(),
                path: record.path_or_relative().to_string(),
            });
        }
    };
    match copy_file_into_project(&src, record, project_dir, name, copy, &|| false)? {
        Some(o) => Ok(o.source),
        None => unreachable!("a copy that cannot be cancelled"),
    }
}

/// The result of [`copy_file_into_project`].
#[derive(Debug, Clone)]
pub struct CopyOutcome {
    /// The `copied` record.
    pub source: Source,
    /// The file this call made in `Audio Files/`; `None` when it reused a file that was there.
    /// A caller that cancels its job later deletes the files it made, and only those.
    pub created: Option<PathBuf>,
}

/// Where a cancel and the byte count of a copy go (P3-4d). `cancelled` is asked before every
/// chunk; `on_bytes` gets the number of bytes just written.
pub struct CopyCtl<'a> {
    /// True when the user cancelled.
    pub cancelled: &'a (dyn Fn() -> bool + Sync),
    /// Called with the size of each chunk written (and with the file size for a reused file).
    pub on_bytes: &'a (dyn Fn(u64) + Sync),
}

/// Chunk size of [`copy_chunked`]: the cancel and the progress are checked once per chunk.
pub const COPY_CHUNK: usize = 4 * 1024 * 1024;

/// `shutil.copy2` in chunks: the bytes plus the access and modification times. Asks `ctl`
/// before each chunk; a cancel stops with `ErrorKind::Interrupted` (the caller removes `dst`).
pub fn copy_chunked(src: &Path, dst: &Path, ctl: &CopyCtl<'_>) -> std::io::Result<()> {
    use std::io::Write;
    let mut input = File::open(src)?;
    let mut out = File::create(dst)?;
    let mut buf = vec![0u8; COPY_CHUNK];
    loop {
        if (ctl.cancelled)() {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        (ctl.on_bytes)(n as u64);
    }
    out.flush()?;
    let meta = std::fs::metadata(src)?;
    let mut times = FileTimes::new();
    if let Ok(m) = meta.modified() {
        times = times.set_modified(m);
    }
    if let Ok(a) = meta.accessed() {
        times = times.set_accessed(a);
    }
    let _ = out.set_times(times);
    Ok(())
}

/// [`copy_into_project`] for a job: copy the file `src` (already resolved and verified by the
/// caller) into `project_dir/Audio Files/`, in chunks, with cancel and byte progress. `record`
/// is the identity the copy must have. Returns `Ok(None)` after a cancel: the temporary file
/// is removed and nothing else is left behind. A failed copy removes its temporary file too.
pub fn copy_into_project_ctl(
    src: &Path,
    record: &Source,
    project_dir: &Path,
    name: Option<&str>,
    ctl: &CopyCtl<'_>,
) -> Result<Option<CopyOutcome>, ProjectError> {
    let out = copy_file_into_project(
        src,
        record,
        project_dir,
        name,
        |s, d| copy_chunked(s, d, ctl),
        ctl.cancelled,
    )?;
    if let Some(o) = &out
        && o.created.is_none()
    {
        (ctl.on_bytes)(record.size.unwrap_or(0).max(0) as u64);
    }
    Ok(out)
}

fn copy_file_into_project(
    src: &Path,
    record: &Source,
    project_dir: &Path,
    name: Option<&str>,
    copy: impl Fn(&Path, &Path) -> std::io::Result<()>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<CopyOutcome>, ProjectError> {
    let audio = abspath(project_dir).join(AUDIO_DIR);
    std::fs::create_dir_all(&audio).map_err(io_err(format!("mkdir {}", audio.display())))?;
    if abspath(src).parent() == Some(audio.as_path()) {
        return Ok(Some(CopyOutcome {
            source: build_source(src, project_dir, "copied")?,
            created: None,
        }));
    }
    let base = name.map_or_else(|| basename(src).to_string(), str::to_string);
    let (stem, ext) = splitext(&base);
    let mut cand = base.clone();
    let mut k = 1;
    while audio.join(&cand).exists() {
        // <base>, <stem> (2).<ext>, ...: reuse a match
        if source_matches(record, &audio.join(&cand)) {
            return Ok(Some(CopyOutcome {
                source: build_source(&audio.join(&cand), project_dir, "copied")?,
                created: None,
            }));
        }
        k += 1;
        cand = format!("{stem} ({k}){ext}");
    }
    let dst = audio.join(&cand);
    let tmp = PathBuf::from(format!("{}.tmp", dst.display()));
    if let Err(e) = copy(src, &tmp) {
        let _ = std::fs::remove_file(&tmp);
        if cancelled() {
            return Ok(None);
        }
        return Err(ProjectError::Io(
            format!("copy {} to {}", src.display(), tmp.display()),
            e,
        ));
    }
    let src_len = std::fs::metadata(src).map_or(u64::MAX, |m| m.len());
    let tmp_len = std::fs::metadata(&tmp).map_or(0, |m| m.len());
    if tmp_len != src_len {
        let _ = std::fs::remove_file(&tmp);
        return Err(ProjectError::Invalid(format!(
            "copy of {} is incomplete",
            src.display()
        )));
    }
    std::fs::rename(&tmp, &dst).map_err(io_err(format!("rename {}", tmp.display())))?;
    let rec = build_source(&dst, project_dir, "copied")?;
    if rec.head_sha1 != record.head_sha1 || rec.size != record.size {
        return Err(ProjectError::Invalid(format!(
            "copy of {} does not match the source",
            src.display()
        )));
    }
    Ok(Some(CopyOutcome {
        source: rec,
        created: Some(dst),
    }))
}

/// `os.path.splitext`: `("Kick In", ".wav")`; a leading dot alone is not an extension.
pub(crate) fn splitext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && name[..i].chars().any(|c| c != '.') => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// `project.consolidate`: copy every referenced source into `Audio Files/` and rewrite its
/// record; returns the names of the tracks changed, in document order.
pub fn consolidate(project: &mut Project) -> Result<Vec<String>, ProjectError> {
    let dir = project.dir.clone();
    let mut changed = Vec::new();
    if let Some(tracks) = project.doc.tracks.as_mut() {
        for (name, tr) in tracks.iter_mut() {
            let referenced = tr.source_ref().is_some_and(|r| r.mode.as_deref() == Some("referenced"));
            if referenced {
                let rec = copy_into_project(tr.source_ref().expect("checked"), &dir, None)?;
                tr.source = Some(Some(rec));
                changed.push(name.clone());
            }
        }
    }
    Ok(changed)
}

// ---------- relink search (P3-4d) ----------

/// An offline track the folder search looks for: its name and the source record it had.
#[derive(Debug, Clone)]
pub struct SearchTarget {
    /// The track name.
    pub track: String,
    /// The record that holds the file name, size and head hash to look for.
    pub record: Source,
}

/// What the search found for one track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchHit {
    /// Exactly one file has the record's size and head hash. `by` is `name` (the file name
    /// is the one in the record) or `content` (another name, same bytes at the start).
    Found {
        /// The file.
        path: PathBuf,
        /// `name` or `content`.
        by: &'static str,
    },
    /// More than one file has the identity: the search lists them and chooses none.
    Ambiguous(Vec<PathBuf>),
    /// No file has the identity. `differs` lists files with the record's name whose size or
    /// head hash differ (the user may accept one with a forced relink).
    Missing {
        /// Files with the same name and another content.
        differs: Vec<PathBuf>,
    },
}

/// The answer for one track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// The track name.
    pub track: String,
    /// What was found.
    pub hit: SearchHit,
}

/// The answer of [`relink_search`].
#[derive(Debug, Clone, Default)]
pub struct SearchReport {
    /// One entry per target, in the order of the targets. Empty after a cancel.
    pub results: Vec<SearchResult>,
    /// Files the walk looked at (all regular files, hidden ones excluded).
    pub files_seen: usize,
    /// Folders the walk entered.
    pub dirs_seen: usize,
    /// True when the walk stopped at [`SEARCH_MAX_FILES`].
    pub truncated: bool,
    /// True when `cancelled` said stop.
    pub cancelled: bool,
}

/// The walk stops after this many files (a search of a whole disk must end).
pub const SEARCH_MAX_FILES: usize = 2_000_000;

struct Cand {
    path: PathBuf,
    size: i64,
    name_lc: String,
}

fn lower_name(p: &Path) -> String {
    basename(p).to_lowercase()
}

/// Hash the heads of `paths` on up to six threads; `None` for a file that cannot be read.
/// Stops early (the map holds fewer entries) when `cancelled` is true.
fn hash_heads(
    paths: &[PathBuf],
    cancelled: &(dyn Fn() -> bool + Sync),
) -> std::collections::HashMap<PathBuf, Option<String>> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let out = Mutex::new(std::collections::HashMap::new());
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map_or(2, std::num::NonZero::get)
        .clamp(1, 6)
        .min(paths.len().max(1));
    std::thread::scope(|sc| {
        for _ in 0..workers {
            sc.spawn(|| {
                while !cancelled() {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(p) = paths.get(i) else { break };
                    let h = head_sha1(p).ok();
                    out.lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(p.clone(), h);
                }
            });
        }
    });
    out.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The folder search of the relink flow. Walks `folder` (sub-folders too; no symbolic link is
/// followed, hidden files and folders are skipped) and, for every target, looks for the file:
///
/// 1. files with the record's file name (case-insensitive) whose size and head hash equal the
///    record's;
/// 2. when step 1 finds none, any file with the record's size and head hash (the file was
///    renamed as well as moved).
///
/// One file is `Found`; two or more are `Ambiguous` and none is chosen; none is `Missing`.
/// A record without a head hash matches by size alone. Files are only read (the first MiB of
/// a candidate whose size fits). `cancelled` is asked between folders and between hashes.
pub fn relink_search(
    folder: &Path,
    targets: &[SearchTarget],
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<SearchReport, ProjectError> {
    if !folder.is_dir() {
        return Err(ProjectError::Invalid(format!("not a folder: {}", folder.display())));
    }
    let want_names: std::collections::HashSet<String> = targets
        .iter()
        .flat_map(|t| {
            [t.record.path.as_deref(), t.record.relative.as_deref()]
                .into_iter()
                .flatten()
                .filter(|p| !p.is_empty())
                .map(|p| lower_name(Path::new(p)))
                .collect::<Vec<_>>()
        })
        .collect();
    let want_sizes: std::collections::HashSet<i64> = targets.iter().filter_map(|t| t.record.size).collect();
    let mut report = SearchReport::default();
    let mut cands: Vec<Cand> = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    'walk: while let Some(dir) = stack.pop() {
        if cancelled() {
            report.cancelled = true;
            return Ok(report);
        }
        report.dirs_seen += 1;
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.filter_map(Result::ok) {
            let fname = entry.file_name().to_string_lossy().into_owned();
            if fname.starts_with('.') {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            // P4-5 (M1): a `<name>.partial` folder is a Save As that is not finished (or a stale one); it may go away
            if ft.is_dir() && fname.ends_with(".partial") {
                continue;
            }
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                report.files_seen += 1;
                if report.files_seen >= SEARCH_MAX_FILES {
                    report.truncated = true;
                    break 'walk;
                }
                let name_lc = fname.to_lowercase();
                let Ok(md) = entry.metadata() else { continue };
                let size = md.len() as i64;
                if want_names.contains(&name_lc) || want_sizes.contains(&size) {
                    cands.push(Cand {
                        path: entry.path(),
                        size,
                        name_lc,
                    });
                }
            }
        }
    }
    // the heads to hash: step 1 candidates with a fitting size, then step 2 for the tracks without a hit
    let names_of = |t: &SearchTarget| -> Vec<String> {
        [t.record.path.as_deref(), t.record.relative.as_deref()]
            .into_iter()
            .flatten()
            .filter(|p| !p.is_empty())
            .map(|p| lower_name(Path::new(p)))
            .collect()
    };
    let by_name = |t: &SearchTarget| -> Vec<&Cand> {
        let names = names_of(t);
        cands.iter().filter(|c| names.contains(&c.name_lc)).collect()
    };
    let mut need: Vec<PathBuf> = Vec::new();
    for t in targets {
        if t.record.head_sha1.is_none() {
            continue;
        }
        for c in by_name(t) {
            if Some(c.size) == t.record.size && !need.contains(&c.path) {
                need.push(c.path.clone());
            }
        }
    }
    let mut hashes = hash_heads(&need, cancelled);
    if cancelled() {
        report.cancelled = true;
        return Ok(report);
    }
    let fits = |t: &SearchTarget, c: &Cand, hashes: &std::collections::HashMap<PathBuf, Option<String>>| -> bool {
        if Some(c.size) != t.record.size {
            return false;
        }
        match &t.record.head_sha1 {
            None => true,
            Some(h) => hashes.get(&c.path).and_then(Option::as_deref) == Some(h.as_str()),
        }
    };
    // step 2 needs the hashes of every same-size file for the tracks that step 1 did not settle
    let mut need2: Vec<PathBuf> = Vec::new();
    for t in targets {
        let step1 = by_name(t).iter().any(|c| fits(t, c, &hashes));
        if step1 || t.record.head_sha1.is_none() {
            continue;
        }
        for c in cands.iter().filter(|c| Some(c.size) == t.record.size) {
            if !hashes.contains_key(&c.path) && !need2.contains(&c.path) {
                need2.push(c.path.clone());
            }
        }
    }
    hashes.extend(hash_heads(&need2, cancelled));
    if cancelled() {
        report.cancelled = true;
        return Ok(report);
    }
    for t in targets {
        let named = by_name(t);
        let mut exact: Vec<PathBuf> = named
            .iter()
            .filter(|c| fits(t, c, &hashes))
            .map(|c| c.path.clone())
            .collect();
        let mut by = "name";
        if exact.is_empty() {
            by = "content";
            exact = cands
                .iter()
                .filter(|c| fits(t, c, &hashes))
                .map(|c| c.path.clone())
                .collect();
        }
        exact.sort();
        exact.dedup();
        let hit = match exact.len() {
            0 => {
                let mut differs: Vec<PathBuf> = named.iter().map(|c| c.path.clone()).collect();
                differs.sort();
                SearchHit::Missing { differs }
            }
            1 => SearchHit::Found {
                path: exact.remove(0),
                by,
            },
            _ => SearchHit::Ambiguous(exact),
        };
        report.results.push(SearchResult {
            track: t.track.clone(),
            hit,
        });
    }
    Ok(report)
}

/// The record for a file the folder search found: the old record with the new `path`,
/// `relative` and `mode` (`copied` when the file sits in `Audio Files/`). Size and head hash
/// stay, because the search has just checked them; so does `mtime`, which keeps the
/// `Analysis/` cache key and the cache current. A record without size or head hash is built
/// from the file ([`build_source`]).
pub fn relinked_source(record: &Source, path: &Path, project_dir: &Path) -> Result<Source, ProjectError> {
    let path = abspath(path);
    let pdir = abspath(project_dir);
    let mode = if path.parent() == Some(pdir.join(AUDIO_DIR).as_path()) {
        "copied"
    } else {
        "referenced"
    };
    if record.size.is_none() || record.head_sha1.is_none() {
        return build_source(&path, project_dir, mode);
    }
    let mut r = record.clone();
    r.mode = Some(mode.to_string());
    r.path = Some(path.to_string_lossy().into_owned());
    r.relative = Some(relpath(&path, &pdir));
    Ok(r)
}

/// The `Analysis/<track>/key.json` content: the identity of the source the cache was built
/// from (`server.cache_key`). A field the record lacks is `null`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CacheKey {
    /// The source record's `size` in bytes.
    pub size: Option<i64>,
    /// The source record's `mtime` in whole seconds.
    pub mtime: Option<i64>,
    /// The source record's `head_sha1` (SHA-1 of the first MiB).
    pub head_sha1: Option<String>,
}

/// `server.cache_key(track)`.
#[must_use]
pub fn cache_key(track: &Track) -> CacheKey {
    let rec = track.source_ref();
    CacheKey {
        size: rec.and_then(|r| r.size),
        mtime: rec.and_then(|r| r.mtime),
        head_sha1: rec.and_then(|r| r.head_sha1.clone()),
    }
}

impl CacheKey {
    /// True when `<analysis_dir>/key.json` holds exactly this key (a missing or unreadable
    /// file is stale, like `server.analysis_dir`).
    #[must_use]
    pub fn matches_file(&self, analysis_dir: &Path) -> bool {
        let Ok(text) = std::fs::read_to_string(analysis_dir.join("key.json")) else {
            return false;
        };
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => v == serde_json::to_value(self).expect("a key serialises"),
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests;
