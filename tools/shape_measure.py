"""#443: measure the track-shape placement against the labelled rows (internal data; aggregates only).

    uv run --no-project --with numpy python tools/shape_measure.py DATA_DIR [--pre 16 --post 8]

DATA_DIR holds `fit_v10.csv` + `fit_v10.f32` (the labelled 960-sample windows at 48 kHz, 480 = the
app's placement at labelling time). Per track (session, role) the tool builds the foot shape from
the track's CLEAR hits (line excursion under 1 % of the peak), taken at the position the current
rule gave them, at 4x resolution with `--pre` samples before the hit and `--post` after, sharpened
by three rounds of re-alignment. Every hit of the track is then placed at the lag of the best
zero-mean normalised correlation against that shape (no label used).

Printed per role: exact, within 1, off by 4 or more, for the rule and for the shape, for all gold
rows, for the rows with an unclear line (2 % or more) and for the owner rows; then the agreement
table (rule and shape agree / differ by 2 or more) with the rule's error share in each group.
Every printed set also prints its hit count; a set with 0 hits is a FAIL (exit 1).

All data comes from the arguments, never from a path in the code.
"""
import argparse
import csv
import sys
from pathlib import Path

import numpy as np


def gold(r):
    """A row with a placed truth and a rule answer (#385's gold rule)."""
    return (r["confirmed"] == "1" or int(r["label_off"]) != 0 or r["owner"] == "1") and r["cur_off"] != ""


def load_rows(data_dir, pre, post):
    """The gold rows as dicts: role, samples (the 960-sample window), rule and label positions."""
    for name in ("fit_v10.csv", "fit_v10.f32"):
        if not (Path(data_dir) / name).is_file():
            sys.exit(f"FAIL: {Path(data_dir) / name} is missing (pass the data dir as an argument)")
    rows = list(csv.DictReader(open(Path(data_dir) / "fit_v10.csv")))
    x = np.fromfile(Path(data_dir) / "fit_v10.f32", dtype="<f4").reshape(-1, 960)
    out = []
    for r in rows:
        if not gold(r):
            continue
        i = int(r["i"])
        out.append({"role": r["role"], "session": r["session"], "owner": r["owner"] == "1",
                    "x": x[i].astype(float), "rule": 480 + int(r["cur_off"]),
                    "label": 480 + int(r["label_off"]), "pre": pre, "post": post})
    return out


def line_exc(x, c):
    """The line excursion in percent of the peak: the 2 ms before `c` against their median."""
    pre = x[c - 96:c]
    pk = np.abs(x[c:c + 240]).max()
    return np.abs(pre - np.median(pre)).max() / pk * 100 if pk > 0 else 99.0


