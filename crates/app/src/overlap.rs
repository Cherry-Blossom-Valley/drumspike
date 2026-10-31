//! P3-15b: the two-stage pipeline of the analysis job (card Q3 of `concurrency-review.md`).
//!
//! The analysis of a track has a preparation (decode, downmix, resample, features) that needs
//! no model, and a model step that needs the one loaded model. The job prepares track i+1 on
//! a rayon pool of [`default_threads`] threads ([`pool`]) while the model runs track i. [`run`] is that pipeline
//! without the audio: a producer thread prepares the items in order and hands each one over
//! a rendezvous channel; the caller's thread consumes them in the same order.
//!
//! Bounds and rules:
//!
//! * At most two prepared items exist at any time: the one in the consumer's hands and the
//!   one the producer holds at the hand-over (the channel has no buffer). The producer starts
//!   the next item only after the hand-over.
//! * A cancel stops the producer before its next item and the consumer before its next
//!   hand-over. The item in the model step finishes. [`run`] returns how many items were
//!   consumed; the caller marks the rest.
//! * An error of one item is handed over like a result and consumed; it does not stop the
//!   pipeline.
//! * The detector session is not touched here: the detector step runs on the caller's
//!   thread as before, one track at a time.

use std::sync::OnceLock;
use std::sync::mpsc::sync_channel;

/// Threads of the preparation pool by default: 8, or the physical cores when there are fewer.
/// The decode of one track is one thread; the resample and the features split their frames
/// over the pool (`par_chunks_mut`, bit-identical for any thread count). Measured on arm64-mac
/// (12 cores, 30 tracks of 5 minutes at 96 kHz, host load under 8 at the start): 4 threads
/// 38.5 s (the preparation is the slower stage), 8 threads 33.6 s, 12 threads 33.7 s.
pub fn default_threads() -> usize {
    physical_cores().clamp(1, 8)
}

/// The physical core count of this host (`sysctlbyname("hw.physicalcpu")` on macOS), 0 when
/// it cannot be read.
/// The physical core count of this host; 0 when it cannot be read.
pub fn physical_cores() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZero::get)
        .unwrap_or(0)
}

/// The pool size: [`default_threads`], or `DRUMSPIKE_PREPARE_THREADS` (a positive number) for
/// a measurement. The pool threads keep the default QoS class: a lower class (utility) made
/// the preparation 7 times slower when other work ran on the host (266.8 s against 38.5 s
/// for the 30 tracks), because the kernel then gives the cores to everything else first.
pub fn pool_threads() -> usize {
    std::env::var("DRUMSPIKE_PREPARE_THREADS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(default_threads)
}

/// The pool the preparation runs on, built once per process (threads `ds-prepare-<n>`).
/// Rayon's `par_*` calls inside [`rayon::ThreadPool::install`] run on this pool, not on the
/// global one.
pub fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(pool_threads())
            .thread_name(|i| format!("ds-prepare-{i}"))
            .build()
            .expect("the preparation pool starts")
    })
}

/// Run `prepare` on every item on the preparation pool, one item ahead of `consume`, which
/// runs on this thread in the items' order. See the module documentation for the bounds.
/// Returns the number of items consumed: `items.len()` unless `cancelled` returned true.
pub fn run<T, P, E>(
    items: &[T],
    prepare: impl Fn(&T) -> Result<P, E> + Sync,
    mut consume: impl FnMut(&T, Result<P, E>),
    cancelled: impl Fn() -> bool + Sync,
) -> usize
where
    T: Sync,
    P: Send,
    E: Send,
{
    let mut consumed = 0;
    std::thread::scope(|s| {
        // No buffer: `send` returns only when the consumer has taken the item, so the
        // producer holds at most one prepared item and starts the next one after the hand-over.
        let (tx, rx) = sync_channel::<Result<P, E>>(0);
        let prepare = &prepare;
        let cancelled = &cancelled;
        s.spawn(move || {
            for item in items {
                if cancelled() {
                    break;
                }
                let r = pool().install(|| prepare(item));
                if tx.send(r).is_err() {
                    break; // the consumer stopped (a cancel)
                }
            }
        });
        for item in items {
            if cancelled() {
                break; // drops `rx`: the producer's pending or next `send` fails and it stops
            }
            let Ok(r) = rx.recv() else {
                break; // the producer stopped before this item (it saw the cancel first)
            };
            consume(item, r);
            consumed += 1;
        }
    });
    consumed
}

#[cfg(test)]
mod tests;
