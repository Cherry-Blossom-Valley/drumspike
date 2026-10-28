# Explainer 5: The body of the hit

Chapter 5 of `docs/product/feature-path.md`. Narration for `tools/media/tts.py`: one `##` heading
per scene; lines that start with `>` are visual notes and are not spoken. Target length: about
60 seconds. The data is real: a kick hit and a snare hit from the demo session
(`tools/media/demo_session.py`), the snippet lengths from `tools/media/explainers/audio_spike.py`,
which mirrors `crates/core/src/spike.rs`.

Facts used: the Audio spike copies the hit's own audio from the onset over its body. The low
component (below 800 Hz, two cascaded lowpass stages, forward and backward) is smoothed. The
snippet runs over the first three clear swings of that low tone. A swing counts when its peak
reaches 25 percent of the largest of the first three. The snippet ends on a zero crossing, at
most 60 ms. Typical lengths: a kick about 15 to 25 ms, a snare about 5 to 10 ms; the demo kick
of the s04 scene measures 8.9 ms (the value the rule gives on that hit, not a typed number).

## s01-not-a-click

> The export dialog's spike types; the Audio row lights up; a click spike (one thin impulse) next to the word Audio.

The export can write an Audio spike. The name sounds like a click. It is not a click. It is a short piece of your own track.

## s02-the-body

> The demo session's kick close mic, the whole hit: the silence before the onset, the attack, the decay; the raw waveform in cobalt, the smoothed low tone in orange over it, the snippet range shaded.

What does the snippet hold? The body of the hit. Drum Spike low-passes the track below 800 hertz. The attack click falls away, the low tone stays.

## s03-count-the-swings

> Zoomed into the snippet: the smoothed low tone alone, its swings marked 1, 2, 3 in turn, one number on its own half-wave.

Then Drum Spike counts the swings of that low tone. One, two, three. A swing only counts when it reaches a quarter of the largest of the first three. Small wiggles do not count.

## s03b-too-small

> A constructed illustration, labelled on screen: a low tone with three counted swings; the band at 25 percent of P over it; between swing 1 and swing 2 a small dip that stays inside the band, marked too small, does not count; the count runs 1, 2, 3 on past the dip.

A dip under a quarter of the largest does not count. It does not extend the swing before it either. The snippet stops at the third full swing, not at the dip.

## s04-the-cut

> The whole hit again; a marker drops onto the zero crossing that ends swing 3, at the edge of the shaded snippet; the text 60 ms appears, then this demo kick 8.9 ms (the measured length), next to kick 15 to 25 ms, snare 5 to 10 ms.

The snippet ends on a zero crossing, at most 60 milliseconds. A kick fills about 15 to 25. A snare fills about 5 to 10.

## s05-real-hits

> A kick hit and a snare hit from the demo session, each the whole hit (silence before the onset, attack, decay), raw with its snippet range shaded.

Two real hits from the demo session. The kick breathes longer, the snare is short and crisp. Each snippet covers its own body, nothing more.

## s06-click-versus-audio

> A DAW lane: the click spike, one thin impulse, against the audio spike, the real waveform body; both aligned at the hit.

Next to the click spike, the audio spike triggers your sampler with the hit's own attack. The phase lines up, and the body carries the punch. That is the Audio spike.
