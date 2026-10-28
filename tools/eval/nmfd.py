"""A7 (#353) and A7b (#377): kit-template NMFD vs SuperFlux on ENST-Drums, F1 at 50 ms per role.

    uv run --with numpy --with soundfile --with mido --with mir_eval \
        python tools/eval/nmfd.py --all            # run all template sources, score, tables
    uv run --with numpy --with soundfile --with mido --with mir_eval \
        python tools/eval/nmfd.py --score-only     # the tables from the existing NMFD run files
    ENST_LIMIT=3 python tools/eval/nmfd.py --all   # a smoke run (like tools/eval/enst.py)

Data and rules are the ones of tools/eval/enst.py (imported): ENST_ROOT, the item index, the tom
label to mic mapping, one-to-one matching at +-50 ms (headline) and +-10 ms. The SuperFlux side
comes from the existing #276 run files (`run/default/<drummer>/<stem>.json`, the DSP detector
whose detection function is SuperFlux). Never commit audio, annotations or derived per-item data.

Template sources (A7b, #377), one run directory each under `<out>/`:

- `oracle` (`nmfd/`, #353): per item and close mic, the templates come only from the item's own
  annotations, the first 10 annotated hits of the drum (>= 5 required, otherwise the mic is
  skipped and counted).
- `self` (`nmfd-self/`): the tom mics take their templates from the mic's own SuperFlux hits in
  the #276 run files - the 10 highest-strength hits (velocity `v`), >= 5 required - no
  annotations. The kick, snare and hat mics keep the annotated templates of `oracle`, so only
  the tom rows change.
- `self2` (`nmfd-self2/`): one refit. The tom templates are the 10 highest-strength hits of the
  `self` run (peak |x| of the mic wav in [-10 ms, +30 ms] around each NMFD hit time).
- `bootstrap` (`nmfd-bootstrap/`, #407): the templates come from the most confident SuperFlux
  hits per mic, no annotations: the mic's own events with the A6 cross-track arbitration (an
  event is dropped when another close mic has a louder event within 10 ms), the 10 highest
  `v` of the rest, >= 5 required (else the mic falls back and is counted). The run uses the
  per-row threshold: a peak must reach `NMFD_THRESH` (default 0.5) times the MEDIAN of the
  row's own local maxima (`nmfd_eval --thresh-ref median`), not a fraction of the row
  maximum. Only the tom mics are run and scored; kick, snare and hat stay on SuperFlux.
- `bootstrap-kit` (`nmfd-bootstrap-kit/`, #409, A7c): the templates pooled per kit, like a full
  song: for every drummer, the arbitrated SuperFlux hits of each close mic are pooled over ALL
  of the drummer's items (the same cross-track arbitration as `bootstrap`), the KIT_HITS
  highest `v` over the whole kit (each hit taken from its own item's wav,
  `nmfd_eval --template-wav/--template-times/--save-template/--templates-file/--max-hits`),
  >= KIT_MIN required (else that drum has no kit template and falls back). Every covered drum
  gets a template row (as in the per-item sources) so the other drums' energy is absorbed in
  its own row. One kit template build per drummer (resumable under `_kit/`), then every item
  runs with the kit's templates; the tom mics are the gated rows.

The wav goes through `target/release/examples/nmfd_eval` (drumspike-core, the shipped detection
function): the app's spectrogram, the NMFD with all templates fixed as W, peak picking per row;
the hits of the mic are the row of its own drum. One result file per item and source (resumes).
The score prints the pooled tom F1 per source and verdicts: A7/A7b FAIL unless the best
self/self2 tom F1 reaches 0.55 (`TOM_F1_MIN`); the A7b bootstrap FAILs unless the pooled tom F1
>= 0.60 and no drummer is below its SuperFlux tom F1; the A7c bootstrap-kit FAILs unless the
pooled tom F1 >= 0.60 AND no drummer is below its SuperFlux tom F1 AND the kit templates cover
>= 90 % of the tom mics (the kick/snare/hat rows are printed but not gated).
"""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "eval"))
import enst  # noqa: E402  (the #276 harness: index, mapping, matching)

