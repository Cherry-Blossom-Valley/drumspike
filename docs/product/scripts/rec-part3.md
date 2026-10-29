# Screen recording, part 3: followers, layers and tempo

Chapters 12 and 13 of `docs/product/feature-path.md`, on the demo session. Narration for
`tools/media/tts.py` (one `##` per scene; `>` lines are the scenario steps and captions for
`tools/media/record.py`, not spoken). Target length: 75 to 90 s. Keys shown in the key overlay.

## p3-follow

> File > Add Tracks…: the sub mic row has a follow check; leave it on. Caption: File > Add Tracks…

A sub mic hears the same hits as the kick in, but softer. Leave the follow check on: the sub takes the hits of the kick, sample-exact.

## p3-follow2

> Select the sub mic; Track > Follow Hits Of > Kick In; Track > Own Detection. Caption: Track > Own Detection

Follow Hits Of sets it later. A follower is read-only: edit the source, and the follower takes the change at once. Own Detection switches it back.

## p3-layer

> Inspector: the Layer section; drop a one-shot WAV on the zone. Caption: The Layer section of the Inspector

A layer plays your own one-shot at every hit. Drop a WAV on the zone in the Inspector.

## p3-layer2

> Press Align; the Onset view: drag the line. Caption: The Onset view

Align puts the one-shot where its attack matches best. Drag the onset line to set when it sounds.

## p3-tempo

> Hits > Analyze All; Track > Detect Tempo…; check the dialog; Accept. Caption: Track > Detect Tempo…

Analyze kick and snare first. Detect Tempo builds the map from their hits: tempo, bar 1, meter and steps. Check it by ear, then Accept.

## p3-bar1

> Track > Set Bar 1 at Playhead. Caption: Track > Set Bar 1 at Playhead

Set Bar 1 moves bar 1 to the playhead, or to the selected hit.

## p3-import

> File > Import Tempo Map…; pick the MIDI file of the DAW. Caption: File > Import Tempo Map…

Import Tempo Map takes the tempo from a MIDI file of your DAW. One undo step.

## p3-grid

> Press G; View > Tempo Ruler. Caption: View > Show Bar and Beat Grid

Press G for the bar and beat grid. The Tempo and Meter rulers show the steps and the changes.

## p3-click

> Press C; Space. Caption: Transport > Metronome Click

Press C for the metronome. The click sounds at the hits, each with the pitch of its role.

## p3-end

> Caption: "Next: automate it".

That is part three: followers, layers and the tempo map. Next: automate it.
