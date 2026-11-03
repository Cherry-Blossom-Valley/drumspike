"""Identity snapshot (refactor plan, phase 0, step 0.2): the gate "the refactor changed no output".

    uv run --project . python tools/identity.py snapshot --tool <service_tool> --out <manifest.json> \\
        [--config ~/.config/drumspike/identity.toml] [--parts golden,enst,sessions] [--env KEY=VALUE ...] [--jobs N]
    uv run --no-project python tools/identity.py compare <a.json> <b.json> [--min part=N ...] [--parts ...]

`snapshot` drives the given `service_tool` (`service_tool cli batch`, the commands of the app) over a corpus and
writes a manifest: per item a SHA-256 of the canonical output, the sub-hashes it is made of and small counts
(events per track). `compare` reads two manifests, prints per part the number of items compared and the number
that differ, lists the differing items and exits 1 on any difference, on a missing item and when fewer items than
the minimum were compared (a filter that cannot fire must not say "identical").

Corpus (parts):
  golden    the synthetic WAVs of `tests/golden_corpus.py` (the WAVs of `tests/golden/*.json`), one track each,
            role kick: analyze, events, MIDI export.
  enst      ENST-Drums items (`phrase`, `solo`, `minus-one`, `MIDI-minus-one`; every `stride`-th item): one project
            per item with the kick, snare, hi-hat, tom and overhead mics: the whole analysis (all session steps),
            events and MIDI per mic.
  sessions  the real Pro Tools sessions as whole session jobs, three stages in one project: `full` (analysis with
            cross bleed, ghost gate, tom bleed and flam; events, MIDI and spike WAV per track, the multi-track MIDI, the tempo suggestion),
            `repick` (`track_repick` with another sensitivity, stored), `reanalyze` (`reanalyze_selection` on one
            time range of every track).

What an item hashes: the events of `events_get` re-serialized in the order they came (all keys including the marks
in `extra`, key order as emitted, times, velocities) and the bytes of every exported file. Nothing else: no paths,
no wall times, no ids. The manifest holds hashes and counts only (no names of sessions or files; the keys carry
the labels of the config). The corpus locations come from a config outside the repository (`identity.toml`):

    [golden]
    enabled = true
    [enst]
    root = "<ENST-drums-public>"
    stride = 1
    [sessions.<label>]
    dir = "<Audio Files folder>"
    tracks = [["<file name>", "<role>", "<key>"], ...]

A manifest is meant to be kept in `~/.cache/drumspike/identity/<sha>.json`, never in git.
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FORMAT = 1
PARTS = ("golden", "enst", "sessions")
# Fewer items compared than this and `compare` fails: the corpus is 20 golden WAVs (3 items each), the ENST items
# (210 at stride 1) and the 3 sessions (75 items: 8 tracks, 3 stages, exports). A smaller corpus needs `--min` on purpose.
DEFAULT_MIN = {"golden": 50, "enst": 100, "sessions": 70}
DEFAULT_CONFIG = Path.home() / ".config" / "drumspike" / "identity.toml"
NOTES = {"kick": 36, "snare": 38, "tom": 47, "hat": 42, "cymbal": 49, "other": 36}
ENST_TYPES = ("phrase", "solo", "minus-one", "MIDI-minus-one")
ENST_MICS = ("kick", "snare", "hi-hat", "tom_1", "tom_2", "tom_3", "overhead_L", "overhead_R")
ENST_ROLES = {"kick": "kick", "snare": "snare", "hi-hat": "hat", "overhead_L": "cymbal", "overhead_R": "cymbal"}
REPICK_SENSITIVITY = 80          # the slider value (0 to 150) of the re-pick case; the analysis runs at the default
REANALYZE_RANGE = (0.30, 0.45)   # fractions of the track duration for the reanalyze-selection case


# ----------------------------------------------------------------------------------------------- items

def sha(data):
    return hashlib.sha256(data).hexdigest()


def canon(value):
    """The canonical bytes of a JSON value: separators fixed, key order and list order as they are (the order the
    tool emitted; no sorting, so a reordered key or event changes the hash)."""
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def make_item(subs, counts=None):
    """One manifest item from its sub-hashes ({name: hex}) and counts ({name: int}). The item hash covers the
    names and the sub-hashes, so a missing sub-output changes it too."""
    subs = dict(sorted(subs.items()))
    return {"sha256": sha(canon(subs)), "sub": subs, "counts": dict(sorted((counts or {}).items()))}


def events_sub(events):
    return sha(canon(events))


# ----------------------------------------------------------------------------------------------- compare

def compare(a, b, minimum=None, parts=None):
    """Compare two manifests. Returns (exit_code, report_lines).

    Per part: items in both, items that differ (hash), items in only one manifest (missing). Exit 1 on any
    difference, any missing item, a part that is absent from a manifest, or fewer items compared than the
    minimum of the part (`minimum`, default DEFAULT_MIN). A part wanted but empty in both fails too.
    """
    minimum = dict(DEFAULT_MIN if minimum is None else minimum)
    parts = list(parts or PARTS)
    lines, bad = [], False
    for ma, name in ((a, "A"), (b, "B")):
        if not isinstance(ma, dict) or ma.get("format") != FORMAT or not isinstance(ma.get("parts"), dict):
            lines.append(f"FAIL manifest {name}: not a format-{FORMAT} identity manifest")
            return 1, lines
    meta_a, meta_b = a.get("meta", {}), b.get("meta", {})
    for k in sorted(set(meta_a) | set(meta_b)):
        if meta_a.get(k) != meta_b.get(k):
            lines.append(f"note: meta {k} differs: {meta_a.get(k)!r} / {meta_b.get(k)!r}")
    for part in parts:
        pa, pb = a["parts"].get(part), b["parts"].get(part)
        if pa is None or pb is None:
            lines.append(f"FAIL {part}: part missing in manifest {'A' if pa is None else 'B'}")
            bad = True
            continue
        ia, ib = pa.get("items", {}), pb.get("items", {})
        both = sorted(set(ia) & set(ib))
        only_a, only_b = sorted(set(ia) - set(ib)), sorted(set(ib) - set(ia))
        differ = [k for k in both if ia[k].get("sha256") != ib[k].get("sha256")]
        need = minimum.get(part, 1)
        lines.append(f"{part}: compared {len(both)} items, {len(differ)} differ, "
                     f"{len(only_a)} only in A, {len(only_b)} only in B (minimum {need})")
        for k in differ:
            sa, sb = ia[k].get("sub", {}), ib[k].get("sub", {})
            what = [s for s in sorted(set(sa) | set(sb)) if sa.get(s) != sb.get(s)]
            ca, cb = ia[k].get("counts", {}), ib[k].get("counts", {})
            cdiff = [f"{c} {ca.get(c)}->{cb.get(c)}" for c in sorted(set(ca) | set(cb)) if ca.get(c) != cb.get(c)]
            lines.append(f"  DIFF {k}: {', '.join(what) or 'item hash'}" + (f" (counts: {', '.join(cdiff)})" if cdiff else ""))
        for k in only_a:
            lines.append(f"  MISSING in B: {k}")
        for k in only_b:
            lines.append(f"  MISSING in A: {k}")
        if differ or only_a or only_b:
            bad = True
        if len(both) < need:
            lines.append(f"FAIL {part}: only {len(both)} items compared, at least {need} wanted")
            bad = True
    lines.append("identical" if not bad else "FAIL: the manifests are not identical")
    return (1 if bad else 0), lines


def load_manifest(path):
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError) as e:
        raise SystemExit(f"FAIL cannot read manifest {path}: {e}")


def parse_min(entries):
    out = dict(DEFAULT_MIN)
    for e in entries or ():
        part, _, n = e.partition("=")
        if part not in PARTS or not n.isdigit():
            raise SystemExit(f"--min wants part=N with part in {PARTS}: {e}")
        out[part] = int(n)
    return out


# ----------------------------------------------------------------------------------------------- the driver

class Svc:
    """One `service_tool cli batch` process: a JSON line in, one answer line out; the sid stays valid."""

    def __init__(self, tool, work, env):
        e = dict(os.environ)
        e.update(env)
        e.update({"DRUMSPIKE_HOME": str(work / "home"), "DRUMSPIKE_PROJECTS": str(work / "projects")})
        (work / "projects").mkdir(parents=True, exist_ok=True)
        self.p = subprocess.Popen([str(tool), "cli", "batch"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, env=e, text=True, bufsize=1)

    def call(self, command, args=None, wait=False):
        """The answer row {command, ok, result | error}; `$sid` in args is the last sid, as in the CLI."""
        self.p.stdin.write(json.dumps({"command": command, "args": args or {}, "wait": wait}) + "\n")
        self.p.stdin.flush()
        line = self.p.stdout.readline()
        if not line:
            raise RuntimeError(f"service_tool ended during {command}")
        row = json.loads(line)
        if not isinstance(row, dict) or "ok" not in row:
            raise RuntimeError(f"unexpected answer to {command}: {line[:200]}")
        return row

    def close(self):
        try:
            self.p.stdin.close()
            self.p.wait(timeout=120)
        except Exception:
            self.p.kill()


def err_sub(row):
    """The sub-hash of a failed step: its error text. A step that fails identically on both sides is identical."""
    return sha(canon(row.get("error")))


def run_project(tool, work, tracks, env, stages, export_spikes):
    """One project through the stages. `tracks` = [(path, role, key)]. Returns {stage: {key: subs}} and
    {stage: {key: counts}} where subs = {"events": hex, "midi": hex, "spikes": hex} (the ones that exist)."""
    svc = Svc(tool, work, env)
    subs, counts = {}, {}
    try:
        row = svc.call("project_new", {"name": "id", "dir": str(work / "projects")})
        if not row["ok"]:
            raise RuntimeError(f"project_new: {row.get('error')}")
        folder = Path(row["result"]["folder"])
        row = svc.call("tracks_add", {"sid": "$sid", "paths": [str(p) for p, _, _ in tracks],
                                      "roles": [r for _, r, _ in tracks],
                                      "notes": [NOTES[r] for _, r, _ in tracks]}, wait=True)
        add = row["result"] if row["ok"] else {}
        added = add.get("added", [])
        # a file the app rejected has no track; its key keeps a sub-hash of the error so a change shows
        names = {}
        for (path, role, key), name in zip(tracks, added):
            names[key] = name
        rejected = [t for t in tracks if t[2] not in names]
        if not row["ok"] or len(added) != len(tracks) - len(add.get("rejected", [])):
            for _, _, key in rejected:
                subs.setdefault("full", {}).setdefault(key, {})["add"] = err_sub(row) if not row["ok"] else sha(canon(add.get("rejected")))
        if names:
            row = svc.call("analyze_start", {"sid": "$sid"}, wait=True)
            if not row["ok"]:
                for key in names:
                    subs.setdefault("full", {}).setdefault(key, {})["analyze"] = err_sub(row)
                return subs, counts
        dur = {}

        def collect(stage):
            for key, name in names.items():
                r = svc.call("events_get", {"sid": "$sid", "name": name})
                s = subs.setdefault(stage, {}).setdefault(key, {})
                if r["ok"]:
                    ev = r["result"]["events"]
                    s["events"] = events_sub(ev)
                    counts.setdefault(stage, {}).setdefault(key, {})["events"] = len(ev)
                    dur[key] = r["result"].get("duration", 0.0)
                else:
                    s["events"] = err_sub(r)

        collect("full")
        r = svc.call("tempo_map_detect", {"sid": "$sid"})      # the tempo suggestion from the kick and snare hits
        subs["full"].setdefault("_tempo", {})["detect"] = sha(canon(r["result"])) if r["ok"] else err_sub(r)
        r = svc.call("export_all", {"sid": "$sid", "formats": ["midi", "spikes"] if export_spikes else ["midi"]}, wait=True)
        if not r["ok"]:
            subs["full"].setdefault("_export", {})["error"] = err_sub(r)
        r = svc.call("export_all", {"sid": "$sid", "formats": ["midi"], "midi_mode": "single"}, wait=True)
        if not r["ok"]:
            subs["full"].setdefault("_export", {})["error"] = err_sub(r)
        exp = folder / "Exports"
        by_name = {name: key for key, name in names.items()}
        for f in sorted(exp.glob("*")) if exp.is_dir() else []:
            kind = "midi" if f.suffix == ".mid" else "spikes" if f.suffix == ".wav" else "file"
            stem = f.name.rsplit("_spike", 1)[0] if "_spike" in f.name else None
            key = by_name.get(stem, "_single" if kind == "midi" else "_other")
            subs["full"].setdefault(key, {})[kind] = sha(f.read_bytes())
            counts.setdefault("full", {}).setdefault(key, {})[kind + "_bytes"] = f.stat().st_size
        if "repick" in stages:
            for key, name in names.items():
                r = svc.call("track_repick", {"sid": "$sid", "name": name, "params": {"sensitivity": REPICK_SENSITIVITY},
                                              "store": True, "keep_edited": False})
                if not r["ok"]:
                    subs.setdefault("repick", {}).setdefault(key, {})["repick"] = err_sub(r)
            collect("repick")
        if "reanalyze" in stages:
            for key, name in names.items():
                d = dur.get(key) or 0.0
                r = svc.call("reanalyze_selection", {"sid": "$sid", "name": name, "t0": REANALYZE_RANGE[0] * d,
                                                     "t1": REANALYZE_RANGE[1] * d, "keep_edited": False})
                if not r["ok"]:
                    subs.setdefault("reanalyze", {}).setdefault(key, {})["reanalyze"] = err_sub(r)
            collect("reanalyze")
    finally:
        svc.close()
    return subs, counts


def items_of(prefix, subs, counts, single_item=None):
    """Manifest items of one project: one per (stage, track key), or one for the whole project (`single_item`)."""
    out = {}
    if single_item:
        flat = {f"{stage}/{key}/{sub}": h for stage, keys in subs.items() for key, ss in keys.items() for sub, h in ss.items()}
        cflat = {f"{stage}/{key}/{c}": n for stage, keys in counts.items() for key, cs in keys.items() for c, n in cs.items()}
        out[single_item] = make_item(flat, cflat)
        return out
    for stage, keys in subs.items():
        for key, ss in keys.items():
            out[f"{prefix}/{stage}/{key}".lstrip("/")] = make_item(ss, counts.get(stage, {}).get(key))
    return out


# ----------------------------------------------------------------------------------------------- the parts

def golden_jobs(scratch):
    """The WAVs of tests/golden_corpus.py written into `scratch`; [(file name, path)] in name order."""
    sys.path.insert(0, str(ROOT / "tests"))
    import golden_corpus  # noqa: E402  (needs numpy: lazy, so `compare` and the unit tests need nothing)
    files = golden_corpus.write_corpus(scratch)
    return sorted((name, scratch / name) for name, _, _ in files)


def part_golden(tool, env, work, cfg, jobs, log):
    wavs = golden_jobs(work / "wav")
    log(f"golden: {len(wavs)} WAVs")

    def one(item):
        name, path = item
        w = work / f"g_{name}"
        subs, counts = run_project(tool, w, [(path, "kick", "t")], env, ("full",), export_spikes=False)
        shutil.rmtree(w, ignore_errors=True)
        return items_of(name, subs, counts)
    items = {}
    with ThreadPoolExecutor(jobs) as ex:
        for r in ex.map(one, wavs):
            items.update(r)
    return items


def enst_index(root, stride):
    """[(drummer, stem, {mic: wav})] of the chosen items, every `stride`-th of each drummer's sorted list."""
    out = []
    for d in sorted(p.name for p in Path(root).glob("drummer_*")):
        stems = []
        for ann in sorted((Path(root) / d / "annotation").glob("*.txt")):
            parts = ann.stem.split("_")
            if len(parts) >= 3 and parts[1] in ENST_TYPES:
                stems.append(ann.stem)
        for stem in stems[::stride]:
            wavs = {m: Path(root) / d / "audio" / m / f"{stem}.wav" for m in ENST_MICS if (Path(root) / d / "audio" / m).is_dir()}
            out.append((d, stem, wavs))
    return out


