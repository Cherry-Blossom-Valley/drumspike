# Screen recording, part 2: check and fix

Chapters 6 to 8 of `docs/product/feature-path.md`, on the demo session. Narration for
`tools/media/tts.py` (one `##` per scene; `>` lines are the scenario steps and captions for
`tools/media/record.py`, not spoken). Target length: 75 to 90 s. Keys shown in the key overlay.

## p2-play

> Space; Space again; Shift+Space; Option+Space, then Option+Space again. Caption: Transport > Half-Speed Play

Play with Space. Shift Space is half speed. Option Space twice puts the playhead back where the play started.

## p2-zoom

> 6, then 5, then 1; T; R. Caption: View > Zoom Presets > 5 ms

Zoom with the number keys. Press 6 for five milliseconds, 1 for the whole session. T zooms in and R zooms out, always around the playhead.

## p2-step

> Tab; L; N; Option+O. Caption: Hits > Next Hit

Tab moves to the next hit, L moves back. N steps through the hits of all tracks. Option O shows the whole session in one strip.

## p2-add

> Move the playhead to the gap; press H. Caption: Edit > Add Hit at Playhead

The detector missed one hit. Put the playhead in the gap and press H. The new hit sits on the playhead.

## p2-delete

> Playhead on the hit; 6 (5 ms); Tab; Delete. Caption: Edit > Delete Hit

One hit too many? Select the hit and press Delete. The spike line goes, the audio stays. Right-click works too.

## p2-nudge

> Playhead on the hit; 6 (5 ms); Edit > Nudge Value > 1 Sample; Tab; A. Caption: Edit > Nudge Value > 1 Sample

A hit a sample off? Nudge. Set the value to one sample and press A. The hit moves one sample closer to the attack.

## p2-undo

> Cmd+Z; Shift+Cmd+Z; Option+Cmd+Z. Caption: Edit > Undo History…

Command Z undoes an edit, Shift Command Z does it again. Option Command Z lists the steps.

## p2-velocity

> Press V; drag a stalk; Option-drag over a range. Caption: View > Velocity Lane

Press V for the velocity lane. Each stalk is the velocity of one hit. Drag a stalk to set it. Option drag draws a line over a range.

## p2-shape

> Inspector: velocity floor, ceiling, Loudness cut; Edit > Recompute Velocities. Caption: Edit > Recompute Velocities

The Inspector sets the floor, the ceiling and the Loudness cut. After a change, Recompute Velocities builds the curve again.

## p2-end

> Caption: "Next: doubtful hits first".

That is check and fix: find your way, edit the hits, shape the velocity. Next: the doubtful hits.
