# M1b4: analysis starts when the audio is here

Ticket #545, umbrella #503 (the DAW plugin program), part of M1 (#510). Follows M1b1 (#537:
one service and one project for each ARA document) and M1b2 (#538: the editor window). The
plugin lives in `plugins/ara/`. Nothing in `static/` or `crates/` changed.

## The finding from LUNA

The first run of the plugin in a real DAW (LUNA, 2026-11-20) showed a gap. LUNA loads the
plugin, opens the editor and gives the audio of the source: the report has the `read` row with
`read_ok` true. But LUNA never calls `requestAudioSourceContentAnalysis`. The report has no
`request` row and `rounds` is 0. The page shows "No tracks yet" for ever.

Until M1b4 the plugin started a round only on that call. The ARA test hosts of the check
(`ara-host` and the `ARATestHost` of the SDK) always send the request, so no stage saw it.

## The rule

**Analysis starts when the audio is here.** The host may ask, and then the answer is the same,
but the plugin does not wait for the question.

- The host enables the samples access of a source. The plugin queues the source at once (message
  `Reading`) if it has no result yet. A round waits for the audio of every queued source of the
  document, with the audio deadline of M1b1 (30 s). So several sources of one document, whose
  audio comes one after the other, are one round, not one round for each source.
- The audio of the source is here (message `Ready`). A source that has no result and is not
  queued yet is queued now. This covers a source whose `Reading` was missed.
- A source that has its result is never queued by a read. This holds for a source restored from
  the archive: the plugin reads its audio again if the host asks for it, but starts no round.
  A restored source that is marked stale (its last round had an error) has no good result and is
  analysed, as before.
- A request of the host for a source that is queued or complete changes nothing. There is no
  second round for the same audio, in either order ("read, then request" and "request, then
  read").

## Role models

We looked at three ARA plugins that run in LUNA. Auto-Align 2 and DynAssist declare no
analyzable content types at all. Melodyne declares several. All three analyse on their own when
the audio is readable. A host need not send an analysis request. This is the reason for the rule
of this ticket: a plugin that waits for the request works in our test hosts and not in LUNA.

## What the host sees

- `isAudioSourceContentAnalysisIncomplete` is true from the start of the read until the source
  has a result, or its analysis ended without one. It did not depend on a request before, and
  it does not now.
- The content readers give the notes after the round. Before it they give none.
- A host that never asked does not wait for the notes, so it would not read them. The plugin
  tells it: `notifyAudioSourceContentChanged` is sent for such a source when its first result
  is here. As before, it is sent inside `notifyModelUpdates`, never from the worker thread. A
  host that did ask gets the same calls as before.

## New audio: the content update rule

The host may change the samples of a source (the user records again, or the host replaces the
audio). It tells the plugin with `updateAudioSourceContent`. When the flag "signal scope remains
unchanged" is NOT set (no flags at all means everything changed), the samples are new:

- The audio of the source gets a new generation. Each generation has its own WAV file
  (`...g1.wav`, `...g2.wav`). The old file stays until the new one is committed, then it is
  deleted. A running read of the old samples is stopped first.
- If the host's samples access is on, the plugin reads the new samples at once. Else it reads
  them at the next enable.
- The source needs a round although it has a result. Until that round ends,
  `isAudioSourceContentAnalysisIncomplete` is true for it, so a host that waits sees the new
  result. The host is told with `notifyAudioSourceContentChanged` when the notes differ.
- The round keeps the hits the user placed by hand (the rule of #538: added, moved and edited
  hits stay and win over detected hits within 20 ms). The hits that the detection placed are made
  again from the new audio. Other tracks are analysed again together with it, as in every round.
- A round that runs before the new samples are read (another source arrives, a retry, a region
  change) analyses this source on the OLD audio. It does NOT end the new analysis: the mark stays
  until a round has analysed the audio of the live generation. The snapshot of a source reads its
  WAV path, its ready mark and its generation under one lock, and the update changes them under
  the same lock, so a round never sees the old path with the new generation.
- If the read of the new audio fails, the source is complete without a new result for the host
  (`analysis_failed`), the report row `source_read_failed` names it, and the plugin reads again
  while the host's samples access is on: three tries, after the retry waits below. The reader is
  made on the host's main thread, so the tries run inside `notifyModelUpdates`.
- The files of the source go away: the old generation when the new one is committed (also the file
  an earlier process left for a restored source), all of them when the source is destroyed.
- A hand-placed hit beyond the end of shorter new audio stays in the project, but it is no note
  for the host (a note past the end of the source would be wrong).
- A change of the properties only (name, rate, count) follows the rules of before.

Stages `no-request-update` (fault `ignore-update`), `no-request-update-between` (`clear-stale`), `no-request-update-failed-read` (`no-read-retry`) and `no-request-files` (`keep-wavs`); a unit test proves that a hand edit survives a content update.

## Retry rule

A failed round is not left until the DAW restarts. The worker tries again by itself, at most three
times, after 2 s, 4 s and 8 s (`DS_ARA_RETRY_MS` sets the first wait for the check; it is kept between 10 ms and 60 s). After the third
retry it writes the row `retry_gave_up` and stops. Anything that changes for the document starts
the count again: a new source, new audio, a region change, an end of a job of the page, a restore,
a request of the host. A request still tries at once. A track that fails for ever (a damaged
file) costs three more rounds, then the plugin leaves it.

Stages `no-request-retry` and `no-request-retry-limit` (faults `no-retry` and `retry-forever`).

## A read that is cancelled or fails

- A round for audio that did not come must not run. A source whose read was cancelled (the host
  turned the samples access off) or failed has no new audio, so it does not force a round of the
  complete tracks. The service sees no change and does nothing. Stage `no-request-cancel`
  (fault `force-failed`).
- When the reader thread cannot start, the plugin says so at once (`Failed`). The source is not
  named late and does not wait for the audio deadline. Stage `no-request-spawn` (fault
  `no-spawn-failed`).
- A source that was queued by its read and is then restored with its result leaves the queue
  and is not analysed. A restored source that is marked stale is analysed when its audio is read.
  Both have unit tests.

## The editor

The editor opens before the audio is here, and the project of the document is empty. When the
first round ends, the plugin sends the event `plugin-reload` and the page loads the tracks. This
is the path of M1b2. It works without a request, because the round does not need one.

## How it is checked

The test host `ara-host` has these options (see the header of `src/bin/ara-host.rs`):

- `--no-request`: the host enables the samples access and never sends the request.
- `--request-order read-first|request-first`: the request comes after the audio was read and
  analysed, or 300 ms before the audio.
- `--slow-read <k>:<ms>`: the audio of source k comes later than the audio of the others.
- `restore ... --enable-access`: the host reads the audio of restored sources.

The new stages and their fault runs (the switch is `DS_ARA_ROUND_FAULT`, listed in
`switches.json`):

| Stage | What it proves | Fault run |
| --- | --- | --- |
| `no-request` | every source has a round, one round, the events equal the reference of the run with requests, no `request` row, the notes are read and the host is told | `request-only`: no round |
| `request-order` | a host that requests gets one round and the same events, in both orders | `always`: a second round after the audio |
| `no-request-group` | three sources, the audio of the last one late: one round, three tracks | `ready-only`: a round for each source |
| `no-request-restored` | restored sources with their result start no round when their audio is read | `read-again`: a round starts |
| `editor-no-request` | the page gets the tracks without a reload by hand | `request-only`: no round |

The C++ `ARATestHost` of the SDK is not changed. It always requests, so it keeps the "host that
requests" case in the `conformance` stage.

## Risk for hosts that do request

- A round starts when the audio of all queued sources is here. A host that enables the access of
  many sources at once, and requests later, may now get its round a little earlier than before.
  The result is the same.
- A host that enables the samples access of a source but wants no analysis of it still gets one.
  The plugin treats the read as the wish. This is the point of M1b4.

## The plugin state

LUNA asks every plugin for its state when it makes an instance. It logged "loadPreset failed"
for our plugin, because the plugin had no CLAP state extension (the VST3 wrapper maps `getState`
and `setState` to it). The plugin now has `clap.state`.

- Save writes a small blob: a magic (`DSARA` and a zero byte) and a version number (1, four
  bytes). It holds no project data. The ARA archive holds the data.
- Load accepts its own blob, an empty stream, a blob of a newer version and anything else. It
  never fails the creation of an instance. It reads at most 64 KiB.
- The stage `state` runs the test host on the CLAP: save, load, and the three odd streams. The
  fault run `DS_ARA_STATE_FAULT=strict` refuses them and the stage fails. No tool in `.deps`
  calls `getState` and `setState` on the VST3 alone, so the two functions also have a unit test,
  and clap-validator runs its own state tests on the CLAP in the stage `validate`.

## Evidence rows for the next LUNA run

These rows change no behaviour. They tell us what a host sends.

- `view_selection`: the counts of playback regions and region sequences in a selection, and
  whether the host gave a time range. The first call, then at most one row a second.
- `view_hide_sequences`: the count of hidden region sequences, same rate limit.
- `process_first`: the sample rate and the block size of the first process call.
- `transport_seen`: the flags of the first process call that carries a transport (seconds
  timeline, beats timeline, tempo, playing) and its position in seconds.

The audio thread writes no row. It sets a few atomics. The worker thread looks at them once a
second and writes the two rows (the worker wakes up each second for this). The editor view
callbacks run on the host's main thread: they only keep the row, and the worker writes the report
file. A row that the rate limit holds back is not lost: the newest one waits and the worker writes
it when the second is over. Without a report file the plugin keeps only the last 512 rows. Unit tests check the
rate limit and that the writer of the rows is not the thread that ran `process`.
