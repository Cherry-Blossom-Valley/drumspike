# Judge: a lead bump by shape (#385)

## The shape judge (#408, shipped)

Measured 2026-11-08 with `tools/judge_measure.py` on the same rows as below (aggregates only). The judge of `static/agent.js`
now measures the line directly before the stroke: the steepest step within 12 samples before to 6 after the hit, walked back
to its foot (the walk stops where a step falls under `FOOT_STEP_SHARE` = 25 % of the steepest), the line from the 12 quietest
samples ending nearest the foot, the samples above the detector's band (2.5 % of the hit's peak over 5 ms) between that line
and the foot walked over, and the hit sample in the guard. `judge` answers `foot_off`, `rise`, `quiet_ratio` and `verdict`
(`late`, `at_foot`, `no_quiet_line`; a line needs `quiet_ratio` of at least `QUIET_MIN` = 3); `DSAgent.audit` lists the `late` hits.

| set (exact / within 1 / summed distance) | main | shape judge |
|---|---|---|
| owner rows, 497 (Session E) | 371 / 492 / 206 | **441 / 493 / 125** |
| Gate | >= 382 / >= 492 / <= 198 | met |
| Session K hand-placed kick hits, left alone | 5 of 5 | 5 of 5 |
| Session K confirmed kick hits, answered 0 | 19 of 19 | 19 of 19 |
| all 4981 gold rows | 2658 / 3982 / 5926 | 2608 / 3719 / 8341 |

The 4484 coordinator rows are not the Gate and get worse in distance: against main 650 rows worse, 614 better, 3717 equal; the
summed distance grows in Sessions C and D (890 to 1965, 1243 to 2968), where the main judge moved on 221 and 242 rows and the
shape judge on 69 and 128 (the others get `no_quiet_line` or stand within the band). The Gate is the owner's rows; the
coordinator's labels there are the noise the sections below describe.

