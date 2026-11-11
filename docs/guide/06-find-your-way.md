# 6. Find your way: play, zoom, move between hits

You have hits (chapters 2 to 4) and files (chapter 5). Now you check them by ear and by eye.
Drum Spike moves like a DAW: play, zoom, and jump from hit to hit. The demo session holds 309
hits over 62.5 s; these keys take you through them fast.

## Steps

1. Press Space. **Transport > Play/Stop** plays from the playhead; press Space again to stop.
   `Option+Space` is play-and-return: the next press of `Option+Space` or Space stops and puts
   the playhead back where the play started. The keys reach the app from anywhere: after a
   click on a button or a slider (the Sensitivity slider of a histogram, for example) Space
   still plays. Only a text field keeps its keys, and the status line tells you when a
   transport key went into one. While the computer is busy (a large build, for example), the
   playhead keeps moving smoothly: frames drawn between two clock updates no longer lose the
   playhead. **Transport > Stop Returns to Start** (`I`) is a mode like
   Suspend Follow when Editing: while it is checked, a Stop (Space while playing, or the ■
   button) puts the playhead back where the play started, exactly like `Option+Space`; the
   setting survives a restart.
2. Press `Shift+Space` for **Transport > Half-Speed Play**: half speed, one octave down. This
   is the setting for a close check of an attack.
3. Zoom with the number keys. The six presets of **View > Zoom Presets > Full** (`1`), **30 s**
   (`2`), **5 s** (`3`), **500 ms** (`4`), **50 ms** (`5`) and **5 ms** (`6`) set the span of the
   lanes. **View > Zoom In** (`T`) and **View > Zoom Out** (`R`) zoom around the playhead.
   **View > Zoom to Project** (`Option+A`) shows the whole session, **View > Zoom Toggle**
   (`E`) fills the lane width with the selected range, or 5 ms around the selected hit, and
   returns to the zoom before. **View > Zoom Presets > Add Current…** saves the span of the
   view under a name; **Organize…** renames and removes your own presets.
4. The main counter in the transport bar at the top, next to the play and stop buttons, shows
   the position: **View > Main Counter > Seconds**,
   **Samples**, **Bars|Beats** and **Timecode** (30 fps) switch the format; **Exchange
   Counters** (a click on the counter) swaps the main and the sub counter.
5. Change the track height with `Option+Up` and `Option+Down`
   (**View > Track Height > Shorter**, **Taller**), or set it directly with `Option+Cmd+1`,
   `Option+Cmd+2` and `Option+Cmd+3` (Small, Medium, Large). Drag the lower border of a
   track row or of its header to give one track its own height; a double-click on the
   border puts it back.
6. The lane toggles: **View > Velocity Lane** (`V`), **View > Other Mics Under Hit** (`O`),
   **View > Bars|Beats Ruler** (`Shift+B`) and **View > Markers Ruler** (`Option+Shift+R`).
7. Step through the hits. **Hits > Next Hit** (`Tab`) and **Hits > Previous Hit**
   (`Option+Tab` in the menu, `L` on the page) move on the selected track; with no track
   selected, they move on all tracks. **Hits > Next Hit on All Tracks** (`Control+Tab` in
   the menu, `N` on the page) and **Hits > Previous Hit on All Tracks** (`Control+Option+Tab`)
   always move on all tracks. Tab and Option+Tab skip the deactivated hits by default;
   **View > Tab Moves To > All Found Hits** visits them too.
8. Press `Option+O` for **View > Show Overview**: the whole session in one strip over the lanes.
   Click it to move fast over a long session.
9. Press `M` for **Edit > Set Marker at Selection** to drop a marker at the selected hit or at
   the playhead. **Transport > Next Marker** (`Option+M`) comes back to it later.

The keys of this chapter are the Pro Tools preset, the default. Drum Spike also has a Cubase
preset: **Drum Spike > Settings…** (`Cmd+,`) opens the Settings window; pick the category
Keyboard and set the row Key preset. The Settings categories are Audio, Keyboard, View,
Audition and click, Pitches, Detector plug-ins and Agent. **Help > Keyboard Shortcuts** lists all
keys of the current preset. **Drum Spike > About Drum Spike** shows the Cherry Blossom Valley logo,
the licence line and links to the website and the source code.

![A kick and a snare track at the 50 ms zoom; the playhead sits on a hit and the lanes show the neighboring tracks at their heights.](img/06-find-your-way.png)

Next: [edit hits](07-edit-hits.md).
