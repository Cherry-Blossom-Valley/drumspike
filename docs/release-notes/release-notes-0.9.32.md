# Drum Spike 0.9.32

Changes since 0.9.31:

### Agent fine-tune

- The judge takes a sample inside the detector's band as the foot only when the line leads to it. When the line crosses zero directly before the stroke, the foot moves back to that crossing. A motion under the band and a dip after a rise now get a reason; a dip gives no suggestion. In a test with late hits planted on real sessions, the judge left 5 of 9872 without a reason (before: 98) (#421).

### Your project on disk

- Remove Track changes nothing on disk until you save. A quit without save, a crash or Undo leave the track's analysis in place. Before, the analysis folder was deleted at once (#427).
- The page shows an error when a script fails, instead of stopping without a message. A lost status call no longer counts as a finished job. Re-analyze asks in the page (#429).
- The Training Mode log is read line by line, so a large log no longer slows the app, and only your user can read the file (#428).

### Local endpoints

- The local endpoint of the window and the bridge refuse a request from its headers, before they read a body, and then close the connection. The body limit is lower. A stale endpoint file of a process that no longer runs gets no token (#424).
- MCP: each command says whether it is destructive, from one table in the command registry (#425).

### Development

- The ENST gate test reads its tom map from a variable, and a check keeps host paths out of the source (#426).

### Delivered with 0.9.27 to 0.9.31

The notes of those releases describe these tickets of the same batch:

- Agent set-up and the help flags (#383, #384).
- The agent drives the open window (#388).
- The judge agrees with the detector, and the placement measure tool (#385, #394).
- The judge measures the line directly before the stroke and measures slow strokes, and the skill's review steps (#408, #409, #415).
- The agent names hand-placed hits through a helper and looks at every hit (#418).
- Training Mode explained, and records that say who made them (#382, #389).
- The Settings dialog as a preferences window, and its pitch labels (#387, #393).
- The agent fine-tune tutorial (#380, #381).
- The bridge answers only its own page and local tools (#422).

### Known limit

- The agent cannot yet list strokes that have no hit (#412).
