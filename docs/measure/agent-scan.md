# Agent scan on the real sessions (#412)

`tools/agent_scan_measure.py` builds a project with ALL tracks of a session (rule a needs the
tom and hat hits; the three sessions here carry a kick and a snare close mic), analyses it and
runs the page helper `DSAgent.scan` through the bridge - untouched first, then with every tenth
hit written away through `flushEvents`. Aggregates only, no file or session names.

Per track: the hits, the detector candidates, the listed candidates on the untouched track and
the cut candidates left out as bleed (at the time of a hit on another track) and as tail
(inside the 60 ms decay of an own hit); then the fault injection: the removed hits, how many
are listed, how many are not listed although a candidate stands within 30 ms (a FAIL), and how
many have no candidate at all within 30 ms (a stroke the flam step adds, or one the chain
smoothed away - the scan cannot find it, that is the stated limit).

| session | track | hits | candidates | listed | left out bleed | left out tail | removed | listed | not listed (candidate present) | no candidate |
|---|---|---|---|---|---|---|---|---|---|---|
| Session A | kick | 445 | 1131 | 332 | 326 | 28 | 44 | 44 | 0 | 0 |
| Session A | snare | 377 | 743 | 13 | 367 | 0 | 37 | 35 | 0 | 2 |
| Session B | kick | 387 | 990 | 272 | 290 | 41 | 38 | 38 | 0 | 0 |
| Session B | snare | 302 | 634 | 24 | 308 | 0 | 30 | 30 | 0 | 0 |
| Session C | kick | 351 | 880 | 231 | 258 | 40 | 35 | 35 | 0 | 0 |
| Session C | snare | 294 | 610 | 30 | 292 | 0 | 29 | 28 | 0 | 1 |

The fault injection is clean: every removed hit that has a candidate is listed on all six
tracks. The five misses are all no-candidate misses (the second stroke of a flam: no candidate
within 30 ms), which the `limit` text states.

## The decision rule

On the untouched tracks the listed candidates are under 10 percent of the track's hits on
three of the six tracks (all snare tracks but Session C, at 3.4, 7.9 and 10.2 percent). Every
kick track is far over it: 74.6, 70.3 and 65.8 percent. As the decision rule asks, the
breakdown of the listed candidates follows and no further rule was added.

Listed candidates by needed sensitivity (the slider value the candidate would need):

| session | track | <=60 | 61-80 | 81-100 | >100 |
|---|---|---|---|---|---|
| Session A | kick | 0 | 7 | 108 | 217 |
| Session A | snare | 0 | 6 | 7 | 0 |
| Session B | kick | 0 | 12 | 95 | 165 |
| Session B | snare | 1 | 15 | 8 | 0 |
| Session C | kick | 1 | 11 | 89 | 130 |
| Session C | snare | 1 | 12 | 17 | 0 |

Listed candidates by distance to the nearest hit of any track:

| session | track | <=10 ms | 10-30 ms | 30-60 ms | 60-200 ms | >200 ms |
|---|---|---|---|---|---|---|
| Session A | kick | 0 | 11 | 122 | 169 | 30 |
| Session A | snare | 0 | 3 | 0 | 5 | 5 |
| Session B | kick | 0 | 2 | 79 | 150 | 41 |
| Session B | snare | 0 | 0 | 0 | 7 | 17 |
| Session C | kick | 0 | 14 | 19 | 143 | 55 |
| Session C | snare | 0 | 1 | 2 | 3 | 24 |

What the rest is: on the kick tracks most listed candidates would need a slider over 100
(217 of 332, 165 of 272, 130 of 231) and stand 60 ms or more from every hit of any track
(169, 150 and 143 of them over 200 ms away on Session B and C kicks) - isolated quiet strokes
in the gaps of the beat, not bleed (the bleed rule already left out 258 to 326 candidates per
kick track) and not tails. The snare tracks list few candidates and most of them also stand
more than 200 ms from every hit; Session C's snare sits at 10.2 percent, a hair over the
line, with the same shape. Stopping here for the owner's answer, as the decision rule asks.