def part_enst(tool, env, work, cfg, jobs, log):
    root = os.path.expanduser(cfg["root"])
    index = enst_index(root, int(cfg.get("stride", 1)))
    if not index:
        raise SystemExit("FAIL enst: no items found under the configured root")
    log(f"enst: {len(index)} items")

    def one(item):
        d, stem, wavs = item
        missing = [m for m, p in wavs.items() if not p.is_file()]
        if missing:
            raise SystemExit(f"FAIL enst {d}/{stem}: missing mics {missing}")
        w = work / f"e_{d}_{stem}"
        src = w / "src"
        src.mkdir(parents=True)
        tracks = []
        for m, p in wavs.items():                      # one link per mic: the track gets the mic's name
            link = src / f"{m}.wav"
            link.symlink_to(p)
            tracks.append((link, ENST_ROLES.get(m, "tom"), m))
        subs, counts = run_project(tool, w, tracks, env, ("full",), export_spikes=False)
        shutil.rmtree(w, ignore_errors=True)
        return items_of("", subs, counts, single_item=f"{d}/{stem}")
    items = {}
    with ThreadPoolExecutor(jobs) as ex:
        for r in ex.map(one, index):
            items.update(r)
    return items


def part_sessions(tool, env, work, cfg, jobs, log):
    labels = sorted(cfg)
    log(f"sessions: {len(labels)}")

    def one(label):
        c = cfg[label]
        base = Path(os.path.expanduser(c["dir"]))
        tracks = []
        for fname, role, key in c["tracks"]:
            if not (base / fname).is_file():
                raise SystemExit(f"FAIL sessions {label}: missing track file for key {key}")
            tracks.append((base / fname, role, key))
        w = work / f"s_{label}"
        subs, counts = run_project(tool, w, tracks, env, ("full", "repick", "reanalyze"), export_spikes=True)
        shutil.rmtree(w, ignore_errors=True)
        return items_of(label, subs, counts)
    items = {}
    with ThreadPoolExecutor(min(jobs, len(labels)) or 1) as ex:
        for r in ex.map(one, labels):
            items.update(r)
    return items


