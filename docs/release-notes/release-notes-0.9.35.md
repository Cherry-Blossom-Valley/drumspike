# Drum Spike 0.9.35

Changes since 0.9.34:

### Agent fine-tune

- The agent scans every track for strokes that the detector computed but that carry no hit. It lists them for you with the reason, sorted by how far the Sensitivity slider would have to move: the near ones with their times, the far ones as counts. It adds no hit on its own, and the scan changes nothing in the project (#412).
- The scan states its limit: a stroke without a detector candidate, and a stroke at the same time as a hit on another track, are not found (#412).
- The skill, the guide chapter and the agent API page describe the new step (#412).

### Development

- A measure tool runs the scan on real recordings and removes every hit once; its report is in `docs/measure/agent-scan.md` (#412).
