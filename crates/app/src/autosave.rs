//! The autosave (P3-4a, plan section 4): while a project is dirty, a background thread writes
//! `Backups/<Name> <timestamp>.drumspike` from the in-memory document every
//! `DRUMSPIKE_AUTOSAVE_SECONDS` (default 120). It never writes the main document, so the
//! document's bytes do not change (Pro Tools behaviour). The files count against the same
//! rotation of ten as the pre-save copies. The thread holds only a weak reference to the
//! service and ends with it. #121: after the write it runs the stale-write check of `project_save`
//! and sends `project-stale` `{sid, message}` once when the document changed on disk. #228: a failed write is kept
//! as `autosave_error` on the project (cleared by a good autosave or a save) and sent as `autosave-failed`
//! `{sid, message}` on the first failure of a streak.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use drumspike_core::project::{
    BACKUPS_DIR, BACKUPS_KEEP, FORMAT, backup_name, free_name, rotate_backups, write_document,
};

use crate::error::AppError;
use crate::registry::{Shared, lock};
use crate::{Inner, Service};

/// The default interval, in seconds.
pub const DEFAULT_SECONDS: f64 = 120.0;

/// `DRUMSPIKE_AUTOSAVE_SECONDS` as a duration: a positive number of seconds, else the default.
#[must_use]
pub fn interval_from(text: Option<&str>) -> Duration {
    let secs = text
        .and_then(|t| t.trim().parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s > 0.0)
        .unwrap_or(DEFAULT_SECONDS);
    Duration::from_secs_f64(secs)
}

/// How often the thread looks at the projects: a quarter of the interval, 25 ms to 500 ms.
#[must_use]
pub fn tick_period(interval: Duration) -> Duration {
    (interval / 4).clamp(Duration::from_millis(25), Duration::from_millis(500))
}

/// Start the thread for a new service.
pub(crate) fn spawn(inner: &Arc<Inner>) {
    let weak = Arc::downgrade(inner);
    let _ = std::thread::Builder::new().name("autosave".into()).spawn(move || {
        loop {
            let Some(inner) = weak.upgrade() else { break };
            let svc = Service { inner };
            let _ = svc.autosave_tick(Instant::now());
            let wait = tick_period(svc.autosave_interval());
            drop(svc);
            std::thread::sleep(wait);
        }
    });
}

impl Service {
    /// Autosave every dirty project whose interval has passed at `now` (the clock starts when
    /// the project becomes dirty and again after each autosave). Returns the files written.
    #[must_use]
    pub fn autosave_tick(&self, now: Instant) -> Vec<PathBuf> {
        let interval = self.autosave_interval();
        let mut written = Vec::new();
        for (_, op) in self.inner.registry.all() {
            let due = {
                let g = lock(&op);
                !g.read_only
                    && g.dirty
                    && g.last_autosave
                        .or(g.dirty_since)
                        .is_some_and(|base| now.saturating_duration_since(base) >= interval)
            };
            if !due {
                continue;
            }
            let outcome = self.autosave_one(&op);
            // #228: the error stays on the project and goes to the page (and the agent status) as well as to stderr.
            let mut g = lock(&op);
            match outcome {
                Ok(p) => {
                    g.note_autosave(Ok(()));
                    written.push(p);
                }
                Err(e) => {
                    eprintln!("autosave failed: {e}");
                    g.note_autosave(Err(&e.message));
                }
            }
            // A failed write waits for the next interval too; it must not retry on every tick.
            g.last_autosave = Some(now);
        }
        written
    }

    /// Write one backup file from the in-memory document and rotate `Backups/`.
    fn autosave_one(&self, op: &Shared) -> Result<PathBuf, AppError> {
        let _span = crate::trace::stage("autosave:write");
        let _io = lock(&self.inner.life.io);
        let (mut doc, dir, name) = {
            let g = lock(op);
            (g.project.doc.clone(), g.project.dir.clone(), g.project.name())
        };
        doc.format = FORMAT;
        doc.name = Some(name.clone());
        let bdir = dir.join(BACKUPS_DIR);
        std::fs::create_dir_all(&bdir).map_err(|e| AppError::internal(format!("{}: {e}", bdir.display())))?;
        let path = bdir.join(free_name(&bdir, &backup_name(&name, None)?));
        write_document(&doc, &path)?;
        rotate_backups(&dir, BACKUPS_KEEP)?;
        // #121: the stale-write check of project_save. The backup holds the edits either way; a document that changed on
        // disk is told to the page now (`project-stale`), before the user saves over it.
        let mut g = lock(op);
        if g.note_stale() {
            eprintln!(
                "autosave: {}",
                g.check_disk().err().map(|e| e.message).unwrap_or_default()
            );
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interval_comes_from_the_text_or_the_default() {
        assert_eq!(interval_from(None), Duration::from_secs(120));
        assert_eq!(interval_from(Some("1")), Duration::from_secs(1));
        assert_eq!(interval_from(Some(" 0.5 ")), Duration::from_millis(500));
        for bad in ["0", "-3", "abc", "", "NaN", "inf"] {
            assert_eq!(interval_from(Some(bad)), Duration::from_secs(120), "{bad}");
        }
    }

    #[test]
    fn the_thread_ends_with_the_service() {
        let svc = Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        let weak = Arc::downgrade(&svc.inner);
        drop(svc);
        let end = Instant::now() + Duration::from_secs(5);
        while weak.strong_count() > 0 {
            assert!(Instant::now() < end, "the service is still alive");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn the_tick_is_a_quarter_of_the_interval_within_limits() {
        assert_eq!(tick_period(Duration::from_secs(120)), Duration::from_millis(500));
        assert_eq!(tick_period(Duration::from_secs(1)), Duration::from_millis(250));
        assert_eq!(tick_period(Duration::from_millis(40)), Duration::from_millis(25));
    }
}
