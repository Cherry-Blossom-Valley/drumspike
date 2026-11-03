# Hit placement bias: measurement (#101, step 1)

Measurement only, no change to the detector. Tool: `crates/core/examples/placement_bias.rs`
(`cargo run --release -p drumspike-core --example placement_bias`).

## Setup

- Synthetic one-shots of kick, snare, tom and hat, placed at an exact onset sample on uniform
  noise floors of -60 and -90 dBFS, at 44.1, 48 and 96 kHz, first polarity +1 and -1, 30 seeds
  each (n = 120 per row).
- Attack of the one-shot: 0 ms (a sharp start at full level), 0.5 ms and 1 ms (a raised-cosine
  ramp from the onset sample).
- Refinement: `refine_rule_peak` with `RefineRule::CURRENT` (FirstMotion), `FM_PEAK_DB` = 33 dB,
  the detector's coarse time 6 ms before the onset.
- Two targets per hit:
  - onset: the true onset sample `k`;
  - foot (owner rule, #101): the last floor-level sample before the first sample that leaves the
    floor. The floor is uniform noise in [-a, a] with a = 10^(floor/20) * sqrt(3), so a sample
    leaves it when |x| > a. Foot = (first index with |x| > a) - 1. For a sharp start that is
    `k - 1`.
- "early" counts the hits placed more than 1 sample before the foot (the fix rule's limit).

## Result (before any change)

Errors in samples, refined time minus the target. Median and 10th / 90th percentile over 120 hits.

| rate | role | attack ms | onset median | p10 | p90 | foot median | p10 | p90 | early | moved | kept | busy | separate |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 44100 | kick | 0 | +0.00 | -0.00 | +0.00 | +1.00 | +1.00 | +1.00 | 0 | 0 | 120 | 0 | 0 |
| 44100 | kick | 0.5 | +3.00 | +3.00 | +3.00 | +3.00 | +3.00 | +3.00 | 0 | 120 | 0 | 0 | 0 |
| 44100 | kick | 1 | +5.00 | +5.00 | +5.00 | +5.00 | +4.00 | +5.00 | 0 | 120 | 0 | 0 | 0 |
| 44100 | snare | 0 | +0.00 | +0.00 | +0.00 | +1.00 | +1.00 | +1.00 | 0 | 28 | 92 | 0 | 0 |
| 44100 | snare | 0.5 | +4.00 | +3.00 | +6.00 | +4.00 | +3.00 | +6.00 | 0 | 120 | 0 | 0 | 0 |
| 44100 | snare | 1 | +8.00 | +6.00 | +11.00 | +7.00 | +5.00 | +9.00 | 0 | 120 | 0 | 0 | 0 |
| 44100 | tom | 0 | +0.00 | -0.00 | +0.00 | +1.00 | +1.00 | +1.00 | 0 | 0 | 120 | 0 | 0 |
| 44100 | tom | 0.5 | +3.00 | +3.00 | +3.00 | +3.00 | +3.00 | +3.00 | 0 | 120 | 0 | 0 | 0 |
| 44100 | tom | 1 | +5.00 | +5.00 | +5.00 | +5.00 | +4.00 | +5.00 | 0 | 120 | 0 | 0 | 0 |
| 44100 | hat | 0 | +0.00 | -0.00 | +0.00 | +1.00 | +1.00 | +1.00 | 0 | 48 | 72 | 0 | 0 |
| 44100 | hat | 0.5 | +5.00 | +4.00 | +7.00 | +5.00 | +4.00 | +7.00 | 0 | 120 | 0 | 0 | 0 |
| 44100 | hat | 1 | +9.00 | +8.00 | +12.00 | +8.00 | +7.00 | +12.00 | 0 | 120 | 0 | 0 | 0 |
| 48000 | kick | 0 | +0.00 | -0.00 | +0.00 | +1.00 | +1.00 | +1.00 | 0 | 0 | 120 | 0 | 0 |
| 48000 | kick | 0.5 | +3.00 | +3.00 | +3.00 | +3.00 | +3.00 | +3.00 | 0 | 120 | 0 | 0 | 0 |
| 48000 | kick | 1 | +5.00 | +5.00 | +5.00 | +5.00 | +4.00 | +5.00 | 0 | 120 | 0 | 0 | 0 |
| 48000 | snare | 0 | +0.00 | -0.00 | +0.00 | +1.00 | +1.00 | +1.00 | 0 | 33 | 87 | 0 | 0 |
| 48000 | snare | 0.5 | +5.00 | +4.00 | +7.00 | +4.00 | +2.00 | +7.00 | 0 | 120 | 0 | 0 | 0 |
| 48000 | snare | 1 | +8.00 | +7.00 | +12.00 | +7.00 | +5.00 | +11.00 | 0 | 120 | 0 | 0 | 0 |
| 48000 | tom | 0 | +0.00 | -0.00 | +0.00 | +1.00 | +1.00 | +1.00 | 0 | 0 | 120 | 0 | 0 |
| 48000 | tom | 0.5 | +3.00 | +3.00 | +3.00 | +3.00 | +3.00 | +3.00 | 0 | 120 | 0 | 0 | 0 |
| 48000 | tom | 1 | +5.00 | +5.00 | +5.00 | +5.00 | +4.00 | +5.00 | 0 | 120 | 0 | 0 | 0 |
| 48000 | hat | 0 | +0.00 | -0.00 | +0.00 | +1.00 | +1.00 | +1.00 | 0 | 44 | 76 | 0 | 0 |
| 48000 | hat | 0.5 | +6.00 | +5.00 | +7.00 | +6.00 | +4.00 | +7.00 | 0 | 120 | 0 | 0 | 0 |
| 48000 | hat | 1 | +10.00 | +8.00 | +14.00 | +9.00 | +7.00 | +13.00 | 0 | 120 | 0 | 0 | 0 |
| 96000 | kick | 0 | +0.00 | -0.00 | +0.00 | +1.00 | +1.00 | +1.00 | 0 | 0 | 120 | 0 | 0 |
| 96000 | kick | 0.5 | +6.00 | +6.00 | +6.00 | +6.00 | +5.00 | +6.00 | 0 | 120 | 0 | 0 | 0 |
| 96000 | kick | 1 | +10.00 | +10.00 | +10.00 | +10.00 | +7.00 | +10.00 | 0 | 120 | 0 | 0 | 0 |
| 96000 | snare | 0 | +0.00 | +0.00 | +1.00 | +1.00 | +1.00 | +2.00 | 0 | 20 | 100 | 0 | 0 |
| 96000 | snare | 0.5 | +8.00 | +6.00 | +10.00 | +7.00 | +5.00 | +10.00 | 0 | 120 | 0 | 0 | 0 |
| 96000 | snare | 1 | +15.00 | +12.00 | +20.00 | +13.00 | +7.00 | +18.00 | 0 | 120 | 0 | 0 | 0 |
| 96000 | tom | 0 | +0.00 | -0.00 | +0.00 | +1.00 | +1.00 | +1.00 | 0 | 0 | 120 | 0 | 0 |
| 96000 | tom | 0.5 | +5.00 | +5.00 | +5.00 | +5.00 | +4.00 | +5.00 | 0 | 120 | 0 | 0 | 0 |
| 96000 | tom | 1 | +9.00 | +9.00 | +9.00 | +9.00 | +6.00 | +9.00 | 0 | 120 | 0 | 0 | 0 |
| 96000 | hat | 0 | +0.00 | -0.00 | +0.00 | +1.00 | +1.00 | +1.00 | 0 | 48 | 72 | 0 | 0 |
| 96000 | hat | 0.5 | +10.00 | +9.00 | +12.00 | +9.00 | +7.00 | +11.00 | 0 | 120 | 0 | 0 | 0 |
| 96000 | hat | 1 | +19.00 | +16.00 | +22.00 | +16.00 | +13.00 | +21.00 | 0 | 120 | 0 | 0 | 0 |

## Reading

- Sharp start (0 ms): the refined time is the first departing sample, one sample after the foot.
  Median foot error +1 for every role and rate (kick, tom: 120 of 120 kept, the walk does not move).
  This is the 1-sample-late bias the owner sees on a sharp hit. No hit is placed early.
- Ramp (0.5 ms and 1 ms): the threshold crossing (33 dB under the peak) lands inside the fall.
  Median foot error +3 to +6 at 0.5 ms and +5 to +9 at 1 ms at 44.1 and 48 kHz, and +5 to +9
  at 0.5 ms and +9 to +16 at 1 ms at 96 kHz. The error grows with the rate because the ramp is
  longer in samples. On a soft attack it is larger than the 1-2 samples the owner sees.
- Rounding (cause 2): every median and percentile in the table is an integer. The seconds-to-samples
  rounding is not the source of a sub-sample bias in this set.
- Drawing (cause 3): the hit line is drawn at `Math.round(tToX(t, W)) + 0.5` (`static/app.js`,
  `drawEventLines`), within one CSS pixel of its time. The sample-line mapping of the wave lane
  (`static/lanes-gl.js`, `u_x0` and `u_ppx`) is not yet checked against it: that needs the
  before/after frames at the highest zoom.

## Verdict so far

- Cause 1 (threshold crossing): bias at or above 0.5 sample in every row except the sharp start.
  A fix is needed: the foot must come back to the last floor-level sample.
- Cause 2 (rounding): no bias measured (integer medians).
- Cause 3 (drawing): not decided; the wave-lane mapping is still to be read and framed.
- Real sessions (Session A to C): not measured. The session audio is not in
  `~/.cache/drumspike/sessions` on this host (the directory does not exist), so the walk-back
  histogram waits for the location of the sessions.

## Step 2: the foot rule (version 4, `RefineRule::Foot`)

The back-track of version 3 stops at the first sample over its threshold. Version 4 takes the foot one
step stricter (`foot_strict` in `crates/core/src/refine.rs`): from that sample, walk back over the samples
that are above the floor level until two samples in a row are at or under the floor. The foot is the last
of those two-in-a-row floor samples before the first sample that leaves the floor, that is the owner's
rule. The floor level is the largest `|x|` of the floor window (`FM_FLOOR_S` before the hit, without its
last `FM_FLOOR_SKIP_S`). The samples are read raw: the 2-tap mean halves the first sample of a slow rise
and would read it as floor. A walk that finds no such pair above the walk's bound keeps the version 3 foot.

Version 4 is the analysis default (`RefineRule::CURRENT`), with one guard (decision for 0.9.6): the strict
foot may move at most 0.25 ms (`FM_STRICT_MAX_S`: 11 samples at 44.1 kHz, 12 at 48 kHz, 24 at 96 kHz) before
the version 3 foot. Further than that, the version 3 foot stays. The guard covers every synthetic ramp up to
1 ms and cuts the tail of the real sessions. Version 3 is still selected by the version number 3 or
`DRUMSPIKE_REFINE_RULE=3`.

Synthetic table, `placement_bias foot`, the same set as step 1 (errors in samples against the foot):

| rate | role | attack ms | onset median | p10 | p90 | foot median | p10 | p90 | early (<-1 vs foot) | moved | kept |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 44100 | kick | 0 | -1.00 | -1.00 | -1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 44100 | kick | 0.5 | +0.00 | -0.00 | +0.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 44100 | kick | 1 | +1.00 | -0.00 | +1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 44100 | snare | 0 | -1.00 | -1.00 | -1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 44100 | snare | 0.5 | +0.00 | -0.00 | +1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 44100 | snare | 1 | +1.00 | -0.00 | +3.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 44100 | tom | 0 | -1.00 | -1.00 | -1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 44100 | tom | 0.5 | +0.00 | -0.00 | +0.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 44100 | tom | 1 | +1.00 | -0.00 | +1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 44100 | hat | 0 | -1.00 | -1.00 | -1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 44100 | hat | 0.5 | +0.00 | -0.00 | +1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 44100 | hat | 1 | +1.00 | -0.00 | +2.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 48000 | kick | 0 | -1.00 | -1.00 | -1.00 | +0.00 | +0.00 | +0.00 | 0 | 120 | 0 |
| 48000 | kick | 0.5 | +0.00 | -0.00 | +0.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 48000 | kick | 1 | +1.00 | +0.00 | +1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 48000 | snare | 0 | -1.00 | -1.00 | -1.00 | +0.00 | +0.00 | +0.00 | 0 | 120 | 0 |
| 48000 | snare | 0.5 | +0.00 | +0.00 | +1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 48000 | snare | 1 | +1.00 | -0.00 | +3.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 48000 | tom | 0 | -1.00 | -1.00 | -1.00 | +0.00 | +0.00 | +0.00 | 0 | 120 | 0 |
| 48000 | tom | 0.5 | +0.00 | -0.00 | +0.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 48000 | tom | 1 | +1.00 | +0.00 | +1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 48000 | hat | 0 | -1.00 | -1.00 | -1.00 | +0.00 | +0.00 | +0.00 | 0 | 120 | 0 |
| 48000 | hat | 0.5 | +0.00 | +0.00 | +1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 48000 | hat | 1 | +1.00 | +0.00 | +3.00 | +0.00 | +0.00 | +0.00 | 0 | 120 | 0 |
| 96000 | kick | 0 | -1.00 | -1.00 | -1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 96000 | kick | 0.5 | +1.00 | +0.00 | +1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 96000 | kick | 1 | +3.00 | +0.00 | +3.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 96000 | snare | 0 | -1.00 | -1.00 | -1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 96000 | snare | 0.5 | +1.00 | +0.00 | +3.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 96000 | snare | 1 | +3.00 | +0.00 | +6.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 96000 | tom | 0 | -1.00 | -1.00 | -1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 96000 | tom | 0.5 | +1.00 | +0.00 | +1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 96000 | tom | 1 | +3.00 | +0.00 | +3.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 96000 | hat | 0 | -1.00 | -1.00 | -1.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 96000 | hat | 0.5 | +1.00 | +0.00 | +2.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |
| 96000 | hat | 1 | +3.00 | +0.00 | +6.00 | +0.00 | -0.00 | +0.00 | 0 | 120 | 0 |

Reading: the foot median is 0 on every row (the fix rule: within +-0.5 sample), no hit is more than
one sample before the foot (`early` is 0 on every row), and the p10 is never before the foot. The
onset column shows the same hits against the true onset: the sharp start sits one sample before it.
Against step 1: the sharp start moves from +1 to 0 on every row, and the ramps move from +3 to +16
(foot medians of step 1) to 0.

Limits of this set: a band-limited sharp start has a ripple before the onset that is above the floor; the
foot rule counts it as leaving the floor (the literal rule), so such a start moves earlier by the length
of the ripple. The unit test `the_foot_rule_stops_on_the_last_floor_level_sample` covers the synthetic
starts at 44.1, 48 and 96 kHz only.

## Step 2: the real sessions (Session A, B, C)

The close mics (kick, snare, toms 1 to 3) of the three sessions, analysed by `service_tool` with the
refinement rule 3 and rule 4 (the same hits, only the refined time differs). Rule 4 is the guarded default
(the strict foot with the 0.25 ms guard above). Shift = rule 4 minus rule 3, in samples, hits matched one to
one within 10 ms. Counted over all hits, run at eb72455.

| role | hits | unchanged | moved earlier | moved later | shift histogram (samples: count) |
|---|---|---|---|---|---|
| kick | 1183 | 0 | 1183 | 0 | -1: 948, -2: 222, -3: 10, -4: 1, -6: 2 |
| snare | 882 | 33 | 849 | 0 | -1: 264, -2: 360, -3: 41, -4: 46, -5: 69, -6: 26, -7: 16, -8: 9, -9: 3, -10: 7, -11: 5, -12: 3 |
| tom | 315 | 8 | 307 | 0 | -1: 158, -2: 106, -3: 21, -4: 8, -5: 4, -6: 1, -7: 1, -8: 1, -9: 4, -11: 2, -12: 1 |

Kick and tom: the foot moves 1 to 2 samples on almost every hit. Snare: most hits move 1 to 5 samples.
No hit moves later.

The guard kept the version 3 foot on 0 kick, 16 snare and 5 tom hits (the unchanged count with the guard
minus the unchanged count of the unguarded run: snare 33 - 17, tom 8 - 3). The unguarded tails (snare -16 to
-67, tom -13 to -70) are the walk-backs that the guard cuts. On the synthetic set (table above) the guard
keeps the version 3 foot on 0 of the 4320 hits (36 rows of 120).

F1, pooled over the three sessions, rule 3 and rule 4 identical at +-20 ms and +-50 ms (the event sets
match; only the times differ):

| role | +-20 ms F1 (rule 3 = rule 4) | +-50 ms F1 (rule 3 = rule 4) |
|---|---|---|
| kick | 0.9771 | 0.9780 |
| snare | 0.8975 | 0.8975 |
| tom | 0.8867 | 0.8867 |

The F1 is not below main. The ENST gate (12 rows) was not run: the ENST dataset is not on this host.

## Version 6 (#293, #294): the steep edge after the foot

The owner's report on 0.9.13 (an own session, 489 kick and 356 snare hits): kick hits sit 3 to 5 samples
before the first sample that visibly leaves the flat line; many snare hits sit at the start of a slow drift
or a bleed stretch, 0.3 to 2 ms before the steep rise of the stroke. A before/after of 0.9.6, 0.9.11 and
0.9.13 on four sessions shows the same placement in all three (same hit counts, median shift 0 ms), so this
is not a regression of the 0.9.12 changes but the walk's design: it stops at the first sample over its
threshold (33 dB under the peak, or 6 dB over the floor), and the bleed or drift before an attack lies over
that threshold. The #239 lead-in made it worse on 22 snare hits (0.25 to 0.8 ms earlier).

Version 6 (`RefineRule::Edge`, the analysis default since 0.9.14):

- the version 3 foot: no strict foot (versions 4 and 5 moved the hit 1 to 3 samples before the first sample
  over the threshold, the opposite of what the engineer sees), no lead-in (`set_lead_in` stays for the
  measurement example, default off);
- then the first steep edge after the foot (`edge_foot`): the first sample `j` in the 5 ms after the foot
  where the signal rises by `EDGE_RISE_DB` (18 dB) under the hit's peak within `EDGE_RISE_S` (0.1 ms),
  looked for only while the level is still under `EDGE_TOP_DB` (12 dB) below the peak. The hit moves to the
  edge's foot: the first sample of that window whose step to the next sample is half the edge's per-sample
  rise. A slow attack without such an edge keeps the version 3 foot; a hard attack keeps its first sample.

The rule was then fitted on the owner's hand-placed hits (2026-11-02, the reported session: kick hits 1 to 332
and snare hits 1 to 168 placed by hand in the app, 107 and 89 of them moved; an unmoved hit counts as correct).
Even hits chose the parameters, odd hits checked them (`tools`-free prototypes, the exact port of `edge_foot`
in Python). Three things changed against the first build:

- the search starts one rise window (0.1 ms) before the walk's foot: on a steep kick the first sample over the
  walk's threshold already lies inside the rise, and from there the next window rises too little, so the first
  build kept the walk's foot, 2 samples after the owner's position;
- the guard compares the stretch before an edge with the level that edge reaches within 0.5 ms
  (`EDGE_GUARD_DB`, 12 dB; 9 to 15 dB give the same result), not with the peak of the whole 5 ms, and the search
  goes on after a rejected edge; the level reaching 12 dB under the peak without an edge ends the search
  (`EDGE_TOP_DB`): a ripple inside the body of a soft hit is no edge;
- the foot's walk-back follows the rising level and takes up to `EDGE_TAIL_SAMPLES` (4) more samples while the
  level rises by more than `EDGE_TAIL_NOISE` (8) times the line noise, so the hit sits on the last sample of the
  line, as version 5's strict foot did. With 2 times the noise the tail took the slow drift of a kick's line for
  the hit (1 to 3 samples early).

Against the owner's positions (the app's own analysis of the 0.9.13 build and of this rule; mean of
|placed - owner| in samples at 48 kHz):

