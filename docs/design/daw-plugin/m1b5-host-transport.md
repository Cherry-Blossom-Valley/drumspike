# M1b5: the page follows the transport of the host

Ticket #546, umbrella #503 (the DAW plugin program), part of M1 (#510). Follows M1b2 (#538: the
editor window). The plugin lives in `plugins/ara/`. Nothing in `static/` or `crates/` changed.

## The finding from LUNA

The first run of the plugin in a real DAW (LUNA, 2026-11-20): the host plays, but the playhead of
the page does not move. M1b2 left `B.hostTransport` unset. The page has had its host mode since
#518: it follows `B.hostTransport.onPosition` and plays no audio of its own. The plugin now
sets `B.hostTransport` and feeds it.

## What the page does now

- The host plays: the page shows the playing state and the playhead moves with the host.
- The host stops: the page shows the stopped state at the position of the host.
- The host locates (stopped or playing): the playhead jumps.
- The page starts no audio. Its Play button is refused with the message "the host plays: start
  playback in the DAW". Stop and locate from the page do nothing yet (a later part).

## The source of the transport

**The CLAP `process` call.** Its transport has the song position in seconds (a fixed point
number, 2^31 per second) and the play flag. This is the only source:

- ARA gives a plugin no transport. The ARA playback controller is a way from the plugin to the
  host: the plugin may ask the host to play or to move. Nothing tells the plugin where the host
  is.
- In the VST3, the wrapper (clap-wrapper) fills the seconds timeline from `projectTimeSamples`.
  So the same call carries the position of a VST3 host.

The position in samples of the host is the position in seconds times the sample rate of
`activate`. The report writes both.

A call is used only when its transport exists and has the flag "seconds timeline". A host that
sends only beats gives nothing, and the page keeps its playhead (see "No transport").

The first value, and its source, are in the report: the row `transport_seen` (first process call
with a transport: the flags, the position, `"source": "clap_process"`) and the row
`transport_page_first` (the first message to a page: the position of the host in seconds and in
samples, the page time, how it was mapped). The next LUNA run shows what LUNA sends.

## The hand-over from the audio thread

The audio thread only stores atomics. It takes no lock, makes no allocation, writes no row and no
log. `transport::note` is called from `process`.

The slot is a **sequence lock** made of atomics (`src/transport.rs`):

- A writer makes the counter odd with a compare-exchange, stores the position, the flags, the
  frame count, the sample rate and the time of the call, and makes the counter even. Half of the
  counter is the generation of the transport.
- Several plugin instances of one host may call `process` at the same time, each on its own audio
  thread. One instance owns the slot (the address of its plugin object is the owner). Another
  instance stores only when the owner has made no call for the stale time (see below), so two
  instances at different positions never alternate: the page sees the owner. A writer that finds
  the lock taken skips its store. No writer ever waits. The sample rate is stored with each write,
  so it stays with the owner.
- A call with a transport but without a seconds timeline stores its play state and no position
  (`has_position` false). The reader then does not trust the older position: it sends nothing and
  the report gets one row `transport_no_position` with the flags. The plugin does not compute a
  position from beats and tempo.
- `deactivate` and `reset` of the owner clear the slot, so a new editor does not move the playhead
  to the position of an earlier session. The next process call fills it again.
- The reader (the request thread of the editor) reads the counter, the fields, and the counter
  again. It throws away a read where the counter changed or was odd. It never blocks the audio
  thread.

Proof: the unit tests `the_audio_thread_stores_only_atomics_and_a_reader_never_sees_a_torn_value`
(two writers and a reader: a torn value is never seen) and
`the_audio_thread_call_allocates_nothing_and_takes_no_lock` (a counting allocator sees no
allocation in 10000 calls; the size of the slot is the sum of its atomics: no mutex, no pointer).
Stage `transport-handover` runs them; its fault run (`DS_ARA_TRANSPORT_FAULT=alloc`: the audio
thread allocates) makes the second test fail.

## From the slot to the page

The editor has one more long poll, `/__ds/transport`, next to the poll of the events. The shell
script of the page (`shell.js`) loops on it. The answer is `{"messages": [..]}` with one message
at most: `{epoch, t_ns, pos, playing}`. The shell adds the `sid` of the page and calls
`B.hostTransport.onPosition`. This is the interface of #518; the page needs no change.

