# Drum Spike 0.9.33

Changes since 0.9.32:

### Hit placement

- Placement rule version 10. A stroke and its polarity-flipped copy now land on the same sample: on 1242 real hits, 4.8 % landed on another sample before, none now. The rule's counts of samples follow the sample rate, so a 96 kHz file is placed like its 48 kHz copy more often (snare hits that move: 17.8 % before, 9.0 % now). A new analysis runs version 10; a project keeps the version it was analysed with (#444).
- Learn This: a hit that Learn adds goes through the placement rule, like every other hit (#439).
- Session steps and flam strokes keep the track's parameters and its Hitpoint shift (#438).
- The second stroke of a flam is placed by the rule of its track, like every other hit, inside the span where the flam rule found it; outside that span the earlier placement stays. On the real sessions about half of the flam strokes move, most of them earlier by under 2 ms. A new analysis runs this flam rule version 2; a project keeps the version it was analysed with (#445).
- Two strokes at the minimum gap are kept or dropped by the gap in whole samples, not by their position in the file (#440).

### Analysis

- An analysis picks its hits at the track's stored sensitivity. Before, Analyze ignored a sensitivity that was already stored and used the default; only a re-pick used it (#454).

### MIDI export

- The MIDI file picks its ticks per quarter note from the tempo and the sample rate, so that one tick is one sample where the MIDI standard allows it. Each note sits on the sample of its hit. Where a whole value does not exist, the export message states how far a note can be from its sample. The spike file is sample exact at every tempo and rate, as before (#441).

### Agent and MCP

- Setting one parameter of a track through the MCP tools or the command line keeps the other stored parameters. Before, one field replaced the whole set, so a new sensitivity deleted the stored Hitpoint shift while the hits kept it. A value of null removes one parameter; `replace: true` stores exactly the sent set (#442).

### Agent fine-tune

- The judge has a second opinion: the pattern of the track's own clear hits. It moves a hit only when both opinions agree within one sample; when they differ, the hit is listed with both positions and is not moved. In the measure on labelled hits the wrong moves fell from 84 to 24 (#447).

### Development

- Two measure tools for the placement rule: invariance on real stems (gain, polarity, offset, hum, noise, resample, cut) and placement by the track's own clear hits. The invariance tool names the rule version it measures (#443, #444).
- The release workflow reads the signing identity from a repository variable (#435).
