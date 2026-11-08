# Drum Spike 0.9.28

Changes since 0.9.27:

### Agent fine-tune

- The judge measures the line directly before the stroke, not at the far left of the view. A drift earlier in the view no longer rejects a hit with a clean edge. A hit stands only if its own sample is within the detector's band, which fixes hits that stayed late (#408).
- The judge answers with a shape verdict: `late`, `at_foot` or `no_quiet_line`, with the foot offset, the rise and the quiet ratio. A hit gets no answer only when no quiet stretch exists before it (#408).
- `DSAgent.audit(track)` lists every late hit, whatever the judge suggested (#408).
- The skill no longer treats "no suggestion" as "leave it". The agent looks at each such hit, moves it when it is late, re-reads every moved hit, and runs the audit before the save. Remaining late hits must be zero, or be listed one by one with a reason (#409).
- The report splits the hits that were not moved into "looked at, right" and "no quiet stretch". Its "not looked at" column must be zero (#409).
- `DSAgent.render` gives a picture of a hit's view, and `DSAgent.backups` names the backup that holds the state before the session (#409).

### Known limit

- The agent cannot yet list strokes that have no hit. This is filed as its own ticket.
