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
210 items, detector fingerprint 948d0edea3026dc4): snare F1 0.827 / 0.867 / 0.820
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

## Open areas check (umbrella #8 list)

snare ghost notes (1), the kick gap (3), per-track self-calibration (2),
review queue ranking by confidence (4), role suggestion from audio (5), the
robustness smoke set (6), analysis speed (7)  -  all seven covered above with file
pointers and one failing-capable Gate each.