| drum | hits | 0.9.13 (version 5) | version 6 | within 1 sample | within 5 samples |
|---|---|---|---|---|---|
| kick | 332 | 0.61 | 0.57 | 89 % to 92 % | 98 % to 99.7 % |
| snare | 168 | 4.99 | 1.14 | 62 % to 80 % | 67 % to 98 % |

The held-out odd hits score 0.53 (kick) and 1.23 (snare). Of the candidates measured on the same positions, no
threshold rule (the first sample over K times the noise, or over the peak minus D dB) and no change-point
picker (the AIC arrival picker of seismology on the raw, the smoothed or the differentiated signal) reached the
0.9.13 kick score; the edge rule is the only one that improves both drums.

The edge may move a hit later than the version 2 time (`BackStep::Moved` either way). The thresholds were
chosen on the Python prototype over three sessions and the owner's hits: 18 dB keeps the small sharp first
step of a snare stroke as the hit and skips a slow drift (which rises under 18 dB per 0.1 ms); the 12 dB
guard stops the search on soft or slow hits before a later swing is taken for the edge. Measured moves of
version 6 against version 5 are in the 0.9.14 gate record (PR #293 fix).

Measured, version 6 against 0.9.13 (version 5) with the app's own analysis (the tool's `cli run`: project,
tracks, analyze, events), hits matched within 5 ms:

