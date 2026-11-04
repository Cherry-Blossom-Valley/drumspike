# Drum Spike 0.9.20

Changes since 0.9.19:

### Engineering

- The code is restructured into clear layers: a pure detection core, one application service with a single command registry, thin adapters (desktop app, command line, MCP) and an editor page split into modules. Detection is unchanged: the same hits, times, velocities and MIDI bytes as 0.9.19 on every reference set we measure. Projects stay compatible with 0.9.19 in both directions.
- The test suite keeps the tests that guard behaviour and drops duplicates and retired prototype checks. The documentation describes the code as it is now.

### Agent interface

- The analysis job reports each track's hit count after the session steps (cross bleed, ghost gate, tom bleed, flam): the count the editor shows and `events_get` answers. Before, the command line and MCP got the count from before those steps (#331).
