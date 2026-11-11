# Explainer 4: Finding the first sample

Chapter 4 of `docs/product/feature-path.md`. Narration for `tools/media/tts.py`: one `##` heading
per scene; lines that start with `>` are visual notes and are not spoken. Target length: about
2.5 to 3 minutes. The hits are real hits of the demo session (the snare top mic, one loud hit and one
ghost note); the placed sample on screen is the answer of the real rule (`parity refine`).

Facts used (from the code only: `crates/core/src/refine/walk.rs`, `edge.rs`, `track.rs`;
`ARCHITECTURE.md`, the line on hit placement; `docs/measure/placement-v10.md`). This is placement rule
version 10, `RefineRule::CURRENT`. Each line gives the constant and its value in the source; a count
of samples is at 48 kHz and scales with the rate in `at_rate` (rate / 48000, never under one sample).
- The detector's time is coarse; the rule starts from the walk back (`first_motion`) from the attack's
  peak (`FM_PEAK_S` = 0.005, 5 ms window): its threshold is `FM_PEAK_DB` = 33 dB under the peak, it goes
  back at most `FM_SPAN_S` = 0.010 (10 ms). Its foot is where the search for the edge begins.
- A steep edge: the signal rises by `EDGE_RISE_DB` = 18 dB under the peak within `EDGE_RISE_S` = 0.0001
  (0.1 ms); it is looked for in `EDGE_SPAN_S` = 0.005 (5 ms) after the foot.
- The line before the stroke: its level is the median (`line_level`, the true median since version 10)
  of the samples in `DEPARTURE_LEVEL_S` = 0.0005 (0.5 ms) before the foot. A sample stands off the line
  at more than `DEPARTURE_VIS_PCT` = 2.5 percent of the hit's peak; line samples are within
  `DEPARTURE_RUN_PCT` = 2 percent.
- The walk back from the edge goes over every sample off the line, of either sign, until a run of
  `LINE_RUN_SAMPLES` = 10 line samples (0.2 ms), at most `DEPARTURE_MAX_S` = 0.0008 (0.8 ms) before the
  edge's first rising sample. The hit is the last sample of that run: the last line sample before the
  stroke's first motion (`EdgeTail::Symmetric`).
- The first motion must stand out: its largest excursion is more than `DEPARTURE_STANDS_OUT` = 2 times
  the largest off-line excursion in the `EDGE_NOISE_S` = 0.002 (2 ms) before the run, with that
  largest excursion never taken under the departure band. A ripple that stays inside the band (the picture's bump is 1 percent of
  the peak, the band is 2.5 percent) is no first motion; the picture's ripple of either sign is 8
  percent of the peak.
- The first motion is a swing, not a flicker: `SWING_MIN_SAMPLES` = 3 (0.06 ms) of one sign in a row,
  and at most `DEPARTURE_BRIDGE_SAMPLES` = 4 samples inside the band between the run and the edge.
- When no steep edge counts, the hit is the strict foot of the first foot, at most `FOOT5_MAX_SAMPLES` =
  3 (0.06 ms) before it (`FootTune::V10`).
- A stroke and its polarity-flipped copy land on the same sample: the line level is the true median
  and the walk reads the distance from the line (`docs/measure/placement-v10.md`: 0 of 1242 hits move).
- At 96 kHz the counts cover the same time as at 48 kHz (`at_rate`).
- A project keeps the version it was analysed with (`RefineRule::from_version`, the track's
  `detector.refine`) until a new analysis.
- The answer is the hit's time in the MIDI file and the spike file.

## s01-recap

> The snare top mic of the demo session, zoomed to about 3 ms; every sample is a dot; the comb-filter picture from explainer 1 shrinks into a corner.

In the first video we saw why a trigger must fire on the exact first sample of a hit. Now we look at how Drum Spike finds that sample. We take one snare hit from the demo session and zoom in until every sample is a dot.

## s02-coarse

> A wide orange band over the waveform: the detector's coarse time, 2 ms early. A marker walks back from the peak to a first foot, a threshold line 33 dB under the peak.

First, the detector says a snare hit is near here. That time is coarse, far too coarse for a trigger. So Drum Spike walks back from the attack's peak to a first foot, 33 decibels under the peak. That foot is only where the search begins.

## s03-edge

> From the foot, a bracket 5 ms wide; inside it the steepest rise lights up, with a label "18 dB in 0.1 ms".

Next it looks for the steep edge of the stroke, in the 5 milliseconds after the foot. An edge is a rise of 18 decibels under the peak within 0.1 milliseconds. The edge tells where the stroke surely is, but not where it began.

## s04-line

> The samples before the foot are shaded; a horizontal line at their median; a band of 2.5 percent of the peak around it.

Before the stroke lies the line. Its level is the median of the samples in the 0.5 milliseconds before the foot. A sample stands off the line when it is more than 2.5 percent of the hit's peak away from it.

## s05-first-motion

> The same hit with an added ripple of 8 percent of the peak before the stroke, drawn once with a positive and once with a negative sign; a marker walks back from the edge over the ripple and stops on the last line sample, a run of 10 dots inside the band is marked.

From the edge, Drum Spike walks back over every sample that stands off the line, of either sign. A ripple of 8 percent of the peak belongs to the stroke. The walk stops at a run of 10 line samples at 48 kilohertz, about 0.2 milliseconds. The hit goes on the last sample of that run, the last sample of the line before the first motion.

## s06-small-bump

> The same hit with a bump of 1 percent of the peak inside the band; the marker does not move to the bump; a label "inside the band, no first motion".

A small bump inside the band is not a first motion. A motion must stand out, at 2 times the largest wobble of the line in the 2 milliseconds before it. This bump is only 1 percent of the peak, so it is part of the line, and the hit does not move.

## s07-polarity

> The hit and its polarity-flipped copy one above the other; each has its marker; the two markers are drawn on the same sample with a thin vertical guide through both.

Now we flip the polarity of the same stroke, so every sample changes its sign. The hit lands on the same sample. The line's level is the true median, and the walk reads the distance from the line, not the sign.

## s08-soft-hit

> The same steps on a ghost note from the demo session: a lower peak, its own band; the marker ends on the last line sample before its first motion.

A soft ghost note works the same way. Its peak is lower, so its edge and its band are measured against its own peak. The marker sits on the last line sample before the first motion, just as on the loud hit.

## s09-limits

> Three short notes appear one after the other: "back at most 0.8 ms", "a swing at least 0.06 ms wide", "no steep edge: the strict foot, at most 0.06 ms"; a last line "96 kHz: the same times, twice the samples".

The rule has limits. The walk back from the edge goes at most 0.8 milliseconds. A motion must be a swing about 0.06 milliseconds wide that runs into the edge. With no steep edge, the hit goes on the strict foot, at most 0.06 milliseconds from the first foot. These times stay the same at 96 kilohertz.

## s10-result

> Zoom out: the whole snare track with every hit on its first sample; the MIDI notes and the spike track lined up under it.

The result goes into the MIDI file and the spike file, on the last line sample before the first motion. A project keeps the placement of its analysis, version 10 today, until a new analysis. Next, we see how loud each hit was, the velocity.
