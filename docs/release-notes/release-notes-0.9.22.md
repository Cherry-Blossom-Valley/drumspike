# Drum Spike 0.9.22

Changes since 0.9.21:

### Training Mode (for refining the detection; off by default)

- View > Training Mode. With it on, every hit you move, add or delete and every Learn This or Forget This mark is written as a record to a training log in the app's home folder, with the rule versions in force, the hit's confidence and a 50 ms window of the audio around the old and the new position. The log never enters the project, and with the mode off nothing changes: no new field, no new file, the same responses as 0.9.21. `tools/training_export.py` turns a log into aggregates and a truth list (#343).
- In Training Mode the review queue (Hits > Next Doubtful Hit) also doubts the placement of a hit: no steep edge at the stored position, a line before it that is noisy or sloping, or onset estimates that disagree by 4 samples or more. The info line names the reason. Measured on a hand-placed session: every kick and snare hit that sits 2 or more samples off the engineer's position is in the queue, at a review cost of 14 % of the kick hits and 24 % of the snare hits (#344).

### Tests

- Invariance tests for detection and placement: a shifted, scaled, noisier, gapped, inverted or extended signal must give the hits the transformation predicts, sample for sample. Two findings on synthetic kits are recorded as ignored tests for a later look (#345).

### Internal

- The trace prints left from the development of placement version 8 are gone (#342); the menu command count in two tests follows the new item (#349).