| session | role | hits | moved earlier | moved later by 0.25 ms or more | median move |
|---|---|---|---|---|---|
| Session E (the report's) | kick | 489 | 0 | 5 | +2 samples |
| Session E | snare | 356 | 0 | 43 | +2 samples |
| Session E | tom (4 mics) | 353, 355, 236, 355 | 0 | 93, 107, 21, 27 | +2 samples |
| Session B | kick | 359 | 0 | 27 | +2 samples |
| Session B | snare | 267 | 0 | 38 | +4 samples |
| Session B | hat | 288 | 0 | 16 | +1 sample |

Hit counts are unchanged on every track (one tom of Session B: 41 to 42). The 2-sample median is the strict
foot that version 5 took before the first sample over the threshold. The report's four kick hits move 4, 4,
4 and 11 samples onto the visible departure; its three snare hits 8, 16 and 27 samples onto the start of
the drift. The synthetic hard attack keeps its first sample; a 0.5 ms raised-cosine rise is placed 1 to 4
samples after its first sample over the floor (`placement_bias`, `test_calib_kit` within its 5-sample limit).

## Version 7 (#321): the walk-back to the line, judged by eye

The owner's report on 0.9.17 (the same session as version 6): kick hits still 2 to 3 samples before the first sample
that visibly leaves the line, some snare hits 2 to 7 samples after the start of the slow drift before the stroke.
Against the owner's hand-placed hits (kick 1..332, snare 1..168): kick 24 hits 2 or more samples early, snare 31 hits
2 or more samples late. Cause: version 6's last step, the walk from the steep edge back to the line, judged each step
against the line noise (at least 8 times the noise, or a sixteenth of the edge's rise) and took at most 4 samples. On a
kick line that is almost digital silence the threshold is tiny and the walk took the invisible first steps; on a
snare line with hi-hat bleed it stopped inside the drift.

Version 7 (`RefineRule::Visible`, `EdgeTail::Visible`, the analysis default since 0.9.18) keeps the edge search and
replaces that walk: back over the samples that stand more than `VIS_TAIL_PCT` (2 %) of the hit's peak off the line,
with the edge's sign, while the sample before is no more than `VIS_TAIL_TOL_PCT` (0.5 %) of the peak larger. The walk
counts only when it reaches the line within `VIS_TAIL_MAX_S` (0.15 ms); otherwise version 6's walk stays. Both bounds
come from the measurement:

- with a 1 ms limit and no reach condition, tom hits on Sessions A and B whose line is a slow wave or an offset on the
  stroke's side moved up to 1 ms early (the walk followed the wave to its zero crossing);
- with the reach condition but a 1 ms limit, the walk still followed such a wave down to a zero crossing;
- on the owner's hits every limit from 6 samples up keeps the whole gain (a drift the engineer counts is short), so
  0.15 ms keeps it and leaves the long stretches to version 6.

Even hits chose the share (2 and 3 % give the same held-out result), odd hits checked it. The app's own analysis
against the owner's hits (mean of |placed - owner| in samples at 48 kHz):

| drum | hits | 0.9.17 (version 6) | version 7 | odd hits, version 6 -> 7 | 2+ samples off, version 6 -> 7 |
|---|---|---|---|---|---|
| kick | 332 | 0.57 | 0.26 | 0.53 -> 0.19 | 24 early -> 2 early |
| snare | 168 | 1.14 | 1.03 | 1.23 -> 1.27 | 31 late -> 25 late |

Other sessions against 0.9.17 (hits matched within 5 ms): hit counts unchanged on every track; no hit moves by
0.25 ms or more on Sessions A, B, C and the two time-aligned sessions; the moved tom hits (all under 0.15 ms) were
checked in panels. ENST-Drums part B: the +-10 ms table is identical to 0.9.17 and the standing gate passes.
