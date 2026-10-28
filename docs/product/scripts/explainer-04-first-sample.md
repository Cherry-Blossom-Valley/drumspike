# Explainer 4: Finding the first sample

Chapter 4 of `docs/product/feature-path.md`. Narration for `tools/media/tts.py`: one `##` heading
per scene; lines that start with `>` are visual notes and are not spoken. Target length: about
2.5 minutes. Use one real snare hit and one real soft (ghost) hit from the demo session.

Facts used (from `crates/core/src/refine.rs`, version 3 "first motion", the shipped rule): the
detector's time is a coarse time a few milliseconds before the attack; the search finds the attack
by the steepest rise of the smoothed envelope; the back-track takes the attack's peak in the next
5 ms; its threshold is 33 dB under that peak, but at least 6 dB over the noise floor (the median
of 1 ms block maxima in the 50 ms before the hit); the walk goes back while the signal stays over
the threshold, passes zero crossings, and stops at a gap of 0.5 ms under the threshold; it never
goes back more than 10 ms or past the middle to the previous hit. Kick tracks use 29 dB instead of
33 dB, because a kick's peak comes 6 to 11 ms after its start.

## s01-recap

> The snare close mic, zoomed to about 50 ms; the comb-filter picture from explainer 1 shrinks into a corner.

In the first video we saw why a trigger must fire on the exact first sample of a hit. Now we look at how Drum Spike finds that sample. We take one snare hit and zoom in.

## s02-coarse

> A wide orange band over the waveform: the detector's coarse time, a few milliseconds before the hit.

First, Drum Spike listens to the track for onsets. It says: a snare hit is near here. The gates and the other mics tell a snare from bleed, but this time is coarse. It is a few milliseconds early or late. For a trigger, that is far too much.

## s03-envelope

> The waveform fades; its envelope (a smooth line over the peaks) is drawn; the steepest rise lights up.

So Drum Spike looks at the waveform itself. It draws the envelope: a smooth line over the loudness. Where the envelope rises most steeply, the attack is. That is the anchor of the hit.

## s04-peak-threshold

> Zoom to 5 ms. The attack's peak gets a marker; a horizontal threshold line drops 33 dB under it; the noise before the hit is shaded, with its own line 6 dB above it.

Now the precise part. Drum Spike takes the peak of the attack. It sets a threshold far under that peak: 33 decibels lower. But the threshold must also stay over the noise and the bleed before the hit. So it is at least 6 decibels over that noise floor.

## s05-walk-back

> A marker walks from the peak to the left, sample by sample (dots visible at 48 kHz); it passes a zero crossing; it stops where the signal stays under the threshold for 0.5 ms.

Then it walks back from the peak, sample by sample. While the signal stays over the threshold, the walk goes on. A zero crossing does not stop it, because the wave passes zero many times inside one hit. The walk stops where the signal stays quiet for half a millisecond. That is the first motion of the attack. The hit goes on this sample.

## s06-soft-hit

> The same steps on a ghost note: lower peak, lower threshold line, the walk ends at its first motion; both hits side by side, each marker on its own first sample.

A soft ghost note works the same way. Its peak is lower, so its threshold is lower too. The walk still ends on the first motion. This is why soft and loud hits get the same point: the start of the attack, not a level.

## s07-safety

> Two short notes appear one after the other: "never more than 10 ms back", "never past the middle to the previous hit"; then a kick waveform with its later peak and the 29 dB line.

Two limits keep the walk honest. It never goes back more than 10 milliseconds, and never past the middle to the previous hit. A kick drum is a special case: its peak comes late, so its threshold is 29 decibels under the peak.

## s08-result

> Zoom out: the whole snare track with every hit on its first sample; the MIDI notes and the spike track lined up under it.

The result: every hit sits on the first sample of its attack. That is what goes into the MIDI file and the spike file. Next, we see how loud each hit was: the velocity.
