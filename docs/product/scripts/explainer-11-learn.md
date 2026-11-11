# Explainer 11: Learn, teach by example

Chapter 11 of `docs/product/feature-path.md`. Narration for `tools/media/tts.py`: one `##` heading
per scene; lines that start with `>` are visual notes and are not spoken. Target length: about
2.5 minutes. Use the demo session (it has ghost notes on the snare and the loud bar with the
isolated bleed of every drum).

Facts used (checked against the code at release 0.9.33, not against a commit; `static/keymap.js`,
`static/learn.js`, `crates/app/src/learn.rs`, `crates/app/src/learn_filter.rs` and
`docs/guide/11-learn.md`):
- K runs Hits > Learn This, Shift+K runs Hits > Forget This, Option+K runs Hits > Remove Example
  (the `page_keys` of the rows `learn_this`, `learn_not` and `learn_forget_this`:
  `static/keymap.js`).
- A track holds at most `MAX_EXAMPLES` = 64 examples of each kind (`crates/app/src/learn.rs`).
- The last rule of a range takes the strongest envelope rise of `RISE_MIN_DB` = 6 dB or more
  (`crates/app/src/learn.rs`); the guide: a ghost note that sits under the Sensitivity is found
  too (`docs/guide/11-learn.md`, step 1).
- The first Forget This of a track turns the Learn filter on; Learn This alone never does
  (`crates/app/src/learn.rs`, `static/learn.js`).
- Forget This alone takes only the marked hit and its near copies: a score-marked hit goes only
  when its attack time lies within `FORGET_ATTACK_MS` = 2.0 ms of the marks' attack span, widened
  by `FORGET_ATTACK_FRAC` = 0.5 of the attack at each end (`crates/app/src/learn_filter.rs`).
- The Add rule brings in the candidates that sit below the Sensitivity and look like a Learn This
  example; a Forget example or an existing hit nearby keeps a candidate out, and an added hit is
  flagged learned (`crates/app/src/learn_filter.rs`). The Add line sits at 0.85 until you drag it
  (`lnAddOf`, `static/learn.js`).
- One step that adds or takes out more than 10 hits, or a quarter of the track's output hits,
  asks Keep or Undo (`lnGuardLimit`, `lnGuardAdds`: `static/learn.js`).
- A hit the filter takes out stays a hit marked filtered: drawn grey, skipped by Tab, out of the
  exports; it is not deleted. An edited hit is never filtered, and a hand edit of a grey hit
  brings it back (`crates/app/src/learn_filter.rs`, `static/learn.js`).
- Both kinds of marks are kept in the project, in the track's `learn` object, and every Learn
  step is one undo step (`crates/app/src/learn.rs`, `static/learn.js`).
- The picture's counts are this story's example: the snare lane shows 5 hits, Learn takes 3 out
  (the unwanted ghost, its near copy and the bleed copy), the Add rule brings 3 missed hits in,
  the histogram counts line reads 5 hits, 2 out, Match 0.80, Add 0.85, and the Inspector's mark
  counts read 1 learn and 2 forget examples.

## s01-track

> The snare track of the song over its whole length: every hit the rules keep as an orange stalk, the ghost notes among them. One ghost is ringed; at a quieter spot there is no stalk at all.

Your rules cleaned this song, and the snare track is still not right. The ghost notes stayed, and you do not want them. And the softest one, the detector never found. The Sensitivity would touch the whole track. Learn teaches this one track.

## s02-forget

> Zoom on the unwanted ghost and its near copy: the snare mic's wave, hits as stalks. Hits > Forget This, Shift+K: the mark bar with a cross sits at the top of the lane, and both ghosts turn grey; the loud backbeat hits stay orange.

Select the unwanted hit and press Shift+K, Forget This. The hit and its near copies turn grey at once. Grey is not deleted: the wave keeps the hit, but the exports leave it out.

## s03-learn

> Zoom on the missed soft one: the wave shows a small bump, no stalk on it. The attack envelope of the stretch; the steepest rise ringed. Hits > Learn This, K: a green mark bar with a dot, and the hit is added, flagged learned.

Now the soft one the detector missed. Select a stretch around it and press K, Learn This. On a stretch with neither hit nor candidate, the step takes the strongest rise of 6 dB or more, adds the hit, and learns it as your example.

## s04-add

> The Inspector's Learn section: the Similarity histogram with the red Match line and the green Add line, the counts line under it. On the small lane, three more soft ghosts turn orange as added, learned hits.

From then on the track watches the hits below the Sensitivity. The ones that look like your example come in, flagged learned. A Forget example nearby, or one of your hits, keeps a candidate out. Drag the Add line for more or fewer.

## s05-bleed

> The loud bar of the song on the snare mic: the kick's copy, a little later and quieter. The same Shift+K takes it too: the copy turns grey.

The same Forget This reaches further now. Your example told the filter the difference between the hits you want and the ones you do not. The similar bleed on this track goes grey with the same keystroke.

## s06-safe

> The lane: a grey hit stays in the lane, Tab skips it. A hand velocity edit turns a grey hit orange again. A dialog with Keep and Undo.

Careful what stays safe. A hit the filter takes out goes grey; it is not deleted. Your hand edits win: an edited hit is never filtered. And when one step adds or takes out more than 10 hits, or a quarter of the track, Drum Spike asks you first: Keep, or Undo.

## s07-project

> The project opened again: the mark bars are still at the top of their lanes, the Inspector reads the counts, one learn and two forget.

The marks are kept in the project. Open the song tomorrow, and they are still there, up to 64 of each kind on a track. Every step is one undo step.

## s08-result

> The snare track before and after: the unwanted ghosts and the bleed copy grey, the missed hits in as learned hits, the counts line under the lanes.

The result on this song: the ghosts you do not want are out of the exports, the missed hits play again, and the rest of the track is untouched. Learn taught it in three keystrokes.
