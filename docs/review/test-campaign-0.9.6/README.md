# Test campaign 0.9.6: summary

The 0.9.6 test and learning phase ran on 2026-10-30 with four agents and the coordinator. It tested the
release on the owner's three sessions (Session A, B, C), ENST (drummer_1 to drummer_3, all mixes) and
STAR Drums (test split). All data is internal; this page holds aggregates only. Each check has its own
report next to this file; the follow-up reports and reviews are linked below.

## Results per report

| Report | Verdict | Main numbers |
|---|---|---|
| [116 detection, Learn, review queue](116-detection-learn-review-queue.md) | 5 PASS, 1 FAIL (tempo) | F1 at ±20 ms pooled: kick 0.977, snare 0.898, tom 0.887. Learn This: 21 of 21 marked hits recovered. Review queue: 130 of 2413 kept hits queued, 0 wrong reasons. Detect Tempo: Session B right, Session A half tempo, Session C 126 vs map 94.5. |
| [117 app flows](117-app-flows.md) | 4 PASS, 2 FAIL | Open, export and reopen: 0 of 351 hits differ. Placement at refine version 5: 387 of 387 hits within 0 to +3 samples of the foot. 0 UI freezes over 500 ms. Undo not exact in 4 of 4 steps; keys Z and Shift+Z do nothing. |
| [118 ENST stress](118-enst-stress-all.md) | gate PASS, 4 S2 findings | Gate: 12 of 12 rows at or above the baseline, 6 improved. Weakest: drummer_3 tom F1 0.41 to 0.56 (bleed). Detect Tempo wet vs dry mix stable on 75 of 135 items. |
| [119 STAR full kit and mix](119-star-full-kit-mix.md) | 3 PASS, 1 FAIL (tempo data) | Full-kit recall fine, role separation poor (tom precision 0.14). With music, kick precision 0.67 to 0.25. The STAR MIDI tempo is a constant 120 placeholder, so its tempo check cannot pass. |
| [136 ENST timing](../136-enst-timing.md) | step 1 done | drummer_1 kick recall at ±20 ms is 0.710: 933 of 938 misses have no kick-path detection within ±50 ms, so the onset pick misses them (not the refinement, not late hits). The late tail is a real lag, largest on hat (p90 12.8 to 15.4 ms). |

## What changed during the campaign

- Hit placement: 0.9.6 moved some snare hits too far ahead (184 of 882 snare hits more than 3 samples).
  Refine version 5 caps the move at 3 samples (#127); released in 0.9.7.
- tokio updated past RUSTSEC-2026-0386 (#133), released in 0.9.7.
- CI runs only what a change needs (#125); a release commit runs no tests again.
- Engineering hardening: R1 (#133), R3 (#135) and R4 edition 2024 (#141, ENST detections identical) merged.

## Open defects from the campaign

| Ticket | Severity | Area |
|---|---|---|
| #130 | S2 | Detect Tempo: half tempo on Session A (fix in progress) |
| #144 | owner decision | Session C reference tempo (the map is off the played grid) |
| #150 | S2 | Tempo and Meter rulers show no value while playing (owner report) |
| #151 | S2 | MIDI export silently uses 120 BPM when no tempo is set (owner report) |
| #154 | S2 | Undo and redo do not restore the exact state |
| #155 | S2 | Keys Z and Shift+Z do not undo and redo |
| #156 | S3 | A failed layer render blocks every later render of that track |
| #142 | S3 | Media compose tests fail instead of skip on a host with part of the media |

Not filed yet, waiting for the owner's priority: tom bleed on drummer_3 (#118), the kick-path pick on
drummer_1 (#136 step 2), role separation and mix precision on full-kit material (#119), tempo stability
between wet and dry mixes (#118), and the hat placement lag (#118, #136).

## Reviews written in the same phase

- [Engineering review](../engineering-review.md) (#121): plan R1 to R10. Done: R1, R3, R4. Waiting for the
  owner: R2 (needs a workflow change) and R5 to R10.
- [AI, ML and agentic ideation](../ai-ideation.md) (#122): top 5 are review-queue ranking with reasons,
  a tom bleed guard, detect once and classify per hit, a "check my session" agent report, and
  cross-project Learn.
- [Native UI and ARA](../native-ui-ara.md) (#123): no UI rewrite (the web UI has headroom); a native
  waveform path is the middle way; ARA starts with VST3/CLAP in REAPER, Pro Tools (AAX) second, with a
  first spike that has pass/fail criteria.

## Lessons for the process

- A report must make its numbers add up and say which metric it uses; #136 needed one round because a
  split did not sum and an F1 was read as a recall.
- A dev host's permission mode can stop a ticket half-way (#117 moved hosts); data access belongs in the
  ticket before assignment.
- A Cargo.lock change must regenerate THIRD_PARTY.md; only the macOS CI caught it.
- The release build needs the owner's Mac unlocked: the notary profile is unreadable while it is locked.
