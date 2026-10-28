# The event policy

R16 (#40). The meaning and preservation of the event flags in one place: one predicate decides
"leaves the output", one decides "kept", and the truth table
(tests/fixtures/event-policy/table.json) drives the Rust test and the page e2e test, so the
two sides cannot drift.

## The flags

| Flag | Set by | Meaning |
|---|---|---|
| `cut` | loudness rule (histogram.rs), flam.rs (stroke under the cut) | detected hit under the track cut level |
| `bleed` | tom_bleed.rs, cross_bleed.rs (never on an edited/kept hit) | bleed from another drum |
| `filtered` | learn_filter.rs; stamped at analysis store time (histogram.rs) | hit unlike the Learn examples; leaves the output |
| `edited` | service `restore` and hand velocity (edit.rs); the document's reserved edit flag | the engineer keeps it |
| `added` / `orig` | add / move op (edit.rs) | structural: `is_manual` = `added` or `t != orig` |
| `learned` | Learn Add / Learn sweep / analysis | a Learn example |
| `flam` | flam.rs (second stroke) | flam stroke |
| `roll`, `rescue`, `stage8` | the model path; carried in `extra` | carried, no output effect; confidence cap (confidence.rs) |

## The policy

    out(e)  = (cut || bleed || filtered)          [Rust, after the strip]
    hgIsCut(e) = (cut || bleed || filtered) && !isManual(e)   [page]
    kept(e) = is_manual(e) || edited === true

- One predicate, one module: `crates/app/src/policy.rs` (`is_out`, `event_out`, `is_manual`,
  `is_kept`). The duplicates call it: `export::is_cut` (MIDI, spikes, Export All, the click
  in transport.rs), `histogram::event_cut` (flam, loudness), `confidence::is_out`, and
  `tempo::is_cut`.
- R1 — a kept hit is never cut, bled or filtered. Rust enforces by stripping the marks on
  edit (edit.rs); the page enforces by the `!isManual` guards. One policy, two mechanisms;
  the truth table proves they agree (a kept hit that still carries a mark is a state no
  document holds, and both mechanisms take it out of the output).
- R2 — tempo: the plan draft expected tempo to keep `filtered` hits. This tree does not:
  `tempo::is_cut` has called the shared predicate since 0.9.4, so a cut, bled **or
  filtered** hit is no tempo candidate (`policy::TEMPO_USES_ONE_PREDICATE`). The behaviour
  is unchanged; the table's `tempo` column pins it.
- R3 — `edited` is the document's reserved flag; `is_manual` is structural
  (`added` or `t != orig`); `is_kept = is_manual || edited`.
- R4 — undo/redo restores field presence via `was`; `was` is an op payload, never a stored
  event flag.

## The truth table

One row per combination: the event JSON plus the expected `audible`, `exportable`, `kept`,
`out`, `tempo` and `grey` (lnGreyHit). The Rust test (crates/app/src/policy.rs) and the page
test (tests/e2e_app/test_event_policy_table.py) read the same file; a planted wrong
expectation fails both.

## Old documents

No migration, no rename: typed fields default to `None`, unknown and ill-typed keys ride in
`extra` and are written back verbatim. A document with every flag (including `stage8`) loads
and re-saves byte-identically (crates/core/src/project/document/tests.rs).
