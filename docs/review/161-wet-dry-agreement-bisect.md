# #161 step 1: the Detect Tempo wet/dry agreement bisect — no code regression, a metric artefact

Question (#161, split from #130): Detect Tempo agrees between the wet mix (all mics) and the
dry mix (close mics) of the same ENST item on 7 of 135 items on main 47ef2c8, but #118
measured 75 of 135 on 0.9.6 "with the same identity metric". Step 1 re-measured the wet/dry
agreement on release builds of v0.9.6 (c5f6c4a), a60cd8f (#127 refine version 5), v0.9.7
(54dac74) and 47ef2c8 (main): same 135 phrase items, same enst.py index (git diff empty
c5f6c4a..main), service_tool `cli run` with one mix track on the kick role, Analyze All,
`tempo_map_detect`, the identity metric on the suggestion (refusals reported separately).
Measurement only; no code change. Both mix rules: average (the #130 script rule) and plain
sum (the #118 check-5 wording), 8 passes.

## The four counted lines

Per build, the identity metric at two precisions, average mix:

| build | exact 1e-6 (the #130/#161 script) | app resolution (see below) | refusals | unstable | octave flips |
|---|---|---|---|---|---|
| v0.9.6 c5f6c4a | 6/135 | 75/135 (75 of 122 judged) | 13 | 47 | 7 |
| a60cd8f | 6/135 | 75/135 (75 of 122 judged) | 13 | 47 | 7 |
| v0.9.7 54dac74 | 7/135 | 75/135 (75 of 122 judged) | 13 | 47 | 7 |
| main 47ef2c8 | 7/135 | 75/135 (75 of 122 judged) | 13 | 47 | 7 |

Plain sum instead of average: 5/5/7/7 at 1e-6 and 81/135 (81 of 126 judged, 9 refusals,
45 unstable, 7 flips) at app resolution on every build. The #118 check-5 line — 75/135
stable, 13 refusals, 47 unstable, 7 pure octave flips — is the average-mix row, and it is
identical on all four builds.

"App resolution" means: the suggestion maps identical AND the start BPM equal at whole-BPM
precision. The detector itself works at that resolution: all 2144 suggestion map values in
the measured rows are whole BPM (example from a v0.9.6 row: start.bpm 86.5012 with error
bar 0.39, suggestion.bpm 87.0), and the #118 counted line reproduces exactly under it.

## The difference is the metric, not the code

- v0.9.6 does not give 75 under today's script — confirmed: 6/135 (average), 5/135 (sum)
  at 1e-6. The difference to #118 is neither the items (same 135, same index) nor the mix
  rule (average vs sum moves the exact count by 1-2 and the app-resolution count by 6).
- The #118 numbers reproduce exactly on the stored v0.9.6 rows when the comparison runs at
  the app's own resolution. The #118 script is not on this host, so its wording ("all 8
  mics summed") cannot be checked against its code; its counted line matches the average
  rule, not the plain sum (81, not 75).
- The 1e-6 comparison is about four orders finer than the detector's own resolution. At
  that precision, float noise alone decides: between the v0.9.6 and the v0.9.7 build
  (the only dependency change is the tokio pin bump #137), 101 of 270 analyses drift their
  float start BPM by ~1e-4, and the 1e-6 metric counts every one of them as unstable while
  every app-visible map stays identical. The same drift is what puts the exact count at
  5-7 instead of 75: two analyses of the same item differ by sub-resolution noise.

## First bad commit: none

- c5f6c4a -> a60cd8f (docs, CI, the #127 placement cap): per-item rows identical to v0.9.6
  (122 of 135 exactly equal; the 13 remaining differ only in the random session id inside
  refusal blobs). The refine-v5 placement change does not feed the tempo path.
- a60cd8f -> 54dac74 (release chores, tokio pin): the ~1e-4 float drift above; one analysis
  changes its quantized map (item 069 dry, 33 -> 144 BPM — a borderline item that disagrees
  wet/dry either way). Counted lines: 6 -> 7 at 1e-6, 75 -> 75 at app resolution.
- 54dac74 -> 47ef2c8 (docs/lints #143, edition 2024 #147, both documented as no behaviour
  change): per-item rows identical to v0.9.7 (13 diffs are session ids again).
- Named cause of the 75 -> 7 delta between the #118 and #130 reports: the metric change
  between the two scripts (app-resolution equality in #118, 1e-6 float equality in #130).
  A measurement artefact. Not the refinement times feeding the tempo detector, not an
  edition 2024 semantic change, not RPIT capture.

## Which number is right

75 of 122 judged (13 refusals) on the average mix — the #118 number — for the question the
ticket asks: does Detect Tempo give the same map for the wet and the dry mix. The applied
map is quantized to whole BPM and the start estimate carries a ~0.4 BPM error bar, so a
1e-4 BPM disagreement is invisible in the app. The 5-7/135 of the 1e-6 metric measures the
float-noise sensitivity of the comparison, not user-visible disagreement. The wet/dry
agreement is unchanged from v0.9.6 through main 47ef2c8.

## Consequence for #130 / #161 step 2

The #130 gate line "wet/dry stable count on the fix build >= the v0.9.6 count" holds at
75 vs 75 under the app-resolution metric; the 7/135 in the #130 gate3 note is the same
artefact. There is no wet/dry regression to fix, so step 2 (a fix behind the analysis
version switch) has no subject; that decision is the coordinator's.

## Probe: the PCM_16 clip is not the cause

The measurement scripts write the mix WAV as PCM_16, which clips a plain sum above 1.0
(measured ENST sums reach ~2.0 peak). Re-run on v0.9.6 with the sum written as float32
(no clip): 5/135 at 1e-6, 79/125 judged at app resolution, 10 refusals. The clip does not
change the picture.

## Evidence

- Measured on the host: release service_tool per build (warm target dir), 135 ENST phrase
  items, 4 builds x 2 mix rules plus the float probe, resumable per-item rows
  (wetdry-<tag>.jsonl) and the RC_/counted lines in the agent state dir; the run script is
  161-bisect.sh next to them.
- Per-item row diffs and the metric sweep: rescore.py, peritem_diff.py, diff_097.py,
  breakdown_097.py, diff_main.py, sweep_metric.py, coarse_all.py in the same dir.
- check_private.py --text-file on this report: rc=0 (host word list sourced).
