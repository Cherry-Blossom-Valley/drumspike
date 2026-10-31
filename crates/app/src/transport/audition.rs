//! The audition (P4-40d): the [`PlayWindow`] of a `transport_play` with `start`, `end`,
//! `loop` or `tracks`, the running [`Audition`], and the thread (`spawn_audition`) that ends
//! it and gives mute and solo back. See the module docs of `super` for the whole transport.

use std::sync::{Arc, Mutex, Weak};
use std::thread::sleep;
use std::time::Duration;

use serde_json::{Value, json};

use super::{HostState, restore_mix};
use crate::error::AppError;
use crate::registry::lock;

/// P4-40d: the window of an audition (a `transport_play` with `start`, `end`, `loop` or `tracks`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayWindow {
    /// Where the window starts (seconds); without it the call's `position`, else 0.
    pub start: Option<f64>,
    /// Where it ends (seconds): the play pauses there, or jumps back to `start` with `looped`.
    /// P4-40e: without an `end` and with a `shot`, the end is the one-shot's length.
    pub end: Option<f64>,
    /// Play the window again and again until `transport_stop` or another play.
    pub looped: bool,
    /// The tracks that are audible during this play and no others (overrides solo and mute of
    /// these names); empty: mute and solo stay as they are.
    pub tracks: Vec<String>,
    /// P4-40e: the track whose layer one-shot plays once as a preview voice at the start of the
    /// window (the Layer view's Play). Without `tracks`, the shot track is the only audible one.
    /// The close mic of the track sounds as in any audition.
    pub shot: Option<String>,
}

impl PlayWindow {
    /// The window of a command's arguments (`start`, `end`, `loop`, `tracks`, `shot`); `None` when
    /// none is given. `tracks` is a list of track names (a wrong type or an empty list is a 400).
    /// `shot` (P4-40e) is a track name whose one-shot plays as a preview; it solos the track when
    /// no `tracks` are given.
    ///
    /// # Errors
    /// 400 when `tracks` is not a non-empty list of strings, or `shot` is not a non-empty track name
    /// (or null).
    pub fn from_args(
        start: Option<f64>,
        end: Option<f64>,
        looped: Option<bool>,
        tracks: Option<&Value>,
        shot: Option<&Value>,
    ) -> Result<Option<PlayWindow>, AppError> {
        let mut tracks = match tracks {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(a)) if !a.is_empty() => a
                .iter()
                .map(|v| v.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| AppError::bad("`tracks` must be a list of track names"))?,
            Some(_) => return Err(AppError::bad("`tracks` must be a non-empty list of track names")),
        };
        let shot = match shot {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| AppError::bad("`shot` must be a track name (or null)"))?
                    .to_string(),
            ),
        };
        if let Some(n) = &shot
            && !tracks.iter().any(|t| t == n)
        {
            tracks.push(n.clone());
        }
        let looped = looped.unwrap_or(false);
        if start.is_none() && end.is_none() && !looped && tracks.is_empty() && shot.is_none() {
            return Ok(None);
        }
        Ok(Some(PlayWindow {
            start,
            end,
            looped,
            tracks,
            shot,
        }))
    }
}

/// The audition that is running (see [`PlayWindow`]); `mute` and `solo` of the host stay as they were.
#[derive(Debug, Clone)]
pub(crate) struct Audition {
    pub(crate) win: PlayWindow,
    pub(crate) start: f64,
}

impl Audition {
    pub(crate) fn json(&self) -> Value {
        // `shot` only when there is one (the plain audition answer stays as it was)
        let mut v =
            json!({"tracks": self.win.tracks, "start": self.start, "end": self.win.end, "loop": self.win.looped});
        if let Some(s) = &self.win.shot {
            v["shot"] = json!(s);
        }
        v
    }
}

/// End the audition: pause, and give mute and solo back to the page's state.
fn finish_audition(st: &mut HostState) {
    st.audition = None;
    st.audition_gen += 1;
    st.epoch += 1;
    if let Some(o) = &st.open {
        o.transport.pause();
    }
    restore_mix(st);
}

/// The thread of an audition: it watches the transport (every millisecond) and gives mute and
/// solo back when the audition is over. The engine ends a windowed audition itself at its end
/// (#201) and wraps a loop (#166): the thread only sees the pause — the engine's end, or a pause
/// from outside (a device loss, a close). It ends when `audition_gen` changes (a stop, another
/// play or the end).
pub(crate) fn spawn_audition(host: &Arc<Mutex<HostState>>, gen_n: u64) {
    let weak: Weak<Mutex<HostState>> = Arc::downgrade(host);
    let _ = std::thread::Builder::new()
        .name("drumspike-audition".into())
        .spawn(move || {
            loop {
                let Some(h) = weak.upgrade() else { return };
                {
                    let mut guard = lock(&h);
                    let st = &mut *guard;
                    if st.audition_gen != gen_n || st.audition.is_none() {
                        return;
                    }
                    let Some(o) = st.open.as_mut() else {
                        st.audition = None;
                        st.audition_gen += 1;
                        return;
                    };
                    if !o.transport.is_playing() {
                        // the engine ended the window (#201) or a pause from outside (a device
                        // loss, a close): the audition is over
                        finish_audition(st);
                        return;
                    }
                }
                drop(h);
                sleep(Duration::from_millis(1));
            }
        });
}
