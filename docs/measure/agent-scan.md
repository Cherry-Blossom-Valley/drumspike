# Agent scan on the real sessions (#412)

`tools/agent_scan_measure.py` builds a project with ALL tracks of a session (rule a needs the
tom and hat hits; the three sessions here carry kick and snare close mics), analyses it and
runs the page helper `DSAgent.scan` through the bridge - untouched first, then with the hits
written away in TEN PASSES, offsets 0 to 9, every tenth hit each, the events restored between
the passes (one offset alone is not enough: on one host one offset found 0 misses where
another found two). Aggregates only, no file or session names.

Per track: the hits, the detector candidates, the listed candidates on the untouched track and
the cut candidates left out as bleed (at the time of a hit on another track); then the fault
injection over all ten passes: the removed hits, how many are listed, and the misses by cause -
no candidate within 30 ms (a stroke the flam step adds, or one the chain smoothed away: the
scan cannot find it, that is the stated limit), the candidate left out as bleed, the candidate
claimed by a remaining hit of the own track (the pair stands inside the minimum gap, one
candidate for two hits: the minimum gap covers "too close to be a second stroke", no stroke is
missing), and a candidate present but not listed (a FAIL, the tool exits 1).

| session | track | hits | candidates | listed | left out bleed | removed | listed | no candidate | left out as bleed | claimed by a remaining hit | not listed (candidate present) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Session A | kick | 351 | 880 | 227 | 302 | 351 | 351 | 0 | 0 | 0 | 0 |
| Session A | kick | 302 | 702 | 261 | 139 | 302 | 302 | 0 | 0 | 0 | 0 |
| Session A | snare | 172 | 172 | 0 | 0 | 172 | 171 | 0 | 1 | 0 | 0 |
| Session A | snare | 234 | 610 | 0 | 381 | 234 | 227 | 3 | 2 | 2 | 0 |
| Session B | kick | 445 | 1131 | 325 | 361 | 445 | 445 | 0 | 0 | 0 | 0 |
| Session B | kick | 444 | 874 | 429 | 1 | 444 | 444 | 0 | 0 | 0 | 0 |
| Session B | snare | 358 | 743 | 1 | 394 | 358 | 347 | 10 | 1 | 0 | 0 |
| Session B | snare | 312 | 312 | 0 | 0 | 312 | 311 | 0 | 1 | 0 | 0 |
| Session C | kick | 387 | 990 | 263 | 340 | 387 | 387 | 0 | 0 | 0 | 0 |
| Session C | kick | 386 | 699 | 268 | 45 | 386 | 386 | 0 | 0 | 0 | 0 |
| Session C | kick | 386 | 699 | 268 | 45 | 386 | 386 | 0 | 0 | 0 | 0 |
| Session C | snare | 275 | 275 | 0 | 0 | 275 | 275 | 0 | 0 | 0 | 0 |
| Session C | snare | 275 | 275 | 0 | 0 | 275 | 275 | 0 | 0 | 0 | 0 |
| Session C | snare | 286 | 634 | 8 | 340 | 286 | 286 | 0 | 0 | 0 | 0 |

The fault injection is clean over all ten offsets: every removed hit that has a candidate is
listed, or its candidate is left out as bleed, or a remaining hit of the pair claims it (the
two claimed cases stand on one snare track: a fast double stroke 14 ms apart that shares one
candidate; the remaining hit claims it 21.4 ms away, inside the 22 ms refine window). The
thirteen no-candidate misses are the second strokes of flams, which the `limit` text states.

## The decision rule

The earlier decision rule ("the listed candidates stay under 10 percent of the track's hits")
does not hold and is withdrawn. On the kick tracks the needed sensitivity of the listed
candidates is continuous - 7 / 108 / 217, 12 / 95 / 165 and 12 / 89 / 130 in the bands up to
80, up to 100 and over 100 - so no gap separates a missed stroke from a far candidate, and
every cut-off inside the scan would be a tuned number. The decision (point 1 of the review of
2026-11-13): the scan stays COMPLETE - no cut-off filters a candidate inside the scan - and
the presentation changes instead. Each candidate carries `need`, the slider value it needs,
sorted nearest to the slider first, and the answer carries `by_need`: counts of the listed
candidates by how far the slider must move (`kept`, `+10`, `+30`, `+50`, `more`) - reading
bands, not a filter. The tail rule (rule b) is dropped with it: on the untouched snare tracks
it left out 0 candidates, so it only hid real strokes (a fast double stroke); the minimum gap
in the matching already covers "too close to be a second stroke", and `left_out` keeps
`bleed` only.