OUT = Path.home() / ".cache" / "drumspike" / "fix-metric-model-vs-dsp" / "enst"
EXAMPLE = ROOT / "target" / "release" / "examples" / "nmfd_eval"
TEMPLATE_HITS = 10
TEMPLATE_MIN = 5
ITERS = 64
ROLES = ("kick", "snare", "hat", "tom")
TOM_F1_MIN = 0.55
TOM_F1_MIN_BOOTSTRAP = 0.60
KIT_HITS = 40
KIT_MIN = 5
KIT_COVERAGE_MIN = 0.90
SOURCES = ("oracle", "self", "self2", "bootstrap", "bootstrap-kit")
RUN_DIR = {"oracle": "nmfd", "self": "nmfd-self", "self2": "nmfd-self2", "bootstrap": "nmfd-bootstrap",
           "bootstrap-kit": "nmfd-bootstrap-kit"}
MIC_DRUM = {"kick": "kick", "snare": "snare", "hi-hat": "hat", "tom_1": "tom_1", "tom_2": "tom_2",
            "tom_3": "tom_3"}


def mic_drum(mic):
    """The drum key of a close mic: the toms are per mic (the template of that mic's drum)."""
    return MIC_DRUM.get(mic, mic)


def drum_refs(refs, mic):
    """[(t, label)] of the mic's drum from an `item_refs` dict (the toms per tom mic)."""
    return refs.get(mic, [])


def is_tom_drum(drum):
    return drum.startswith("tom")


def self_templates(sf_runs, drummer, stem, mic):
    """The tom mic's own SuperFlux hits as a template: the TEMPLATE_HITS highest-strength hits of
    the #276 run file (bleed and cut events excluded), None under TEMPLATE_MIN. The strength is
    the event's velocity `v` (NMFD_STRENGTH=rise switches to the dB rise)."""
    p = sf_runs / drummer / f"{stem}.json"
    if not p.is_file():
        return None
    key = os.environ.get("NMFD_STRENGTH", "v")
    ev = [e for e in json.loads(p.read_text())["events"].get(mic, [])
          if not e.get("bleed") and not e.get("cut")]
    if len(ev) < TEMPLATE_MIN:
        return None
    top = sorted(ev, key=lambda e: -float(e[key]))[:TEMPLATE_HITS]
    return sorted(float(e["t"]) for e in top)


def arbitrated_events(sf_runs, drummer, stem, mic, close_mics):
    """The mic's own SuperFlux events (bleed and cut excluded) with the A6 cross-track
    arbitration: an event is dropped when another close mic has a LOUDER event within 10 ms.
    [(t, v)] or None when the run file is missing."""
    p = sf_runs / drummer / f"{stem}.json"
    if not p.is_file():
        return None
    others = {}
    for other in close_mics:
        if other == mic:
            continue
        q = sf_runs / drummer / f"{stem}.json"
        if not q.is_file():
            continue
        others[other] = [(float(e["t"]), float(e["v"])) for e in
                         json.loads(q.read_text())["events"].get(other, [])]
    kept = []
    for e in json.loads(p.read_text())["events"].get(mic, []):
        if e.get("bleed") or e.get("cut"):
            continue
        t, v = float(e["t"]), float(e["v"])
        louder = any(abs(t2 - t) <= 0.010 and v2 > v
                     for evs in others.values() for t2, v2 in evs)
        if not louder:
            kept.append((t, v))
    return kept


def bootstrap_templates(sf_runs, drummer, stem, mic, close_mics):
    """The mic's most confident SuperFlux hits with the A6 cross-track arbitration (no
    annotations): of the mic's own events (bleed and cut excluded), an event is dropped when
    another close mic has a LOUDER event within 10 ms; the TEMPLATE_HITS highest `v` of the
    rest are the template times, None under TEMPLATE_MIN."""
    kept = arbitrated_events(sf_runs, drummer, stem, mic, close_mics)
    if kept is None:
        return None
    if len(kept) < TEMPLATE_MIN:
        return None
    top = sorted(kept, key=lambda p: -p[1])[:TEMPLATE_HITS]
    return sorted(t for t, _ in top)


def hit_strengths(wav, times, pre=0.010, post=0.030):
    """Peak |x| of `wav` in [t - pre, t + post] per hit time (the refit's strength)."""
    import soundfile as sf
    x, sr = sf.read(str(wav))
    if x.ndim > 1:
        x = x[:, 0]
    out = []
    for t in times:
        i0, i1 = max(0, int((t - pre) * sr)), min(len(x), int((t + post) * sr))
        seg = x[i0:i1]
        out.append(float(np.max(np.abs(seg))) if len(seg) else 0.0)
    return out


