# Explainer 9: Doubtful hits first

Chapter 9 of `docs/product/feature-path.md`. Narration for `tools/media/tts.py`: one `##` heading
per scene; lines that start with `>` are visual notes and are not spoken. Target length: about
90 seconds. The picture is the real output of the app on a generated two-track session (a snare
top and a snare bottom track): the hits, the confidence of each hit (`track_confidence`), the
queue order and the text of the info line are the app's answers, read at render time. The
narration calls it "a song", "your tracks", "its track".

Facts used (checked against the code at release 0.9.33, not against a commit; the sources are
`crates/app/src/confidence.rs`, `static/review.js` (the queue and the info line), `static/keymap.js`
(the keys) and `docs/guide/09-doubtful-hits.md`):
- Every hit gets a confidence `c` from 0 to 1 (`rate`). On the tracks of the picture (the
  analysis keeps no activations) it is the level of the hit against the median of the hits of its track in the output (`level_factor`):
  1 at the median or louder, 0 at `LEVEL_SPAN_DB` = 24 dB under it, linear between.
- A hit is doubtful when its confidence is under `DOUBT_BELOW` = 0.5 (`doubt`).
- A hit that you added or moved has confidence 1 (`edited by hand`), and a hit that is grey, out of
  the output, does not count (`not in the output`): `crates/app/src/confidence.rs`, `rate`.
- The rating is computed when the page asks; it is never stored in the hits, and the detection does
  not change: `crates/app/src/confidence.rs` (module doc).
- The queue holds the doubtful hits of all tracks, lowest confidence first, ties by time, then by
  track order (`rqQueue`); `D` (`review_next`) is Hits > Next Doubtful Hit and jumps to the first
  one and selects it, `Shift+D` (`review_prev`) steps back: `static/review.js`, `static/keymap.js`.
- The status line says `doubtful hit 1 of 6` (the place and the size of the queue),
  `no more doubtful hits` after the last one and `no doubtful hits` for an empty queue:
  `static/review.js` (`reviewStep`).
- The info line has a Doubt field: the confidence with two decimals, a colon, and the reasons
  joined by a semicolon (`rqWhy`), for example `0.40: soft gate: ghost note below the level gate;
  14 dB under the track median`. A hit that the level gate alone would have refused names the
  rescue that kept it:
  `soft gate: ghost note below the level gate`,
  `shape rescue below the level gate`,
  `weak flux, kept by the time-domain attack`
  (`crates/app/src/confidence.rs`, `rate`; `docs/guide/09-doubtful-hits.md`).
- `A` (`audition`) is Hits > Audition Hit and plays the selected hit on its track alone; `Delete`
  (`delete_hit`) removes a hit; `Left` (`nudge_earlier`) and `Right` (`nudge_later`) move it by the
  nudge value: `static/keymap.js`. An edited hit has confidence 1, so your edit decides.
- The review queue only points at hits. It never changes the detection, and a hit that you do not
  touch keeps its place in the exports: `docs/guide/09-doubtful-hits.md`.

## s01-problem

> Two snare tracks of one song, every hit a stalk on its track: more than a hundred hits.

A song has hundreds of hits. Checking every one is slow, and most are clear. Only a few are less sure. A very quiet hit, or a hit that a rule had to rescue. Those are the ones worth your ears.

## s02-confidence

> The same hits, the height of each stalk is its confidence. A dashed line at 0.5; the stalks under it turn orange. Two labels: 1 as loud as the typical hit, 0 at 24 dB under it.

Drum Spike gives every hit a confidence, from its level against the typical level of its track. A hit as loud as the typical one is fully sure. The quieter, the less sure, down to zero at 24 decibels under. Under one half, a hit is doubtful.

## s03-queue

> The doubtful hits are numbered in the order of the queue, across both tracks. A playhead jumps from one to the next; the status line changes from "doubtful hit 1 of 6" to "doubtful hit 2 of 6".

Press D for Next Doubtful Hit. The queue holds the doubtful hits of all your tracks, lowest confidence first. The playhead jumps to the first one. Press D again.

## s04-why

> The first doubtful hit zoomed to 80 ms, its waveform, and the info line with the Doubt field as the app writes it.

The info line says why a hit is doubtful. It gives the confidence, how far the hit sits under its track, and which rule rescued it. That tells you what to check.

## s05-decide

> The same hit with four short rows: A audition, keep, Delete, Left and Right to nudge; then the line "your edit decides".

Press A to audition the hit. A real hit stays as it is. If it is not real, delete it, or nudge it with the arrow keys. Once you edit a hit, your edit decides. The queue only points. It never changes a hit.

## s06-result

> Both tracks again, every hit as before; the few doubtful ones ringed, with the count; the word Export.

You check a short list, not the whole song. When the queue is empty, the status line says so, and you export. Every hit you did not touch keeps its place in the export.
