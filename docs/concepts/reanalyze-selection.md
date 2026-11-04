# Re-analyze Selection

Status: built (#243, #265). The owner chose option B: a separate command `Hits > Re-analyze Selection`
(key Shift+R, also in the range context menu), enabled only while a range is selected. The Inspector
button `Re-analyze` and Analyze All stay whole-track and ignore the range. The code is
`crates/app/src/reanalyze.rs`; the user text is in `docs/guide/`.

## The wish

Re-analyze only a selected time range of one track, without a new kind of selection and without
changing what the old buttons mean.

## Base in the app

- A range selection on one track is a drag in the wave lane (one at a time, a blue band). Delete and
  the nudge keys act on every hit of the range as one undo step. Learn This and Forget This (K,
  Shift+K) turn the range into a Learn mark. A click, K or Escape clears it.
- The run uses the track's current settings (the Inspector values).
- When the range holds edited hits (magenta), the Keep dialog asks whether they stay.

## Rules (the engine side)

These rules settle the edge cases once.

1. Scope. The detector runs on the track with a pad of 100 ms on both sides (so the spectral frames and the
   refine-to-attack walk see their context). Only hits whose refined time lies inside `[t0, t1]` are taken.
2. Replaced and untouched. Hits of the track inside the range are replaced by the new ones. Hits outside are
   not touched at all (same times, ids, velocities).
3. Edges. A new hit is dropped when it is closer than the minimum gap to the nearest untouched hit outside
   the range. A hit outside is never moved by the run. So no double hit appears at an edge, and a hit that
   straddles the edge belongs to the side where its refined attack lies.
4. Edited and manual hits inside the range follow the Keep dialog as today (the dialog counts only the
   hits of the range). No edited hit in the range: no dialog, as today.
5. Learn marks and followers. Learn marks of the track stay (as in a whole-track run) and are applied to the
   new hits inside the range. Follower tracks refresh only for the range.
6. One undo step ("Re-analyze Selection") that restores the hits of the range, and the backup file of the
   previous events is written as for a whole-track run.
7. After the run. The info line says `Kick In: re-analyzed 0.900 to 2.100 s, 3 hits replaced by 4`. The
   export is the export of the track as it is: it has no notion of the run.
8. Several tracks. The range belongs to one track today. A run acts on that track only.

## Options not taken

- A: let the Inspector button act on the range. It makes one control mean two things by a hidden state.
- C: a saved region with its own settings. It is the largest change of the concept; a separate ticket
  if the owner wants it.
- D: a confirm dialog. Only the pre-run line (scope in the Keep dialog and in the info line) is used.
- E: a time range over several tracks. It changes Delete, nudge and Learn too; a separate ticket.
