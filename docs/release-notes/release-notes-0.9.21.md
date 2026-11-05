# Drum Spike 0.9.21

Changes since 0.9.20:

### Detection

- Hit placement version 8. Two corrections to where a hit lands on the waveform, both from a report on 0.9.20: a kick hit sat 4 samples before the first visible motion when the line before it stood at a small offset (the slow wave after the previous stroke); the walk-back to the line now measures from the line's level, not from zero. And a snare hit sat 1 to 2 samples into its attack when the stroke's attack rose too gently for the steep-edge rule; such a hit now sits on the last line sample before the attack leaves the floor. Measured against hand-placed hits of a time-aligned session: snare hits within one sample of the engineer's position rise from 83 % to 96 %, kick hits from 98.8 % to 99.4 %; no hit on five reference sessions moves by 0.25 ms or more (#338).

### Documentation

- AGENTS.md keeps only the rules that past mistakes taught; the code map, the placement of new things and the architecture diagram are in ARCHITECTURE.md and the README (#337).
