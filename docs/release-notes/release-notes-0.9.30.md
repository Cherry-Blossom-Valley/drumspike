# Drum Spike 0.9.30

Changes since 0.9.29:

### Agent fine-tune

- The agent names the hand-placed hits through a helper, `DSAgent.handPlaced(track)`, on every track. The training log is too large for an agent to read, so a run on a project with Training Mode records stopped and asked the user about hits it had moved itself (#418).
- A hit counts as looked at only after `DSAgent.look` on that hit. A row of `plan` or `audit` is a listing. A hit whose foot lies far from it is a look and a listing, never a move by that number (#418).

### Delivered with 0.9.27 to 0.9.29

The notes of those releases describe these tickets of the same batch:

- Agent set-up and the help flags (#383, #384).
- The agent drives the open window (#388).
- The judge agrees with the detector, and the placement measure tool (#385, #394).
- The judge measures the line directly before the stroke and measures slow strokes, and the skill's review steps (#408, #409, #415).
- Training Mode explained, and records that say who made them (#382, #389).
- The Settings dialog as a preferences window, and its pitch labels (#387, #393).
- The agent fine-tune tutorial (#380, #381).

### Known limit

- The agent cannot yet list strokes that have no hit (#412).
