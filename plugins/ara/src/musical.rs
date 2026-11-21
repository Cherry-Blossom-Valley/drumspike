//! M1c1 (#560): the musical context of the host (tempo entries and bar signatures), read once per change.
//!
//! The content access controller of the host is optional. The plugin reads on the thread that the host calls it on
//! (the main thread: the end of an edit cycle, the creation and the update of a musical context), and keeps the
//! result in a snapshot, so an export on a request thread never calls the host. [`read`] makes the snapshot and the
//! report row `musical_context`: what the host offers and what the conversion did, so the first run in a host says
//! what it gives.

use crate::ara::*;
use crate::tempo_ara::{tempo_from_ara, Bar, Converted, Entry};
use serde_json::{json, Value};

/// What the plugin knows of the host's timeline.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// The converted map, or `None` (no controller, no content, or a conversion that failed).
    pub tempo: Option<Converted>,
    /// The row of the report.
    pub report: Value,
}

unsafe fn slot<T: Copy>(base: *const T, declared: usize, off: usize) -> Option<T> {
    (declared >= off + std::mem::size_of::<usize>()).then(|| base.read_unaligned())
}

/// Read the tempo entries and bar signatures of the musical context `ctx` through the content access controller of
/// `host`. Main thread only (the caller checks).
///
/// # Safety
/// `host` is null or the host's valid `ARADocumentControllerHostInstance`, and `ctx` is the host ref of a live
/// musical context.
pub unsafe fn read(host: *const ARADocumentControllerHostInstance, ctx: usize) -> Snapshot {
    let no = |why: &str, extra: Value| Snapshot {
        tempo: None,
        report: json!({"event": "musical_context", "controller": false, "reason": why, "detail": extra}),
    };
    if host.is_null() {
        return no("no-host", Value::Null);
    }
    let declared = std::ptr::addr_of!((*host).structSize).read_unaligned();
    let end = std::mem::offset_of!(ARADocumentControllerHostInstance, contentAccessControllerInterface)
        + std::mem::size_of::<usize>();
    if declared < end {
        return no("struct-too-short", json!({"declared": declared, "needed": end}));
    }
    let iface = std::ptr::addr_of!((*host).contentAccessControllerInterface).read_unaligned();
    let cref = std::ptr::addr_of!((*host).contentAccessControllerHostRef).read_unaligned();
    if iface.is_null() {
        return no("no-controller", Value::Null);
    }
    let size = std::ptr::addr_of!((*iface).structSize).read_unaligned();
    macro_rules! f {
        ($name:ident) => {
            match slot(
                std::ptr::addr_of!((*iface).$name),
                size,
                std::mem::offset_of!(ARAContentAccessControllerInterface, $name),
            )
            .flatten()
            {
                Some(f) => f,
                None => return no("function-missing", json!({"function": stringify!($name)})),
            }
        };
    }
    let available = f!(isMusicalContextContentAvailable);
    let grade = f!(getMusicalContextContentGrade);
    let create = f!(createMusicalContextContentReader);
    let count = f!(getContentReaderEventCount);
    let data = f!(getContentReaderDataForEvent);
    let destroy = f!(destroyContentReader);
    let mctx = ctx as ARAMusicalContextHostRef;
    let tempo_ok = available(cref, mctx, kARAContentTypeTempoEntries as ARAContentType) != 0;
    let bars_ok = available(cref, mctx, kARAContentTypeBarSignatures as ARAContentType) != 0;
    let tempo_grade = grade(cref, mctx, kARAContentTypeTempoEntries as ARAContentType) as i64;
    let mut entries: Vec<Entry> = Vec::new();
    let mut bars: Vec<Bar> = Vec::new();
    if tempo_ok {
        let r = create(cref, mctx, kARAContentTypeTempoEntries as ARAContentType, std::ptr::null());
        if !r.is_null_ref() {
            for i in 0..count(cref, r) {
                let p = data(cref, r, i) as *const ARAContentTempoEntry;
                if !p.is_null() {
                    let e = p.read_unaligned();
                    entries.push(Entry { t: e.timePosition, q: e.quarterPosition });
                }
            }
            destroy(cref, r);
        }
    }
    if bars_ok {
        let r = create(cref, mctx, kARAContentTypeBarSignatures as ARAContentType, std::ptr::null());
        if !r.is_null_ref() {
            for i in 0..count(cref, r) {
                let p = data(cref, r, i) as *const ARAContentBarSignature;
                if !p.is_null() {
                    let b = p.read_unaligned();
                    bars.push(Bar { num: b.numerator, den: b.denominator, q: b.position });
                }
            }
            destroy(cref, r);
        }
    }
    let (tempo, reason, notes) = if !tempo_ok {
        (None, "no-tempo-content".to_string(), Vec::new())
    } else {
        match tempo_from_ara(&entries, &bars) {
            Ok(c) => {
                let n = c.notes.clone();
                (Some(c), String::new(), n)
            }
            Err(why) => (None, why.to_string(), Vec::new()),
        }
    };
    let report = json!({"event": "musical_context", "controller": true, "tempo_available": tempo_ok,
        "tempo_grade": tempo_grade, "entries": entries.len(), "bars_available": bars_ok, "bars": bars.len(),
        "reason": reason, "notes": notes,
        "bpm": tempo.as_ref().map(|c| c.map.bpm), "steps": tempo.as_ref().map(|c| c.map.steps.len())});
    Snapshot { tempo, report }
}

trait NullRef {
    fn is_null_ref(&self) -> bool;
}
impl<T> NullRef for *mut T {
    fn is_null_ref(&self) -> bool {
        self.is_null()
    }
}
impl NullRef for usize {
    fn is_null_ref(&self) -> bool {
        *self == 0
    }
}
