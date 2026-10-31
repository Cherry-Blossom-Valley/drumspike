//! Optional spans for the main-thread and lock review (P3-15c, card Q4).
//!
//! With the cargo feature `trace` the app records three kinds of spans into a Chrome trace
//! file (open it in `chrome://tracing` or Perfetto):
//!
//! | Span | Where | Fields |
//! |---|---|---|
//! | `ipc` | every IPC call, on the thread that dispatches it (the main thread of the app): the whole command for a non-async command, the dispatch for an async one | `name`, `thread` |
//! | `cmd` | the body of a command on the blocking pool (`blocking` in `apps/desktop/src-tauri`) | `name`, `thread` |
//! | `job` | the stages of the background jobs (analysis, lanes, copy, export, import, relink) | `name`, `thread` |
//! | `lock_wait` | a [`crate::registry::lock`] call that had to wait | `at` (<file:line> of the caller), `thread` |
//!
//! `DRUMSPIKE_TRACE=<file.json>` turns the recording on ([`init_from_env`]); [`finish`] writes
//! the end of the file. Without the feature every function here is an empty inline function,
//! [`Span`] has no fields and no tracing crate is built: release builds pay nothing.

/// True when the crate was built with the feature `trace`.
pub const ENABLED: bool = cfg!(feature = "trace");

/// An entered span; the span ends when the value is dropped. Without the feature `trace`
/// it is an empty value.
#[must_use = "the span ends when this value is dropped"]
pub struct Span {
    #[cfg(feature = "trace")]
    _entered: Option<tracing::span::EnteredSpan>,
}

#[cfg(feature = "trace")]
mod imp {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Subscriber};
    use tracing_chrome::{ChromeLayerBuilder, FlushGuard};
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::registry::LookupSpan;

    /// The writer thread of the trace file; `finish` drops it, which writes the closing `]`.
    pub(super) static GUARD: Mutex<Option<FlushGuard>> = Mutex::new(None);
    /// Set by `finish`: the layer then ignores everything. A span that ends during
    /// `process::exit` would else reach the Chrome layer after its thread-local values are gone.
    pub(super) static DONE: AtomicBool = AtomicBool::new(false);

    /// The Chrome layer behind the `DONE` gate.
    struct Gate<L>(L);

    impl<S, L> Layer<S> for Gate<L>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
        L: Layer<S>,
    {
        fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
            if !DONE.load(Ordering::Acquire) {
                self.0.on_new_span(attrs, id, ctx);
            }
        }
        fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
            if !DONE.load(Ordering::Acquire) {
                self.0.on_record(id, values, ctx);
            }
        }
        fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
            if !DONE.load(Ordering::Acquire) {
                self.0.on_event(event, ctx);
            }
        }
        fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
            if !DONE.load(Ordering::Acquire) {
                self.0.on_enter(id, ctx);
            }
        }
        fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
            if !DONE.load(Ordering::Acquire) {
                self.0.on_exit(id, ctx);
            }
        }
        fn on_close(&self, id: Id, ctx: Context<'_, S>) {
            if !DONE.load(Ordering::Acquire) {
                self.0.on_close(id, ctx);
            }
        }
    }

    pub(super) fn thread() -> String {
        let t = std::thread::current();
        t.name().map_or_else(|| format!("{:?}", t.id()), str::to_string)
    }

    pub(super) fn init(path: &std::path::Path, offset_ms: f64) -> bool {
        let mut g = GUARD.lock().unwrap_or_else(|p| p.into_inner());
        if g.is_some() {
            return false;
        }
        let Ok(file) = std::fs::File::create(path) else {
            return false;
        };
        let (layer, guard) = ChromeLayerBuilder::new()
            .writer(file)
            .include_args(true)
            .include_locations(false)
            .build();
        if tracing::subscriber::set_global_default(tracing_subscriber::registry().with(Gate(layer))).is_err() {
            return false;
        }
        *g = Some(guard);
        // the trace clock starts here: this mark gives the offset from the start of the process
        tracing::info!(name: "trace_start", process_ms = offset_ms, thread = %thread());
        true
    }
}

