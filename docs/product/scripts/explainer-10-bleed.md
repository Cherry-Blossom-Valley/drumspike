# Explainer 10: Is it a hit or is it bleed?

Chapter 10 of `docs/product/feature-path.md`. Narration for `tools/media/tts.py`: one `##` heading
per scene; lines that start with `>` are visual notes and are not spoken. Target length: about
3 minutes. Use the demo session (it has bleed, a flam and a tom fill).

Facts used (all checked against commit 498f4c0; module docs of
`crates/core/src/detect/tom_bleed.rs`, `crates/core/src/detect/cross_bleed.rs`,
`crates/core/src/detect/flam.rs`, `crates/app/src/tom_bleed.rs`, `crates/app/src/cross_bleed.rs`
and `crates/app/src/ghost_gate.rs`; the detection on main is spectral onset detection with gates
and cross-mic rules):
- Tom band 50 to 400 Hz, the level window [-0.5, +8) ms at a hit, transfer gains calibrated from
  the strong hits, two mics of one drum never explain each other, the kick and snare close mics as
  sources (snare excess under 9.2 dB, kick excess under 4.0 dB): `crates/core/src/detect/tom_bleed.rs`,
  the rule description and version 2.
- Tom pair rule: two hits on mics of different drums within 12 ms, the one 6 dB or more under the
  other is bleed; a hit with an own-mic attack rise of 10 dB or more stays (version 4 guard):
  `crates/core/src/detect/tom_bleed.rs`, versions 3 and 4.
- Cross-track rule: a tom hit that coincides within 10 ms with a hit of a kick, snare or hi-hat
  close mic, starts later than it and is quieter in that mic's own band is bleed:
  `crates/core/src/detect/cross_bleed.rs` (coincide_ms 10.0) and `crates/app/src/cross_bleed.rs`.
- Ghost-note gate: a soft snare candidate passes when its spectral shape matches the snare template
  (cosine 0.92 or more) and no kick or tom hit lies within 30 ms (`CLEAR_MS`):
  `crates/app/src/ghost_gate.rs`.
- Flam: a second attack 12 to 50 ms after the hit that rises 9 dB or more, through a 1 kHz high
  pass; the bleed gate with the other mics' transfer gains; a late-peaking mic (a snare bottom) is
  skipped: `crates/core/src/detect/flam.rs` and `crates/app/src/flam.rs` (LATE_REASON).
- A marked hit stays in the project, shows grey and leaves the output; `restore` takes the mark
  off; an edited hit never carries it: `crates/app/src/tom_bleed.rs`, `crates/app/src/edit.rs`.
- Every close mic of the session is a source for the others: `crates/app/src/cross_bleed.rs`
  (the mics of `run_step`), `crates/app/src/tom_bleed.rs` (the sources of version 2).

## s01-bleed

> Four close mics stacked: kick, snare, two toms. One snare hit: the snare mic peaks, and smaller copies of it appear on both tom mics, a little later.

Every close mic hears more than its own drum. When the snare is hit, the tom mics hear it too, a little later and quieter. Engineers call this bleed. A trigger on the tom mic must not fire on the snare.

## s02-session

> The whole session analyzed: the status line at the bottom says what the rules did, for example tom bleed: 14 hits out of the output.

The rules run over the whole session, after the per-track analysis. Every close mic is a source for every other mic, so load every close mic you have. A hit that a rule marks keeps its place, shows grey, and stays out of the exports.

## s03-tom-bleed

> For one tom mic hit: the tom-band level (50 to 400 Hz) on every mic; arrows from the other drums' mics with their transfer gains; the sum explains the level, so the hit turns grey.

For every tom hit, Drum Spike looks at the low band where toms live, 50 to 400 hertz, in a small window around the hit. It knows how much of each drum normally arrives at each mic. It calibrated that from the strong hits of the session. If another drum explains the level on this mic, the hit is bleed. Two mics of one drum never explain each other. The kick and snare close mics are sources too.

## s04-cross

> A tom mic hit and a snare hit 5 ms apart, zoomed: the snare mic's first motion starts earlier, and the snare mic is louder in its own band. The tom hit turns grey.

A real hit is loudest and earliest on its own mic. So the cross-track rule checks the times of arrival. A tom hit that starts later than a kick, snare or hi-hat hit within 10 ms of it, and is quieter in that mic's own band, is bleed.

## s05-pairs

> Two tom hits 8 ms apart on different drums: the quieter one, 6 dB under the other, turns grey; a hit with a clear attack of its own stays.

Of two hits on different tom drums within 12 ms, the one that plays 6 dB or more quieter is bleed. A hit whose own mic shows a clear attack rise stays.

## s06-ghost

> The snare lane: a soft hit whose shape matches the snare template survives; a soft bump next to a tom hit, within 30 ms, is dropped.

A quiet snare hit stays when its spectral shape matches the snare template, and no kick or tom hit lies within 30 ms of it. This keeps the ghost notes, and drops the kick and tom bleed on the snare mic.

## s07-flam

> A flam on the snare at 5 ms zoom: the one kept hit; the attack envelope shows a second rise 20 ms later; a marker appears with the label flam.

A flam is two strokes very close together. Drum Spike looks inside that one hit for a second attack, on the snare mic itself: 12 to 50 ms after the hit, and rising 9 dB or more. If the other mics do not explain it as bleed, it becomes its own hit, on its own first sample. A snare bottom mic peaks too late for this; the rule skips it.

## s08-control

> The grey bleed hit is selected; View > Other Mics Under Hit shows what the other mics heard; restore brings it back; D steps to the next doubtful hit.

You stay in control. Grey hits are only marked, never deleted. View Other Mics Under Hit shows you what the other mics heard at that moment. One edit brings a hit back, and an edited hit is never marked again. The review queue takes you to the doubtful hits first.

## s09-result

> All tracks: clean spikes on every mic, bleed in grey, the flam split.

The result: one hit per real stroke on every mic, and the bleed left out.
