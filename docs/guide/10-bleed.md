# 10. Bleed and the hard cases

A close mic hears the whole kit, not only its drum. A tom mic picks up the snare, a snare
mic picks up the kick. After the analysis, Drum Spike cleans this up per session, so you do
not edit bleed by hand.

The rules run on the analysed tracks, in this order:

- A tom hit that another close mic explains is bleed: a tom hit that a kick, snare or
  hi-hat close mic hears louder and earlier, and a tom hit on a different drum's mic within
  12 ms that plays 6 dB or more quieter. The hit keeps its place, but the app marks it and
  leaves it out of the output: grey in the lane, and no MIDI, spike or click.
- A quiet snare hit stays only when its spectral shape matches the snare template and no
  kick or tom hit lies within 30 ms of it. This keeps the ghost notes and drops the kick and
  tom bleed on the snare mic.
- The second stroke of a flam becomes a hit of its own, with the tag flam. This runs on
  snare close mics whose Split flams check is on, and it is on by default; another mic that
  explains the second stroke as bleed wins. The flam hit plays in the exports like any hit.

## Steps

1. Analyze (chapter 3) or press **Hits > Analyze All**. The status line at the bottom of the
   editor says what the rules did, for example `tom bleed: 14 hits out of the output`.
2. Select a track and open the Inspector. The line under the track numbers says what the
   rules did to this track, for example `Bleed rules: 4 hits out of the output · Flams: 6
   hits added`.
3. A rule-marked hit shows grey, like a hit under the loudness cut (chapter 8). To bring a
   bleed hit back, edit it, and edit it in a way that keeps the time: drag the hit's stalk
   in the velocity lane (chapter 8). A hand velocity marks the hit edited, and an edited
   hit is never marked, so it plays in the exports again. Nudging also works, but a nudge
   back to the detected time makes the hit unedited again, and the bleed mark can return.
   The agent API (chapter 14) has a `restore` edit that unmarks a hit without moving it.
4. To turn the flam rule off for one snare track, clear the Split flams check in the
   Inspector. The change applies on the next analyze.
5. To check a hit against the other mics, select it and press `O` for **View > Other Mics
   Under Hit**. A panel under the lanes shows a small lane per other track in a window
   around the hit, with the hit time as a line. This is how you see that the quiet hit on
   the tom mic is the snare. Press `O` again to close the panel.

For how Drum Spike tells a hit from bleed, see the 3b1b explainer "Is it a hit or is it bleed?"
(coming).

Next: [Learn: teach by example](11-learn.md).
