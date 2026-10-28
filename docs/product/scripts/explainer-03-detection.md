# Explainer 3: How Drum Spike hears a hit

Chapter 3 of `docs/product/feature-path.md`. Narration for `tools/media/tts.py`: one `##` heading
per scene; the text under it is spoken. Visual notes are in the `>` lines (not spoken). Target
length: about 90 seconds. The data is real: the demo session (`tools/media/demo_session.py`),
its snare close mic `s top`, and the core's own detector output (`crates/core/src/detect/onset.rs`)
read through `target/release/parity` (the log spectrogram with `features`, the spectral flux with
`onsets --odf-out`, the kept hits from the `onsets` JSON).

Facts used (all shown from the core's output on the demo session): loudness alone fails because
bleed and decays are loud too; the spectrogram shows the hits as stripes; the spectral flux counts
only the rises in the drum's own band, for the snare 100 to 300 hertz; the peaks of the flux are
the candidate hits, at least 20 milliseconds apart; the gates keep the candidates that are loud
enough against the track's own reference level and whose shape fits the template of the track's
loud hits, and the tom mic's gates drop the kick's copy and the snare's copy (only the tom hit
survives the kit check on the t2 track); hand-off to explainer 4
(where exactly does the hit start).

## s01-loudness

> The demo session's snare close mic (s top) around the kit check; a level meter curve under it; the snare hit and the kick's copy both push the meter up.

A level meter only knows loud. On the demo session's snare mic, the hits are loud. But the kick bleeds into the same mic, and its copy is loud too. And a hit stays loud while it decays. Loudness alone cannot find a hit.

## s02-spectrogram

> The detector's own log spectrogram of the demo snare track: time left to right, frequency bottom to top; the hits stand up as stripes.

So Drum Spike reads the spectrum over time. This is the detector's own spectrogram of the demo snare track. Each stripe is one hit: energy rising in many bands at once.

## s03-flux

> The spectral flux curve of the snare band under the spectrogram; a decay region marked: falling energy adds nothing.

The detector measures spectral flux: how much energy rises from one frame to the next, in the drum's own band. A rise counts. A decay does not: falling energy adds nothing. This curve is the detector's own output.

## s04-peaks

> The flux curve alone; the peaks marked as candidates in grey; the kept hits in orange; the text 20 ms between two close peaks.

The peaks of the flux are the candidate hits. A peak counts when it stands over the moving median, and two hits sit at least 20 milliseconds apart.

## s05-gates

> The demo session's tom mic (t2) around the kit check, 1.6 s wide: the tom hit carries an orange hit mark; the kick's copy and the snare's copy carry none; the text: loud enough for the tom's reference level, and a shape that fits the tom template.

Then the gates, on the demo session's tom mic. A candidate must be loud enough against the track's own reference level, and its shape must fit the template of the track's loud hits. The tom hit passes. The kick's copy and the snare's copy have the wrong shape. No hit.

## s06-handoff

> The whole demo snare track with the kept hits as orange stalks; the text: now, where exactly does the hit start?

What is left are the real hits of this track. Now one question is open: where exactly does a hit start? The next video walks to the first sample.
