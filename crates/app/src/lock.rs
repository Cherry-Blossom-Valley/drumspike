//! P4-5 step 5: the cross-process project lock. A process that opens a project for writing takes an
//! advisory exclusive `flock` on `<project>/.drumspike.lock` (`std::fs::File::try_lock`) and holds it until the
//! project is closed or the process ends (the OS drops the lock then, so a crash leaves no stale lock). The
//! file holds `{"pid", "client"}` for the message of the process that fails to get the lock.
//!
//! This is the outer layer. The in-process layer is the P4-3 store guard (`jobs::Jobs::begin_store`: one
//! analysis, re-pick store or params write per project). The lock says which process may write, the guard says
//! which job of that process. A read-only open takes no lock.
//!
//! The service takes locks only when a client name is set (`Service::set_lock_client`: the app, the command-line
//! tool and the MCP server do; the tests and the test bridge do not).

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::error::AppError;

/// The lock file in a project folder.
pub const LOCK_FILE: &str = ".drumspike.lock";

/// A held lock; dropping it releases it.
#[derive(Debug)]
pub struct ProjectLock {
    file: File,
    path: PathBuf,
}

/// The result of asking for a lock. Only another holder refuses a project (`acquire` answers 409 then); a
/// folder or file system that cannot hold a lock (read-only folder, a volume without `flock`) opens the project
/// unlocked, and the open answer says why (`lock: "unavailable: ..."`).
#[derive(Debug)]
pub enum Lock {
    /// The lock is held; dropping the value releases it.
    Held(ProjectLock),
    /// No lock could be made; the text says why (starts with a lower case word, e.g. "read-only file system").
    Unavailable(String),
}

impl Lock {
    /// The text of the open answer's `lock` field: `held` or `unavailable: <why>`.
    #[must_use]
    pub fn note(&self) -> String {
        match self {
            Lock::Held(_) => "held".to_string(),
            Lock::Unavailable(why) => format!("unavailable: {why}"),
        }
    }

    /// The lock, when held.
    #[must_use]
    pub fn into_held(self) -> Option<ProjectLock> {
        match self {
            Lock::Held(l) => Some(l),
            Lock::Unavailable(_) => None,
        }
    }
}

/// Who holds a lock, from the file (`None` when it cannot be read).
fn holder(file: &mut File) -> Option<(u32, String)> {
    let mut text = String::new();
    file.rewind().ok()?;
    file.read_to_string(&mut text).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    Some((v["pid"].as_u64()? as u32, v["client"].as_str()?.to_string()))
}

/// An error text without a leading capital ("Read-only file system (os error 30)" to "read-only file system").
fn why(e: &std::io::Error) -> String {
    let t = e.to_string();
    let t = t.split(" (os error").next().unwrap_or(&t).to_string();
    let mut c = t.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => t,
    }
}

/// Test hook: while set, every `try_lock` fails with this error kind (a file system without `flock`).
static FAKE_ERROR: Mutex<Option<std::io::ErrorKind>> = Mutex::new(None);

/// Make every following lock fail as a volume without `flock` support does (`None` ends it). For tests; process wide.
#[doc(hidden)]
pub fn fake_lock_error(kind: Option<std::io::ErrorKind>) {
    *FAKE_ERROR.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = kind;
}

fn try_lock(file: &File) -> Result<(), TryLockError> {
    if let Some(kind) = *FAKE_ERROR.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
        return Err(TryLockError::Error(std::io::Error::new(
            kind,
            "operation not supported",
        )));
    }
    file.try_lock()
}