Constants and how they were chosen: `FOOT_STEP_SHARE` 0.25 (the value the owner's session proposed) and `QUIET_MIN` were swept
over 0.15 to 0.5 and 1 to 6 on the gold rows. The owner rows do not separate them (441 exact for every pair, within 1 between 490
and 493, distance 120 to 132), so the proposed 0.25 stays and `QUIET_MIN` is the value that keeps the ring case (steepest step under
3 times the largest quiet step) out of the line; 1.5 would have given 120 instead of 125 on the owner rows. No constant was tuned to
a named hit. Two shape rules came from rows the first version got wrong, both stated by principle: the line is the quietest
stretch ending nearest the foot, not the 12 samples before it (a ramp of motion there pulled the median off the line), and a view
whose steepest step near the hit is under the band holds no edge the detector reads, so the hit stands (its stroke rises past the
view). A later move for a hit inside the band was tried and costs 55 exact on the owner rows (441 to 386), so it stays out.

### What the owner rows can and cannot show (coordinator check, 2026-11-08)

The Gate of #408 on the owner rows does not test this fix. A judge that never moves anything scores 441 / 491 / 130 there, against 441 / 493 / 125 for the shape judge, because 441 of the 497 rows need no move and only 3 of the 56 owner-moved rows have the hit sample over the band.

The rows that test it are those with the hit sample over the detector's band. These are mostly coordinator labels, so read them as a direction and not as truth:

| rows, hit sample over the band | n | main exact | shape exact | main distance | shape distance |
|---|---|---|---|---|---|
| label 2 or more samples earlier | 341 | 71 | 121 | 1617 | 1024 |
| label 1 sample earlier | 114 | 0 | 101 | 116 | 31 |
| label 0 | 38 | 32 | 10 | 50 | 96 |

A null answer falls from 1054 of the 4981 rows to 74. A hit whose own sample is inside the band now stands. That costs the 23 owner rows labelled later, 20 of which main answered exactly. Neither judge answers the 29 owner rows labelled one sample earlier. Every later Gate of the judge names the never-move baseline and a slice that contains the failure being fixed.

## The band is a level (0.9.29)

A real agent run on Session K showed a blind spot of the shape judge. It compared one sample's step with the detector's band.
The band is a level, so a slow stroke passed the band while every step stayed under it. The judge then left early with
"at the foot": 62 of 63 kick hits of Session K took that exit. A test that shifted 20 of those hits 2 to 8 samples late found
none of them. Two changes close it: the early exit reads the level's range near the hit, and a hit stands only when nothing
between the foot and the hit passed the band (a hit in a dip after the first motion is late).

Measured on the 4981 gold rows, against a judge that never moves anything (exact / within 1 / summed distance):

| set | never move | 0.9.28 | now |
|---|---|---|---|
| all gold rows | 2412 / 3547 / 9682 | 2608 / 3719 / 8341 | 2802 / 4003 / 5422 |
| kick | 1613 / 2391 / 2704 | 1733 / 2474 / 2371 | 1809 / 2584 / 1805 |
| label 0 (should not move) | 2412 / 2412 / 0 | 2384 / 2400 / 96 | 2380 / 2396 / 128 |
| owner rows | 441 / 491 / 130 | 441 / 493 / 125 | 441 / 493 / 125 |

484 rows are better and 23 worse than 0.9.28. The 20 shifted hits: summed distance 90 before, 31 now, 17 of them `late`. The
owner rows hold few late hits, so they cannot show this gain; they show that the change costs them nothing. A third change was
measured and dropped: taking the foot from the first band crossing after the line cost owner rows (441 to 438 exact).

## A foot the line leads to (#421)

The test with 20 shifted hits left 3 unmoved. To find the shape classes, the test became a tool: `tools/judge_measure.py
--inject` judges every row with a placed truth (2468 rows, confirmed or owner) with its hit shifted 2, 3, 5 and 8 samples
late of the label, 9872 injected hits. On the 0.9.29 judge 98 of them read "at the foot" without a reason. Two classes:

- **A zero crossing.** The stroke starts with a small ripple: a half-wave to one side, through the line, a half-wave to the
  other side, then the edge. A late hit on the crossing stands on the line's level, so the judge called it the foot. Most
  of these are snare hits of Session J. Rule: a sample within the band is the foot only when the line leads to it. A foot
  that the level reached in one run from beyond the band on the other side is a crossing, and the foot goes back before
  the ripple. A step no larger than the line's own largest step ends the run, so a slow wave before the stroke stays line
  (#408). When the ripple stays under the band the hit stands (owner rule A) and the judge says so.
- **A dip.** A slow rise passes the band, dips back into it and then the edge comes; the kick hits of Session K that the
  20-hit test missed are of this class. The owner rows hold such a foot both ways: on one of them the dip is the labelled
  hit. A rule that moves these hits was measured and costs that row (441 to 440 exact, distance 125 to 131). The judge
  therefore gives no answer for a dip (`suggest` null, with the reason), so the agent leaves the hit and lists it.

No constant was added: the rule uses the detector's band and the line's own largest step.

| measure | 0.9.29 to 0.9.31 | now |
|---|---|---|
| all gold rows (exact / within 1 / summed distance) | 2802 / 4003 / 5422 | 2864 / 4152 / 5183 |
| owner rows | 441 / 493 / 125 | 441 / 493 / 125 |
| injected hits flagged as late, of 9872 | 9686 | 9762 |
| injected hits left without a reason | 98 | 5 |
| the same, by shift 2 / 3 / 5 / 8 | 38 / 52 / 3 / 5 | 4 / 0 / 1 / 0 |

202 gold rows are better and 71 worse. The largest group of worse rows are late hits whose foot is now read further back,
past the 12-sample range: the judge lists them as late without a suggestion where it suggested a partial move before.
Letting the suggestion follow such a foot was measured and dropped (summed distance 5357). On Session K the kick answers
are unchanged (3 moves of 63, the 19 confirmed hits stand); the snare gets 7 moves of 50, before 4. With every Session K
kick hit shifted the same four ways, 2 of 252 read "at the foot" without a reason before and 0 now (both are dips and are
listed).

## The pattern of the track as a second opinion (#447)

The owner's observation: an engineer knows how the clear hits of a track look and places an unclear hit by that. The judge
looks at one hit at a time. It now has that second opinion.

Per track the page builds a pattern from the track's own clear hits. A hit is clear when the 2 ms before it stay within 1 %
of the peak of the 5 ms after it; a hit that stands up its rise is therefore never clear. The pattern is the median of the
clear hits' feet (1/3 ms before the hit to 1/6 ms after it, each scaled to its own peak), re-aligned to itself three times.
Each hit then gets the offset where its own samples match the pattern best (zero-mean normalised correlation, 14 samples
each way at 48 kHz). No label and no value of a session is used; all spans are times and follow the rate. A match of the
whole attack was measured first and dropped: the body of a snare stroke differs from hit to hit, the first motion does not.

The rule: the judge moves a hit only when the pattern confirms the move within one sample. A hit the judge would move
while the pattern does not confirm it, and a hit the judge would leave while the pattern puts its start 2 or more samples
away, gets no suggestion and a reason that names both positions. The agent lists it for the user. A track with fewer
than 40 hits has no pattern. The pattern has no opinion on a hit it does not match: when the best match lies at the end of
the search range, or when the correlation is under 0.5 (less than a quarter of the hit's foot is explained). Without
this rule the table reads 22 wrong moves and 445 listed hits: the rule gives 2 wrong moves back and takes 18 hits off the
list. It is there because an opinion from a match of 0.1 is no opinion, and because it kept a right move in the page run
(a kick of the other slope than the rest of its track). Higher floors were measured and bring more wrong moves back.

Measured with `tools/judge_measure.py --at-current`: every labelled hit at the position of the current placement rule,
which is the case of a fine-tune run on a fresh analysis (4912 gold rows; 69 have no current position).

| measure | judge alone | with the pattern |
|---|---|---|
| wrong moves (the hit ends further from the label) | 84 | 24 |
| hits 2 or more samples off, left with an answer of 0 | 547 | 423 |
| hits listed for the user (no suggestion) | 141 | 427 |
| exact / within 1 | 2920 / 4147 | 2920 / 4150 |
| owner rows: exact / within 1 | 440 / 493 | 439 / 492 |
| owner rows: wrong / left 2 or more off / listed | 2 / 3 / 5 | 2 / 1 / 10 |
| injected hits flagged as late, of 9872 | 9762 | 9762 |
| injected hits left without a reason | 5 | 3 |
| injected hits left with a reason, not listed | 73 | 1 |
| injected late hits whose move the pattern took back | - | 205, of which 49 would have landed on the label |

Reading: the pattern does not make the judge more exact. It makes it safer: seven of ten wrong moves go, and almost every
planted late hit the judge left with a reason is now on the list. The price is a longer list, mostly on snare tracks, and
one owner row whose right move is now a listed hit. At the labelling-time positions (the tool's default run) the list is
longer still (161 to 876) and the exact count falls (2864 to 2661): those positions came from an older placement rule that
left almost half of the snare hits 2 or more samples off, so fewer hits are clear and fewer right moves are confirmed.

Limits: 4484 of the gold rows are labels from an agent with an older judge, so they favour the judge alone; the owner's 497
rows are the fair part. Toms have no labels. All 20 labelled tracks got a pattern.

## The earlier rounds (#385, #394; before the shape judge)

Measured with `tools/judge_measure.py` (test: `tests/test_judge_measure.py`). Aggregates only. No judge change ships: **on the
one-rise-sample shape the gold rows carry both answers (532 foot, 236 edge, 161 neither), so no rule of size or sign reproduces
the labels, and the judge stays as on main until the ground truth is sharpened.** The rows that split are the coordinator's
labels (4484 of the 4981 gold rows, the labelling round of 2026-11-06); the owner's own 497 rows (`owner = 1`, all Session E)
are the cleaner set: the candidate takes them from 382 to 439 exact and the summed distance from 198 to 143, with 26 rows worse
and 80 better, so it still fails "no row worse". The candidate itself is on the branch `crew/claude@x86-mac/385-candidate-judge`
(no PR; `static/agent.js` only), so it can ship unchanged if the owner accepts the residue.

## Runs

Gold rows: `confirmed = 1 OR label_off != 0 OR owner = 1`, Sessions A to J, 4981 rows. Per session: exact / within 1 sample /
summed distance of the judge's move to the label (no move counts as 0).

- Before: the judge on main (departure at max(3 x spread, 0.03) of the local peak).
- Try 1: decision A as a fixed band: departure at the detector's 2.5 %, run band 2 %, walk back over every sample over 2 %.
- Candidate: the old departure level plus the foot of the steep edge (walk back over lead-in samples over 2 % that rise into
  the edge, stop at the first one that returns toward the line).

| Session | before | try 1 | candidate | stroke-peak | guard |
|---|---|---|---|---|---|
| A | 32/167/265 | 66/176/238 | 71/177/212 | 17/126/325 | 15/157/279 |
| B | 76/218/418 | 72/213/458 | 89/218/425 | 59/220/400 | 33/214/458 |
| C | 166/257/866 | 145/218/1206 | 170/267/950 | 155/273/621 | 157/237/890 |
| D | 242/337/1217 | 223/293/1515 | 251/314/1358 | 88/226/1383 | 195/322/1243 |
| E | 399/533/304 | 465/525/259 | 467/530/240 | 333/532/348 | 379/534/309 |
| F | 383/481/258 | 377/455/375 | 391/469/254 | 179/484/446 | 310/485/283 |
| G | 619/749/514 | 644/736/568 | 638/739/496 | 539/752/586 | 606/751/518 |
| H | 415/508/339 | 453/507/342 | 447/511/300 | 314/513/397 | 420/516/289 |
| I | 386/473/409 | 364/441/507 | 383/470/395 | 210/368/834 | 298/394/564 |
| J | 296/416/1013 | 320/431/983 | 323/446/969 | 132/284/1624 | 245/372/1093 |
| all | 3014/4139/5603 | 3129/3995/6451 | 3230/4141/5599 | 2026/3778/6964 | 2658/3982/5926 |

Rows against before: try 1 648 worse / 737 better; candidate 526 worse / 705 better; stroke-peak 1572 worse / 410 better;
guard 540 worse / 100 better. The guard variant is the second #394 round (measured 2026-11-08 by hermes@linux-arm64): the
rule's bands stay on the local-peak shares, as on main, and a guard applies owner rule A directly when the judge would
move the hit earlier (`suggest` < 0): if every sample from the judge's first departure up to the hit stands within the
detector's own band (2.5 % of the hit's peak, `DEPARTURE_VIS_PCT`) of the line level, the bump is under the band and is
not the stroke's first motion, and the answer is `suggest: 0` with `why: "under the detector's band"`. Without a readable
hit's peak the guard does not apply and main's answer stands. No other constant. Branch
`crew/hermes@linux-arm64/394-judge-measure-a-lead-bump-against-the`, pushed, not merged (the decision below fails).

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
| stroke-peak | 323 | 491 | 251 | 64 worse / 4 better |
| guard | 371 | 492 | 206 | 13 worse / 2 better |

The coordinator's labels (4484 rows) for comparison: before 2632/3647/5405, try 1 2692/3511/6276 (620 worse / 657 better),
candidate 2791/3652/5456 (500 worse / 625 better), stroke-peak 1703/3287/6713 (1508 worse / 406 better).

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

## Session K, the case this ticket came from (rule A check)

Rule A: a hit that stands at the foot of the main stroke gets no counter-suggestion. The sets, with who made each (the kick
track of Session K):

- 5 hand-placed kick hits: the owner's own records from before the run. Judged at the owner's placement.
- 19 confirmed kick hits: made by the owner's agent under decision A. Judged at the confirmed placement.
- 18 moved kick hits: made by the owner's agent, each 1 or 2 samples earlier. Judged at the placement before the move
  ("at from") and at the placement the run chose ("at to").

Suggestions of 0 or none (the rule holds), and for the moves at "from" the suggestions that land on the sample the run moved to:

| set | n | before | try 1 | candidate | stroke-peak | guard |
|---|---|---|---|---|---|---|
| owner hand-placed, 0 or none | 5 | 4 | 4 | 4 | 1 | 5 |
| agent confirms, 0 or none | 19 | 4 | 8 | 4 | 0 | 19 |
| agent moves at "to", 0 or none | 18 | 15 | 13 | 14 | 1 | 18 |
| agent moves at "from", suggestion = the run's move | 18 | 5 | 0 | 2 | 0 | 0 |

Neither variant closes the case. On the 19 confirmed hits the judge on main counter-suggests 15 (8 times 5 samples earlier,
7 times 6; the other 4 give 0 or none); the candidate still counter-suggests 15 and try 1 counter-suggests 11. One of the 5
owner hits gets a counter-suggestion on main (5 samples earlier) and still gets one under try 1 and the candidate (6 samples
earlier). Of the 18 moves, the judge on main would have suggested the run's own move on 5 and the candidate on 2. The Session K
kick bump is therefore not removed by the foot walk of the candidate or by the fixed band of try 1; this section does not
establish why.

## The stroke-peak reference (#394, not shipped)

The two rules above measure against different references: the page judge's bands are shares of the look window's peak (for a
kick, the early rise), the detector's 2.5 % is a share of the hit's peak (`pk`, the largest sample from the foot over
`EDGE_SPAN_S` = 5 ms). #394 put the judge on the detector's reference: the bands unchanged, the shares converted by the two
peaks' ratio (`look` still answers `samples` as shares of the local peak; `judge` takes the ratio; `look` answers
`stroke_peak`, the hit's peak in raw samples; without a readable stroke peak the judge suggests nothing with
`why: "the stroke peak is not readable"`). The Gate window (a bump of 2 % of the stroke's peak that reads as 8 % of the
local peak) gives suggestion 0 (`tests/test_judge_measure.py`, `tests/js_unit/test_judge.js`).

It does not close the case either: the Session K kick bump is gone (the 19 confirmed hits get no counter-suggestion any
more), but every answer moves 1 to 3 samples late (all 19 read `1`, `2` or `3` after the hit), so on the gold rows it is
the worst variant yet: 1572 worse / 410 better against before; on the 497 owner rows 64 worse / 4 better. Both decision
arms fail: (a) Session K 0 or none, 1 and 0 against try 1's 4 and 8; (b) owner rows, 64 worse. The branch is pushed
without merging; the coordinator puts the numbers to the owner.

Worse rows by shape (the candidate's key, computed on the variant's own departure and stroke-peak shares; 1572 worse,
410 better, 384 of the worse rows are a suggestion that appears, 9 one that disappears):

| shape | worse | better |
|---|---|---|
| bLLLLLLLLLLLS (one rise sample, then line) | 581 | 73 |
| LLLLLLLLLLLLS (no lead sample over 2 %) | 494 | 138 |
| LLbLLLLLLLLLS | 94 | 2 |
| bLbLLLLLLLLLS | 73 | 6 |
| LbLLLLLLLLLLS | 64 | 35 |
| LbbLLLLLLLLLS | 54 | 6 |
| bLLbLLLLLLLLS | 46 | 3 |
| bbLLLLLLLLLLS | 33 | 15 |
| bLbbLLLLLLLLS | 29 | 4 |
| bbbbbbbbbbbbS (no line, a slow ramp) | 9 | 2 |

The bulk of the damage is the line itself: against the stroke's peak the line level is small, so the departure is found
1 to 3 samples into the rise where the label sits at or before the hit. The reference is right for the bump but wrong
for the departure: the detector's 2.5 % is a share of the hit's peak, but its departure search runs on the raw samples
against the line, not on shares of the peak (edge.rs walks the level, `vis` only gates visibility).

## The guard (#394, last round, not shipped)

The guard applies owner rule A directly and leaves every other case as on main (see the runs table): the departure, line
check and suggestion stay on the local-peak shares; only when the judge would move the hit earlier (`suggest` < 0) and
every sample from the judge's first departure up to the hit stands within the detector's own band (2.5 % of the hit's
peak, `DEPARTURE_VIS_PCT`, in `stroke_peak`) of the line level does it answer `suggest: 0`, `why: "under the detector's
band"`. Session K closes (5, 19, 18; the at-from moves get no counter-suggestion, but also never the run's own move), and
against main the guard changes only 646 of the 4981 gold rows (540 worse, 100 better, 6 with the same distance). By
construction it only changes rows where main moved earlier through a sub-band stretch. On the 497 owner rows 15 change:
2 where the label sits at the hit (better) and 13 where it sits at the earlier sample (worse), so the decision arm "0
worse on the owner rows" fails and the judge stays as on main; the owner decides rule A.

The changed rows by shape (the candidate's key, computed on the local-peak shares at main's departure; 178 keys, the
bulk in the three shapes below, the rest 1 to 10 rows each):

| shape | worse | better |
|---|---|---|
| LLLLLLLLLLLLS (no lead sample over 2 %) | 249 | 26 |
| bLLLLLLLLLLLS (one rise sample, then line) | 62 | 3 |
| bbbbbbbbbbbbS (no line, a slow ramp) | 60 | 40 |

## Session K shape keys (candidate; not shipped)

Kick 30 moves of 63, snare 9 of 50, tom 0 (before: 31, 10, 0). Hits whose suggestion differs between strokes of one shape key:
0 after (kick 17 keys, snare 11); not defined before (the old judge has no shape key).
