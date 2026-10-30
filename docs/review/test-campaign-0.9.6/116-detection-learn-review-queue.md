# Test campaign 0.9.6: detection, Learn and review queue on Session A-C (defaults: Sensitivity 50, refine rule 4)

Build: main at v0.9.6 (c5f6c4a), release service_tool, fp c3bbb4383063ce33. Data: the shared data folder on this host, Session A/B/C, read-only. Per-file tables stay in the results dir on the host. The reference hits are identical to the #101 cached derivation (checked True), so the F1 comparison is valid. Checks 3-5 ran on the corrected harness (30 ms mark reach, tag caps, whole-number check on the suggested map); the earlier checks345 job had loaded pre-correction versions of those scripts.

1. Analyze All F1 at +-20 ms, pooled over the three sessions (tp/fp/fn): kick 1131/52/1, F1 0.9771, floor 0.9571 = #101 0.9771 - 0.02, PASS. snare 727/155/11, F1 0.8975, floor 0.8775, PASS. tom 266/49/19, F1 0.8867, floor 0.8667, PASS. All three F1 equal the #101 session table to 4 decimals. At +-50 ms kick improves to 1132/51/0 (F1 0.9780); snare and tom unchanged.
2. Hit placement, rule 4 vs rule 3, matched 1:1 within 10 ms, shift in samples: kick n=1183, all moved earlier (-1 to -6); snare n=882, 33 unchanged, the rest earlier (-1 to -12); tom n=315, 8 unchanged, the rest earlier (-1 to -12). Hits moved later = 0. PASS
3. Learn This / Forget This on the kick and snare mics of the three sessions: Learn This 10 scenarios, 21 marked missed hits, 21 of them in the output after (30 ms mark reach), 16 hits added, 0 added without a reference. Forget This 30 scenarios, 90 marked unwanted hits, 90 of them out after, 90 hits removed, 0 removed that match a reference. PASS
4. Review queue, counted over the kept hits of the three sessions: kept 2413, queued (doubtful) 130; queued reasons unreadable 0; doubt flag wrong 0; level claims checked 4, not matching the audio 0; gate/rescue reasons 126, on a hit at or above the track median 0; 'low confidence' fallback uses 0; source=level hits with c != recomputed level factor 0. PASS
5. Detect Tempo: Session A detected 66.9994 BPM, map [67.0], session 134.0, delta to nearest octave 0.00, PASS. Session B detected 125.9999 BPM, map [126.0], session 126.0, delta 0.00, PASS. Session C detected 125.999 BPM, map [126.0], session 94.5, delta 31.50, FAIL. Check 5 FAIL (Session C).
6. Analyze All per session (rule 4): Session A 39.7 s wall, peak RSS ~670 MB; Session B 30.1 s, ~557 MB; Session C 101.9 s, ~605 MB (macOS ru_maxrss, reported in bytes). Measurement only, no pass/fail criterion in the ticket: PASS (measured).

## Findings

- S2 detect/tempo: Session C: Detect Tempo suggests 126.0 BPM; the session tempo is 94.5 BPM, no octave relation (189.0 / 47.25), so the suggested map does not match the session tempo map. Repro: Detect Tempo on Session C at the defaults. Measured: delta 31.5 BPM.
- S3 detect/tempo: Session A: the suggested map is the half tempo 67.0 (session 134.0); the 134.0 alternative carries comb 0.946. The octave comparison passes, but the applied map is the half. Repro: Detect Tempo on Session A at the defaults.
- S4 polish/tempo: the raw start estimates are fractional (66.9994 / 125.9999 / 125.999) while the suggested maps are whole numbers (67.0 / 126.0 / 126.0); a surface showing the start value reads fractional.
- S3 data: Session C: 9 tom reference hits are claimed by no mic (silent mics) and drop out of the reference; check 1 covers the rest.

## Ideas

- Tempo: when octave candidates score alike, prefer the higher octave (Session A case).