def snapshot(args):
    import tomllib
    tool = Path(args.tool).resolve()
    if not tool.is_file():
        raise SystemExit(f"FAIL service_tool not found: {tool}")
    cfg_path = Path(args.config).expanduser()
    try:
        cfg = tomllib.loads(cfg_path.read_text())
    except (OSError, ValueError) as e:
        raise SystemExit(f"FAIL config {cfg_path}: {e}")
    env = dict(e.split("=", 1) for e in args.env or ())
    parts = [p for p in (args.parts.split(",") if args.parts else PARTS)]
    for p in parts:
        if p not in PARTS:
            raise SystemExit(f"unknown part {p}")
    t0 = time.time()

    def log(msg):
        print(f"[{time.time() - t0:7.1f}s] {msg}", flush=True)
    fns = {"golden": (part_golden, cfg.get("golden", {})), "enst": (part_enst, cfg.get("enst", {})),
           "sessions": (part_sessions, cfg.get("sessions", {}))}
    out = {"format": FORMAT, "meta": {"tool_sha256": sha(tool.read_bytes())[:16], "env": env,
                                      "config_parts": parts}, "parts": {}}
    work = Path(tempfile.mkdtemp(prefix="identity-"))
    try:
        for part in parts:
            fn, pcfg = fns[part]
            if not pcfg or pcfg.get("enabled") is False:
                raise SystemExit(f"FAIL {part}: not configured in {cfg_path}")
            items = fn(tool, env, work, pcfg, args.jobs, log)
            out["parts"][part] = {"items": dict(sorted(items.items()))}
            log(f"{part}: {len(items)} items")
    finally:
        shutil.rmtree(work, ignore_errors=True)
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    Path(args.out).write_text(json.dumps(out, indent=1, sort_keys=True) + "\n")
    log(f"manifest written: {args.out}")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("snapshot", help="run service_tool over the corpus and write a manifest")
    s.add_argument("--tool", required=True)
    s.add_argument("--out", required=True)
    s.add_argument("--config", default=str(DEFAULT_CONFIG))
    s.add_argument("--parts", help="comma list of golden,enst,sessions (default all)")
    s.add_argument("--env", action="append", help="KEY=VALUE for the tool (an env pin); recorded in the manifest")
    s.add_argument("--jobs", type=int, default=3, help="projects in parallel")
    c = sub.add_parser("compare", help="compare two manifests; exit 1 on any difference")
    c.add_argument("a")
    c.add_argument("b")
    c.add_argument("--min", action="append", help="part=N: the least number of items to compare")
    c.add_argument("--parts", help="comma list of the parts to compare (default all three)")
    args = ap.parse_args(argv)
    if args.cmd == "snapshot":
        return snapshot(args)
    code, lines = compare(load_manifest(args.a), load_manifest(args.b), parse_min(args.min),
                          args.parts.split(",") if args.parts else None)
    print("\n".join(lines))
    return code


if __name__ == "__main__":
    sys.exit(main())
