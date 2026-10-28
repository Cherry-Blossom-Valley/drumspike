# Explainer 8: From waveform to velocity

Chapter 8 of `docs/product/feature-path.md`. Narration for `tools/media/tts.py`: one `##` heading
per scene; lines that start with `>` are visual notes and are not spoken. Target length: about
2 minutes. Use the demo session's snare track (ghost notes, normal hits, accents).

Facts used (from `crates/core/src/velocity.rs` and `crates/app/src/histogram.rs`): the level of a
hit is its peak (the largest sample value) from 5 ms before to 60 ms after the hit, in dBFS; the
default map is linear in dB: -40 dBFS gives velocity 20 (the floor), -6 dBFS gives 127 (the
ceiling), levels outside are clamped; floor and ceiling are per track (`vel_lo_db`, `vel_hi_db`),
set with the handles of the Inspector's loudness histogram; the loudness cut (`vel_cut_db`)
leaves out the hits under a level; a hand edit in the velocity lane overrides the computed value.

## s01-question

> The snare track with three hits marked: a ghost note, a normal hit, an accent.

A drummer does not hit every note the same. A ghost note is barely there. An accent is loud. A trigger should follow that. So Drum Spike gives every hit a velocity, from 1 to 127, like a MIDI keyboard.

## s02-peak

> One hit; a window from 5 ms before to 60 ms after it lights up; the largest sample in it gets a marker and its level in dBFS.

First, it measures how loud the hit is. It looks at a short window: from five milliseconds before the hit to sixty milliseconds after it. The largest value in that window is the peak. Drum Spike reads it in decibels full scale.

## s03-why-db

> Two scales side by side: linear amplitude (the ghost note almost at zero) and decibels (the ghost note clearly above the floor).

Why decibels? Our ears hear loudness in steps of ratios, not in steps of sample values. On a linear scale, a ghost note sits almost at zero. In decibels, it has its own clear place.

## s04-map

> A straight line on a graph: x = peak level in dBFS from -60 to 0, y = velocity; the floor at -40 dBFS / 20, the ceiling at -6 dBFS / 127, flat outside.

Then a straight line turns the level into a velocity. By default, a peak at minus 40 decibels gives velocity 20. A peak at minus 6 decibels gives 127. In between, the line is straight. Softer than the floor stays at 20; louder than the ceiling stays at 127.

## s05-histogram

> The Inspector's loudness histogram of the snare: a cluster of ghost notes, a cluster of backbeats; the floor and ceiling handles are dragged; the velocities in the lane follow live.

Every track is different. A snare top mic is loud, a tom mic is quieter. So you set the floor and the ceiling per track. The Inspector shows a histogram of all hit levels. Drag its handles, and the velocities follow at once.

## s06-cut

> The loudness cut handle is dragged up to just above a small cluster of very quiet bumps; those hits turn grey (cut) and leave the export.

The histogram also shows hits that are too quiet to be real, for example soft bleed. Set the loudness cut above them. Those hits are left out of the export.

## s07-lane

> View > Velocity Lane (V): stalks under the waveform; one stalk dragged by hand; then an Option-drag draws a line over a crescendo.

Last, the velocity lane. It shows every velocity as a stalk. Drag a stalk to change one hit. Hold Option and draw a line to shape a whole crescendo. A hand edit always wins over the computed value.

## s08-result

> The exported MIDI notes with their velocities under the snare track; a sampler plays soft and loud layers.

The result: MIDI notes that play soft where the drummer played soft, and loud where they played loud. Your sampler picks the right layer for every hit.
