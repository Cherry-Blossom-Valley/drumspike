# Test campaign 0.9.6: ENST stress, all drummers and mixes (#118)

Build: v0.9.6 (main at the tag) service_tool, fingerprint 6ce71b8c442cbbfb, config dsp, all 210 items re-measured by the gate this session. Data: ENST drummer_1..3, internal only; per-file tables stayed in a private results file on the host. No audio, no file names below.

## Planned checks

1. enst_gate.py at 0.9.6: PASS — 12 rows compared, all 12 at or above the committed baseline (6 improved: drummer_1 snare 0.808→0.827, drummer_1 tom 0.730→0.734, drummer_2 snare 0.851→0.867, drummer_2 hat 0.901→0.922, drummer_2 tom 0.406→0.408, drummer_3 hat 0.897→0.906).
2. Per drummer x mic F1 at +-20 ms: 16 cells measured. 3 weakest (drummer, mic, role): drummer_3/tom_3 (tom) F1 0.407 (P 0.265, R 0.877); drummer_3/tom_2 (tom) F1 0.441 (P 0.289, R 0.934); drummer_3/tom_1 (tom) F1 0.556 (P 0.398, R 0.922). All three are tom cells of drummer_3, and all fail on precision, not recall: the tom mics output 2.2-3.3x more hits than there are reference tom hits (tom_3: 1454 output vs 455 ref).
3. Failure clusters (misses + false hits at +-20 ms, grouped by phrase genre x sticks/rods/brushes/mallets, 44 nonzero clusters of 44): total 8967 misses and 5733 false hits. Worst clusters: salsa/sticks 737 misses + 578 false; rock/sticks 578 + 769; charleston/sticks 537 + 72; funk/sticks 418 + 303; celtic-rock/sticks 372 + 187; disco/sticks 326 + 468. Brush clusters fail almost only on misses (country/brushes 500 misses, 37 false; waltz/brushes 237 + 72), stick clusters on both. Mallet/rod/hands clusters are small (all under 130 per cluster).
4. Placement at the new default (rule 4), matched hits at +-20 ms, offset in samples at 44100 Hz (positive = detector later than the annotation): kick n=9634, median +21.9, p10 -10.8, p90 +214.8; snare n=8153, median +34.0, p10 -8.0, p90 +437.3; tom n=2072, median +63.4, p10 -13.8, p90 +381.0; hat n=11101, median +105.2, p10 -2.9, p90 +625.5. The hat tail reaches ~14 ms late at p90.
5. Detect Tempo on the 135 phrase items, wet mix (all 8 mics summed) vs dry mix (6 close mics summed), one mix track per analysis (kick-only path), stable = start BPM and every step BPM identical: 75/135 stable (55.6%); start BPM equal in the same 75. 13/135 (9.6%) the mix analysis found fewer than 8 hits and the detection refused (400: "5 kick and snare hits: the detection needs at least 8"). Of the 47 unstable, 7 are pure octave flips (2x). Per drummer stable: drummer_1 14/42, drummer_2 34/49, drummer_3 27/44.

## Findings

- S2 detection: tom bleed on drummer_3 — the tom mics fire on far more hits than the tom annotations (tom_3: 1454 output vs 455 ref at +-20 ms, F1 0.407). Repro: any drummer_3 phrase/solo with tom mics in the session; measured pooled over that drummer's items. The other drummers' tom cells are 0.57-0.78.
- S2 detection: drummer_1 kick recall 0.714 at +-20 ms (2350 output vs 3234 ref) while drummer_2/3 kick recall is 0.98+; the 0.9.6 gate passes it at +-50 ms (0.849), so the misses are 20-50 ms late or dropped.
- S2 tempo: Detect Tempo is not stable between the wet and the dry mix of the same file — 60/135 differ (44% of the items, 7 of them pure octave flips, 13 refuse with too few hits). Repro: sum the mics of one phrase item two ways (close only vs with overheads), run the detector + tempo_map_detect on each mix; compare the suggestion. Measured: 75/135 identical.
- S2 tempo/mix detector: on 13 of 270 mix analyses the detector returned fewer than 8 hits on a full drum phrase and tempo_map_detect refused (MIN_HITS). Whatever the mix, a played phrase should yield far more than 5 events.
- S3 placement: hat hits sit late against the annotation (median +105 samples, p90 +625 at 44100 Hz); kick/snare/tom medians are +22 to +63 samples. The gate F1 is unaffected (windows are wider), but at the 20 ms scale the hat tail is a systematic lag.
- S3 friction: the build on this host needs libdbus/gdk dev files that are not installed; a sysroot under the user tree unblocks it (PKG_CONFIG_PATH), but a fresh agent otherwise cannot build v0.9.6.

## Ideas

- A bleed guard on tom mics (drummer_3 tom_1..3 fire on ~2/3 non-tom hits) would lift the tom F1 by ~0.3 for that drummer; the per-mic bleed table in the score already has the data.
- Let tempo_map_detect read all role tracks (kick+snare) of a multi-mic session instead of one mix, and compare suggestions per mic pairing; the mix-sum path loses the role split.
- An octave referee exists for the tempo (it flips 7 files); a wet/dry consistency vote (analyze both mixes, prefer the suggestion the two agree on) would remove most of the remaining instability at 2x the analysis cost.
- The 13 refusals could downgrade to a warning with the map flagged low-confidence instead of a 400.

Private evidence: results dir on the host (per-file F1 cells, per-cluster counts, per-file tempo rows).

check_private.py --text-file on this report: rc=0.
