# Test campaign 0.9.6: STAR Drums full-kit and mix case (#119)

Agent: hermes@arm64-mac. Build: main at v0.9.6 (release, `service_tool` / `role-from-audio`).
Data: STAR Drums test split on the host (internal only, CC BY-NC-SA). 31 songs with a
re-synthesized full-kit stem and 18-class annotations; 26 of them also have a mix with music.
Method: the full-kit stem and the mix were each analyzed as one project with five role tracks
(kick, snare, tom, hat, cymbal) over the same audio — the DSP detector runs once per role, as it
would on five mics. Class mapping: BD→kick; SD, SS→snare; CHH, OHH, PHH→hat; HT, MT, LT→tom;
RD, CRC, RB, SPC, CHC→cymbal; CL, CLP, TB, CB→other (counted, not scored). Scoring: one-to-one
nearest matching (mir_eval.onset semantics) at ±20 ms, pooled over songs.

## Check 1 — full-kit stem, per role (PASS)

| role | precision | recall | F1 | ref hits | detected |
|---|---|---|---|---|---|
| kick | 0.67 | 0.95 | 0.79 | 7937 | 11226 |
| snare | 0.39 | 0.81 | 0.53 | 5259 | 10870 |
| tom | 0.14 | 0.77 | 0.24 | 1656 | 8965 |
| hat | 0.63 | 0.51 | 0.56 | 15647 | 12635 |
| cymbal | 0.17 | 0.71 | 0.27 | 2694 | 11301 |

Recall is decent everywhere; precision is the problem — the per-role detectors each fire on the
other drums' hits, so tom and cymbal carry 5–7× their reference count.

## Check 2 — mix with music, drop per role (PASS, on 26/31 songs)

| role | P (mix) | R (mix) | F1 (mix) | ΔF1 vs check 1 |
|---|---|---|---|---|
| kick | 0.25 | 0.84 | 0.39 | −0.40 |
| snare | 0.37 | 0.40 | 0.39 | −0.14 |
| tom | 0.11 | 0.43 | 0.18 | −0.06 |
| hat | 0.48 | 0.38 | 0.42 | −0.14 |
| cymbal | 0.10 | 0.52 | 0.17 | −0.10 |

The big drop is kick precision (0.67 → 0.25): bass-heavy music triggers the kick detector —
detected kick events more than triple (11k → 19k over the subset). Snare recall halves on the mix.

## Check 3 — role suggestion on the full-kit stem (PASS)

0/31 songs get an overhead-style suggestion. 21/31 get no suggestion at all (none), 9 get tom,
1 gets kick. Nothing suggests overhead or room-like roles; when the classifier answers at all it
names a close-drum role. Median confidence 0.40, range 0.05–0.96.

## Check 4 — Detect Tempo vs the song MIDI (FAIL)

Counted misses beyond 1 BPM: 30 of 31 songs. But every MIDI file in the test split carries the
default 120 BPM and nothing else — the reference is a placeholder, not the songs' tempo. The
detected tempos are plausible and self-consistent (repeated kits give repeated values); the
check fails against the reference as specified, and the reference itself is the finding.

## Check 5 — is the full-kit case a product case?

What works: recall on a single full-kit stem is usable (kick 0.95, snare 0.81, tom 0.77,
cymbal 0.71 at ±20 ms), so a user dumping one overhead-style stem does get most hits into the
grid, and tempo detection produces stable, plausible values. What does not: the per-role
detectors do not classify — they each over-fire on the whole kit, so the roles are badly
separated (tom and cymbal precision below 0.2), and adding music collapses kick precision to
0.25. The role suggestion never names an overhead-style role, so the app would put such a track
under the wrong role by default. A user would need: a mode that detects once and assigns a
class per hit (rather than five role-parameterized passes), and a role suggestion that can
answer "overhead" for a full-kit stem. Without that, the full-kit import produces a grid that
is mostly right in time and mostly wrong in lane.

## Findings

- S2 wrong result, detection: role separation on a full-kit stem is poor — tom precision 0.14
  (8965 detected vs 1656 ref), cymbal 0.17, snare 0.39. Repro: import a full-kit stem, analyze,
  look at the lanes: hits appear in every lane at roughly the same times.
- S2 wrong result, detection (mix): kick precision drops 0.67 → 0.25 with music (11k → 19k
  detected over the 26-song subset); bass content fires the kick detector.
- S2 wrong result, role suggestion: the full-kit stem never gets an overhead-style role
  (21 none, 9 tom, 1 kick over 31 songs).
- S2 wrong result, data: every STAR test-split MIDI tempo is the constant default 120 BPM, so
  Detect Tempo has no usable reference here (30/31 differ by more than 1 BPM).
- S3 friction, data: 5 of the 31 test-split songs have no mix file; the mix check covers 26.
- S3 friction, role suggestion: median confidence 0.40 with several near-zero values while the
  suggestion is still applied (one checked song: confidence 0.06, no role).

## Ideas

- Detect once per hit and classify into roles (a multi-class head), instead of one
  role-parameterized pass per track; this is the single change that fixes checks 1 and 2.
- Let the role suggestion answer an overhead/room-class role, and gate low-confidence
  suggestions behind an explicit "unassigned" state.
- For tempo evaluation, derive a real reference (beat annotations or per-song metadata), not
  the MIDI header.

Counted findings: 6.