def refit_templates(wav, prev_hits):
    """The TEMPLATE_HITS highest-strength hits of the previous run (None under TEMPLATE_MIN)."""
    if prev_hits is None or len(prev_hits) < TEMPLATE_MIN:
        return None
    s = hit_strengths(wav, prev_hits)
    top = sorted(zip(s, prev_hits), key=lambda p: -p[0])[:TEMPLATE_HITS]
    return sorted(t for _, t in top)


def kit_pool(sf_out, drummer, idx):
    """The arbitrated hits of each close mic pooled over ALL of the drummer's items (A7c, #409:
    the kit's templates, like a full song). {mic: [(v, wav, t)]} sorted by `v`, highest first;
    only mics of the drummer's items whose wav exists. Every drum gets a template (like the
    per-item sources) so the other drums' energy is absorbed in its own row."""
    sf_runs = sf_out / "run" / "default"
    pooled = {}
    for stem, item in idx[drummer].items():
        for mic in enst.CLOSE_MICS:
            wav = item["wav"].get(mic)
            if wav is None or not Path(wav).is_file():
                continue
            p = sf_runs / drummer / f"{stem}.json"
            if not p.is_file():
                continue
            kept = arbitrated_events(sf_runs, drummer, stem, mic, enst.CLOSE_MICS)
            if kept is None:
                continue
            pooled.setdefault(mic, []).extend((v, str(wav), t) for t, v in kept)
    return {mic: sorted(lst, key=lambda p: -p[0]) for mic, lst in pooled.items()}


def kit_template_file(root, out, drummer, idx, log=print):
    """Build the kit template JSON of one drummer (resumable, under
    `<RUN_DIR[bootstrap-kit]>/_kit/<drummer>.json`): per close mic, the KIT_HITS highest-`v`
    pooled hits, each taken from its own item's wav. Returns (path, [covered mics]); the path
    is None when no mic reaches KIT_MIN pooled hits."""
    kit_dir = out / RUN_DIR["bootstrap-kit"] / "_kit"
    dst = kit_dir / f"{drummer}.json"
    if dst.is_file():
        return dst, json.loads(dst.read_text())["mics"]
    pooled = kit_pool(out, drummer, idx)
    mics = [m for m in enst.CLOSE_MICS
            if len(pooled.get(m, [])) >= KIT_MIN]
    if not mics:
        return None, []
    if not EXAMPLE.is_file():
        raise SystemExit(f"FAIL: {EXAMPLE} is missing: cargo build --release -p drumspike-core --example nmfd_eval")
    kit_dir.mkdir(parents=True, exist_ok=True)
    args = [str(EXAMPLE)]
    for mic in mics:
        top = pooled[mic][:KIT_HITS]
        by_wav = {}
        for _, wav, t in top:
            by_wav.setdefault(wav, []).append(t)
        for wav, ts in by_wav.items():
            args += ["--template-wav", wav, "--template-times",
                     ",".join(f"{t:.6f}" for t in sorted(ts))]
        args += ["--template-new"]
    args += ["--max-hits", str(KIT_HITS), "--save-template", str(dst)]
    r = subprocess.run(args, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"nmfd_eval kit build failed ({r.returncode}): {r.stderr[-400:]}")
    doc = json.loads(dst.read_text())
    doc["mics"] = mics
    dst.write_text(json.dumps(doc))
    log(f"kit templates {drummer}: {len(mics)} mics, {len(doc['templates'])} templates")
    return dst, mics


def run_example_file(wav, templates_file, thresh_ref=None):
    """One wav through the nmfd_eval example with the templates of a kit template file."""
    args = [str(EXAMPLE), "--wav", str(wav), "--iters", str(ITERS),
            "--templates-file", str(templates_file)]
    if thresh_ref == "median":
        args += ["--thresh-ref", "median", "--thresh", os.environ.get("NMFD_THRESH", "0.5")]
    elif os.environ.get("NMFD_THRESH"):
        args += ["--thresh", os.environ["NMFD_THRESH"]]
    r = subprocess.run(args, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"nmfd_eval failed ({r.returncode}): {r.stderr[-400:]}")
    return json.loads(r.stdout.strip().splitlines()[-1])["hits"]


def kit_coverage(out, root):
    """(covered, total, per drummer {drummer: (covered, total)}) tom mics with a kit template
    (A7c coverage: the tom mics of the session mics that the drummer's kit template covers)."""
    per = {}
    for d in enst.DRUMMERS:
        tom_mics = [m for m in enst.TOM_MICS if m in enst.session_mics(root, d)]
        dst = out / RUN_DIR["bootstrap-kit"] / "_kit" / f"{d}.json"
        covered = len(set(json.loads(dst.read_text())["mics"]) & set(tom_mics)) if dst.is_file() else 0
        per[d] = (covered, len(tom_mics))
    total = sum(v[1] for v in per.values())
    cov = sum(v[0] for v in per.values())
    return cov, total, per



