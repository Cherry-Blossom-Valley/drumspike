# 130: why Detect Tempo misses on Session A and Session C

Step 1 of #130 (measure, no code). Inputs: the kick and snare trigger mics of Session A and
Session C, analyzed at the defaults (644 hits on A: 351 kick + 293 snare; 689 on C: 387 +
302). The detector's global period pick was instrumented temporarily to dump every candidate
row. Score of a candidate: comb_rel x sqrt(prior) x (1 + backbeat); candidates with
comb_rel < 0.6 are gated out. The tom tracks are not in this probe; the full pipeline
(all present mics) agrees on the pick and on the runner-up comb (0.946 vs 0.971 on A).
Cross-checks use the #101 cached reference onsets (Fourier pulse and inter-onset structure).
Session B is cited only as the control: same mechanism, correct result.

## Session A (map 134.0, detector suggests 67.0)

Per-candidate table (comb relative to the best, prior as sqrt, backbeat referee, score
before the gate, on-beat/quarter-beat weight shares):

| BPM | comb rel | sqrt prior | backbeat | score | gated | on | quarter |
|------|-------|-------|---------|-------|-----|------|------|
| 67.00 | 1.000 | 0.838 | +0.733 | 1.452 | no | 0.388 | 0.284 |
| 134.01 | 0.971 | 0.994 | -0.193 | 0.779 | no | 0.674 | 0.037 |
| 44.67 | 0.905 | 0.602 | -0.239 | 0.414 | no | 0.247 | 0.017 |
| 268.01 | 0.777 | 0.715 | -0.187 | 0.451 | no | 0.952 | 0.003 |
| 89.33 | 0.745 | 0.956 | +0.072 | 0.763 | no | 0.321 | 0.016 |
| 53.59 | 0.704 | 0.713 | -0.050 | 0.477 | no | 0.224 | 0.343 |
| 201.01 | 0.231 | 0.871 | -0.500 | 0.101 | yes | 0.374 | 0.280 |
| 100.51 | 0.264 | 0.984 | -0.500 | 0.130 | yes | 0.289 | 0.329 |

Snare inter-onset histogram (units of the histogram mean): at the 67 pick (period 0.895 s)
the value at 2 periods is 24.57, at 1 period 3.97, at 3 periods 3.60; at 134 (0.448 s):
1p 5.09, 2p 3.97, 3p 6.64. The reference snare onsets repeat every 1.791 s (117 of 172
inter-onset intervals in the 1.75-1.80 s bin); the reference kick onsets sit at 0.448,
0.895 and 1.343 s. Map check: the Fourier pulse of the reference onsets is 134.00 with a
sharp peak (next candidate 184.3 far below); in every 10 s window the 268 component (the
8th-note grid, 2 x 134) stays at 0.91-1.00 and 134 at 0.28-1.00 over the full 302 s - the
tempo is constant, no free intro, no meter change. The map tempo is the real one.

Named cause: half-time backbeat. The snare plays every 4 beats of the true 134 (once per
1.791 s), so the snare recurrence sits at 2 periods of 67. The backbeat referee reads that
as "snare on 2 and 4" and credits 67 (+0.733) while penalizing 134 (-0.193); the half tempo
also holds a small comb edge (1.000 vs 0.971: the off-beat 8ths of the kick count as its
teeth). The (1 + backbeat) factor (1.73 vs 0.81) decides over the prior, which favors 134
(0.838 vs 0.994). The sub-beat evidence that 134 is the beat (quarter-beat share 0.284 at
67 vs 0.037 at 134: the kick fills the 0.448 s grid) is never consulted: the octave
referee by shares stays out whenever a snare is present. The referee cannot distinguish
"snare every 4 beats at T" from "snare every 2 beats at T/2" - that is the defect.

## Session C (map 94.5, detector suggests 126.0)

Per-candidate table:

| BPM | comb rel | sqrt prior | backbeat | score | gated | on | quarter |
|------|-------|-------|---------|-------|-----|------|------|
| 63.00 | 1.000 | 0.806 | +0.021 | 0.822 | no | 0.345 | 0.437 |
| 126.00 | 0.934 | 0.999 | +0.764 | 1.646 | no | 0.515 | 0.044 |
| 42.00 | 0.917 | 0.564 | -0.036 | 0.498 | no | 0.193 | 0.017 |
| 252.01 | 0.892 | 0.751 | -0.071 | 0.623 | no | 0.951 | 0.004 |
| 84.00 | 0.889 | 0.936 | -0.127 | 0.726 | no | 0.324 | 0.004 |
| 50.40 | 0.837 | 0.676 | -0.079 | 0.521 | no | 0.218 | 0.389 |
| 189.01 | 0.286 | 0.898 | -0.500 | 0.128 | yes | 0.358 | 0.435 |
| 94.50 | 0.330 | 0.971 | -0.500 | 0.160 | yes | 0.324 | 0.189 |

The map tempo 94.5 scores comb 0.330 of the best and is gated: it is not a candidate at
all. Snare inter-onset histogram: at 126 (0.476 s): 1p 4.77, 2p 31.67, 3p 3.69 - the snare
recurs every 0.952 s, i.e. every 2 beats of 126 (a textbook backbeat; the referee's +0.764
is correct). At 94.5 (0.635 s): 1p 0.23, 2p 0.22, 3p 31.90 - nothing on the 94.5 grid
itself. Map check: every reference inter-onset interval is an integer multiple of 0.238 s
(observed 1 to 5 steps), in every 10 s window across the full 230 s; the 0.238 s grid
component (252 BPM) stays at 0.83-1.00 everywhere - constant tempo, no free intro. Beat
units present on that grid: 0.476 s (126), 0.952 s (63), 0.714 s (84). The map's 0.635 s is
8/3 grid steps - not on the grid (94.5 = 3/4 x 126 = 3/2 x 63; matches the ticket's delta
31.5). No window is dominated by 94.5 (its best window score is 0.45 while the grid
component stays above 0.83), and the raw Fourier maximum over 50-250 BPM lands on 94.5
(0.260) with 157.5 (0.242) nearly equal - the signature of a 1.905 s accent period (whose
harmonic family is k x 31.5 BPM: 63, 94.5, 126, 157.5, 189), not of a beat; a true beat
does not show a 5/3 partner at the same strength.

Named cause: the map tempo is off the performed grid. The performance is metric and
constant on a 0.238 s grid whose beat units are 126/63/84; 94.5 has no support on it
(comb 0.33, gated). The detector's 126 is consistent with the audio, and the same
structure gives Session B its correct 126. The miss is in the reference-tempo anchor (the
raw Fourier maximum used by the check), not in the detector scoring: no honest scoring of
these hits can make 94.5 win without breaking Session B.

## Consequences for step 2

- Session A: fix the referee. When a snare is present, the backbeat verdict needs the
  sub-beat corroboration (or a cap on the (1 + backbeat) factor when the candidate's
  quarter-beat share shows the off-beat 8ths of a finer grid). Behind an analysis version
  switch, target: A suggests 134, B and C unchanged.
- Session C: the gate as written ("Detect Tempo equals the session map") is not reachable
  by detector changes - the map tempo has no on-grid support in the audio. The decision
  (correct the reference-tempo derivation, the session map, or the gate for C) belongs to
  the owner; step 2 for C should wait for it.
