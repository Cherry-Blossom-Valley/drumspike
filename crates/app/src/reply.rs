//! R15 (#43): the transport plumbing of a command, shared by every adapter. [`Reply`] is what a
//! command answers (JSON, or bytes for `peaks_tile` and `samples_window`); [`Channels`] carries the
//! progress messages of a long command. The command dispatcher (`call`), the test bridge
//! (`bridge`), the command-line tool and the MCP server all use these types from here, so none of
//! them depends on the HTTP bridge for its plumbing.

use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

/// The answer of a command: JSON, or bytes for `peaks_tile` and `samples_window`.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Json(Value),
    Bytes(Vec<u8>),
}

/// Most messages one channel keeps; the oldest are dropped (a page that never polls).
pub(crate) const QUEUE_CAP: usize = 1000;

/// A callback that takes the messages of a channel: id and message.
type Sink = Box<dyn Fn(&str, Value) + Send + Sync>;

/// The message queues of the channels, by id.
#[derive(Default)]
pub struct Channels {
    queues: Mutex<HashMap<String, VecDeque<Value>>>,
    cv: Condvar,
    /// P4-5: a callback that takes the messages instead of the queue (the command-line tool and the MCP server).
    sink: Option<Sink>,
}

impl Channels {
    /// Channels whose messages go to `sink` (channel id, message) at once; nothing is queued.
    pub fn with_sink(sink: impl Fn(&str, Value) + Send + Sync + 'static) -> Channels {
        Channels {
            sink: Some(Box::new(sink)),
            ..Channels::default()
        }
    }

    /// Add a message to the queue of `id` and wake the waiting poll.
    pub fn push(&self, id: &str, msg: Value) {
        if let Some(sink) = &self.sink {
            sink(id, msg);
            return;
        }
        let mut g = self.queues.lock().unwrap_or_else(|e| e.into_inner());
        let q = g.entry(id.to_string()).or_default();
        if q.len() >= QUEUE_CAP {
            q.pop_front();
        }
        q.push_back(msg);
        self.cv.notify_all();
    }

    /// Take every message of `id`. Waits up to `timeout` for the first one; an empty list
    /// means the time ran out.
    pub fn poll(&self, id: &str, timeout: Duration) -> Vec<Value> {
        let end = Instant::now() + timeout;
        let mut g = self.queues.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(q) = g.get_mut(id)
                && !q.is_empty()
            {
                return q.drain(..).collect();
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Vec::new();
            }
            g = self.cv.wait_timeout(g, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}
