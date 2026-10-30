# ENST timing, step 1: drummer_1 kick late or dropped at 20 ms, the late placement tail

Ticket #136, follow-up of the 0.9.6 test campaign (#118). Step 1 only: measurement, no code
change. DSP detector over all 210 ENST public items, drummer_1/2/3, at refine rules 3, 4, 4b
and 5 (rule 5 = the #127 refine version, cap 3 samples). Aggregates per drummer and role only;
per-file tables stay on the agent host.

## Finding 1: drummer_1 kick recall 0.714 at ±20 ms vs 0.849 at ±50 ms

Drummer_1 kick references: 3234. At the 0.9.6 default (refine rule 4), matched at ±20 ms:
2296 (recall 0.710). Misses at ±20 ms: 938, counted split:

| split (rule 4) | refs |
|---|---|
| late: a detection 20–50 ms after the reference | 3 |
| early: a detection 20–50 ms before the reference | 3 |
| dropped: no detection of any role within 50 ms | 933 |

Recall and the split are identical under refine rules 3, 4, 4b and 5 (recall 0.710 each).

The gap is therefore NOT a population of 20–50 ms late detections: only 3 refs are late
(their first estimate p50 27.9 ms, p90 32.8 ms), and 933 of 938 misses have no detection of
any role within 50 ms at all.

Cause of the dropped refs (counted, aggregated over the 933 refs, 51 items):

- 156 of the 933 have a kick detection within 150 ms, but beyond the ±50 ms window.
- For the rest, the nearest detection within 50 ms is on another role: overhead 368,
  hi-hat 244, snare 76, tom 34, kick 7. The kick onset is picked on the hat/overhead/snare
  paths but not on the kick path.

**Named cause: the onset detector's pick.** The kick-band onset is missed on the kick path
while co-occurring hat/overhead onsets are picked. The refinement is excluded as the cause
(identical recall under rules 3/4/4b/5), and wrong matching is excluded (nearest-match check,
below). The annotation convention is not indicated: the same references are matched at ±50 ms
in #118, so the events exist in both annotation and audio.

## Finding 2: the late placement tail per role

Matched-detection offsets vs the annotation, per drummer and role (rule 4), in ms
(p10 / p50 / p90), with the wrong-match check:

| drummer/role | n | p10 | p50 | p90 | matched not nearest | beyond 20 ms: wrong match | real lag |
|---|---|---|---|---|---|---|---|
| drummer_1 kick | 2296 | -1.2 | 2.7 | 7.7 | 0 | 0 | 5 |
| drummer_1 snare | 2303 | -0.1 | 0.9 | 11.1 | 0 | 0 | 56 |
| drummer_1 hat | 3093 | -0.1 | 2.6 | 14.0 | 0 | 0 | 258 |
| drummer_2 kick | 3349 | -0.2 | 0.4 | 3.6 | 0 | 0 | 0 |
| drummer_2 snare | 3194 | -0.2 | 1.1 | 9.8 | 0 | 0 | 29 |
| drummer_2 hat | 3888 | 0.0 | 2.3 | 12.8 | 0 | 0 | 207 |
| drummer_3 kick | 3989 | -0.2 | 0.4 | 3.0 | 0 | 0 | 0 |
| drummer_3 snare | 2656 | -0.2 | 0.6 | 9.1 | 0 | 0 | 27 |
| drummer_3 hat | 4120 | -0.1 | 2.3 | 15.4 | 0 | 0 | 431 |

Every matched detection is the nearest detection (0 not-nearest), and every matched detection
beyond 20 ms is a real lag, not a later stroke matched. Refine rules 3/4/4b/5 move the p90 by
at most ~0.3 ms per role, so the #127 refine version 5 does not change the tail.

**Named cause: a real lag of the picked onset in the detector (peak-pick timing), not a wrong
match.** The tail is largest on hat (p90 12.8–15.4 ms) and snare (p90 9.1–11.1 ms) and small
on kick for drummer_2/3 (p90 ~3 ms); drummer_1 kick p90 7.7 ms sits between.

## Step 2

Awaiting the coordinator's OK. The causes above point at the onset detector's pick (kick
sensitivity / per-band pick), behind the analysis version switch; the ENST gate rows,
drummer_1 kick recall at ±20 ms and session F1 would be the acceptance lines.
