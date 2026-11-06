# Drum Spike 0.9.23

Changes since 0.9.22:

### Hit placement

- Placement version 9: the hit sits on the last line sample before the stroke's first motion, whatever its sign. A stroke often opens with a small swing of the other sign than its steep edge (a bump before a kick edge, a dip and a rise before a snare edge); version 8 followed the edge's sign only and stopped at the zero crossing after that swing, 7 to 14 samples late on such snares. Version 9 walks back over the swing when it stands out from the line before it and is a swing rather than a flicker; a line that is a slow wave or an offset stays the line, as before. Measured against hand-placed and labelled hits of three sessions: snare hits 2 or more samples off fell from 75 % to 27 % and from 88 % to 22 % on the two affected sessions, kicks from 44 % to 23 % and 41 % to 29 %; the session that version 8 already placed well stays as good or better; toms unchanged. Projects analysed before keep their hit times until you analyse again (#351).
