Changes since 0.9.4:

### Lanes
- The playhead keeps moving smoothly while the computer is busy (for example during a large build): frames drawn between two clock updates no longer lose the playhead.
- Drag the bottom edge of a track header to change that track's height, as on the lane edge; a double-click resets it.
- New keys: Option+Cmd+1/2/3 for Track Height Small/Medium/Large, Shift+B for the Bars|Beats ruler, Shift+R for the Markers ruler.

### Stability
- Long sessions use less memory: the cache of decoded tracks is bounded by bytes (512 MiB), and a file that several views ask for at the same time is decoded once.
- A damaged waveform cache file on disk now gives a clear error instead of a crash or a huge memory allocation.
- Spike files are written in blocks, so an export of a long track no longer holds the whole file in memory.

### Guide
- The guide shows the seven pictures of chapters 6 to 13.

### About
- Help > About shows the Cherry Blossom Valley logo, the licence line and links to the website and the source code.
