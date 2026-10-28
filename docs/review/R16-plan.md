# R16 plan: event meaning and preservation policy — a single domain definition

Ticket #40, step 1 (plan only; no code until the owner answers).

Scope of this document: (1) the event-policy table — every flag, and what each
combination means for audible / exportable / kept / restored / Learn-suppressed /
provenance; (2) where Rust and page decide it today, file:line; (3) how one fixture
would drive both test suites; (4) how old documents stay readable.

## 1. The flags

An event is the JSON object on a track's event list (`t` required; everything else
optional). Typed fields are `t, v, added, orig, edited, rise, rise_db,
timing_confidence, polarity, id` (crates/core/src/project/document.rs:95-124);
every other key (`cut, bleed, filtered, learned, flam, roll, rescue, stage8`,
detector records) lives in `Event.extra` and round-trips verbatim
(document.rs:122-123, 343, 361).

| Flag | Set by | Meaning |
|---|---|---|
| `cut` | loudness rule: histogram.rs:75/:95 (typed + JSON paths), flam.rs:135 (flam stroke below cut) | detected hit under the track cut level |
| `bleed` | tom bleed rule tom_bleed.rs:274; cross-bleed cross_bleed.rs:278; never on an edited/kept hit (tom_bleed.rs:127-130) | bleed from another drum, not a real hit on this track |
| `filtered` | Learn filter learn_filter.rs:502/:614; also stamped at analysis store time histogram.rs:220 | hit unlike the Learn examples; stays in the lane, leaves the output |
| `edited` | service `restore` (edit.rs:212) and hand velocity (edit.rs:231-233); undo brings it back via `was` (edit.rs:135) | the document's reserved edit flag (document.rs:108-109) — the engineer keeps it |
| `added` / `orig` | add op edit.rs:176; move op edit.rs:178/:220 | structurally detect a hand hit: `is_manual` = `added` or `t != orig` (edit.rs:66-78) |
| `learned` | Learn Add edit.rs:171-173; Learn sweep learn.rs:830/:848; analysis histogram.rs:222 | a Learn example (P4-38b) |
| `flam` | flam rule flam.rs:133 (second stroke, plus `cut` if under the cut) | flam stroke |
| `roll`, `rescue`, `stage8` | **not written in this tree** — the model path (analyze.rs:58-60, comment at analyze.rs:8); carried in `extra`, capped by confidence.rs:102-107 | model-path tags |

## 2. The policy table

One predicate decides "leaves the output", everywhere except tempo:

    out(e)  = (cut || bleed || filtered) && !is_manual(e)     [ && !edited, see rule R1 ]

and "kept" (survives re-pick / Re-analyze / the bleed and flam rules):

    kept(e) = is_manual(e) || edited === true

Per combination (M = manual, E = edited flag):

| Combination | Audible (playback/click) | Exportable (MIDI, spikes) | Kept (re-pick / Re-analyze) | Restored | Learn-suppressed | Provenance |
|---|---|---|---|---|---|---|
| plain detected hit | yes | yes | no (re-detected) | n/a | no | detector record |
| `cut` | no (transport.rs:1054) | no (export.rs:67-72) | no | un-cut when level rises or re-run | no | detector.cut |
| `bleed` | no | no | no | `restore` op removes bleed, sets `edited` (edit.rs:208-212) | no | detector.bleed / cross_bleed |
| `filtered` | no | no | no | `restore` (edit.rs:209) | yes (grey, Tab skips) | detector.learn |
| `learned` (not filtered) | yes | yes — forced back into the pick (learn_filter.rs:511) | re-marked by the rule re-run | Forget removes | no (it is the example) | detector.learn |
| `flam` (first stroke uncut) | yes | yes | re-run keeps non-edited flams preserved (flam.rs:115) | no op; rule re-run re-decides | filtered with its first stroke (learn_filter.rs:597-618) | detector.flam |
| M (added or moved) | yes | yes | kept — merge_edited wins inside MERGE_WINDOW 0.020 (edit.rs:284, 291-301) | was/undo restores `orig`, field presence | never filtered (manual guards) | hand edit |
| E (`edited`) | yes | yes | kept (is_kept, edit.rs:84) | was/undo restores `edited` | stripped (R1) | hand edit |
| M or E **with** cut/bleed/filtered present | the marks must not gate: Rust strips them (edit.rs:252-256); the page's predicates guard with `!isManual` | | | | | |
| `rescue`/`roll`/`stage8` on top of any row | as the base row decides | as the base row decides | carried | was-restorable | as base row | confidence tag cap (confidence.rs:102-107, lowest tag wins) |

Rules (the policy in one place):
- R1 — a kept hit (M or E) is never cut, bled or filtered. Rust enforces by
  stripping (edit.rs:250-258); the page enforces by predicate guards (app.js:5963,
  5964, 6060, 3094). One policy, two mechanisms.
