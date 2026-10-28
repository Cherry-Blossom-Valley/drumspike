Changes since 0.5.0:

Fixes and features from the owner's test of 0.5.0 on a real song (Session E, 6 tracks).

### Tempo
- Detect Tempo no longer bends the tempo when a song has a sparse section (for example one kick pair per bar and no snare). Such a section fits a beat a third of a bar long as well as the real beat; the detection took that for a short tempo change, then dropped it but kept its beat count, and the whole grid came out slower and shifted. A tempo change now stays only when its beats hold more kick and snare hits than the tempo before it. On the owner's test song (Session E, 280 s) Detect Tempo now suggests 118 BPM with no steps and bar 1 at 0 s, on the bar lines of the hits (it suggested 117.48 BPM, and its bar 1 was a sixteenth note off the hits); the snare sits on beats 2 and 4.
- Track > Set Bar 1 at Playhead moves bar 1 of the tempo map to the playhead, or to the selected hit when a hit is selected. The tempo, the tempo steps, the meter and the meter changes stay. One undo step; the status line says "bar 1 at 2.025 s".
- Bars|Beats ruler: the bar-1 marker is drawn in the accent colour. Drag it to move bar 1, or double-click it and type the time in seconds. A click elsewhere on the ruler puts the playhead, as before.

### Tracks
- A project with tracks always has a selected track. The first track is selected when you open a project. A click on empty lane space and Escape clear the hit selection only, and a second click on the selected track header keeps it selected. When the selected track leaves the project, the next track in order is selected, else the previous one. The Inspector shows "No track selected" only for a project without tracks.
- Track > Remove Track removes the selected track from the project, with its hits, Learn examples and layer settings. Delete or Backspace does the same when no hit and no tempo marker is selected. A dialog asks first ("Remove track t2 and its 67 hits?"); the audio file is not touched. Cmd+Z brings the track back with its hits, settings and mix; the waveform is built again. A track that followed the removed track detects on its own from then on. Agents and the command line: `tracks_remove`.

### Playback
- Follow "smooth" on a zoomed-in view (under about 5 s at 48 kHz) no longer stutters or flickers: the view stands still and flips one page when the playhead reaches 90 % of it, with the playhead at a quarter of the new view. A wider view glides as before.