def run_example(wav, template_sets, log=None, thresh_ref=None):
    """One wav through the nmfd_eval example: {row index: [hit times]}. NMFD_THRESH (optional)
    overrides the peak threshold for a sweep. `thresh_ref='median'` (#407 bootstrap) thresholds
    against the median of the row's own local maxima, default factor 0.5."""
    args = [str(EXAMPLE), "--wav", str(wav), "--iters", str(ITERS)]
    if thresh_ref == "median":
        args += ["--thresh-ref", "median",
                 "--thresh", os.environ.get("NMFD_THRESH", "0.5")]
    elif os.environ.get("NMFD_THRESH"):
        args += ["--thresh", os.environ["NMFD_THRESH"]]
    for ts in template_sets:
        args += ["--template", ",".join(f"{t:.6f}" for t in ts)]
    r = subprocess.run(args, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"nmfd_eval failed ({r.returncode}): {r.stderr[-400:]}")
    return json.loads(r.stdout.strip().splitlines()[-1])["hits"]


def tom_templates(source, drummer, stem, mic, item, refs, out, close_mics=None):
    """The template times of one tom mic under `source` (None when not usable)."""
    if source == "bootstrap":
        return bootstrap_templates(out / "run" / "default", drummer, stem, mic,
                                   close_mics or enst.CLOSE_MICS)
    if source == "self":
        return self_templates(out / "run" / "default", drummer, stem, mic)
    if source == "self2":
        p = out / RUN_DIR["self"] / drummer / f"{stem}.json"
        if not p.is_file():
            return None
        prev = json.loads(p.read_text())["hits"].get(mic)
        if prev is None:
            return None
        return refit_templates(item["wav"][mic], [float(t) for t in prev])
    times = sorted(t for t, _ in drum_refs(refs, mic))
    return times[:TEMPLATE_HITS] if len(times) >= TEMPLATE_MIN else None


def do_run(root, out, source, log=print):
    """Every item of every drummer: per close mic, the NMFD hits of one template source
    (resumable, one JSON per item under RUN_DIR[source])."""
    if not EXAMPLE.is_file():
        raise SystemExit(f"FAIL: {EXAMPLE} is missing: cargo build --release -p drumspike-core --example nmfd_eval")
    if source == "self2" and source not in SOURCES:
        raise SystemExit(f"FAIL: unknown source {source}")
    idx = enst.index_items(root)
    tom_maps = {d: enst.tom_mapping(root, d) for d in enst.DRUMMERS}
    limit = int(os.environ.get("ENST_LIMIT", "0") or 0)
    skipped = 0
    n = 0
    kits = {}
    if source == "bootstrap-kit":
        # one kit template build per drummer (resumable), applied to every item
        for d in enst.DRUMMERS:
            kits[d] = kit_template_file(root, out, d, idx, log=log)
    for drummer, items in idx.items():
        for stem, item in items.items():
            dst = out / RUN_DIR[source] / drummer / f"{stem}.json"
            if dst.is_file():
                continue
            present = [m for m in enst.CLOSE_MICS if m in item["wav"] and item["wav"][m].is_file()]
            if source == "bootstrap-kit":
                kit_path, kit_mics = kits[drummer]
                covered = [m for m in enst.CLOSE_MICS if m in kit_mics and m in present]
                drums = [mic_drum(m) for m in covered]
                if kit_path is None or not drums:
                    skipped += 1
                    log(f"skip {drummer}/{stem} ({source}): no kit template for a present mic")
                    continue
                hits = {}
                for mic in present:
                    if mic in covered:
                        rows = run_example_file(item["wav"][mic], kit_path, thresh_ref="median")
                        hits[mic] = rows[covered.index(mic)]
                    else:
                        hits[mic] = None
                result = {"drummer": drummer, "stem": stem, "source": source, "drums": drums,
                          "templates": {m: "kit" for m in covered}, "hits": hits}
                dst.parent.mkdir(parents=True, exist_ok=True)
                dst.write_text(json.dumps(result))
                n += 1
                log(f"nmfd {source} {drummer}/{stem} ({n}, {len(drums)} kit drums)")
                if limit and n >= limit * len(idx):
                    log(f"ENST_LIMIT {limit}: stopping")
                    return
                continue
            refs = enst.item_refs(enst.read_annotation(item["ann"]), tom_maps[drummer])
            # one template per drum: oracle = the first TEMPLATE_HITS annotated hits of the drum on
            # its mic; self/self2 = the tom mics take the mic's own (refit) hits instead;
            # bootstrap (#407) = every drum takes its mic's most confident arbitrated hits.
            template_sets, drums, templates = [], [], {}
            for mic in present:
                drum = mic_drum(mic)
                if source == "bootstrap":
                    ts = bootstrap_templates(out / "run" / "default", drummer, stem, mic,
                                             enst.CLOSE_MICS)
                elif is_tom_drum(drum) and source in ("self", "self2"):
                    ts = tom_templates(source, drummer, stem, mic, item, refs, out)
                else:
                    times = sorted(t for t, _ in drum_refs(refs, mic))
                    ts = times[:TEMPLATE_HITS] if len(times) >= TEMPLATE_MIN else None
                templates[mic] = ts
                if ts is not None:
                    template_sets.append(ts)
                    drums.append(drum)
            if not template_sets:
                skipped += 1
                log(f"skip {drummer}/{stem} ({source}): no drum with {TEMPLATE_MIN}+ template hits")
                continue
            result = {"drummer": drummer, "stem": stem, "source": source, "drums": drums,
                      "templates": templates, "hits": {}}
            for mic in present:
                row = drums.index(mic_drum(mic)) if mic_drum(mic) in drums else None
                if row is None or (source == "bootstrap" and not is_tom_drum(mic_drum(mic))):
                    result["hits"][mic] = None       # too few template hits, or not run (bootstrap)
                    continue
                rows = run_example(item["wav"][mic], template_sets,
                                   thresh_ref="median" if source == "bootstrap" else None)
                result["hits"][mic] = rows[row]
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.write_text(json.dumps(result))
            n += 1
            log(f"nmfd {source} {drummer}/{stem} ({n}, {len(template_sets)} drums)")
            if limit and n >= limit * len(idx):
                log(f"ENST_LIMIT {limit}: stopping")
                return