/// A span `ipc` for the dispatch of the IPC call `name`, on the calling thread.
#[inline]
pub fn ipc(name: &str) -> Span {
    #[cfg(feature = "trace")]
    {
        Span {
            _entered: Some(tracing::info_span!("ipc", name = %name, thread = %imp::thread()).entered()),
        }
    }
    #[cfg(not(feature = "trace"))]
    {
        let _ = name;
        Span {}
    }
}

/// A span `cmd` for the body of the Tauri command `name`, on the calling thread.
#[inline]
pub fn command(name: &str) -> Span {
    #[cfg(feature = "trace")]
    {
        Span {
            _entered: Some(tracing::info_span!("cmd", name = %name, thread = %imp::thread()).entered()),
        }
    }
    #[cfg(not(feature = "trace"))]
    {
        let _ = name;
        Span {}
    }
}

/// A span `job` for one stage of a background job (`analyze:decode`, `lanes:track`, ...).
#[inline]
pub fn stage(name: &str) -> Span {
    #[cfg(feature = "trace")]
    {
        Span {
            _entered: Some(tracing::info_span!("job", name = %name, thread = %imp::thread()).entered()),
        }
    }
    #[cfg(not(feature = "trace"))]
    {
        let _ = name;
        Span {}
    }
}

/// A span `lock_wait` while a lock taken at `at` is waited for.
#[inline]
pub fn lock_wait(at: &'static std::panic::Location<'static>) -> Span {
    #[cfg(feature = "trace")]
    {
        let at = format!("{}:{}", at.file(), at.line());
        Span {
            _entered: Some(tracing::info_span!("lock_wait", at = %at, thread = %imp::thread()).entered()),
        }
    }
    #[cfg(not(feature = "trace"))]
    {
        let _ = at;
        Span {}
    }
}

/// Start the recording into `path`. `offset_ms` is the time since the start of the process at
/// this call (stored in the first event, `trace_start`; the event comes after the setup, which
/// took about 6 ms in the debug app, so `selftest/trace_spans.py` calibrates the offset against
/// the `selftest_clock` spans). False without the feature, when a recording runs already, or
/// when the file cannot be created.
#[must_use]
pub fn init(path: &std::path::Path, offset_ms: f64) -> bool {
    #[cfg(feature = "trace")]
    {
        imp::init(path, offset_ms)
    }
    #[cfg(not(feature = "trace"))]
    {
        let _ = (path, offset_ms);
        false
    }
}

/// [`init`] with the file named by `DRUMSPIKE_TRACE`; false when it is not set.
#[must_use]
pub fn init_from_env(offset_ms: f64) -> bool {
    match std::env::var_os("DRUMSPIKE_TRACE").filter(|v| !v.is_empty()) {
        Some(p) => init(std::path::Path::new(&p), offset_ms),
        None => false,
    }
}

/// End the recording: the file gets its closing bracket. Call it before `process::exit`.
/// Spans that end later are not written.
pub fn finish() {
    #[cfg(feature = "trace")]
    {
        let guard = imp::GUARD.lock().unwrap_or_else(|p| p.into_inner()).take();
        if guard.is_some() {
            imp::DONE.store(true, std::sync::atomic::Ordering::Release);
        }
        drop(guard);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_can_be_made_and_dropped_without_a_recording() {
        let i = ipc("project_open");
        let a = command("project_open");
        let b = stage("lanes:track");
        let c = lock_wait(std::panic::Location::caller());
        drop((c, b, a, i));
        finish();
    }

    #[test]
    fn without_the_variable_there_is_no_recording() {
        // the test process never sets DRUMSPIKE_TRACE; init into a folder that does not exist fails
        if std::env::var_os("DRUMSPIKE_TRACE").is_none() {
            assert!(!init_from_env(0.0));
        }
        assert!(!init(std::path::Path::new("/nonexistent/dir/trace.json"), 0.0));
        assert_eq!(ENABLED, cfg!(feature = "trace"));
    }

    #[cfg(not(feature = "trace"))]
    #[test]
    fn without_the_feature_a_span_is_empty() {
        assert_eq!(std::mem::size_of::<Span>(), 0);
        assert!(!init(&std::env::temp_dir().join("ds_trace_off.json"), 0.0));
    }
}
