Changes since 0.5.1:

### Learn
- Learn This (K) on a range or at the playhead where the detection has no hit now adds the hit
  (flagged `learned` in the project) and stores the example. The status says what the Add rule does
  from here. Learn This never removes a detected hit and never turns the Learn filter on; cutting
  stays with Forget This (Shift+K), which keeps today's refusals. A K on a detected hit only stores the example.
- With a Learn This example on the track (the Learn filter on or off), candidates below the
  Sensitivity that look like the example become hits (the Add rule, threshold 0.85). A Forget example within 50 ms or an existing hit
  within reach excludes one; the added hits run through the same analysis as every hit and leave
  MIDI, spikes and the click like every hit. Remove Example and Forget All take them away again.
- The Similarity histogram has a second draggable line, "Add", with the live count of the hits the
  rule would add. The guard asks Keep or Undo when one step adds more than max(10, 25 %) hits.

### Editing
- A scrub or a drag redraws the lanes at most 60 times per second on a 120 Hz display (less work for the CPU); playback and the playhead are unchanged.
- Every undo step has a name and a place. Edit > Undo and Redo read "Undo Delete Hit (s top, 12.345 s)"
  (Nudge Hit, Move Hit, Add Hit, Set Velocity, Recompute Velocities, Shift Hits, Re-pick Hits, Set Loudness
  Range, Learn This, Set Bar 1, Remove Track and so on). Undo and Redo select the track and the hit, scroll
  the view to the place and say in the status line what they did.
- Undo after Delete Hit gives the hit back with all its stored fields (the keep flag is read from
  the service), so the project is clean again after the undo.
- Undo back to the saved state clears the unsaved mark (the dot in the title). A save sets the state; a
  redo or a new edit marks the project unsaved again. The check compares the documents
  (new command `project_clean_check`).
- Edit > Undo History lists the last steps, newest first; the undone steps are grey above them. A click
  goes back to that row (or forward, for a grey row).
- The lane cursor says what a click does: a plus while Option is down (add a hit), an eraser cross
  for Option+Shift over a hit (delete), a resize cursor on the bar-1 marker of the Bars|Beats ruler
  and on the Match and Add lines of the Similarity histogram.
- Option+Shift-click on a hit deletes it (one undo step, like the Delete key). Every other click is unchanged.
- Review markers: M sets a marker at the selected hit or the playhead, the Markers ruler shows
  them (a click puts the playhead, a click on a marker selects it, Delete removes it, a double-click takes
  its note), and Option+M / Option+Shift+M jump to the next and the previous one. The markers live in the
  project (`markers`, a pass-through key) and in the agent API (`markers_get`, `markers_set`: one more call
  with the answer's `previous` is the undo). The MIDI export of markers waits for the check whether Pro
  Tools and Logic import MIDI marker events.
- View > Velocity Lane: a thin lane under the spike lane of every track, one stalk per hit, the height is
  the velocity 1 to 127. Stalks of hits that leave the output (cut, tom bleed, Learn filter) are grey; the
  two dashed lines are the floor and the ceiling of the loudness settings. Drag a stalk to change the
  velocity, Option-drag to draw a straight line over a range, Cmd-drag for freehand. One gesture is one
  undo step ("Set Velocity (s top, 3 hits)"). A hand velocity keeps the manual colour and survives
  Recompute Velocities and a re-pick (`events_edit` takes `edited: true` on a velocity op).

### Playback
- Audition: A plays the selected hit on its track alone, from 30 ms before the hit to 250 ms after it
  (Cubase preset: Option+A). Shift+A loops that window until Escape or Space. With a range selection,
  A loops the range and the ruler shows the loop band. Half-Speed applies when the page plays at half
  speed as you press the key. Mute, solo and the playhead stay as they are; the status says
  "audition s top 12.345 s". The click is off during an audition.
- Settings > Audition on Tab: Tab (and the other keys that step to a hit) also plays the hit it steps
  to, unless the page plays.
- `transport_play` takes `start`, `end`, `loop` and `tracks` (an audition: the named tracks are the only
  audible ones for that play, mute and solo unchanged afterwards).
