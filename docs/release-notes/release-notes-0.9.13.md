Changes since 0.9.12:

### Analyze
- Analyze works again for a project whose track names carry numbers on both sides of a dash, for example "Tom 1-2" or "Kick 2-1". Such a name was read as a track range and the analysis stopped with "the pattern ... matches no track". The role of the track was never the cause. A track's own name now always names it; the name patterns for scripts (globs and ranges) work as before.

### Roles
- Track names from other studios get their role from a list of common terms for each drum: kick, snare, tom, hat and cymbal, each with about 20 spellings (BD, KickIn, SNTop, Tom1HH, Floor Tom, HH, OH L, Ride, China and so on). Names written as one word (KickIn, SNTop) and Pro Tools comp and take suffixes are read correctly. A name that is one typo away from a term gets the role as a guess, marked "guessed" in the Add Tracks list and in the Inspector. Room and ambience mics, busses, mixes and prints get no role from the name.
- A role you set by hand always wins and never blocks the analysis.
- The Name pattern field of the Track Inspector is gone. The hint under the role still names the words the role came from.
