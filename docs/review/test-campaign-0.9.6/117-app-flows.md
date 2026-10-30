# Test campaign 0.9.6: app flows end to end (#117)

Build: main at 277ed6bd693a80aac412b51990030c65332619d1, ds-bridge debug build of the same commit; the app driven in Chromium through ds-bridge and the e2e harness (tests/e2e_app). Data: copies of the owner's sessions (Session A, B, C), internal only; every session was first copied to a scratch folder and the app wrote project files only into the copies. Per-file tables, frames and logs stayed in a private results file on the host ("results dir on the host"). No audio, no file names below. Check 5 ran against refine version 5 (#127, cap 3 samples); all other checks at the 0.9.6 default (version 4).

## Planned checks

1. Open + Analyze All, three sessions: PASS — Session A 12 lanes, Session B 9 lanes, Session C 12 lanes, analyze_status done on every track, peaks drawn for every lane; Session A 351 events on the dense track, Session C 387.
2. Editing: add, delete, nudge, move + undo/redo every step: FAIL — 4 steps recorded, undo not exact in 4/4 steps, redo not exact in 4/4 steps (event counts match after every undo, but the state hash differs; the differing field is not identified yet — see findings).
3. Export every kind + reopen: PASS — MIDI per-track 12 files, MIDI single 13 files, spikes 31 files, layer render (WAV) one file of 67,745,176 bytes; after reopen and re-analyze 351 events vs 351 before, 0 differ (time to the sample, velocity, role).
4. Keys: every key in static/keymap.js once: FAIL — 2 of 85 pressable keys do nothing (undo Z, redo Shift+Z); 26 of 111 not pressable by the harness (not verdicts). The 26 are key-name mapping failures of the harness (9 macOS Cmd, 7 Option, 10 unaliased names such as Left/Right/Plus/Return/Numpad).
5. Zoom + scroll at the highest zoom, hit line on the foot of the wave (#101), refine version 5: PASS — on the dense track (387 hits, 48 kHz), against the #101 floor-leave foot (first sample above 2x the pre-hit noise median) all 387 hits sit 0 to +3 samples of the hit line, max +0; the walk-back distance a stricter foot (last amplitude minimum before the peak) would add is median 511 samples; against a 10%-of-peak foot the hits sit median +36 samples early-side (the refine targets the floor, not the 10% point).
6. Stress: long session, UI freezes over 500 ms: PASS — 0 freezes, max frame gap 361.4 ms during analyze_all/add hit/undo, 1 frame over 20 ms.

## Findings

- S2 editing: undo does not restore the exact state. Repro: open a session copy, Analyze All, add a hit, delete a hit, nudge +1, move +10, then undo each step and compare the full event state. Measured: event counts match after every undo, but the state hash differs in 4/4 steps; redo likewise 4/4. The differing field is not identified (candidate: a per-event or per-track field the undo skips, or an async write that lands after the compare).
- S2 keys: undo/Z and redo/Shift+Z do nothing and open no modal, while Control+Z / Control+Y undo and redo correctly (check 2 used them). Repro: open a session, edit a hit, press Z. Measured: 2 of 111 keys no-op.
- S3 export: a failed or interrupted layer render leaves a folder named like the target file (Exports/<track>_layer.wav/ plus a <track>_layer.wav.drumspike), and every later render of that track fails with 400 "the path is a folder" until the folder is removed by hand. Repro: render the layer of a track, remove the produced WAV (or let the render fail), render again. Measured: 1 host, reproducible across 3 attempts.
- S3 keys/testability: the keymap stores macOS-style names (Cmd, Option) and unaliased names (Left, Right, Plus, Return, Numpad) that a plain Keyboard.press cannot send; 26/111 keys are unverifiable on a Linux harness without a name map. Not an app defect; a mapping table in the e2e harness (or aliases in the keymap) would close it.

## Ideas

- The undo check would pinpoint the differing field if undo returned the changed paths (or if events_get exposed a revision counter per event); today only the hash differs.
- A one-line "last render failed: remove the folder" hint in the 400 would turn the S3 export trap into a self-service fix.
- The 10%-of-peak foot sits median 36 samples after the stored hits; if the owner wants hits visually inside the attack ramp, that is the number a change would have to trade against the #101 gate.

Private evidence: results dir on the host (per-lane event tables, key list, check 5 screenshot and probe JSON, export file list, driver log).

check_private.py --text-file on this report: rc=0.
