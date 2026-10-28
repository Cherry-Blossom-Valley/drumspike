"""Attribution flags (card P2-6): the parity check of `parity attribute` against the frozen Python reference.

    cargo build --release -p drumspike-devtools --bin parity
    uv run --project . python tools/parity/check_attribution.py manifest --samples ~/.cache/drumspike/sessions --out /tmp/p26
    uv run --project . python tools/parity/check_attribution.py check /tmp/p26/session_a.json

`manifest`: one manifest per session (Session_A, Session_B, Session_C) with the
app's hits per close mic. P3-12: the manifests are frozen in tests/golden/frozen/attribution.json
(recorded on arm64-mac at ad54ff2; the sample paths below {SAMPLES}); the command writes them with
the given sample folder. They held the CLI onsets of `tests/golden/detect/<stem>.json` (R4),
filtered by the track's role class, min-gap deduplicated (20 ms), refined with the retired
engine's refinement (the P2-9 window) and deduplicated again. The source audio is read by
`parity attribute`, never copied.

`check <manifest>`: runs `target/release/parity attribute` and compares with the frozen
reference of the retired Python implementation (numpy and scipy.signal.sosfilt, with
engine._refine_sr for the source attack): the flag of every hit, the source, the lag, the
transfer gains and the excess values (tolerance 1e-6 dB).

The measurements of the report section "P2-6 Attribution flags" (`eval`, `blind`) needed the
retired engine.py and eval_chains.py; they ran at f9c3814 and are not repeated.
"""
import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests"))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import frozen  # noqa: E402

STORE = frozen.store("attribution")

SESSIONS = {"Session_A": "Session A", "Session_B": "Session B",
            "Session_C": "Session C"}
SLUG = {"Session_A": "session_a", "Session_B": "session_b", "Session_C": "session_c"}
PARITY = ROOT / "target" / "release" / "parity"

def load_json(path, kind=dict):
    d = json.loads(Path(path).read_text())
    if not isinstance(d, kind):
        raise SystemExit(f"{path}: expected {kind.__name__}, got {type(d).__name__}")
    return d


# ---------------------------------------------------------------- manifests

def cmd_manifest(args) -> int:
    """The frozen manifests (P3-12: made by the retired engine, see the module text), with the sample folder."""
    samples = Path(args.samples).expanduser()
    out = Path(args.out).expanduser()
    out.mkdir(parents=True, exist_ok=True)
    golden = sorted((ROOT / "tests" / "golden" / "detect").glob("*.json"))
    for s in SESSIONS:
        detect = {g.name: frozen.digest(load_json(g)) for g in golden
                  if load_json(g).get("set") == "real" and load_json(g).get("session") == s}
        man = STORE.full_value(STORE.ref(f"manifest/{SLUG[s]}", {"session": s, "golden_detect": detect}))
        if frozen.is_digest(man):
            raise SystemExit(f"manifest/{SLUG[s]}: only the digest is frozen here")
        for tr in man["tracks"]:
            tr["path"] = str(samples / tr["path"].removeprefix("{SAMPLES}/"))
        (out / f"{SLUG[s]}.json").write_text(json.dumps(man, indent=1))
        print(f"{SLUG[s]}: {len(man['tracks'])} tracks, {sum(len(tr['hits']) for tr in man['tracks'])} hits", flush=True)
    return 0


def normalized(man, samples=None):
    """The manifest with every sample path below {SAMPLES} (the folder of the three sessions)."""
    out = json.loads(json.dumps(man))
    for t in out["tracks"]:
        k = t["path"].find(SESSIONS[out["session"]])
        t["path"] = "{SAMPLES}/" + t["path"][k:]
    return out


def run_rust(manifest: Path, out: Path, params=None) -> dict:
    """`parity attribute`; `params` overrides the manifest's params (written to a temp copy)."""
    if not PARITY.exists():
        raise SystemExit(f"{PARITY} missing: cargo build --release -p drumspike-devtools --bin parity")
    src = manifest
    if params is not None:
        m = load_json(manifest)
        m["params"] = params
        src = out.with_suffix(".manifest.json")
        src.write_text(json.dumps(m))
    r = subprocess.run([str(PARITY), "attribute", str(src), str(out)], capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"parity attribute failed: {r.stderr[-800:]}")
    return load_json(out)


def cmd_check(args) -> int:
    man = Path(args.manifest).expanduser()
    m = load_json(man)
    out = Path(args.rust_out).expanduser() if args.rust_out else man.with_suffix(".rust.json")
    t0 = time.time()
    rust = run_rust(man, out)
    t_rust = time.time() - t0
    t0 = time.time()
    ref = STORE.full_value(STORE.ref(f"reference/{m['session']}", {"manifest": frozen.digest(normalized(m))}))
    if frozen.is_digest(ref):
        raise SystemExit(f"reference/{m['session']}: only the digest is frozen here")
    flags, transfers = ref["flags"], ref["transfers"]
    t_py = time.time() - t0
    bad, nhit, nb, nr, maxd = [], 0, 0, 0, 0.0
    rt = {(g["source"], g["target"]): g for g in rust["transfers"]}
    for g in transfers:
        r = rt.pop((g["source"], g["target"]), None)
        if r is None or r["hits"] != g["hits"] or r["quiet_only"] != g["quiet_only"]:
            bad.append(("transfer", g, r))
            continue
        d = abs(r["gain_db"] - g["gain_db"])
        maxd = max(maxd, d)
        if d > args.tol:
            bad.append(("transfer_gain", g, r))
    if rt:
        bad.append(("transfer_extra", list(rt)))
    for t, ft, tr in zip(m["tracks"], flags, rust["tracks"]):
        for h, f, rh in zip(t["hits"], ft, tr["hits"]):
            nhit += 1
            for k in ("bleed", "resonance"):
                a, b = f[k], rh[k]
                if (a is None) != (b is None):
                    bad.append((k, t["id"], h["sample"], a, b))
                    continue
                if a is None:
                    continue
                nb += k == "bleed"
                nr += k == "resonance"
                if k == "bleed" and (a["source"] != b["source"] or a["lag_samples"] != b["lag_samples"]):
                    bad.append((k + "_source", t["id"], h["sample"], a, b))
                for key in ("excess_db", "dominance_db", "confidence", "rise_db", "median_db"):
                    if key in a:
                        d = abs(a[key] - b[key])
                        maxd = max(maxd, d)
                        if d > args.tol:
                            bad.append((k + "_" + key, t["id"], h["sample"], a, b))
    res = {"manifest": str(man), "hits": nhit, "bleed": nb, "resonance": nr, "transfers": len(transfers),
           "mismatches": len(bad), "max_abs_diff_db": maxd, "rust_s": round(t_rust, 2),
           "rust_attribute_s": round(rust["seconds"], 3), "python_s": round(t_py, 1)}
    print(json.dumps(res))
    for b in bad[:20]:
        print("MISMATCH", b)
    return 1 if bad else 0


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    m = sub.add_parser("manifest")
    m.add_argument("--samples", required=True)
    m.add_argument("--out", required=True)
    c = sub.add_parser("check")
    c.add_argument("manifest")
    c.add_argument("--rust-out")
    c.add_argument("--tol", type=float, default=1e-6)
    a = p.parse_args(argv)
    return {"manifest": cmd_manifest, "check": cmd_check}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