`B` does not exist when the shell script runs (it is a `const` of `backend.js`). The shell sets
`B.hostTransport` when the page has loaded, and then calls `appHostFollow()`, as `transport.js`
says a late shell must. A message that comes before the page has a `sid` waits; the newest one is
delivered when the page is ready. After a reload of the project a stopped position is sent again.

`t_ns` is the time of the position on the monotonic clock of the plugin. The page estimates the
offset to its own clock from the fastest message, so the clocks need not agree.

### The update rate

The request thread looks at the slot every 8 ms while the host plays. While the host is stopped
and nothing changed it does no work and waits 20 ms (it reads atomics only; no call of the audio
thread can wake it). The list of regions is fetched only when a look needs it, at most every
200 ms, and while the host is stopped only when a change counter of the regions moved (a unit test
`a_change_of_the_regions_while_stopped_is_fetched_once_after_the_wait`): 0 fetches in 2 s of a stopped host (bound 2 in the unit test
`a_stopped_host_costs_no_region_fetch_and_a_playing_one_at_most_five_a_second`), 5 a second while
the host plays. The `Pacer` decides:

| Case | Message |
| --- | --- |
| First look at a transport | one, with a new epoch |
| The play state changed | one, with a new epoch |
| The position jumped (a locate, a loop, a step of the mapping) | one, with a new epoch |
| The host plays | one every 40 ms (25 a second), the same epoch |
| The host is stopped and nothing changed | none |

A jump is a difference between the position of the message and the position that the last message
predicts, of more than 18 ms or 1.5 blocks (`frames / rate`), whichever is larger. The page slews
errors up to 20 ms and ignores larger ones, unless the epoch is new (then it anchors again), so 18
ms is the floor where the plugin asks for the hard anchor. A block of 1024 frames at 44.1 kHz is
23 ms: a host that calls ahead of the clock is no jump. A jump message goes out at most once every
40 ms; the newest position goes at the next look (an offline bounce gives at most 25 a second).
The unit test is `a_block_of_23_ms_is_no_jump_and_an_offline_bounce_gives_at_most_25_jumps_a_second`.

The measured rate while the host plays is 22 to 24 messages a second (stage
`editor-transport`, limit 15 to 40). While the host is stopped and unchanged it is 0.

## The mapping: song position to page time

The page shows source time (the time of the audio of the source), one playhead for all tracks.
The playback regions give the mapping, the same regions the notes use (the regions of the last
round, `round_regions`, else the live ones). A region has `start_in_source` (`start_mod`), a
length, and `start_in_song` (`start_play`).

- **Inside a region:** `page time = start_in_source + (song - start_in_song)`. When several
  regions cover the position (several sources at the same place, the usual drum session), the
  source that was made first wins, then the region with the earliest start (`total_cmp`, so a
  region before song 0 is right). The other sources of such a session have the same offset.
- **Outside every region:** the region that ended last before the position gives the offset.
  Before every region, the first region gives it. The playhead runs on in the source time of that
  region. The page cannot hold a playhead that plays (it extrapolates until the next message), so
  the plugin does not ask it to. The playhead stays at 0 where the offset gives less than 0
  (before the start of the page time), and the page stops it at the end of its longest track.
- **No region** (a source without playback regions): the song position is the page time.
- A region with a time stretch is not handled: its length in the song is taken as its length in
  the source. The plugin analyses the source, not the stretched audio.
- A step of the mapping (the song passes from one region to the next, which has another offset)
  is a jump: one message with a new epoch.

Where the sources do not sit at the same place of the song (the test host lays them back to back
by default), one playhead cannot be right for all of them. It follows the first source.

Stage `editor-transport` uses one region for two sources (start in the source 0.4 s, length
2.4 s, at 2.0 s of the song): the page time is the song position minus 1.6 s. It checks a
position inside the region, one after its end (page 2.9 s) and one before its start (page 0 s).
The fault run `DS_ARA_TRANSPORT_FAULT=identity` (no regions) makes it fail by 1.6 s.

## No transport

- **No process call at all** (a host that only uses ARA, or does not process the plugin): the slot
  stays empty. The long poll answers with no message. The page keeps its playhead (at 0 on a new
  page), shows the stopped state, and has no error row. Stage `editor-transport-none`.
- **Process calls without a transport** (`transport` is null, or has no seconds timeline): the
  same. Stage `editor-transport-none`, kind `null`.
- **A host that plays and stops calling `process`:** some hosts stop the calls of a plugin when
  the transport stops. A last call that said "playing" and is older than 300 ms, or three blocks
  when that is longer (a block of 16384 frames at 44.1 kHz is 372 ms), counts as a stop at the
  last position: one message "stopped". The report has the row `transport_stale` once. The
  next LUNA run shows whether LUNA does this.