impl ProjectLock {
    /// Take the lock of the project folder `dir` for `client` (`app`, `cli`, `mcp`). 409 only when another process
    /// holds it (the message names its client and pid). Any other failure (the file cannot be made, the file system has
    /// no locks) is [`Lock::Unavailable`], not an error: the project opens unlocked.
    ///
    /// # Errors
    ///
    /// Returns a 409 error when another process holds the lock on `dir`.
    pub fn acquire(dir: &Path, client: &str) -> Result<Lock, AppError> {
        let path = dir.join(LOCK_FILE);
        // A lock file this user cannot write (another user's 0644 file on a shared folder) can still be locked: the
        // read-only descriptor takes the same `flock`. Only when no descriptor can be made, or none can be locked, the
        // lock is unavailable.
        let opened = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path);
        let (mut file, writable) = match opened {
            Ok(f) => (f, true),
            Err(e) => match File::open(&path) {
                Ok(f) => (f, false),
                Err(_) => return Ok(Lock::Unavailable(why(&e))),
            },
        };
        match try_lock(&file) {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let who = holder(&mut file).map_or_else(
                    || "another process".into(),
                    |(pid, client)| format!("{client} (pid {pid})"),
                );
                return Err(AppError::conflict(format!(
                    "the project is open for writing in {who}: close it there, or open it read-only"
                )));
            }
            Err(TryLockError::Error(e)) => return Ok(Lock::Unavailable(why(&e))),
        }
        if !writable {
            return Ok(Lock::Held(ProjectLock { file, path })); // held; the holder text cannot be written
        }
        let body = json!({"pid": std::process::id(), "client": client}).to_string();
        file.set_len(0)
            .and_then(|_| file.rewind())
            .and_then(|_| file.write_all(body.as_bytes()))
            .ok();
        Ok(Lock::Held(ProjectLock { file, path }))
    }

    /// The pid and client named in the lock file of `dir` (whoever holds it), for tests and messages.
    #[must_use]
    pub fn holder_of(dir: &Path) -> Option<(u32, String)> {
        holder(&mut File::open(dir.join(LOCK_FILE)).ok()?)
    }

    /// The lock file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ProjectLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(l: Result<Lock, AppError>) -> ProjectLock {
        l.unwrap().into_held().expect("held")
    }

    #[test]
    fn a_second_lock_of_the_same_folder_is_refused_until_the_first_is_dropped() {
        let dir = std::env::temp_dir().join(format!("ds_lock_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = held(ProjectLock::acquire(&dir, "app"));
        assert_eq!(
            ProjectLock::holder_of(&dir),
            Some((std::process::id(), "app".to_string()))
        );
        let e = ProjectLock::acquire(&dir, "cli").unwrap_err();
        assert_eq!(e.code, 409);
        assert!(e.message.contains("app (pid"), "{}", e.message);
        assert_eq!(a.path(), dir.join(LOCK_FILE));
        drop(a);
        let b = held(ProjectLock::acquire(&dir, "cli"));
        assert_eq!(ProjectLock::holder_of(&dir).unwrap().1, "cli");
        drop(b);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_that_cannot_hold_the_file_is_unavailable_not_an_error() {
        match ProjectLock::acquire(Path::new("/nonexistent/folder"), "cli").unwrap() {
            Lock::Unavailable(why) => assert!(why.contains("no such file"), "{why}"),
            Lock::Held(_) => panic!("held"),
        }
        let l = ProjectLock::acquire(Path::new("/nonexistent/folder"), "cli").unwrap();
        assert!(l.note().starts_with("unavailable: "));
    }

    #[test]
    fn a_lock_file_that_cannot_be_written_is_still_locked_through_a_read_only_descriptor() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ds_lock_ro_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = held(ProjectLock::acquire(&dir, "app"));
        let file = dir.join(LOCK_FILE);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
        // the write-mode open fails now; the second writer must still see the first one's lock
        let e = ProjectLock::acquire(&dir, "cli").unwrap_err();
        assert_eq!(e.code, 409, "{}", e.message);
        assert!(e.message.contains("app (pid"), "{}", e.message);
        drop(first);
        // nobody holds it: the read-only descriptor takes the lock, and a third writer is refused
        let second = held(ProjectLock::acquire(&dir, "cli"));
        assert_eq!(ProjectLock::acquire(&dir, "app").unwrap_err().code, 409);
        drop(second);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn why_drops_the_os_error_and_the_capital() {
        let e = std::io::Error::from_raw_os_error(30);
        assert_eq!(why(&e), "read-only file system");
    }
}
