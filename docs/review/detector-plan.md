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

Gate that can fail: `uv run python tools/eval/enst_tom_fp.py` already FAILs when the cause
counts do not sum to the total or when the `none` share is above 10 % (today: PASS, 0.6 %).
The work package adds two checks to it: the per-rule columns must sum to the flagged total,
and the left share of the hat cause must be at most 50 % (a proposed limit; today it is
78 %, so the added check FAILs until the hat cause is handled). The package also needs
`tools/eval/enst_gate.py` within 2 % of baseline on every aggregate, counted lines pasted.

## Open areas check (umbrella #8 list)

snare ghost notes (1), the kick gap (3), per-track self-calibration (2),
review queue ranking by confidence (4), role suggestion from audio (5), the
robustness smoke set (6), analysis speed (7), tom false positives (8)  -  all eight
covered above with file pointers and one failing-capable Gate each.
