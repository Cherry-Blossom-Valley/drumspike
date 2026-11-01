Changes since 0.9.11:

### Hits
- New Hits > Toggle Hit Active (X) switches the selected hit between active and deactivated. Your choice wins over the automatic rules and stays after a re-analyze, a threshold change and a Learn change. Export, MIDI, spikes and clicks follow it.
- New Hits > Re-analyze Selection (Shift+R) analyzes only the hits in the selected range of one track. Hits outside the range keep their times. Hits you added or moved by hand inside the range stay. The entry is also in the range context menu. The Markers Ruler shortcut is now Option+Shift+R.

### Detection
- Kick and snare hits are placed more exactly on the attack. Fewer hits sit late after a small first rise, and a hit no longer lands about 0.75 ms before the attack when the noise before it is uneven.

### Help
- New Help > Drum Spike Help opens the manual inside the app, with the same chapters and pictures as the online guide. Help > Keyboard Shortcuts is its shortcuts section.

### Toolbar
- A new ear button turns Audition on Tab on and off. The shortcut is U, and the same switch is in Transport > Audition on Tab.
- In a narrow window the toolbar wraps to a second row. No control is cut off or made narrower. The window is at least 640 px wide.

### Inspector
- The refine window is shown as two numbers in ms (Refine window low and high). A change is kept at once, and Undo brings the old value back.
- A new Name pattern field shows the words of the track name that the role came from. You can edit it, unless you or the audio set the role.

### Export
- The project remembers the "Compensate sampler offset" checkbox, like "Markers in MIDI".
- An export of one spike type also updates an older spike WAV of the other type in the same folder (synthetic or audio), so no spike file keeps old hits after your edits. The export message names these files.

### Agent API
- `track_layer_set` takes `range` (the dB range of the velocity curve, 0 to 96).
- New `export_compensate_set` keeps the Compensate choice. `export_all` without `compensate` uses it.
- New `reanalyze_selection` re-analyzes the hits of a time range on one track.
- `export_all` lists the older spike files it also rewrote in `also_updated`.
