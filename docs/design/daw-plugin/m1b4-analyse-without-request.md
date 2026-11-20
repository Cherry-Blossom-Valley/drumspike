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

## What the host sees

- `isAudioSourceContentAnalysisIncomplete` is true from the start of the read until the source
  has a result, or its analysis ended without one. It did not depend on a request before, and
  it does not now.
- The content readers give the notes after the round. Before it they give none.
- A host that never asked does not wait for the notes, so it would not read them. The plugin
  tells it: `notifyAudioSourceContentChanged` is sent for such a source when its first result
  is here. As before, it is sent inside `notifyModelUpdates`, never from the worker thread. A
  host that did ask gets the same calls as before.

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