def upsample(x, up):
    """A band-limited `up`x copy of one window (FFT zero-padding, no scipy)."""
    n = len(x)
    f = np.fft.rfft(x)
    g = np.zeros(n * up // 2 + 1, dtype=complex)
    g[:len(f)] = f
    return np.fft.irfft(g, n * up) * up


def zn(v):
    v = v - v.mean()
    n = np.linalg.norm(v)
    return v / n if n > 0 else v


def best_lag(xu, c, tmpl_u, pre, search, up):
    """The lag (in samples, 1/`up` steps) of the best zero-mean normalised match around `c`."""
    best, bl = -2.0, 0.0
    length = len(tmpl_u)
    for d in range(-search * up, search * up + 1):
        a = (c - pre) * up + d
        if a < 0 or a + length > len(xu):
            continue
        s = float(np.dot(zn(xu[a:a + length]), tmpl_u))
        if s > best:
            best, bl = s, d / up
    return bl, best


def shape_track(rows, pre, post, search, up, rounds=3):
    """The shape placement of one track: the template from the clear hits, then every hit placed.

    Returns (positions, used_clear): the estimated rule-relative lag per row in row order and the
    indices of the hits the template was built from."""
    xs = [r["x"] for r in rows]
    cur = [r["rule"] for r in rows]
    xu = [upsample(x, up) for x in xs]
    clear = [k for k in range(len(rows)) if line_exc(xs[k], cur[k]) < 1.0]
    if len(clear) < 15:   # a track with hardly any clear line: the least-excursion rows instead
        clear = sorted(range(len(rows)), key=lambda k: line_exc(xs[k], cur[k]))[:max(15, len(rows) // 4)]
    lag = {k: 0.0 for k in clear}
    t = zn(np.zeros((pre + post) * up))
    for _ in range(rounds):
        segs = []
        for k in clear:
            a = int(round((cur[k] - pre + lag[k]) * up))
            s = xu[k][a:a + (pre + post) * up]
            pk = np.abs(s).max()
            segs.append(s / pk if pk > 0 else s)
        t = zn(np.median(np.array(segs), axis=0))
        for k in clear:
            lag[k], _ = best_lag(xu[k], cur[k], t, pre, search, up)
        m = np.median(list(lag.values()))   # the template keeps the rule's average position (no drift)
        for k in clear:
            lag[k] -= m
    est = []
    for k in range(len(rows)):
        l, _ = best_lag(xu[k], cur[k], t, pre, search, up)
        est.append(cur[k] + l)
    return np.array(est), clear


def measure(rows, pre=16, post=8, search=14, up=4):
    """The aggregates: {"sets": {set: {"n", "rule": [exact, within1, 4+] %, "shape": ...}},
    "agreement": {group: {"n", "share", "rule_exact", "rule_4+"}}, "tracks": [(session, role, n, n_clear)]}.

    Sets: "<role>", "<role> unclear line", "owner <role>". A row's shape error is the rounded
    shape position against the label; the rule's is its own position against the label."""
    tracks = {}
    for r in rows:
        tracks.setdefault((r["session"], r["role"]), []).append(r)
    tot = {}
    agreement = {}
    for (_, role), rs in sorted(tracks.items()):
        est, _clear = shape_track(rs, pre, post, search, up)
        lab = np.array([r["label"] for r in rs])
        cur = np.array([r["rule"] for r in rs])
        e_rule = cur - lab
        e_shape = np.round(est) - lab
        unclear = np.array([line_exc(r["x"], r["label"]) >= 2.0 for r in rs])
        own = np.array([r["owner"] for r in rs])
        for name, mask in ((role, np.ones(len(rs), bool)), (f"{role} unclear line", unclear), (f"owner {role}", own)):
            if not mask.any():
                continue
            for method, e in (("rule", e_rule), ("shape", e_shape)):
                tot.setdefault((name, method), []).extend(e[mask].tolist())
        d = np.round(est) - cur
        for gname, mask in (("agree", d == 0), ("differ by 2+", np.abs(d) >= 2)):
            if not mask.any():
                continue
            agreement.setdefault((role, gname), []).extend(zip(e_rule[mask].tolist(), e_shape[mask].tolist()))
    sets = {}
    for (name, method), e in tot.items():
        e = np.array(e)
        sets.setdefault(name, {})[method] = {"n": len(e), "exact": 100 * float((e == 0).mean()),
                                             "within1": 100 * float((np.abs(e) <= 1).mean()),
                                             "off4": 100 * float((np.abs(e) >= 4).mean())}
    ag = {}
    for (role, gname), pairs in agreement.items():
        a = np.array(pairs)
        n_all = sets[role]["rule"]["n"]   # the share is of all rows of the role, 1-off rows included
        ag[f"{role} {gname}"] = {"n": len(a), "share": 100 * len(a) / n_all,
                                 "rule_exact": 100 * float((a[:, 0] == 0).mean()),
                                 "rule_off4": 100 * float((np.abs(a[:, 0]) >= 4).mean()),
                                 "shape_exact": 100 * float((a[:, 1] == 0).mean())}
    return {"sets": sets, "agreement": ag,
            "tracks": [{"session": s, "role": role, "rows": len(rs)} for (s, role), rs in sorted(tracks.items())]}


def print_report(res, out=sys.stdout):
    """The readable tables; every set with 0 hits is reported and makes the run FAIL."""
    fail = False
    print("per role: exact / within 1 / off by 4+, percent of the set", file=out)
    for name, methods in res["sets"].items():
        for method in ("rule", "shape"):
            m = methods.get(method)
            if m is None or m.get("n", 0) == 0:
                print(f"FAIL: {name}: 0 hits", file=out)
                fail = True
                continue
            print(f"{name:22} {method:5} n={m['n']:5}  exact={m['exact']:5.1f}  within1={m['within1']:5.1f}  off4+={m['off4']:5.1f}", file=out)
    print("rule and shape agreement (no label used):", file=out)
    for g, m in res["agreement"].items():
        print(f"{g:22} n={m['n']:5} ({m['share']:4.1f}%)  rule exact={m['rule_exact']:5.1f}  rule 4+={m['rule_off4']:5.1f}  shape exact={m['shape_exact']:5.1f}", file=out)
    if not res["agreement"]:
        print("FAIL: the agreement table is empty", file=out)
        fail = True
    for t in res["tracks"]:
        print(f"track {t['session']} {t['role']}: {t['rows']} rows", file=out)
    return 1 if fail else 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("data_dir", help="the labelled rows: fit_v10.csv + fit_v10.f32")
    ap.add_argument("--pre", type=int, default=16)
    ap.add_argument("--post", type=int, default=8)
    ap.add_argument("--search", type=int, default=14, help="the lag search radius, in samples")
    ap.add_argument("--up", type=int, default=4, help="the template resolution factor")
    a = ap.parse_args(argv)
    rows = load_rows(a.data_dir, a.pre, a.post)
    if not rows:
        sys.exit("FAIL: no gold rows in the data")
    res = measure(rows, pre=a.pre, post=a.post, search=a.search, up=a.up)
    print(f"gold rows {len(rows)} over {len(res['tracks'])} tracks")
    return print_report(res)


if __name__ == "__main__":
    sys.exit(main() or 0)
