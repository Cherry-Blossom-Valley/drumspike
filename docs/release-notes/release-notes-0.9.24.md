# Drum Spike 0.9.24

Changes since 0.9.23:

### Detection

- The cross-track bleed rule judges every hit on the first motion of its own signal, not on the refined hit position. Before, a change of a hit's placement by a fraction of a millisecond (as placement version 9 made on snares) could flip the bleed verdict of a tom hit next to it. Now the verdict does not move with the placement; the hit lists of the measured sessions are unchanged (#355).

### Training Mode

- Every move and add record says how precisely the editor saw the hit: `sample` when a sample was a pixel or wider on screen, `ms` at a coarser zoom. The record carries the view (the visible span in milliseconds and the pixels per sample); a delete record carries the view without a precision. The export tool prints the counts and histograms per precision (#353).
- Hits > Confirm Hit logs the selected hit as reviewed, right where it is. Nothing in the project changes: no edit, no undo step, no unsaved mark. The item is enabled only with Training Mode on and a hit selected. The export tool counts the confirm records per role (#354).
