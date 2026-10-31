//! R10/R11: the shared atomic-file helper: one unique temporary file in the destination's
//! folder, opened with `create_new`, its cleanup owned by the guard, one durability policy:
//! the payload is flushed and synced to disk before the rename and the destination folder is
//! fsynced after it (best effort, ignored where the file system refuses). Two writers in one
//! process never share a temporary inode, and a failed write leaves the destination and the
//! folder exactly as they were.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// One step per created file, so two writers in the same nanosecond still differ.
static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique_suffix() -> String {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("{}-{}-{}", std::process::id(), nanos, n)
}

/// `_fsync_dir`: flush the folder entry; errors are ignored (not every file system allows it).
fn fsync_dir(path: &Path) {
    if let Ok(f) = File::open(path) {
        let _ = f.sync_all();
    }
}

/// The real finish of [`TempFile::commit`]: sync the file, rename, fsync the folder.
fn finish_real(mut f: File, tmp: &Path, dest: &Path) -> io::Result<()> {
    f.flush()?;
    f.sync_all()?;
    drop(f); // close before the rename (Windows renames only closed files)
    std::fs::rename(tmp, dest)?;
    if let Some(dir) = dest.parent() {
        fsync_dir(dir);
    }
    Ok(())
}

/// A unique temporary file in the folder of `dest` (R10): created with `create_new`, so two
/// writers never open one inode, and removed by the guard unless it was committed.
pub struct TempFile {
    path: PathBuf,
    dest: PathBuf,
    file: Option<File>,
    committed: bool,
}

impl TempFile {
    /// Create the temporary file for writing `dest`. Already-used names are skipped; every
    /// other error (a missing folder, no permission) comes back as it is.
    pub fn create(dest: &Path) -> io::Result<Self> {
        let name = dest
            .file_name()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("no file name in {}", dest.display()),
                )
            })?
            .to_string_lossy()
            .into_owned();
        let dir = dest
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut last = None;
        for _ in 0..64 {
            let tmp = dir.join(format!(".{name}.{}.tmp", unique_suffix()));
            match File::options().write(true).create_new(true).open(&tmp) {
                Ok(file) => {
                    return Ok(Self {
                        path: tmp,
                        dest: dest.to_path_buf(),
                        file: Some(file),
                        committed: false,
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "no unique temporary file name")))
    }

    /// The path of the temporary file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The open temporary file, or an error when it was already taken (only possible after
    /// `commit_via` moved it; the type never closes it before that).
    fn file_mut(&mut self) -> io::Result<&mut File> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the temporary file is already closed"))
    }

    /// Write all of `bytes` to the temporary file.
    pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file_mut()?.write_all(bytes)
    }

    /// Flush and sync the temporary file to disk, close it, rename it onto `dest`, fsync the
    /// destination folder (best effort). On any error the guard removes the temporary file.
    pub fn commit(self) -> io::Result<()> {
        self.commit_via(finish_real)
    }

    fn commit_via(self, finish: impl FnOnce(File, &Path, &Path) -> io::Result<()>) -> io::Result<()> {
        let mut s = self;
        let f = s
            .file
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the temporary file is already closed"))?;
        finish(f, &s.path, &s.dest)?; // an error keeps committed=false: the drop removes the file
        s.committed = true;
        Ok(())
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        self.file.take(); // close before the removal
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Write `bytes` to `path` atomically: a unique temporary file in the same folder (created
/// with `create_new`, so two writers never share one inode), synced to disk, renamed over
/// `path`; the folder is fsynced (best effort). On any error the temporary file is removed
/// and `path` stays exactly as it was. This is the shared helper behind `io::write_atomic`,
/// `write_document` and the app's JSON/preset/activation writers (R10).
pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_bytes_atomic_via(path, bytes, std::io::Write::write_all, finish_real)
}

/// [`write_bytes_atomic`] with the write and the finish injected (the tests inject failures).
fn write_bytes_atomic_via(
    path: &Path,
    bytes: &[u8],
    write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
    finish: impl FnOnce(File, &Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let mut t = TempFile::create(path)?;
    write(t.file_mut()?, bytes)?;
    t.commit_via(finish)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("drumspike-core-atomic-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn temp_leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn commit_replaces_the_destination_and_leaves_no_temporary() {
        let d = tmp_dir("basic");
        let dest = d.join("x.bin");
        write_bytes_atomic(&dest, b"one").unwrap();
        write_bytes_atomic(&dest, b"two!").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"two!");
        assert!(temp_leftovers(&d).is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn two_writers_get_two_temporary_files_and_each_rename_is_whole() {
        let d = tmp_dir("two");
        let dest = d.join("x.bin");
        let mut a = TempFile::create(&dest).unwrap();
        let mut b = TempFile::create(&dest).unwrap();
        assert_ne!(a.path(), b.path(), "create_new hands out two distinct names");
        a.write_all(b"AAA").unwrap();
        b.write_all(b"BBB").unwrap();
        a.commit().unwrap();
        b.commit().unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"BBB");
        assert!(temp_leftovers(&d).is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_injected_write_failure_leaves_no_temporary_and_no_destination() {
        let d = tmp_dir("wfail");
        let dest = d.join("x.bin");
        let e = write_bytes_atomic_via(
            &dest,
            b"payload",
            |_, _| Err(io::Error::other("write failed")),
            finish_real,
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "write failed");
        assert!(!dest.exists(), "the destination was not created");
        assert!(temp_leftovers(&d).is_empty(), "the guard removed the temporary file");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_injected_sync_failure_leaves_no_temporary_and_no_destination() {
        let d = tmp_dir("sfail");
        let dest = d.join("x.bin");
        let e = write_bytes_atomic_via(&dest, b"payload", std::io::Write::write_all, |_, _, _| {
            Err(io::Error::other("sync failed"))
        })
        .unwrap_err();
        assert_eq!(e.to_string(), "sync failed");
        assert!(!dest.exists(), "the destination was not created");
        assert!(temp_leftovers(&d).is_empty(), "the guard removed the temporary file");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_failing_rename_leaves_no_temporary_and_the_destination_untouched() {
        let d = tmp_dir("rfail");
        // a destination that is a directory: the final rename fails
        let dest = d.join("sub");
        std::fs::create_dir_all(&dest).unwrap();
        assert!(write_bytes_atomic(&dest, b"payload").is_err());
        assert!(dest.is_dir(), "the destination is unchanged");
        assert!(temp_leftovers(&d).is_empty(), "the guard removed the temporary file");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_missing_folder_is_an_error_and_creates_nothing() {
        let d = tmp_dir("gone");
        let dest = d.join("missing").join("x.bin");
        assert!(write_bytes_atomic(&dest, b"payload").is_err());
        assert!(temp_leftovers(&d).is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
