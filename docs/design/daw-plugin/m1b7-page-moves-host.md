# M1b7: the page moves the host

Ticket #554, umbrella #503 (the DAW plugin program), part of M1 (#510). Follows M1b5 (#546: the
page follows the transport of the host). This part is the other direction. The plugin lives in
`plugins/ara/`. Nothing in `static/` or `crates/` changed.

## What it does

Play, stop and locate in the page move the transport of the host, as the ARA editors of other
plugins do. Before this part, the Play button was refused ("the host plays: start playback in the
DAW"), and a click in the ruler moved only the playhead of the page.

The way of a request:

1. The page calls `B.hostTransport.play`, `stop` or `locate` (the interface of #518).
2. The shell script (`shell.js`) calls a shell command of the plugin: `plugin_host_play`,
   `plugin_host_stop` or `plugin_host_locate`. They are not commands of the app. The editor
   classes them as shell commands, like `plugin_report`.
3. A request thread decides what to call (`playback::plan`, no host call yet).
4. The request thread puts the calls in the queue of host requests of the editor and waits at most
   2 s (see "Order, floods and a closed editor").
5. The main thread calls the ARA playback controller of the host.
6. The host moves. Its position comes back through the transport follow of M1b5. That moves the page.

## The ARA rule for the thread and the state

The playback controller is an optional host interface. The header `ARAInterface.h` says:

- "This optional host interface allows the plug-in to request playback state changes."
- "The functions in this interface may be called concurrently, but not from render-threads."
- "The host may choose to ignore any of these requests."
- In the host instance: "OPTIONAL INTERFACE: plug-in must check playbackControllerInterface is not NULL before calling!"
- "the function pointers in this struct must remain valid until all document controllers on the
  plug-in side that use it have been destroyed."

The header does not name the main thread for this interface. It says only "not from
render-threads". The plugin takes the stricter rule, for two reasons:

- The ARA test host of the SDK examples (`ARAPlaybackController.cpp`) runs
  `ARA_VALIDATE_API_THREAD (_araDocumentController->wasCreatedOnCurrentThread ())` in every call of
  its playback controller and of its other host interfaces. A host built like it refuses a call
  from another thread.
- The example plug-in (`ARATestDocumentController.cpp`) sends its playback requests through
  `dispatch_async (dispatch_get_main_queue (), ...)`, the thread of the document controller.

So the plugin calls the host on the main thread, the thread of the document controller. The
request thread of the editor never calls it. The state rules:

- **Presence.** The plugin reads the interface once, in `createDocumentController`
  (`playback::probe`). The struct of the host instance must be long enough to hold the field. The
  pointer must not be null. Each function must lie inside the `structSize` of the interface and
  must not be null. A function that fails this is "not offered". The row `playback_offer` of the
  report says what the host gave.
- **Lifetime.** The plugin keeps the interface in its document, under a lock. `destroyDocumentController`
  takes it away under the same lock, on the same thread that makes the calls. A request that comes
  after that finds nothing and the page is told "the host took no request".
- **Render threads.** The audio thread is not touched by this part. No call to the host runs there.
- **Ignored requests.** The host may ignore a request or run it late. The plugin does not assume that
  it did the work. The page follows only what the host reports (see "The page does not move itself").

The stage `editor-playback-thread` checks it from two sides. The test host records the thread of
every call (`pthread_main_np`). The plugin counts its calls by thread (`editor_close` row). The fault
run `DS_ARA_PLAYBACK_FAULT=direct` makes the request thread call the host. Both checks fail with
"off the main thread".

## Order, floods and a closed editor

Each request of the page runs on its own thread. Without a rule, a drag could send locate A, B, C
to the host as A, C, B, and a stop could pass a locate. So:

- **One queue per editor** (`playback::Queue`). The scheme handler runs on the main thread, so it
  gives each request a sequence number in the order of arrival. The queue keeps that order. One
  drain job on the main thread runs the whole queue.
- **Newest locate wins.** A locate that has not run and has a later locate in the queue is
  dropped. The page gets `{ok: false, superseded: true}`. That is no error, and the shell does
  nothing for it. Play, stop and the last locate keep their order. A request that is older than one
  that already ran is dropped the same way.
- **A small limit.** At most 8 requests wait. The next one is answered at once ("too many
  requests wait for the host").
- **Not the budget of the app commands.** A host request waits up to 2 s for the main thread.
  If it used one of the 96 slots of `MAX_INFLIGHT`, a blocked main thread (menu tracking, a modal
  dialog) and the 25 ms watch would fill them and every other command would get 503. The routes
  `plugin_host_*` are not counted (`counts_inflight`); the queue limit bounds them.
- **A closed editor.** The drain job checks `closed` before each call. A request that was queued
  before `detach` makes no call and the answer is "the editor is closed". A request whose thread
  stopped waiting (2 s) is marked late and does nothing.

Tests: `playback::tests::requests_run_in_the_order_of_arrival_and_the_newest_locate_wins`,
`..::the_queue_holds_a_few_requests_and_the_drain_job_starts_once`,
`..::a_request_queued_before_the_editor_closed_makes_no_call` and
`editor::tests::requests_to_the_host_do_not_use_the_budget_of_the_app_commands` (stage
`playback-map`). Fault `DS_ARA_PLAYBACK_FAULT=no-supersede` makes the first one fail with "the
newest locate wins".

## The mapping: page time to song position

The page shows source time. The host wants song time. The mapping is the inverse of the follow of
M1b5, with the regions of the last round (`transport::song_time`, next to `page_time`):

`song = start_in_song + (page time - start_in_source)`, for one region.

A source can have several regions, and several sources can lie at different places. One rule picks
the region:

1. A region that holds the page time in its source and also holds the host position in the song.
2. Else the first region that holds the page time in its source.
3. Else (the time is in no region) the region that holds the host position gives the offset.
4. Else the first region.

"First" is the lowest source index, then the earliest start in the song. The song position is
never below 0.

Example: one source, two regions of the same audio, at song 0 s to 2 s and at song 10 s to 12 s.
The host stands at 11 s. A locate to page time 1 s goes to 11 s. The host stands at 5 s or has no
position: it goes to 1 s.

### The selected track, and bounds

- **The track of the page.** Stems of one length hold the same source time in several sources. The
  shell sends the selected track (`S.selTrack`) with each locate and play. The plugin uses the
  regions of the sources of that track (`playback::regions_for`). With no selected track, or a
  track with no region, it uses all regions and the rule above (the lowest source index first).
- **What the playhead shows.** The follow of M1b5 has no track: it maps a song position with the
  first source. `page_time` and `song_time` are inverse to each other for the first source, and for
  any track that has the same offset (the usual drum session). For a track with another offset, a
  locate puts the host at the song position of that track. The follow then shows the first source's
  page time for that position, so the playhead can move to another place after the host answers. This is
  the limit of one playhead for all tracks.
- **Bounds.** A time that is not a number, or is above 24 h, is refused ("that time is out of
  range"). A time below 0, or above the length of the source (the longest source of the selected
  track), is clamped before it is mapped (`playback::bound_time`).

**No region** (no round yet, or a source without a playback region): nothing is sent. The page shows
"no playback region maps this time yet: the host cannot be moved". The follow of M1b5 treats "no
region" as "song time is page time". A locate does not, because a wrong position in the host is
worse than no move.

Unit tests (stage `playback-map`): inside a region, a source with two regions, no region, a time
before the start of the region (the offset still holds; below 0 stops at 0), and the plan of each
request. The fault run `DS_ARA_PLAYBACK_FAULT=identity` sends the page time as song position and the
test of the plan fails with "off by".

## Play

`play(pos)` asks the host to start. One deadline of 2 s counts for the whole chain, from the call of
the page: the wait for the main thread, the call, and the wait for the host to say it plays. A click in the ruler and then Play is the usual order, so when
`pos` maps to a song position that is more than 5 ms from the host position, and the host stands
still, the plugin first sets the position and then starts. The page time of the position is a time of
the source; the host starts from the song position that the mapping gives.

The page shows "playing" only when the host says so. The promise of `play` ends with the first
message that says playing. If none comes in 2 s, it fails with "the host did not start
playback" and the page shows its usual "playback failed" text. The host may still start later. Then
the follow of #546 shows playing (the page is not in its play any more), and the shell takes the
failure text away. Stage `editor-playback-late` (the test host starts 2.5 s late; fault
`DS_ARA_PLAYBACK_FAULT=keep-error`: the text stays).

## What the page does itself (found in `static/`, not changed)

- **Play.** `appPlay` calls `host.play(pos0, speed)` and waits for it. After that it sets its
  own playing state and starts its clock (`AC.running = true`) until the first position of the host
  anchors it. The shell makes the promise wait for the host, so the page does not show a play that
  the host did not start. Speed is not sent (ARA has no speed).
- **Stop.** `appPause` sets the stopped state at once (it does not wait), sets `S.playhead` to its
  own estimate and then calls `host.stop(p)`. The message of the host then re-anchors the page at the
  host position. The page moves first here and cannot be told to wait.
- **Locate while the host plays.** The click sets the playhead of the page (`PH.v`, at once). The
  next frame of `tick()` sends it to `appSeekNow`, at most every 60 ms, which calls `host.locate`
  and does not wait for it. The page shows its own click until the first host message comes.
- **Locate while the host is stopped.** The page never calls `host.locate`. `appTickPos` runs only
  in `tick()`, and `tick()` runs only while the page plays. A click in the ruler of a stopped page
  sets `PH.v` and `PH.user` and nothing else. The shell script has a watch for this: every 25 ms, when the page is
  not playing and `PH.user` is set and `PH.v` is not a value that the host or the page's own stop
  put there, the shell calls `locate`. This is the one place where the shell reads page state; the
  names are listed in `PAGE_GLOBALS` of the shell, and the stage `editor-shell-functions` checks them.
- **A host that does not follow.** The page does not fall back by itself. The shell does it: after
  a request that the host did not follow (no message of the host in 1 s, or a refusal), the shell
  shows the last host position again, if the host was stopped. A host that plays sends its
  position anyway. The fallback does not run when a newer locate was sent after the one it belongs
  to, and not while the pointer is down in the ruler (it asks again every 150 ms until the pointer is
  up). Stage `editor-playback-drag` (a drag of 3 s against a host that does nothing; fault
  `no-drag-guard`: the playhead jumps back under the pointer). Stage `editor-playback-refused`, fault `DS_ARA_PLAYBACK_FAULT=no-fallback`.

So the page does move its own playhead at a click (`PH.v`) and at a stop, before the host answers.
The gate line "the page never moves its own playhead" holds in this form: the position that stays
is the host's. The test host snaps a position to a grid (`--snap 0.5`), so the click and the answer
differ and the stage can tell them apart. The page shows its click for the time of the loop, then the
host position.

### Need in `static/` (one paragraph, for a later ticket)

In host mode `static/transport.js` should call `B.hostTransport.locate(PH.v)` itself when the user
moves the playhead while the page is stopped (in `appTickPos`, or in the setter of `S.playhead`), and
should not move `PH.v` in host mode before the host answers: it should keep the shown position until
a host message arrives, or restore the last host position when `locate` or `stop` resolves with a
refusal. Then the watch in the shell (`setInterval` in `shell.js`) and the "show the host position
again" fallback can go.

## No controller, or a call that is not offered

The plugin reads the offer once per document (`playback_offer`). When the host gives no interface, or
an interface without the call:

- The Play button is disabled, with the text as its title. The ruler gets the text as its title.
- A click, Play or Stop shows the text in the status line. No request goes out.
- The text is one short sentence that names the host (`clap_host.name`, copied at `create`):
  "<host> gives plugins no control of play, stop or locate", or "<host> does not offer <the call> to plugins".
- The answer of the plugin is HTTP 200 with `{ok: false, reason}`. No error row is written
  (`command_errors` stays 0).

Stages: `editor-playback-none` (no interface at all) and `editor-playback-partial` (the interface
struct ends before `requestSetPlaybackPosition`: start and stop work, locate is refused).

## Report rows

For the next run in a DAW (LUNA) to show what it offers:

| Row | When |
| --- | --- |
| `playback_offer` | at the creation of the document controller: the sizes of the struct and of the interface, and `request_start`, `request_stop`, `request_set_position` |
| `playback_request` | the first request of each kind (play, stop, locate): the page time, the song position in seconds and samples, the rule of the mapping, the host position, the calls made, the thread |
| `playback_unavailable` | the first refused request of each kind: the offer, the number of regions, the text |

## The test host

`ara-host <bundle> editor --editor-mode playback --transport follow --playback full|ignore|partial [--snap <s>]`.

- `full`: the three calls (and the two cycle calls, which do nothing). The host records each request
  (kind, seconds, samples, wall time, the song position it landed on, the thread) and moves its
  transport: start plays, stop stops, a position goes to the grid. The row `playback_record` is
  printed at the end.
- `ignore`: the same record, and the transport does not move.
- `partial`: the interface struct ends before `requestSetPlaybackPosition`.
- No `--playback`: the host gives no controller (the pointer is null).

The process thread (`--transport follow`) reads the transport of the host every block, so the
position that the plugin reads in `process` is the one the controller set.

## What is measured

| Gate line | Stage | Fault run |
| --- | --- | --- |
| 1 calls to the host, on the main thread | `editor-playback` (parts `-play`, `-stop`, `-thread`) | `DS_ARA_PLAYBACK_FAULT=skip` ("no play request"), `direct` ("off the main thread") |
| 2 mapping, one rule for several regions | `playback-map`, `editor-playback` (part `-locate`) | `DS_ARA_PLAYBACK_FAULT=identity` ("off by") |
| 3 the loop page, host, page within 100 ms | `editor-playback` (part `-loop`) | `DS_ARA_TRANSPORT_FAULT=off` ("did not come back") |
| 4 the test host records and moves; CLAP and VST3 | `editor-playback` | as 1 to 3 |
| 5 no controller, or no call | `editor-playback-none`, `editor-playback-partial` | `error-row` ("an error row was written"), `anonymous` ("does not name the host") |
| 6 no audio thread, no own audio | `editor-playback` (part `-own-audio`, and `editor-transport-own-audio` of M1b5) | `DS_ARA_EDITOR_FAULT=page-transport` ("transport command") |
| 7 report rows | `editor-playback` (part `-report`), `editor-playback-none` (part `-report`) | |
| a host that ignores the requests | `editor-playback-refused` | `no-fallback` ("stayed") |
| no fallback in a drag | `editor-playback-drag` | `no-drag-guard` ("jumped back under the pointer") |
| a host that starts late | `editor-playback-late` | `keep-error` ("stuck") |
| order, newest locate, closed editor, budget, bounds, selected track | `playback-map` | `no-supersede` ("the newest locate wins") |

The limit of 100 ms is the number of the ticket. A miss may be the load of the machine or a defect.
On a miss the stage `editor-playback` measures once more and prints both results with the load
average. The loop fails only if both runs miss.

Each stage fails when its run did not happen: a run with no page report, a hidden page or no record of
the host fails every part with "no usable run" (`editor-playback-fault-norun` and its siblings).

### Measured

The test host calls its process every 10.7 ms (512 frames at 48 kHz). Its transport grid is 0.5 s.
Result on the development Mac, CLAP and plugin in the VST3 alike:

- **Locate, stopped:** the page shows the host position 12 to 21 ms after the shell calls `locate`
  (18 to 32 ms after the click in the ruler: the watch of the shell waits up to 25 ms).
- **Locate, playing:** 17 to 47 ms after the call. The page shows its own click for 0 to 2 frames.
- **Play:** the page shows playing 36 to 49 ms after the call.
- **Stop:** the page stands at the host position 30 to 34 ms after the call.
- **Limit:** 100 ms. The calls reach the host on its main thread (5 of 5), and the host records the
  song position of every locate with 0 samples difference.

## Not done

- Cycle (loop) and tempo: not in scope. `requestSetCycleRange` and `requestEnableCycle` are not used.
- Key commands of the host (the space bar in the plugin window): not in scope.
- Speed: the page's half speed play is a normal play for the host.
- A region with a time stretch: as in M1b5, its length in the song is its length in the source.
- LUNA: what it offers is not known yet. The rows `playback_offer`, `playback_request` and
  `playback_unavailable` of the next LUNA run will show it.
