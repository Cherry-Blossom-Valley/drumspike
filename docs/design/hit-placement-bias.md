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

Version 3 stays the analysis default (`RefineRule::CURRENT`). Version 4 is selected by the version number
4 or `DRUMSPIKE_REFINE_RULE=foot`. The default change waits for the ENST gate and the owner's decision.

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
