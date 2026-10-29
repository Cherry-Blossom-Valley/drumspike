# Detector #8 step 0: baseline numbers and the work packages

Status: plan (ticket #73, step 0 of umbrella #8). No detector change is in this PR.

Decision number (umbrella #8): **ENST snare recall > 0.70 at precision >= 0.95.**
The standing gate is `tools/eval/enst_gate.py` (compares against
`tools/eval/enst_gate_baseline.json`, FAIL on any aggregate more than 2 % below it).

## Baseline

Committed baseline `tools/eval/enst_gate_baseline.json` (F1, +-50 ms window, config
`dsp`, git sha 629c16d5, 2026-10-25):

| drummer | kick | snare | hat | tom |
|---|---|---|---|---|
| drummer_1 | 0.849 | 0.808 | 0.938 | 0.730 |
| drummer_2 | 0.984 | 0.851 | 0.901 | 0.406 |
| drummer_3 | 0.982 | 0.820 | 0.897 | 0.392 |

Fresh gate run on main (this PR, counted lines in the PR description; config `dsp`,
210 items, detector fingerprint dadf4539aa159029): snare F1 0.827 / 0.867 / 0.820
(drummers 1/2/3) - every aggregate at or above its baseline, `rows compared: 12`,
`PASS`.

Snare against the decision number (P / R at +-50 ms, config `dsp`): drummer_1
0.987 / 0.712, drummer_2 0.980 / 0.778, drummer_3 0.976 / 0.707. Pooled over the
three drummers (ref 12510, output 9360, matched 9182): **P 0.981, R 0.734, F1 0.840**
- snare recall > 0.70 at precision >= 0.95 holds pooled and per drummer
(drummer_3 at 0.707 is the closest margin).

The real-session check (`tools/learn_eval.py baseline --samples
~/.cache/drumspike/sessions`) prints its counted lines in the PR description too.

## What the detector is today

One paragraph so the packages below have anchors: the analysis path is
`crates/core/src/analyze.rs` (`analyze_track`: decode -> `detect_events` ->
role filter -> dedup -> refine -> shift -> velocity). The detector is
`crates/core/src/detect/onset.rs` (SuperFlux flux on the log-filtered spectrogram of
`detect/features.rs`, resample via `detect/resample.rs`, median-based adaptive peak
picking, loudness/attack/shape gates, per-role `OnsetParams::for_role`). Session-level
post rules: `detect/attribution.rs` (cross-mic bleed/resonance, transfer-gain
calibration), `detect/tom_bleed.rs` (rule CD), `detect/cross_bleed.rs` (first-motion),
`detect/flam.rs` (second stroke). Review support: `crates/app/src/confidence.rs`
(`track_confidence`). Role suggestion: `crates/core/src/role.rs`
(`role_from_audio[_confidence]`), wired as the hint mode in `crates/app/src/tracks.rs`.

## The work packages

Ordered by expected effect on the snare number (the decision number). Each package has
a Gate that can fail.

### 1. Snare ghost notes: judge the soft gate on ENST, close the false-positive hole

What the code already does: the soft/ghost-note rescue in
`crates/core/src/detect/onset.rs` (`OnsetParams::soft_*`, the A1 probe notes at
`OnsetParams::for_role` for snare): a candidate below the level gate is kept when its
flux is a real rise and its spectral shape is within `soft_sim` of the loud-hit
template down to `soft_gate_db`. The kick/tom clearance (`soft_busy_peaks`,
`A1: the soft rescue was denied ... a kick or tom hit sits within 30 ms`) guards false
positives.

What is missing: the soft gate was tuned on the three real sessions, not scored per
role on ENST; the ENST gate only sees the hard threshold output. Wire-brush/wire-buzz
false hits match the loud-hit template at least as well as the ghosts do (the
a1-probe2 measurement, comment on `soft_sim`)  -  the template is the wrong reference
for those.

Work package: score the soft gate's per-role effect on ENST (hits the hard gate misses
vs false positives it adds) with a variant of `tools/eval/enst.py`; from that, retune
`soft_sim`/`soft_gate_db` for snare or add the missing discriminator (e.g. attack
p95, the a1-probe2 number, as a second gate condition).

Gate that can fail: `uv run python tools/eval/enst_gate.py` must keep every aggregate
within 2 % of baseline AND a new soft-gate report must show snare recall strictly above
the hard-gate snare recall at precision >= 0.95, printed with counted lines. Fails if
either number regresses.

### 2. Per-track self-calibration: derive the level reference from the track itself

What the code already does: `OnsetParams::for_role` is fixed per role (tuned on ENST,
phase2 P2-5); the loudness gate reads the track reference from the track's own 90th
percentile (`onset.rs`, loudness vs track reference). Session-level calibration exists
only for the bleed rules (`detect/attribution.rs`, transfer gains `G(B→A)`;
`detect/tom_bleed.rs`).

What is missing: nothing adapts the flux threshold/weak ratio per track. A very quiet
or a heavily compressed track (the real sessions vary) sits at a different working
point of the fixed thresholds.

Work package: a per-track calibration pass before detection  -  from the track's flux
distribution, scale the adaptive-threshold statistics and `weak_ratio`  -  behind an
`AnalyzeParams` switch, default off.

Gate that can fail: with the switch on, `tools/eval/enst_gate.py` prints every
aggregate within 2 % of baseline and the real-session check (learn_eval baseline)
prints snare and kick F1 not below the switch-off numbers, counted lines pasted; a
case with the switch off on a 12 dB quieter render of the same material must show the
gap between quiet and loud shrinking. Fails when an aggregate regresses.

Measured on ENST (#77, 2026-10-29): no flux statistic is stable enough to use, so the
switch is not built. Package 2 closes as "no code": the finding is this section.

- Probe 1, the 90th percentile of the flux per track (q90): snare spreads from 0.0002 to
  1.29 within the snare close mic. It cannot serve as the per-role nominal level.
- Probe 2, three candidate statistics per item at the original level and at a render
  12 dB quieter (gain 10^(-12/20)). The statistics are the 10th percentile (q10), the
  median (med) and the median of the 20 strongest local maxima (pk). Spread = max / min
  of the loud value over the hit lists (001-029) and the phrases (036-089), drummer_1:

  | mic | q10 | med | pk |
  |---|---|---|---|
  | kick | 0.00025 to 0.01632 (about 65x) | 0.00071 to 0.12824 (about 180x) | 0.01299 to 11.70 (about 900x) |
  | snare | 0.00000 for every item (degenerate) | 0 to 0.01507 (zero floor) | 0.00168 to 6.78 (about 4000x) |
  | hi-hat | 0 to 0.00116 (zero floor) | 0.00013 to 0.05655 (about 435x) | 0.00113 to 6.43 (about 5700x) |

  No statistic is below 4x in any mic. Drummers 2 and 3 were not tabulated; the
  decision does not depend on them, because drummer_1 already fails every statistic.
- Option (c) follows from the table: the per-track switch and its level references are
  dropped, and the code of the WIP commit 2a039af is reverted. The probes are not kept in
  the tree; these numbers are the record.

### 3. The kick gap: measure kick on the real sessions and close what the measurement names

What the code already does: kick on ENST is the strongest role (0.98 F1, drummers 2/3)
but the committed snare-adjacent number to watch is the pooled one; the kick values
were checked on the real sessions and the synthetic corpus (comment at
`OnsetParams::for_role`, line ~222). The real-session check of this PR prints kick
P/R/F per track.

What is missing: no per-track kick failure analysis exists on the real sessions (which
tracks lose recall, and to which cause: double strokes at 20 ms min-gap, low-tuned
kicks under the tom clearance, or the offset).

Work package: a kick miss-cause pass over the real-session check (reuse the
`reachable`/unmatched-list output of `learn_eval.py baseline` and the candidate list
`repick_candidates`), then the fix the causes name (candidate re-pick window or
clearance change), one cause at a time.

Gate that can fail: the report must attribute >= 90 % of the kick false negatives of
the three sessions to a named cause with counted lines, and after the fix
`tools/eval/enst_gate.py` keeps every kick aggregate within 2 % of baseline. Fails if
the attribution does not reach 90 % or an aggregate regresses.

#### The measured outcome (#76, PR #92, merged as 556d271, 2026-10-29)

The miss-cause pass (`tools/kick_miss_causes.py kick`, the counted lines of the #76 gate
chain at 20dfd9f) found no false negative to attribute: with the shape rescue in place the
kick has 0 false negatives at Sensitivity 50 on all three sessions - 0/302, 0/444, 0/386
used refs (offsets +6.2/+8.4/+2.6 ms). Kick F1 at the working point 0.925 / 0.999 / 0.999
(Session A/B/C; Sensitivity 100 0.637 / 0.796 / 0.782, oracle 0.959 / 1.000 / 1.000). The
kick gap is therefore Session A precision, not recall: the rescue's false keeps, 52 on the
sessions against 4 true keeps. A 0 FN run is a valid result, not a passed attribution, so
on 0 false negatives the tool prints `no kick FN at Sensitivity 50: nothing to attribute`
instead of an attributed percentage (#98).

The rescue probe of #76 labelled every rescue-worthy candidate (similarity >= 0.85,
-45 <= rel_db < -6) on the kick mics: 2871 true / 222 false keeps on the ENST items, 4
true / 52 false on the sessions. The true and false keeps overlap in rel_db, similarity
and attack on both corpora: no per-candidate cut on those three measurements separates
them (the counted distributions are in the #76 probe record). Package 3b probes the false
side on other dimensions.

#### The measured outcome (#98, the false-side probe, 2026-10-29)

The probe re-derived the #76 keep set exactly: 2871 true / 222 false keeps on the ENST
items, 0/49, 4/2, 0/1 true/false on Session A/B/C; every keep carries #76's own verdict.
The kick-band template rebuilt from the candidates reproduces the detector's own
`similarity` to max 5.9e-07 (service_tool `learn-eval` mode `xprobe`, one decode and
spectrogram per mic). Three new dimensions per keep: the A6 cross-role claim (the loudest
co-incident other-mic hit's band level minus the keep's kick-mic band level, 10 ms), the
competing template (the keep's kick-band flux shape vs every other track's template over
the same bands) and the inter-onset position (distance to the nearest snare/tom event at
Sensitivity 50, the containing interval, the position in it). A cut qualifies when it
keeps >= 99 % of the 2871 ENST true keeps and all 4 session true keeps. The counted best
cuts, per corpus pooled (the full per-threshold tables are in the probe record):

| Dimension (the removal rule) | ENST true kept | ENST false removed | Session true kept | Session false removed |
|---|---|---|---|---|
| xclaim: claimed by a louder co-incident hit (claim > 43.0 dB) | 2847/2871 (99.16 %) | 9/222 | 4/4 | 1/52 |
| comp: another track's template matches better (margin > -0.005) | 2843/2871 (99.02 %) | 3/222 | 4/4 | 1/52 |
| gap: far from the snare/tom pattern (d_near > 0.4 ms) | 2844/2871 (99.06 %) | 7/222 | 4/4 | 1/52 |
| gap: near a pattern hit (d_near < th) | no cut keeps the floor; the best keeps 2842/2871 and removes 3/222, 0/52 | | | |
| gap: inside a small inter-onset interval (gap < th) | no cut keeps the floor; the best keeps 2842/2871 and removes 3/222, 0/52 | | | |

No dimension separates the false side. The qualifying cuts remove at most 1 of the 52
session false keeps and 3-9 of the 222 ENST false keeps, and pay 24-28 ENST true keeps
for that. The false keeps sit where the true keeps sit on xclaim, comp and gap, as they
do on rel_db, similarity and attack (#76). The rescue's false side has no per-candidate
cut; package 3's remaining lever for the Session A precision gap is not a candidate
filter.

### 4. Review queue ranking: rank by the detector's own gate verdicts

What the code already does: `crates/app/src/confidence.rs`  -  `track_confidence`
returns `{confidence: [{c, doubt, why}]}` per hit, the page steps through doubtful
hits lowest confidence first. On the DSP path `c` is the tag cap x
`level_factor` (`level_factor`, `LEVEL_SPAN_DB = 24`): the level evidence alone (#281
D16  -  the activation margin moved to the detector-branch path).

What is missing: the detector already computes per-candidate gate verdicts and stores
them (`Candidate` in `onset.rs`, `detect_onsets_and_cands`, `repick_candidates`), but
`track_confidence` never reads them; a hit that passed only via the soft rescue is
doubtful for a different reason than one at the flux threshold, and the `why` line
cannot say which.

Work package: thread the stored candidate verdicts (gate flags of the hit's candidate)
into `confidence.rs` and extend `why` to name the gate; no detection change.

Gate that can fail: a `service_tool`-level test on a generated session asserts the
`why` string of a soft rescued hit names the soft gate and its `c` is below
`DOUBT_BELOW` while a loud hit's is not; the parity gate
`tools/parity/check_analyze.py` must stay byte-identical (no detection change). Fails
if either assertion fails.

### 5. Role suggestion from audio: score the hint mode on ENST and publish the threshold

What the code already does: `crates/core/src/role.rs`
(`role_features`, `role_from_audio`, `role_from_audio_confidence`, `role_threshold`);
the app uses it as the hint mode (`crates/app/src/tracks.rs`, "role_from_audio
(#393) for the audio suggestion"). An evaluation exists: `tools/eval/role_enst.py`
(#281 item 10 / #393) over the ENST mics via the `role-from-audio` binary.

What is missing: the score of `role_enst.py` is not connected to
`role_threshold`  -  the suggested threshold is not the measured one, and the confusion
(presented role for each wrong mic) is not in any doc of record.

Work package: run `role_enst.py`, set `role_threshold` from its measured
precision/recall tradeoff, and record the confusion matrix in this doc's successor.

Gate that can fail: `uv run python tools/eval/role_enst.py` must print per-role
accuracy and FAIL if any close-mic role's accuracy drops below the measured floor of
the run that set the threshold (the number goes into the check). Fails below the floor.

#### The measured score (#78, the run of 2026-10-29, linux-arm64, build of a7b3936)

1538 mic files, the thresholds confirmed (`kick` 0.05, `tom` 0.60, snare/hat/cymbal
no bar). At the operating point: overall precision 92.6 % (398/430 suggested),
coverage 28.0 %. Per suggested role: kick 94.4 % (203/215), tom 90.7 % (195/215);
snare, hat and cymbal are never suggested. Per-drummer precision 95.1 % / 88.5 % /
94.8 % (drummer_1/2/3). The recorded floors of the gate (`ROLE_FLOORS` in
`tools/eval/role_enst.py`): kick 203/215, tom 195/215.

The coverage floor: the #393 gate's 30 % does not hold on this host's run of the
#393 thresholds. role_enst.py was run at the #393 decision build (3da1e63, the
0.9.4 import -- `role.rs` and `role_threshold` are unchanged since) and at main
alike, and both print the same line: precision 92.6 %, coverage 28.0 %. No later
detector change lowered the coverage; the 30 % line never held. The gate keeps a
coverage check at the measured 28.0 % (`PASS_COVERAGE`, a FAIL under it), so a
future detector change that trades coverage away still fails the gate.

Confusion matrix at the operating point (rows: the mic's true role, columns: the
suggested role; `none` = no suggestion):

| true \ suggested | kick | tom | none |
|---|---|---|---|
| kick (210)  | 203 | 3   | 4   |
| snare (210) |     | 2   | 208 |
| tom (488)   | 12  | 195 | 281 |
| hat (210)   |     |     | 210 |
| cymbal (420)|     | 15  | 405 |

The confusion is one-directional: 12 true tom files suggest kick and 15 true cymbal
files suggest tom (the wash reads as a tom ring); a snare never draws a suggestion
beyond 2 tom votes.

### 6. Robustness smoke set: keep the synthesised cases and add the real-recorded ones

What the code already does: `crates/core/tests/robustness.rs` (#281 C14)  -  every
signal synthesised in-test, run through the public `analyze_track` with
`Detector::Dsp`; cases include brush, rods, electronic kit (`Voice::Brush`,
`Voice::ElectronicKick/Snare`, `Voice::Rods`), `quiet-50`, `clipped` at 0 dBFS, 48 kHz
sources; each case prints a table row (matched within 1 ms and within the case band,
runtime) and asserts the band from `BANDS`. Runs as part of `cargo test
--workspace` in CI.

What is missing: real recorded material in the named classes (brushes, electronic kit,
clipped and very quiet real files, a 20-minute take)  -  the bands were validated on
synthesis only; the finding report the test header names
(`docs/migration/reports/c14-robustness.md`) does not exist in this tree.

Work package: record/collect the six real-material cases as named host fixtures (same
read-only rule as the sessions), run them through `analyze_track` in an ignored test,
and write the report; bands from synthesis stay the regression gate.

Gate that can fail: the report must print the counted table rows for all six cases
with a stated pass/fail per band; the ignored test fails (or the gate refuses) when a
case has no result row. Fails when a real case breaches its band without a finding.

### 7. Analysis speed: a standing budget gate

What the code already does: features/frames are rayon-parallel and bit-stable
(`detect/features.rs`); `tools/learn_eval.py timing` measures
`track_learn_scores` cold/warm on one 6-minute track through `service_tool run`
(`cmd_timing`); the robustness test prints per-case runtime but asserts no budget.

What is missing: no standing speed budget anywhere  -  a regression in the spectrogram
or peak-picking path ships unnoticed.

Work package: a speed gate  -  extend the robustness run (or a new ignored test) to
assert analysis throughput of a reference render is not worse than a recorded budget
(e.g. the learn_eval timing number fixed as a constant), with the host named in the
assertion message.

Gate that can fail: the ignored budget test FAILs when the reference render's
wall-clock per minute of audio exceeds the recorded budget by > 10 %; the gate
(`cargo test -p drumspike-core --test robustness -- --ignored`) runs it and prints the
measured throughput. Fails over budget.

### 8. Tom false positives: the cause of each one and the rule that should remove it

What the code already does: four session-level rules act on the tom close mics.
Rule CD (`detect/tom_bleed.rs`, versions 1 to 5) marks a tom hit as bleed when its
tom-band excess over a played other-drum mic is under 9.2 dB; version 2 adds the kick
and snare close mics as sources (9.2 dB snare, 4.0 dB kick); version 5 keeps a shared
kick source under 8.0 dB. The pair rule (`tom_pairs`, versions 3 and 4) keeps the
louder of two output hits on different drums within 12 ms. The cross-track rule
(`detect/cross_bleed.rs`, A6) judges each tom hit by first-motion lead and level
against the kick, snare, hi-hat and other tom mics. The transfer gains the rules use
are in `detect/attribution.rs`.

Measured (`tools/eval/enst_tom_fp.py`, config `dsp`, build a12badbc565958ae, 210
items, +-50 ms, counted on the release run of #90): 6232 scored tom false positives
over the three drummers (583 / 2432 / 3217). The nearest annotation of another drum
gives the cause; "removed" counts the hits the rules flagged (`bleed` or `cut`) that
sit at that cause, "left" the scored FPs:

| cause | left | share of FPs | removed | left share of (left + removed) |
|---|---|---|---|---|
| kick | 1708 | 27.4 % | 1517 | 53 % |
| snare | 1398 | 22.4 % | 803 | 64 % |
| hat | 2423 | 38.9 % | 668 | 78 % |
| cymbal | 443 | 7.1 % | 324 | 58 % |
| tom-other | 223 | 3.6 % | 312 | 42 % |
| none | 37 | 0.6 % | 7 | 84 % |

The shares sum to 100 %, the cause counts to 6232 (the script's sum check). The worst
mic is drummer_3 tom_2 (P 0.185 at R 0.976, 1121 output hits).

Counted per drummer and cause (scored FPs, all tom mics; shares of the drummer's total,
from the run log of #90):

| drummer | kick | snare | hat | cymbal | tom-other | none | total | shares % (kick / snare / hat / cymbal / tom-other / none) |
|---|---|---|---|---|---|---|---|---|
| drummer_1 | 75 | 234 | 199 | 37 | 36 | 2 | 583 | 12.9 / 40.1 / 34.1 / 6.3 / 6.2 / 0.3 |
| drummer_2 | 789 | 490 | 951 | 177 | 13 | 12 | 2432 | 32.4 / 20.1 / 39.1 / 7.3 / 0.5 / 0.5 |
| drummer_3 | 844 | 674 | 1273 | 229 | 174 | 23 | 3217 | 26.2 / 21.0 / 39.6 / 7.1 / 5.4 / 0.7 |
| all | 1708 | 1398 | 2423 | 443 | 223 | 37 | 6232 | 27.4 / 22.4 / 38.9 / 7.1 / 3.6 / 0.6 |

Counted per drummer and tom mic (rows: the mic; the cause columns as above):

| drummer | mic | kick | snare | hat | cymbal | tom-other | none | total |
|---|---|---|---|---|---|---|---|---|
| drummer_1 | tom_1 | 27 | 200 | 131 | 21 | 21 | 1 | 401 |
| drummer_1 | tom_2 | 48 | 34 | 68 | 16 | 15 | 1 | 182 |
| drummer_2 | tom_1 | 89 | 377 | 431 | 72 | 2 | 3 | 974 |
| drummer_2 | tom_2 | 700 | 113 | 520 | 105 | 11 | 9 | 1458 |
| drummer_3 | tom_1 | 56 | 329 | 406 | 30 | 45 | 4 | 870 |
| drummer_3 | tom_2 | 190 | 195 | 395 | 46 | 94 | 4 | 924 |
| drummer_3 | tom_3 | 598 | 150 | 472 | 153 | 35 | 15 | 1423 |

Split by drummer: hat is the largest cause for drummers 2 and 3 (39.1 % and 39.6 %), so the
hat-first step fits them. Drummer_1 is the exception: its largest cause is snare (40.1 %),
with hat second (34.1 %). On drummer_2 tom_2, kick is the largest cause (700 of 1458).

Two fingerprints appear in the run log. Build a12badbc565958ae is the build of the run files
that the tables above are counted on; detector fingerprint dadf4539aa159029 is the one the
gate scoring printed for the same 210 items. Flagged hits on the
own tom annotations: 25 (real strokes the rules removed).

What the table does not say: which rule flagged each removed hit. The output carries
`bleed` and `cut` only, so the share each rule removes is not measured yet. The
reasons below are the rules' stated design, not yet tested:

- Hat (39 % of the FPs, 78 % left): rule CD and version 2 measure levels in the tom band
  (50 to 400 Hz); a hi-hat has little energy there, so its excess over the tom mic is
  not low. The A6 first-motion test is the rule that should catch it, and hat strokes
  are bright and weak, so the lead test may not separate them.
- Kick and snare (50 % of the FPs): version 2 covers them, but the kick threshold of
  4.0 dB is set against real accents (`tom_bleed.rs` header). Bleed above the threshold
  is caught; hypothesis, not measured: the remainder may sit at the threshold.
- Cymbal (7 %): the overheads are not in the source list of A6 (kick, snare, hi-hat,
  other toms). The 324 removed cymbal hits show some are caught anyway, so the source
  list is the question to test, not the whole answer.

Work package: add the rule that set each flag to `enst_tom_fp.py` (a column per rule: CD,
v2 kick, v2 snare, v3 pair, v5 shared kick, A6), from the per-hit evidence the run files
carry (if they do not, the evidence goes into the run output first). Then take the hat
cause, the largest pooled one (the largest for drummers 2 and 3; drummer_1's largest is
snare, see the split above), and decide on the measured table: add hi-hat and overheads as
sources of A6 with a threshold set on ENST, or record why the rule cannot separate them.
No detector change in this package before the table is in the report.

Step 1 (#96), measured per rule (`enst_tom_fp.py`, the same 210 items; the run is tool build
1421cb996b4076ac on the run files of detector fingerprint e91751e0e9208064, the merged tree
of PR #100 at bd2ef53, log `results/96-merged.result.log`; reporting only: the flags and every
score stay the same as main; the gate rows of the tree and of main's build agree to 3 decimals).
Rule names come from the core rules' own marks (`TomBleed::by`): CD, v2-kick, v2-snare, v5-kick, and `pair` for the v3
pair rule. A hit can have several rules; the first-rule table uses the fixed order CD >
v2-kick > v2-snare > pair > v5-kick:

| drummer | CD | v2-kick | v2-snare | pair | v5-kick | cut | unattributed | sum | flagged total |
|---|---|---|---|---|---|---|---|---|---|
| drummer_1 | 51 | 9 | 120 | 3 | 44 | 0 | 0 | 227 | 227 |
| drummer_2 | 66 | 149 | 110 | 1 | 520 | 0 | 0 | 846 | 846 |
| drummer_3 | 568 | 477 | 767 | 1 | 770 | 0 | 0 | 2583 | 2583 |

The per-rule sums equal the flagged totals (the added check of this package passes on the
first-rule view). A6 dropped 447 / 935 / 1413 candidates; A6 survived to the CD step 0 / 0 / 0,
so the A6 overwrite question (#97) closes on this measurement.

Hat cause, the FPs left: 2423 of the 6232 scored. The left share is left / (left + removed) of
the hat cause = 2423 / (2423 + 668) = 78.4 % (limit 50 %, information only in this step). A6
keys exist on n=37 of the 2423 hat FPs; the other 2386 have no coinciding A6 hit, by design, so
an A6 threshold cannot remove them, and step 2 starts from that. On the 37: the A6 lead (ms)
quartiles p25 / p50 / p75 are -0.11 / 0.07 / 0.20 and the A6 level (dB against the coinciding
source hit) quartiles are -21.22 / -19.75 / -18.27. Whether a threshold separates these 37 from
the real hat strokes is not measured yet; step 2 tests it against the own annotations.

Gate that can fail: `uv run python tools/eval/enst_tom_fp.py` already FAILs when the cause
counts do not sum to the total or when the `none` share is above 10 % (today: PASS, 0.6 %).
The work package adds two checks to it: the per-rule columns must sum to the flagged total,
and the left share of the hat cause must be at most 50 % (a proposed limit; today it is
78.4 %, so the added check FAILs until the hat cause is handled). The package also needs
`tools/eval/enst_gate.py` within 2 % of baseline on every aggregate, counted lines pasted.

Step 1 (#103), why A6 sees no coinciding hat hit, counted per drummer (`enst_tom_fp.py`,
tool build 766c3399db3ee97b, the same 210 items, run files of detector fingerprint
664e695d29b46ef2): 2368 hat FPs left, 2334 of them without an A6 coinciding hit (no
`a6_lead_ms`; A6 judged 34 and did not drop them). The hat mic's own detected hits near
each such FP (the cause window +-50 ms, the A6 coincide window 10 ms, from the run files):

| drummer | no hat-mic hit | outside the A6 window | present, not a source | total |
|---|---|---|---|---|
| drummer_1 | 5 | 28 | 152 | 185 |
| drummer_2 | 36 | 151 | 726 | 913 |
| drummer_3 | 208 | 290 | 738 | 1236 |
| all | 249 | 469 | 1616 | 2334 |

The third column is the surprise: 1616 hat FPs have a hat-mic detected hit within the A6
coincide window (2697 of the 7861 scored tom events without A6 keys overall, clustered in
162 of the 210 items), yet the run file carries no A6 evidence on them. The hat mic IS a
source in the rule's run: in the sampled item (a drummer_1 disco phrase, sticks)
the dropped candidates coincide with hat hits within 2.5 to 4.9 ms and carry lead/ratio. So
the evidence exists inside the rule but does not reach the stored events of the kept hits.
Suspected cause (code read, not yet measured): after the A6 re-detection, the kept hits'
keys are matched to the stored events by an exact time (`(t - e.t).abs() < 1e-3` in
`crates/app/src/cross_bleed.rs`), while `analyze_detected` re-refines the times before the
store, so the exact match misses. This is a reporting-path defect first: the hat FPs may be
reachable by A6 after all, which changes step 2 (a wider window or candidate-source change
may not be needed; the evidence repair is). Step 2 starts here: repair the key path, re-run
the table, and only then decide the fix the remaining counts name.

Step 2 (#103), the measured verdict. The repair is confirmed
(`crates/app/src/cross_bleed.rs`: the kept-keys match now takes the nearest kept hit within
25 ms instead of an exact time; hat FPs without A6 keys went from 2334 to 35) and the hat
pair's own keys are recorded on every hit with a coinciding hat hit
(`crates/core/src/detect/cross_bleed.rs`, `a6_hat_lead_ms`/`a6_hat_level_db`), because the
stored `lead_ms`/`ratio_db` are the loudest source's, not the hat's - the -32/-10 pair
calibrated on the stored keys fired on the wrong distribution (the stored-key sweep
predicted 857 drops; the hat-pair truth is 433). The sweep on the hat-pair
keys (build of the 103-hatcal job, 210 ENST items; 1269 of 2236 hat-cause scored FPs and
160 real strokes carry hat-pair keys; hat FPs p25/50/75 level -22.3/-14.8/-7.3 dB, lead
-7.4/-5.1/-2.9 ms; real strokes p50 -6.6 dB): the distributions overlap. Best pair
(level <= -20 dB, lead >= -12 ms): 398 hat FPs removed, 18 real strokes lost, left-share
76.4 % -> 62.8 % (the limit is 50 %); at level <= -26 the removals fall to 197 for 12 real
strokes; nothing at <= -32 fires. No pair reaches 50 % without a tom recall loss the gate
would not accept, so no hat verdict fires: the recording stays, the rule does not. The
verdict pair is a measured dead end on this evidence; a different rule (band energy, not
the A6 coincidence form) is the remaining option and is out of this package's scope.

## Open areas check (umbrella #8 list)

snare ghost notes (1), the kick gap (3), per-track self-calibration (2),
review queue ranking by confidence (4), role suggestion from audio (5), the
robustness smoke set (6), analysis speed (7), tom false positives (8)  -  all eight
covered above with file pointers and one failing-capable Gate each.
