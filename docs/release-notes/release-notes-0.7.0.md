Changes since 0.6.0:

### Learn
- A hit that Learn added (the Add rule, or the hit of a Learn This example) and that you delete stays deleted on the next Learn change; undo of the delete, or Learn This (K) on its spot, brings it back.
- On a tom track, Learn no longer adds the bleed of the other toms, the snare or the kick: the tom bleed rule now judges the hits that Learn adds, and greys the bleed hits as it does after an analysis.
- Learn This (K) on a range with no hit and no clear attack puts the new hit on the softest attack it can find in the range (a rise of 2 dB or more) that is not the bleed of a louder hit on another close mic, not at the range start; a range with no clear attack gets the hit at its centre.

### Layers
- The Layer section shows the first 200 ms of the one-shot with a line at its onset. Drag the line to
  place the onset by hand (one `track_layer_set {onset}` on release, in samples of the one-shot); the
  render, the playback and the Align place the one-shot by it, and the export checker follows. Reset
  puts the line back on the measured onset; a new one-shot clears the override. Drag the wave pans,
  the wheel zooms. With a sampler calibration the view also shows the bounce with a line at its
  sampler offset, and Play hears the one-shot once from its first sample (the close mic sounds
  underneath, as with the audition key).

### Performance
- The track and master meters paint on one canvas per meter instead of writing transforms on about
  60 DOM elements per frame. On a 12-core Intel host the meter path at 25 meter messages per
  second with 30 tracks falls from 18.98 % to 7.42 % of one core (measured with the e2e harness), and
  rows outside the scroll viewport are not painted. The look is unchanged: the same gradient, hold
  lines and clip light.
