# Drum Spike 0.9.18

Changes since 0.9.17:

### App

- Option+Tab steps back to the previous hit again. In 0.9.15 to 0.9.17 the key moved the keyboard focus over the buttons instead, because the window took Tab keys with a modifier before the menu saw them; Control+Tab and Control+Option+Tab (the steps over all tracks) had the same fault (#317).

### Detection

- Hits sit closer to the start of the attack. Kick hits no longer land a few samples before it, and snare hits that landed a little late now sit earlier. A new analysis now puts a hit on the last sample that is still on the line, as you see it on screen. Placement rule version 7 follows the attack's tail back while it is clearly off the line, by at most 0.15 ms. On a hand-checked session, the mean distance to the hand-placed kick hits fell from 0.57 to 0.26 samples. Kicks 2 or more samples early fell from 24 to 2, and snares 2 or more samples late fell from 31 to 25. On the public ENST-Drums set, the ±10 ms results did not change (#321).

### Project

- The repository page and the landing page now link to GitHub Sponsors and Buy Me a Coffee (#278).
