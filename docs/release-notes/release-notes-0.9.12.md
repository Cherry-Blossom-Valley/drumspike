Changes since 0.9.11:

### Toolbar
- A new ear button turns Audition on Tab on and off. The shortcut is U, and the same switch is in Transport > Audition on Tab.
- In a narrow window the toolbar wraps to a second row. No control is cut off or made narrower. The window is at least 640 px wide.

### Inspector
- The refine window is shown as two numbers in ms (Refine window low and high). A change is kept at once, and Undo brings the old value back.
- A new Name pattern field shows the words of the track name that the role came from. You can edit it, unless you or the audio set the role.

### Export
- The project remembers the "Compensate sampler offset" checkbox, like "Markers in MIDI".

### Agent API
- `track_layer_set` takes `range` (the dB range of the velocity curve, 0 to 96).
- New `export_compensate_set` keeps the Compensate choice. `export_all` without `compensate` uses it.
