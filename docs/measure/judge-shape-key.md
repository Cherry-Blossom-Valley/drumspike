# Judge: a lead bump by shape (#385)

Measured with `tools/judge_measure.py` (test: `tests/test_judge_measure.py`). Aggregates only. No judge change ships: **on the
one-rise-sample shape the gold rows carry both answers (532 foot, 236 edge, 161 neither), so no rule of size or sign reproduces
the labels, and the judge stays as on main until the ground truth is sharpened.**

## Runs

Gold rows: `confirmed = 1 OR label_off != 0 OR owner = 1`, Sessions A to J, 4981 rows. Per session: exact / within 1 sample /
summed distance of the judge's move to the label (no move counts as 0).

- Before: the judge on main (departure at max(3 x spread, 0.03) of the local peak).
- Try 1: decision A as a fixed band: departure at the detector's 2.5 %, run band 2 %, walk back over every sample over 2 %.
- Candidate: the old departure level plus the foot of the steep edge (walk back over lead-in samples over 2 % that rise into
  the edge, stop at the first one that returns toward the line).

| Session | before | try 1 | candidate |
|---|---|---|---|
| A | 32/167/265 | 66/176/238 | 71/177/212 |
| B | 76/218/418 | 72/213/458 | 89/218/425 |
| C | 166/257/866 | 145/218/1206 | 170/267/950 |
| D | 242/337/1217 | 223/293/1515 | 251/314/1358 |
| E | 399/533/304 | 465/525/259 | 467/530/240 |
| F | 383/481/258 | 377/455/375 | 391/469/254 |
| G | 619/749/514 | 644/736/568 | 638/739/496 |
| H | 415/508/339 | 453/507/342 | 447/511/300 |
| I | 386/473/409 | 364/441/507 | 383/470/395 |
| J | 296/416/1013 | 320/431/983 | 323/446/969 |
| all | 3014/4139/5603 | 3129/3995/6451 | 3230/4141/5599 |

Rows against before: try 1 648 worse / 737 better; candidate 526 worse / 705 better. The Gate "no row worse" fails for both.

## Worse rows by shape (candidate)

Key: the classes of the samples before the departure, from the edge back (L within 2 % of the line, b above, S the departure).

| shape | worse | better |
|---|---|---|
| bLLLLLLLLLLLS (one rise sample, then line) | 194 | 503 |
| bbbbbbbbbbbbS (no line, a slow ramp) | 26 | 8 |
| bbLLLLLLLLLLS | 21 | 58 |
| bbbLLLLLLLLLS | 21 | 7 |
| no shape (suggestion appears or disappears) | 74 | |

## Refine: the one-rise-sample shape, split by label side

With the wider key (the first pre-edge sample over 2 %, then the line): 929 rows. Against before: 284 worse, 571 better, 74 equal.
Label side per row: **foot** (label one before the rise sample) 532, **edge** (label at the edge, the old answer) 236, **neither** 161.

Measured: the rise sample's size r (share of peak over the line), the edge's first step s (sign included), r/s.

- Sign: rise against the step's sign is 154 rows; worse/better there is 63/91, no cleaner than the same sign (221/480).
- r/s deciles: foot share 69 % (lowest decile) falling to 28 % (r/s > 1.9); edge share 23 % to 9 %; the rest is neither side
  (up to 63 % at r/s > 1.9).
- Best single band on r/s, s or r for "walk back iff in band": 534, 531, 529 of 929 right, the same as "always walk back" (532).
- Same-shape rows carry both labels: in the r/s 0.2 to 0.28 range 196 are foot, 81 edge.

A rule of the form "one lead sample counts as the stroke when it rises by at least a fixed share of the edge's first step" would
take about 57 % of the rows to the label's side, not 90 %. The labels disagree with each other on one shape (see the next section
for whose labels).

## Who made the labels, and the owner rows alone

The 4981 gold rows are not all the owner's hand. 497 rows have `owner = 1` (56 moved, 441 kept; all in Session E): the owner's
own review. The other 4484 are the coordinator's labels from the labelling round of 2026-11-06 (2513 moved, 1971 confirmed). The
sentence "the owner's hand and the app's placement disagree on one shape" above therefore reads: the coordinator's labels split
on that shape, which may be the noise of one reviewer. The owner rows alone (Session E; exact / within 1 / summed distance):

| set (Session E, 497 rows) | exact | within 1 | distance | vs before |
|---|---|---|---|---|
| before | 382 | 492 | 198 | |
| try 1 | 437 | 484 | 175 | 28 worse / 80 better |
| candidate | 439 | 489 | 143 | 26 worse / 80 better |

The coordinator's labels (4484 rows) for comparison: before 2632/3647/5405, try 1 2692/3511/6276 (620 worse / 657 better),
candidate 2791/3652/5456 (500 worse / 625 better).

The one-rise-sample shape (929 rows with the wider key), split by label side:

| rows | n | foot | edge | neither | candidate worse / better |
|---|---|---|---|---|---|
| all | 929 | 532 | 236 | 161 | 284 / 571 |
| owner = 1 | 101 | 78 | 20 | 3 | 23 / 78 |
| owner = 0 (coordinator) | 828 | 454 | 216 | 158 | 261 / 493 |

On the owner rows the foot side is 77 % of the shape (78 of 101) and "neither" almost vanishes (3), but 23 of those rows are still
worse, so the cut is short of the 9 in 10 the refine asked for. Decision rule: neither try 1 nor the candidate has 0 rows worse
on the owner rows (28 and 26 worse), so the judge stays as on main.

## Session K (candidate; not shipped)

Kick 30 moves of 63, snare 9 of 50, tom 0 (before: 31, 10, 0). Hits whose suggestion differs between strokes of one shape key:
0 after (kick 17 keys, snare 11); not defined before (the old judge has no shape key).

## Open

Whether a confirmed row that kept the app's placement is a sample-exact review or an acceptance goes to the owner. If the gold
definition narrows to owner-moved rows, a follow-up ticket reruns the tool.