def load_runs(out, source, limit):
    """{(drummer, stem): result} of one source; {} when the source has no run files."""
    d = out / RUN_DIR[source]
    if not d.is_dir():
        return {}
    runs = {}
    for p in sorted(d.glob("*/*.json")):
        if p.parent.name.startswith("_"):
            continue          # kit template builds and other internal files, not run results
        runs[(p.parent.name, p.stem)] = json.loads(p.read_text())
    return runs


def do_score(out, root, log=print):
    """The pooled F1 at +-50 ms per role and template source; the tom comparison table and the
    A7b verdict (self or self2 tom F1 >= TOM_F1_MIN)."""
    idx = enst.index_items(root)
    tom_maps = {d: enst.tom_mapping(root, d) for d in enst.DRUMMERS}
    anns = {d: {stem: enst.item_refs(enst.read_annotation(item["ann"]), tom_maps[d])
                for stem, item in idx[d].items()} for d in enst.DRUMMERS}
    limit = os.environ.get("ENST_LIMIT", "")

    # the SuperFlux side: the #276 run files (the DSP detector's detection function is SuperFlux)
    sf = {}   # (drummer, mic): {"ref": [...], "est": [...]}
    for d in enst.DRUMMERS:
        for stem in idx[d]:
            p = out / "run" / "default" / d / f"{stem}.json"
            if limit and not p.is_file():
                continue          # a smoke run: score only the items that exist
            if not p.is_file():
                raise SystemExit(f"FAIL {p} is missing (the SuperFlux side; run tools/eval/enst.py --all first)")
            res = json.loads(p.read_text())
            for mic in enst.CLOSE_MICS:
                if mic not in res["events"]:
                    continue
                cell = sf.setdefault((d, mic), {"ref": [], "est": []})
                cell["ref"] += sorted(t for t, _ in anns[d][stem].get(mic, []))
                cell["est"] += sorted(float(e["t"]) for e in res["events"][mic]
                                      if not e.get("bleed") and not e.get("cut"))

    # the NMFD sides
    runs = {s: load_runs(out, s, limit) for s in SOURCES}
    nmfd = {}   # (source, drummer, mic): {"ref": [...], "est": [...]}
    skipped = []
    # the SuperFlux estimates per item and mic (the #407 fallback for a mic without a template)
    sf_est = {}
    for d in enst.DRUMMERS:
        for stem in idx[d]:
            p = out / "run" / "default" / d / f"{stem}.json"
            if not p.is_file():
                continue
            res = json.loads(p.read_text())
            for mic, evs in res["events"].items():
                sf_est[(d, stem, mic)] = sorted(float(e["t"]) for e in evs
                                                if not e.get("bleed") and not e.get("cut"))

    fell_back = {"bootstrap": 0, "bootstrap-kit": 0}
    for source in SOURCES:
        for (d, stem), res in runs[source].items():
            for mic, hits in res["hits"].items():
                cell = nmfd.setdefault((source, d, mic), {"ref": [], "est": []})
                cell["ref"] += sorted(t for t, _ in anns[d].get(stem, {}).get(mic, []))
                if hits is None:
                    if source in fell_back:
                        # #407/#409: a mic without a template falls back to SuperFlux on that
                        # mic (its run is the SuperFlux one); the fallback is counted for the
                        # tom mics only (the non-tom mics are never run in these sources).
                        if mic in enst.TOM_MICS:
                            fell_back[source] += 1
                        cell["est"] += sf_est.get((d, stem, mic), [])
                    else:
                        skipped.append(f"{source} {d}/{stem} {mic}: < {TEMPLATE_MIN} template hits")
                    continue
                cell["est"] += sorted(float(t) for t in hits)

    role_of = {m: enst.MIC_ROLE[m] for m in enst.CLOSE_MICS}
    sf_pooled_by_drummer = {}
    for d in enst.DRUMMERS:
        for role in ROLES:
            mics = [m for m in enst.CLOSE_MICS if role_of.get(m) == role]
            ref, est = [], []
            for m in mics:
                cell = sf.get((d, m))
                if cell:
                    ref += cell["ref"]
                    est += cell["est"]
            if ref:
                score = enst.prf(ref, est, enst.WINDOWS[0])
                assert score is not None
                sf_pooled_by_drummer[(d, role)] = score[2]
    f1 = {}   # (source, drummer, role) -> f1 at 50 ms
    for source in SOURCES:
        for d in enst.DRUMMERS:
            for role in ROLES:
                mics = [m for m in enst.CLOSE_MICS if role_of.get(m) == role]
                ref, est = [], []
                for m in mics:
                    cell = nmfd.get((source, d, m))
                    if cell:
                        ref += cell["ref"]
                        est += cell["est"]
                if not ref:
                    continue
                score = enst.prf(ref, est, enst.WINDOWS[0])
                assert score is not None
                f1[(source, d, role)] = score[2]

    for (d, role), v in sf_pooled_by_drummer.items():
        f1[("superflux", d, role)] = v

    lines = ["### A7/A7b (#353, #377): kit-template NMFD on ENST-Drums "
             "(Gillet & Richard, ISMIR 2006; CC BY-NC-ND 4.0, aggregate numbers only)", "",
             "Template sources: oracle = the first 10 annotated hits per drum per item; "
             "self = the tom mics' 10 highest-velocity SuperFlux hits (no annotations); "
             f"self2 = one refit from the self run's hits. NMFD {ITERS} iterations, "
             "all drums' templates as W.",
             "One-to-one matching at +-50 ms (headline), pooled over the drummer's items.", "",
             "| drummer | role | detector | ref | est | P | R | F1 |", "|---|---|---|---|---|---|---|---|"]
    for source in SOURCES:
        for d in enst.DRUMMERS:
            for role in ROLES:
                mics = [m for m in enst.CLOSE_MICS if role_of.get(m) == role]
                ref, est = [], []
                for m in mics:
                    cell = nmfd.get((source, d, m))
                    if cell:
                        ref += cell["ref"]
                        est += cell["est"]
                if not ref:
                    continue
                score = enst.prf(ref, est, enst.WINDOWS[0])
                assert score is not None
                lines.append(f"| {d} | {role} | {source} | {len(ref)} | {len(est)} | "
                             f"{score[0]:.3f} | {score[1]:.3f} | {score[2]:.3f} |")
    lines.append("")
    lines.append("F1 at +-50 ms per role, pooled over the three drummers, per template source:")
    lines.append("")
    lines.append("| role | " + " | ".join(("superflux",) + SOURCES) + " |")
    lines.append("|---|" + "---|" * (len(SOURCES) + 1))
    pooled = {}   # (detector, role) -> f1, pooled over the drummers
    for role in ROLES:
        for source in ("superflux",) + SOURCES:
            vals = [f1[k] for k in f1 if k[2] == role and k[0] == source]
            if vals:
                pooled[(source, role)] = float(np.mean(vals))
    for role in ROLES:
        row = [pooled.get((s, role), float("nan")) for s in ("superflux",) + SOURCES]
        lines.append(f"| {role} | " + " | ".join(f"{v:.3f}" for v in row) + " |")
    tom_vals = {s: pooled.get((s, "tom"), float("nan")) for s in ("self", "self2")}
    best = max(tom_vals.values())
    ok = best >= TOM_F1_MIN
    lines.append("")
    lines.append((f"PASS: the best self-template tom F1 is {best:.3f} >= {TOM_F1_MIN}."
                  if ok else
                  f"FAIL: the best self-template tom F1 is {best:.3f} < {TOM_F1_MIN} "
                  "(a measured FAIL is a valid result; see the report)."))
    if skipped:
        lines += ["", f"Skipped mics (fewer than {TEMPLATE_MIN} template hits): {len(skipped)}"]

    # #407 A7b bootstrap: per drummer and pooled tom F1, SuperFlux vs NMFD-bootstrap; PASS only
    # when the pooled tom F1 >= 0.60 AND no drummer's tom F1 is below its SuperFlux value.
    bs_lines = ["", "### A7b bootstrap (#407): tom mics, SuperFlux vs NMFD-bootstrap", "",
                "Bootstrap templates per drum (no annotations): the mic's own SuperFlux events with",
                "the A6 cross-track arbitration (no louder event on another close mic within 10 ms),",
                f"the {TEMPLATE_HITS} highest-`v` hits, >= {TEMPLATE_MIN} required (else the mic falls",
                "back). Per-row threshold: a peak must reach 0.5 x the MEDIAN of the row's own local",
                "maxima (the row's own distribution, not its loudest frame).", "",
                "| drummer | SuperFlux tom F1 | NMFD-bootstrap tom F1 |", "|---|---|---|"]
    sf_tom = {k[1]: v for k, v in f1.items() if k[0] == "superflux" and k[2] == "tom"}
    bs_tom = {k[1]: v for k, v in f1.items() if k[0] == "bootstrap" and k[2] == "tom"}
    ok_bs = bool(bs_tom)
    for d in enst.DRUMMERS:
        sf_v, bs_v = sf_tom.get(d), bs_tom.get(d)
        if sf_v is None or bs_v is None:
            ok_bs = False
        bs_lines.append(f"| {d} | {sf_v:.3f} | {bs_v:.3f} |" if sf_v is not None and bs_v is not None
                        else f"| {d} | {sf_v} | {bs_v} |")
        if sf_v is not None and bs_v is not None and bs_v < sf_v:
            ok_bs = False
    if bs_tom:
        pooled_bs = float(np.mean([bs_tom[d] for d in bs_tom]))
        sf_pool = float(np.mean([sf_tom[d] for d in sf_tom])) if sf_tom else float("nan")
        bs_lines.append(f"| pooled | {sf_pool:.3f} | {pooled_bs:.3f} |")
        ok_bs = ok_bs and pooled_bs >= TOM_F1_MIN_BOOTSTRAP
        bs_lines += ["", (f"PASS: pooled tom F1 {pooled_bs:.3f} >= {TOM_F1_MIN_BOOTSTRAP} and no "
                          "drummer is below its SuperFlux value." if ok_bs else
                          f"FAIL: pooled tom F1 {pooled_bs:.3f} vs {TOM_F1_MIN_BOOTSTRAP} or a "
                          "drummer below its SuperFlux value (a measured FAIL is a valid result)."),
                     "", f"Tom mics that fell back to SuperFlux (fewer than {TEMPLATE_MIN} "
                     f"arbitrated hits): {fell_back['bootstrap']}"]
        lines += bs_lines
    else:
        lines += ["", "A7b bootstrap (#407): no bootstrap run files yet."]

    # #409 A7c bootstrap-kit: kit templates pooled per drummer (like a full song); per drummer
    # and pooled tom F1, SuperFlux vs NMFD-bootstrap-kit, plus the template coverage; PASS only
    # when the pooled tom F1 >= 0.60 AND no drummer is below its SuperFlux value AND coverage
    # >= 90 % of the tom mics.
    kit_tom = {k[1]: v for k, v in f1.items() if k[0] == "bootstrap-kit" and k[2] == "tom"}
    if kit_tom:
        cov, total, cov_per = kit_coverage(out, root)
        kit_lines = ["", "### A7c bootstrap-kit (#409): kit templates pooled per drummer", "",
                     f"Kit templates per close mic: the {KIT_HITS} highest-`v` arbitrated hits pooled",
                     "over ALL of the drummer's items (each hit from its own wav), like a full song;",
                     f">= {KIT_MIN} pooled hits required (else that drum falls back to SuperFlux).",
                     "Per-row threshold as in A7b (0.5 x the median of the row's local maxima).", "",
                     "| drummer | SuperFlux tom F1 | NMFD-bootstrap-kit tom F1 | coverage |",
                     "|---|---|---|---|"]
        ok_kit = bool(kit_tom)
        for d in enst.DRUMMERS:
            sf_v, kt_v = sf_tom.get(d), kit_tom.get(d)
            c, t = cov_per.get(d, (0, 0))
            cov_s = f"{c}/{t}" if t else "-"
            if sf_v is None or kt_v is None:
                ok_kit = False
                kit_lines.append(f"| {d} | {sf_v} | {kt_v} | {cov_s} |")
            else:
                kit_lines.append(f"| {d} | {sf_v:.3f} | {kt_v:.3f} | {cov_s} |")
                if kt_v < sf_v:
                    ok_kit = False
        pooled_kit = float(np.mean([kit_tom[d] for d in kit_tom]))
        sf_pool = float(np.mean([sf_tom[d] for d in sf_tom])) if sf_tom else float("nan")
        cov_share = cov / total if total else 0.0
        kit_lines.append(f"| pooled | {sf_pool:.3f} | {pooled_kit:.3f} | {cov}/{total} |")
        ok_kit = ok_kit and pooled_kit >= TOM_F1_MIN_BOOTSTRAP and cov_share >= KIT_COVERAGE_MIN
        kit_lines += ["", (f"PASS: pooled tom F1 {pooled_kit:.3f} >= {TOM_F1_MIN_BOOTSTRAP}, no "
                           f"drummer below its SuperFlux value, coverage {cov}/{total} "
                           f"({cov_share:.0%}) >= {KIT_COVERAGE_MIN:.0%} of the tom mics."
                           if ok_kit else
                           f"FAIL: pooled tom F1 {pooled_kit:.3f} vs {TOM_F1_MIN_BOOTSTRAP}, a "
                           "drummer below its SuperFlux value, or coverage "
                           f"{cov}/{total} ({cov_share:.0%}) < {KIT_COVERAGE_MIN:.0%} "
                           "(a measured FAIL is a valid result)."),
                      "", f"Tom mic items that fell back to SuperFlux (no kit template): "
                      f"{fell_back['bootstrap-kit']}"]
        lines += kit_lines
    else:
        lines += ["", "A7c bootstrap-kit (#409): no bootstrap-kit run files yet."]

    text = "\n".join(lines) + "\n"
    (out / "nmfd-scores.json").write_text(json.dumps({
        "f1": {f"{k[0]}|{k[1]}|{k[2]}": v for k, v in f1.items()},
        "pooled": {f"{k[0]}|{k[1]}": v for k, v in pooled.items()},
        "skipped": skipped,
        "bootstrap_fell_back": fell_back}))
    (out / "nmfd.md").write_text(text)
    log(text)
    return ok


def main(argv=None):
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    ap.add_argument("--all", action="store_true", help="run every template source per item and mic, then score")
    ap.add_argument("--source", choices=SOURCES, default=None,
                    help="run only one template source (default: all)")
    ap.add_argument("--templates", choices=SOURCES, default=None,
                    help="alias of --source (the gate spells it --templates bootstrap)")
    ap.add_argument("--score-only", action="store_true", help="the tables from the existing run files")
    ap.add_argument("--out", default=str(OUT))
    a = ap.parse_args(argv)
    root = os.environ.get("ENST_ROOT", str(Path.home() / ".cache" / "drumspike" / "datasets" / "ENST-drums-public"))
    if not Path(root).is_dir():
        raise SystemExit(f"FAIL: ENST_ROOT {root} is missing")
    out = Path(a.out)
    if a.all:
        for source in SOURCES if a.source is None else (a.source,):
            do_run(root, out, source)
    if a.templates is not None:
        do_run(root, out, a.templates)
        do_score(out, root)
    if a.all or a.score_only:
        do_score(out, root)
    return 0


if __name__ == "__main__":
    sys.exit(main())
