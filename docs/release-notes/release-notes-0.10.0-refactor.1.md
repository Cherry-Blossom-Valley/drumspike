# Drum Spike 0.10.0-refactor.1 (Apple silicon test build)

A test build of the engineering refactor (branch refactor/structure). It is not a feature release.

### What is the same

- Detection is unchanged: the same hits, times, velocities and MIDI bytes as 0.9.19 on every reference set we measure.
- Project files stay compatible with 0.9.x.

### What changed inside

- The code has clear layers: a pure detection core, one application service with a single command registry, thin adapters (desktop shell, bridge, CLI) and a page split into modules.
- The menu code and the page script are split into small modules. The test suite keeps the tests that guard behaviour and drops duplicates and retired prototype checks.
- docs/engineering/refactor-plan.md and ARCHITECTURE.md describe the design.

### Who should use it

- Apple silicon Macs only. Intel Macs: stay on 0.9.x.
- Test on copies of your projects. Report anything that looks or feels different from 0.9.19.
