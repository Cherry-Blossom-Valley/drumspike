# Drum Spike 0.9.25

Changes since 0.9.24:

### Detection

- Placement version 10: the line is judged by its own noise, not by a fixed share of the peak. On a noisy line a gentle opening swing of either sign now counts as the stroke's first motion (the visibility band and the run length scale with the line's spread, the fixed shares stay as a floor), where version 9 refused the whole walk-back and fell 2 or more samples late on such snares and kicks; a line that is a slow wave or an offset stays the line, as before. Projects analysed before keep their hit times until you analyse again (#362).
