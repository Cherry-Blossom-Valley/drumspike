# ENST timing, step 1: drummer_1 kick late or dropped at 20 ms, the late placement tail

Ticket #136, follow-up of the 0.9.6 test campaign (#118). Step 1 only: measurement, no code
change. DSP detector over all 210 ENST public items, drummer_1/2/3, at refine rules 3, 4, 4b
and 5 (rule 5 = the #127 refine version, cap 3 samples). Aggregates per drummer and role only;
per-file tables stay on the agent host.

## Finding 1: drummer_1 kick recall 0.710 at ±20 ms; the 0.849 at ±50 ms is the gate's F1 row, not a recall

Drummer_1 kick references: 3234. At the 0.9.6 default (refine rule 4), matched one-to-one per
item at ±20 ms: 2296 (recall 0.710). Misses at ±20 ms: 938, counted split. Definitions: the
split is on the kick mic's detections only (the kick-path est list of the full-session run);
"late" = a kick detection 20–50 ms after the reference; "early" = a kick detection 20–50 ms
before the reference and no late one; "no kick-path detection within ±50 ms" = the rest. The
three buckets are disjoint and sum to the miss count:

| split (rule 4, kick-path detections) | refs |
|---|---|
| late: a kick detection 20–50 ms after the reference | 3 |
| early: a kick detection 20–50 ms before the reference, no late one | 2 |
| no kick-path detection within ±50 ms | 933 |
| **misses at ±20 ms** | **938** |

Recall and the split are identical under refine rules 3, 4, 4b and 5 (recall 0.710 each).

Reconciliation with #118 (which reported "drummer_1 kick recall 0.849 at ±50 ms"): that number
is the gate row `drummer_1|kick` of `enst_gate_baseline.json` — an F1 at ±50 ms, not a recall.
Reproduced with the gate's own matcher (enst one-to-one, pooled over the drummer's items, est
without bleed/cut flags) on the run files of both this ticket's rule-4 run and the 0.9.6 build:
F1 0.849 in both, exactly the baseline row. The gate-style recall at ±50 ms is 0.742
(precision 0.992, 2418 kick detections vs 3234 refs); per-item one-to-one matching gives 0.712
— consistent with the 933 bucket above. The matchers are the same; the difference is the metric
(F1 vs recall), and #118's prose mislabeled the F1 row as a recall. No contradiction remains.

Cause of the 933 refs with no kick-path detection within ±50 ms (one bucket per reference,
counted; the buckets sum to 933):

- nearest kick detection 50–150 ms away: 155
- no kick detection within 150 ms, and the nearest detection within 50 ms is on another mic
  (the kick onset is picked on the other mics' paths but not on the kick path): overhead_L 194,
  hi-hat 219, overhead_R 119, snare 62, tom_1 16, tom_2 15 — 625
- no kick detection within 150 ms, nearest detection of any mic 50–150 ms: 106
- no detection of any mic within 150 ms: 47

**Named cause: the onset detector's pick.** The kick-band onset is missed on the kick path
while co-occurring hat/overhead/snare/tom onsets are picked on their paths. The refinement is
excluded as the cause (identical recall under rules 3/4/4b/5), and wrong matching is excluded
(nearest-match check, below).

## Finding 2: the late placement tail per role

Matched-detection offsets vs the annotation (rule 4), one-to-one per item at ±20 ms, in ms
(p10 / p50 / p90), with the wrong-match check. Tom rows are new in this revision: the tom mics
are pooled per drummer (the gate's tom aggregation), which the first version lacked.

| drummer/role | n | p10 | p50 | p90 | matched not nearest | beyond 20 ms: wrong match | real lag |
|---|---|---|---|---|---|---|---|
| drummer_1 kick | 2296 | -1.2 | 2.7 | 7.7 | 0 | 0 | 5 |
| drummer_1 snare | 2303 | -0.1 | 0.9 | 11.1 | 0 | 0 | 56 |
| drummer_1 hat | 3093 | -0.1 | 2.6 | 14.0 | 0 | 0 | 258 |
| drummer_1 tom | 679 | -0.3 | 1.0 | 6.1 | 0 | 0 | 0 |
| drummer_2 kick | 3349 | -0.2 | 0.4 | 3.6 | 0 | 0 | 0 |
| drummer_2 snare | 3194 | -0.2 | 1.1 | 9.8 | 0 | 0 | 29 |
| drummer_2 hat | 3888 | 0.0 | 2.3 | 12.8 | 0 | 0 | 207 |
| drummer_2 tom | 555 | -1.2 | 1.7 | 10.8 | 0 | 0 | 1 |
| drummer_3 kick | 3989 | -0.2 | 0.4 | 3.0 | 0 | 0 | 0 |
| drummer_3 snare | 2656 | -0.2 | 0.6 | 9.1 | 0 | 0 | 27 |
| drummer_3 hat | 4120 | -0.1 | 2.3 | 15.4 | 0 | 0 | 431 |
| drummer_3 tom | 905 | -0.2 | 1.6 | 8.4 | 0 | 0 | 1 |

Tom note: the tom mics fire on far more hits than the tom annotations (drummer_2 3803 tom
detections, drummer_3 6654, against 555/905 matched tom refs at ±20 ms) — the tom false
positives of #118/#103; they do not change the matched-offset percentiles.

Every matched detection is the nearest detection (0 not-nearest), and every matched detection
beyond 20 ms is a real lag, not a later stroke matched. Refine rules 3/4/4b/5 move the p90 by
at most ~0.3 ms per role, so the #127 refine version 5 does not change the tail.

**Named cause: a real lag of the picked onset in the detector (peak-pick timing), not a wrong
match.** The tail is largest on hat (p90 12.8–15.4 ms) and snare (p90 9.1–11.1 ms); tom
(p90 6.1–10.8 ms) and drummer_1 kick (7.7) sit between; kick for drummer_2/3 is ~3 ms.

## Step 2

Awaiting the coordinator's OK. The causes above point at the onset detector's pick (kick
sensitivity / per-band pick), behind the analysis version switch; the ENST gate rows,
drummer_1 kick recall at ±20 ms and session F1 would be the acceptance lines.
