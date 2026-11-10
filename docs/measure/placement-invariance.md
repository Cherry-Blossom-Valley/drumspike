# Measure: placement invariance and the track's own clear hits (#443)

Two tools judge every later change of the placement rule by measurement instead of by argument. Both read their data from
arguments, print the count of hits per line, and fail loudly on 0 hits. The rule itself runs through the repo's devtools
binaries (`parity refine`, rule version 9; `service_tool core-analyze` for the full track) - no second copy of the core.

    uv run --no-project --with numpy,soundfile python tools/placement_invariance.py MEASURE_DIR SAMPLES_ROOT \
        --bounds tools/placement_invariance.bounds.json
    uv run --no-project --with numpy python tools/shape_measure.py DATA_DIR

`MEASURE_DIR` holds the stored tool runs (`*before.json`), `SAMPLES_ROOT` the stems (`<session>/Audio Files/*.wav`),
`DATA_DIR` the labelled rows (`fit_v10.csv` + `fit_v10.f32`). The tools print aggregates only; no audio and no label
file reaches the repo.

## Tool 1: `tools/placement_invariance.py` - the same stroke, a changed input

For each hit of each kick, snare and tom track the tool cuts an excerpt (0.120 s before, 0.080 s after the hit), places
it with the current rule, then places a changed copy of the same audio and counts how far the answer moved (a difference
counts from half a sample). One line per change: gain -12 dB and +6 dB, polarity flip, a DC offset of 1 % of the hit's
peak, 50 Hz hum at 2 % of the peak, white noise 50 dB under the peak, the coarse time moved by -3/-1/+1/+3 ms, the stem
resampled to 96 kHz (compared in 48 kHz samples), and the whole excerpt delayed by 1 to 7 samples and run through the
full track analysis, where the hit must move by exactly that count. The window records of one line are batched into one
`parity refine` job (silence-guarded excerpts in one wav), the delay line is one `service_tool core-analyze` batch per
delay. `--bounds` (one bound per line and role, JSON) exits non-zero when a share is over its bound; the first bounds
are the measured values rounded up.

The measured shares on the internal stems (aggregates only; a difference counts from half a sample):

| change | moved, kick / snare / tom | moved by 2+, kick / snare / tom |
|---|---|---|
| gain -12 dB, +6 dB | 0 / 0 / 0 | 0 / 0 / 0 |
| polarity flip | ~2 / ~3 / ~6 % | ~0.5 / ~0.7 / ~1.5 % |
| DC offset of 1 % of the peak | ~22 / ~24 / ~16 % | ~7 / ~8 / ~10 % |
| 50 Hz hum at 2 % of the peak | ~15 / ~18 / ~12 % | ~4 / ~10 / ~8 % |
| white noise 50 dB under the peak | ~5 / ~7 / ~8 % | ~2 / ~3 / ~2 % |
| coarse time -3 / -1 / +1 / +3 ms | 0 to ~7 % (tom the worst) | see the tool |
| 96 kHz resample | ~7 / ~17 / ~11 % | ~3 / ~13 / ~8 % |
| whole excerpt delayed 1 to 7 samples, full track | 90 to 99.7 % placed exactly | tom the worst |

Read: level and sign changes are safe; additive offsets and hum are not (the rule's line reads relative to zero, an
offset moves it); the coarse time should not matter and mostly does not, tom being the exception; a resample moves a
minority of hits by a sample or two. The delay line is the end-to-end check: the same audio cut a few samples earlier
must give the same hits, moved by exactly that count.

## Tool 2: `tools/shape_measure.py` - the track's own clear hits as the reference

The engineer's way, measured: per track (session, role) take the clear hits (line excursion under 1 % of the peak) at
the position the current rule gave them, build the foot shape from them - `--pre` 16 samples before the hit and `--post`
8 after at 48 kHz, at 4x resolution, median over the clear hits, sharpened by three rounds of re-alignment - then place
every hit of the track at the lag of the best zero-mean normalised correlation against that shape (level and offset do
not matter, no label is used).

Against the labels (exact / within 1 / off by 4+), in percent, on the gold rows:

| set | current rule | shape of the clear hits |
|---|---|---|
| kick, all gold rows | 60.1 / 88.3 / 2.0 | 65.2 / 92.3 / 1.3 |
| snare, all gold rows | 50.5 / 72.7 / 10.6 | 55.6 / 81.9 / 5.8 |
| kick, unclear line (2 % or more) | 43.0 / 68.0 / 9.6 | 56.8 / 84.2 / 5.8 |
| snare, unclear line | 33.4 / 51.3 / 30.5 | 51.9 / 73.5 / 14.7 |

And the agreement table - what the two methods say together, without any label: where rule and shape agree (85 % of the
kick rows, 69 % of the snare rows) the rule is almost always right (0.4 % / 2.0 % off by 4 or more); where they differ
by 2 or more (5 % / 17 %), the rule is off by 4 or more for 30 % / 52 % of those rows - that is the doubt list.

Learned on the way: only the foot works (a template of the whole attack fails for snare, the body differs from hit to
hit); the shape inherits the rule's average position on the clear hits - one correction per track, not one error per
hit; toms have no labels and are not scored.

## Tests

`uv run pytest -q tests/test_placement_invariance.py tests/test_shape_measure.py` run on generated strokes and rows, no
internal data. They include the planted defects of the Gate: a placer that reads `|x|` from zero fails the DC line, and
a hit moved by 3 samples shows as "differ by 2 or more" in the shape tool's agreement table.