- R2 — exactly one "out of output" predicate everywhere: `export.rs is_cut`
  (export.rs:67-72, used by MIDI export :674, Export All :746, commands.rs:494),
  `histogram.rs event_cut` :57-62 (flam.rs:104, loudness), click (transport.rs:1054),
  page `hgIsCut` (app.js:5963). The one deliberate exception: tempo detection's
  local `is_cut` (tempo.rs:120-121) omits `filtered` — old-Learn-filter hits stay
  in tempo. The plan either codifies that exception or removes it; owner call.
- R3 — `edited` is the document's reserved flag; `is_manual` is structural
  (`added` or `t != orig`). `is_kept = is_manual || edited` (edit.rs:80-85).
- R4 — undo/redo restores field presence byte-identically via `was`
  (edit.rs:115-145; page `hitWas` app.js:5917-5921). `was` is an op payload,
  never a stored event flag.

## 3. Where Rust and page decide it today (file:line index)

Rust:
- predicates: `is_manual` edit.rs:66-78; `is_kept` edit.rs:80-85; `is_cut`
  export.rs:67-72; `event_cut` histogram.rs:57-62; tempo `is_cut` tempo.rs:120-121;
  `is_out` confidence.rs:130-134.
- stripping on edit: edit.rs:250-258 (comment edit.rs:7 admits the page duplicates it).
- merge: `merge_edited` edit.rs:291-301 (page mirror `mergeKept`/`hgMerge`).
- rules that respect kept: tom_bleed.rs:127-130, flam.rs:99, cross_bleed.rs:278.
- click: transport.rs:1054. Learn scoring skips out-hits: learn.rs:566, 961, 1156-1159.
- document round-trip: document.rs:95-124 (typed), :122-123/:343/:361 (`extra`),
  :337/:355 (`edited`).

Page (static/app.js):
- canonical flag list `normEvents` :70-84; ids :72/:86.
- predicates: `isManual` :89, `isKept` :93, `hgIsCut` :5963, `hgCutOnly` :5964,
  `lnGreyHit` :5245 (live preview :5233-5243).
- flag stripping: `eventsBody` :3094 (delete cut on manual; bleed/filtered on
  manual or edited), sent via `pushEvents` :3097 / `flushEvents` :3130.
- gated UI: grey lane drawing :2685-2687, spike lane :3835-3837, histogram :5986,
  Tab skip :2789, counts :4892/:5076-5077/:6285, inspector :6280-6290/:7321,
  Keep/Discard dialog `pendingKeep` :6634 → `keep_edited` on `B.analyze` :6639.
- undo/redo: `hitWas` :5917-5921, `delKeep` :5923, `edOwns` :3031.

The known mismatch to fix in R16: Rust decides "out" by *stripping* marks on kept
hits (edit.rs:252-256) while the page decides by *predicates* (`!isManual` guards),
kept in sync only by comments (edit.rs:7, app.js:90-93). R16 should make one side
the definition (the service) and reduce the page to reading it, with the shared
fixture proving both agree.

## 4. One fixture driving both test suites

Proposed fixture: `tests/fixtures/event-policy/project.drumspike` — a small
document whose tracks carry one event per combination in the table above
(plain, cut, bleed, filtered, learned-this, learned-filtered, flam pair,
added, moved-with-orig, edited-restore, edited-velocity, rescue, roll), plus the
detector records that set them.

- Rust suite: a `drumspike-core` cross-load test (pattern: `cargo test -p
  drumspike-core --test project_cross_load`, justfile:37) loads the fixture and
  asserts the typed/extra split, `Event` fields and `extra` keys round-trip; a
  `drumspike-app` test runs the policy functions (`is_kept`, `is_cut`,
  `merge_edited`, apply_ops restore) against the fixture's event lists and asserts
  the "Audible/Exportable/Kept" columns.
- Python/page suite: pytest (tests/, via the app bridge; fast loop justfile:13-15)
  opens the same fixture through the service and asserts, per combination: the
  click schedule, the export (MIDI note count, spike count) and the page's own
  predicates (`hgIsCut`, `lnGreyHit`, `isKept`) evaluated against the same events —
  so the page predicate and the Rust predicate are checked against one truth table.
  The e2e_app shard (justfile:17-20) covers the visible bits (grey, Tab skip) on
  the same document.

The fixture is data, not code: both suites read the same file, so any future flag
addition is one fixture edit plus a new row, not two suites drifting.

## 5. Old documents stay readable

- Format 1 has no migration: missing keys default to `None`
  (document.rs:233-234, 462); only `t` is required (document.rs:331).
- Unknown/extra keys — the whole flag set here — live in `extra` and are written
  back unchanged (document.rs:122-123, 343, 361); a typed key that is `null` or
  wrong-typed also stays in `extra` verbatim (document.rs:38-48, 13-21), and a
  field the code sets wins over an ill-typed stored value (document.rs:19-21).
- R16 therefore adds no migration and no rename: the single domain definition is a
  documentation + predicate + fixture change. Old projects (including hand-made or
  third-party documents with flags this tree never wrote, e.g. `stage8`) load and
  re-save losslessly; the policy table only defines what each flag *means* to the
  output paths, and any flag not in the table keeps today's "carried, no output
  effect" behaviour.
