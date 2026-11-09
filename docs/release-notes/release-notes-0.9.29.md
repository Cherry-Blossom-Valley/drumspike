# Drum Spike 0.9.29

Changes since 0.9.28:

### Agent fine-tune

- The judge now measures slow strokes. It compared one sample's step with the detector's band, which is a level, so on kicks with a slow attack it answered "at the foot" without measuring. A late hit on such a kick was neither suggested nor listed by the audit (#415).
- A hit in a dip after the stroke has passed the detector's band is late. It no longer stands because its own sample is back on the line (#415).

### Delivered with 0.9.27 and 0.9.28

The notes of those releases describe these tickets of the same batch:

- Agent set-up and the help flags (#383, #384).
- The agent drives the open window (#388).
- The judge agrees with the detector, and the placement measure tool (#385, #394).
- The judge measures the line directly before the stroke, and the skill's review steps (#408, #409).
- Training Mode explained, and records that say who made them (#382, #389).
- The Settings dialog as a preferences window, and its pitch labels (#387, #393).
- The agent fine-tune tutorial (#380, #381).

### Known limit

- The agent cannot yet list strokes that have no hit (#412).
