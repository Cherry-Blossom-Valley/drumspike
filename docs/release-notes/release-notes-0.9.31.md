# Drum Spike 0.9.31

Changes since 0.9.30:

### Security

- The page-mode bridge (`drumspike bridge`, the agent's way to work without an open window) now answers only its own page and local tools. Before, a web page in a browser on the same Mac could send commands to the bridge while it ran. A request that names another site now gets a refusal before any command runs. If you use the bridge, update. The local endpoint of the open window was not affected: it needs a token.

### Delivered with 0.9.27 to 0.9.30

The notes of those releases describe these tickets of the same batch:

- Agent set-up and the help flags (#383, #384).
- The agent drives the open window (#388).
- The judge agrees with the detector, and the placement measure tool (#385, #394).
- The judge measures the line directly before the stroke and measures slow strokes, and the skill's review steps (#408, #409, #415).
- The agent names hand-placed hits through a helper and looks at every hit (#418).
- Training Mode explained, and records that say who made them (#382, #389).
- The Settings dialog as a preferences window, and its pitch labels (#387, #393).
- The agent fine-tune tutorial (#380, #381).

### Known limit

- The agent cannot yet list strokes that have no hit (#412).