What LUNA does is not known yet: the first run had no transport rows. The rows `transport_seen`,
`transport_page_first` and `transport_stale` of the next LUNA report give the answer.

## Risks

- **A host with a loop.** Each wrap is a jump: one message with a new epoch, then 25 a second.
  Stage `editor-transport-measure` plays a loop of 2.5 s.
- **Offline render or bounce** (faster than real time): the position moves faster than the wall
  clock, so most messages are jumps. The page anchors again each 40 ms. The playhead is correct
  to within 40 ms of the host but may look rough.
- **Latency.** The position is the one of the block that the host processes now. The sound comes
  out later by the output latency of the host. The page is ahead of the sound by that time. The
  plugin does not compensate.
- **Two documents in one host process** share the slot. The instance that writes first owns it.
- **A host whose calls come in bursts** (two blocks in one moment) makes errors of up to a block
  (about 10 ms). They are below the jump limit.
- **The page does not re-enter its follow branch** after a stop. In `static/transport.js`, the
  function `appOnPlayhead(m)` sets `S.playing = true`, starts `tick()` and shows the playing state
  only inside the branch `if (!AC.active)`. A stopped message (`hostMode() && !m.playing`) leaves
  `AC.active` true, so a later message with `playing: true` only slews `AC` and the page never
  shows playing. The smallest change: in that function, run the branch also when
  `hostMode() && m.playing && !S.playing` (or set `AC.active = false` in the stopped branch). The
  shell sets `AC.active = false` when the host starts to play and the page is not playing, and
  checks that `AC` and `appHostFollow` exist (stage `editor-shell-functions`). With the page
  change the line of the shell can go.
- **The slot is cleared or has no position while the page plays.** Many hosts stop by ending the
  process calls and calling `reset` or `deactivate`. The slot is then cleared, and the stale rule
  would never run. So when the slot is gone, or has no position, and the last message to the page said
  "playing", the page gets one message "stopped" at the last position it was given. Unit tests
  `a_cleared_slot_while_the_page_plays_sends_stopped_at_the_last_position_once` and
  `a_host_that_plays_with_no_position_stops_the_page_once`; stage `editor-transport-deactivate`
  (fault `DS_ARA_TRANSPORT_FAULT=keep-playing`).
- **A reload of the project** re-sends the last stopped position after 1.5 s, unless a newer
  message came in.

## What is measured

| Gate line | Stage | Fault run |
| --- | --- | --- |
| 1 source of the transport | `editor-transport` (part `-source`) | |
| 2 mapping | `editor-transport` (parts `-follow`, `-locate`) | `DS_ARA_TRANSPORT_FAULT=identity` |
| 3 hand-over | `transport-handover` | `DS_ARA_TRANSPORT_FAULT=alloc` |
| 4 follows, stops, jumps, also in the VST3 | `editor-transport` (parts `-follow`, `-stop`, `-locate`, `-jump`, `-rate`) | `DS_ARA_TRANSPORT_FAULT=off` ("playhead did not move"), `flood` |
| 5 no audio of the page | `editor-transport` (part `-own-audio`) | `DS_ARA_EDITOR_FAULT=page-transport` |
| 6 frame time | `editor-transport-measure` | `slow-frames`, `DS_ARA_TRANSPORT_FAULT=off` |
| 7 no transport | `editor-transport-none` | `DS_ARA_TRANSPORT_FAULT=invent` |

The test host plays a script (`--transport script`): stop at 2.5 s, play, stop, locate, play,
locate while playing, stop, locate before and after the region. Its process calls come every
10.7 ms (512 frames at 48 kHz) on absolute deadlines. The host writes its own truth on the wall
clock (the row `transport_script`). The page driver samples the position that the frame loop
draws, once a frame, with the same clock. The stage compares them.

The 50 ms limit is judged over the stable part of each play stretch: the first 0.5 s after a start
or a locate and the last 0.1 s before the next change are not counted (a loaded machine may be late
there). Result on the development Mac: the page is within 5 ms of the host while it plays (the largest
error over all samples of three play stretches; the three sample points are within 1 ms), a stop and a locate
show the exact position, and a locate of the playing host moves the page by 1.25 s at once.
The frame time with the host playing in a loop is 18 ms at the 95th percentile (limit 33 ms).
