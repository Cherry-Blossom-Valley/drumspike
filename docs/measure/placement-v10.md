# Measure: placement rule version 10 (#444)

Version 10 is version 9 with two measured defects removed. Nothing else changed: the same edge, the same walk back to
the line, the same fits.

1. **The line's level is the true median.** Version 9 took the upper of the two middle values when the count of line
   samples was even. A stroke and its polarity-flipped copy then read two different levels, and a sample at the edge of
   the visibility band fell on one side for `x` and on the other for `-x`.
2. **The counts of samples follow the rate.** Five constants are counts fitted at 48 kHz (the run of line samples, the
   edge's tail, the least width of a swing, the bridge between a pre-swing and the edge, the cap of the strict foot).
   Version 9 used the same numbers at every rate, so at 96 kHz they covered half the time. Version 10 scales them with
   `rate / 48000` (never under one sample). At 48 kHz the counts are the counts of version 9.

A project keeps the version it was analysed with (the track's `detector.refine` number); a re-pick of such a track runs
version 9 as before. A new analysis runs version 10.

## The invariance lines (`tools/placement_invariance.py`)

1242 hits of the internal stems (400 kick, 400 snare, 442 tom), the share of hits that land on another sample when the
input changes; `--rule 9` against `--rule 10` on the same hits:

| change | version 9, moved / by 2+ / by 5+ | version 10 |
|---|---|---|
| polarity flip, kick | 1.2 / 0.2 / 0.2 % | 0 / 0 / 0 |
| polarity flip, snare | 5.5 / 2.2 / 0.0 % | 0 / 0 / 0 |
| polarity flip, tom | 7.2 / 1.8 / 0.0 % | 0 / 0 / 0 |
| 96 kHz resample, kick | 6.5 / 4.0 / 0.8 % | 5.2 / 3.2 / 0.5 % |
| 96 kHz resample, snare | 17.8 / 14.0 / 7.8 % | 9.0 / 7.0 / 2.0 % |
| 96 kHz resample, tom | 16.5 / 12.2 / 5.7 % | 13.8 / 10.4 / 5.7 % |
| gain, DC offset, hum, noise, coarse time | as before | within 1 point of version 9 on every line |

The polarity line is 0 by design now, and its bound is 0. The rate line is better, not gone: what remains comes from
the parts of the rule that are already in time (the resampled audio is not the same audio sample by sample) and from
the tom line, which the counts do not reach. The bounds file holds today's values for both.

The delay line (the same audio cut some samples earlier, through the full track analysis, the hit must move by exactly
that count), with rule version 10 named in the job:

| cut earlier by | kick, placed exactly | snare | tom |
|---|---|---|---|
| 1 to 7 samples | 100 % | 100 % | 99.2 to 99.6 % |
| 100, 240, 331 samples | 99.1 to 99.7 % | 95.2 to 98.0 % | 79.3 to 84.5 % |

At the long cuts the tom line loses 6.4 to 8.8 % of its hits and places 8.0 to 11.2 % five or more samples off; the
snare line loses 1.6 to 2.4 %. That is the detector's frame grid (the hit list and the coarse time change with the cut),
and toms are where it shows. The bounds file records today's values, so a later fix of the detector shows as a better
line.

Correction (2026-11-11): the first version of this page gave lower numbers for the delay line (74 to 84 % exact at the
long cuts for every role). Those were measured with rule version 1: `service_tool core-analyze` runs version 1 when a
job names no rule, and the tool's delay line named none. The tool now names the rule in the job, a test holds that, and
the bounds file holds the version 10 values. With version 1 the same run gives the old numbers again.

## Against the labels

On the tool path (the full analysis of ten internal sessions, version 9 against version 10):

- The hit lists are equal: 10485 hits, none lost, none new. 242 hits (2.3 %) sit on another sample: kick 0.8 %,
  snare 2.5 %, tom 6.1 %, hat 2.8 %; 176 of the 242 by one sample.
- The owner-placed rows (one session, kick and snare) are equal for both versions: kick 84.0 % exact and 99.4 % within
  one sample, snare 64.9 % and 96.4 %. No owner-placed row is worse.
- The agent-placed labels of the ten sessions agree with version 10 slightly less than with version 9: the exact share
  changes by -0.9 to +0.3 points per session and role, with two snare tracks at -1.6 and -4.0 points (the second: 15
  of 377 hits, 6 of them to 2 or more samples). These labels were placed while looking at version 8 and 9 positions and are not confirmed, so
  they do not decide between two positions one sample apart; they show that version 10 is not a different rule.
- The fine-tune judge (`tools/judge_measure.py --at-current`), with the hits at their version 10 positions where the
  20 ms rows reproduce the rule: 2919 exact and 4147 within one sample of 4912 gold rows, against 2920 and 4150.

## Tests

`crates/core/src/refine/tests.rs`: 240 generated strokes on a noisy line whose pre-swing opens near the visibility
band land on the same sample as their flipped copies (under version 9 a part of them does not, the test asserts that
too, so it can fail); 24 generated strokes sampled at 48 and at 96 kHz land within one 48 kHz sample of each other
(under version 9 two of them are 11 and 14 samples apart); version 9 stays reachable by its number. The golden of the
rules holds a row for version 10; the rows of versions 1 to 9 did not change.
