"""P4-14 stage 1b: rule B of stage 1 checked on the new sessions, with tom references from the mixer's sample tracks.

    uv run python tools/tom_bleed_new.py run        # the app chain (refinement version 2) on the drum mics
    uv run python tools/tom_bleed_new.py refs       # the tom references: per-tom sample tracks, or a combined one split
    uv run python tools/tom_bleed_new.py validate   # the split on Session F's per-tom tracks summed (its truth)
    uv run python tools/tom_bleed_new.py attribute  # `parity attribute` (P2-6), as in stage 1
    uv run python tools/tom_bleed_new.py score      # base, P2-6 removed and rule B per session and tom mic
    uv run python tools/tom_bleed_new.py all

Sessions (read only): Session H, Session F, Session G, Copy of Session E, Session D.
Session I is left out: each of its drum mics is split into several files (`_01`, `_03`, `_04`, all
full length), and which of them the mix uses is in the Pro Tools session only.

References. Session F has one sample track per tom (`t1 samp` .. `t4 samp`): their onsets (`tools/learn_eval.py`
`ref_onsets`) are the references, independent of the mics. The other sessions have one combined tom sample or trigger
track. Its hits are split by the sample itself (`split_hits`):
  1. clusters of hits with the same waveform: the first 40 ms (from 2 ms before the onset) of every channel, lowpass
     1.5 kHz, best normalised correlation within +-1 ms, average linkage down to 0.5 (a sample replacer plays one
     sample, or a few velocity layers, per tom); clusters of at least 5 % of the hits (and 5 hits) are the groups, a
     smaller cluster joins the group it resembles most;
  2. each whole group is named by the tom mic that is loudest at most of its hits (each mic against its own 99.9th
     percentile 1 ms frame, the P4-6b level); the mic levels never split hits, they only name a group;
  3. the pitch of each group (the spectral peak from 40 to 300 Hz, 10 to 150 ms after its isolated hits) is reported.
`validate` sums Session F's four per-tom tracks and splits the sum: the share of hits that reach their own tom
is the split's accuracy. The match is as in stage 1 (one offset per mic, one to one within +-20 ms). The split-free
check: a combined reference hit that some tom mic found before rule B and no tom mic keeps after it is a real hit lost,
whatever tom it was. The tom mics are close mics only (`tools/tom_bleed.py` `is_close_tom`): no sample or trigger
track counts as another tom.

Rule B is fixed from stage 1 (`RULE_B`), not refit. Outputs are under `~/.cache/drumspike/tom-bleed/new/`.
"""
import argparse
import json
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

sys.path.insert(0, str(Path(__file__).resolve().parent))
import learn_eval as le  # noqa: E402
import miss_causes as mc  # noqa: E402
import rescue_real as rr  # noqa: E402
import tom_bleed as tb  # noqa: E402

OUT = tb.OUT / "new"
RULE_B = [("level_db", -5.267), ("tom_rel_db", 2.146)]   # stage 1, chosen on Session A
MAJOR_SHARE = 0.05
MAJOR_MIN = 5
LINK_MIN = 0.5
SEG = (-0.002, 0.038)
LOWPASS = 1500.0
F0_WINDOW = (0.010, 0.150)
ISOLATED_S = 0.150

TOMS4 = ("t1", "t2", "t3", "t4")


def session(dir_, mics, toms_ref):
    return {"dir": dir_, "excluded": set(), "mics": mics, "toms_ref": toms_ref}


def drum_mics(k, s, t, hh, oh):
    """The mic table of a session: kick, snare, the four toms, the hat and one or two overheads."""
    m = {"k in": (k, "kick", "k in"), "s top": (s, "snare", "s top")}
    m.update({name: (stem, "tom", name) for name, stem in zip(TOMS4, t)})
    m["hh"] = (hh, "hat", "hh")
    for i, stem in enumerate(oh):
        m[f"oh{i + 1}" if len(oh) > 1 else "oh"] = (stem, "cymbal", "oh")
    return m


SESSIONS = {
    "Session H": session("Session H", drum_mics("k in_01", "s top_01", ["t 1_01", "t 2_01", "t 3_01", "t 4_01"],
                                                  "hh_04", ["oh_03.L", "oh_03.R"]),
                       {"combined": ["t samp_01.L", "t samp_01.R"]}),
    "Session F": session("Session F",
                                drum_mics("k in_01", "s top_01", ["t 1_01", "t 2_01", "t 3_01", "t 4_01"], "hh_01",
                                          ["oh_01.L", "oh_01.R"]),
                                {"per_tom": {t: [f"{t} samp_01.L", f"{t} samp_01.R"] for t in TOMS4}}),
    "Session G": session("Session G",
                                        drum_mics("k in_01", "s top_01", ["t1HH_01", "t2_01", "t3_01", "t4Ri_01"],
                                                  "hh_10", ["oh_01.L", "oh_01.R"]),
                                        {"combined": ["t trigger_01.L", "t trigger_01.R"]}),
    "Session E": session("Session E",
                                drum_mics("k in-AtA2", "s top-AtA2", ["t1 hh-AtA2", "t2-AtA2", "t3-AtA2", "t4 ri-AtA2"],
                                          "hh-AtA2", ["oh-AtA2.L", "oh-AtA2.R"]),
                                {"combined": ["t sample sd3_03.L", "t sample sd3_03.R"]}),
    "Session D": session("Session D",
                              drum_mics("k in", "s top", ["t1 hh", "t2", "t3", "t4 ri"], "hh", ["oh HH", "oh Ri"]),
                              {"combined": ["t trigger.L", "t trigger.R"]}),
}


def ref_paths(sess, stems):
    return [tb.SAMPLES / SESSIONS[sess]["dir"] / "Audio Files" / f"{s}.wav" for s in stems]


def read_channels(paths):
    """Every channel of every file, cut to the shortest (the L and R files of a split stereo track)."""
    chans, sr = [], None
    for p in paths:
        x, sr = sf.read(str(p), dtype="float32", always_2d=True)
        chans += [x[:, c] for c in range(x.shape[1])]
    n = min(len(c) for c in chans)
    return [c[:n] for c in chans], int(sr)


# ---------------------------------------------------------------- the split by the sample itself

def lowpass(x, sr, fc=LOWPASS):
    X = np.fft.rfft(np.asarray(x, float))
    X[np.fft.rfftfreq(len(x), 1 / sr) > fc] = 0
    return np.fft.irfft(X, len(x))


def waveforms(chans, sr, onsets, seg=SEG):
    """One unit vector per hit: the lowpassed `seg` window of every channel, concatenated. (W, samples per channel)"""
    lp = [lowpass(c, sr) for c in chans]
    a0, n = int(round(seg[0] * sr)), int(round((seg[1] - seg[0]) * sr))
    rows = []
    for t in onsets:
        a = int(round(t * sr)) + a0
        v = np.concatenate([np.pad(c[max(0, a):a + n], (max(0, -a), max(0, a + n - len(c)))) for c in lp])
        rows.append(v / (np.linalg.norm(v) + 1e-12))
    return np.asarray(rows).reshape(len(rows), -1), n


def similarity(W, n, n_chans, max_lag):
    """Pairwise normalised correlation, the best over shifts of up to `max_lag` samples (every second one)."""
    sim = W @ W.T
    for s in range(1, max_lag + 1, 2):
        Ws = np.roll(W.reshape(len(W), n_chans, n), s, axis=2).reshape(len(W), -1)
        sim = np.maximum(sim, np.maximum(W @ Ws.T, Ws @ W.T))
    np.fill_diagonal(sim, 1.0)
    return sim


def average_linkage(sim, th=LINK_MIN):
    """Clusters (lists of indices, largest first): merge the pair with the highest mean similarity while it is >= th."""
    k = len(sim)
    if k == 0:
        return []
    tot, size, alive = sim.astype(float).copy(), np.ones(k), np.ones(k, bool)
    members = [[i] for i in range(k)]
    while alive.sum() > 1:
        a = tot / np.outer(size, size)
        a[~alive] = -9
        a[:, ~alive] = -9
        np.fill_diagonal(a, -9)
        i, j = np.unravel_index(int(np.argmax(a)), a.shape)
        if a[i, j] < th:
            break
        tot[i] += tot[j]
        tot[:, i] += tot[:, j]
        size[i] += size[j]
        alive[j] = False
        members[i] += members[j]
    return sorted((sorted(members[i]) for i in np.flatnonzero(alive)), key=lambda c: (-len(c), c[0]))


def major_groups(clusters, sim, share=MAJOR_SHARE, min_hits=MAJOR_MIN):
    """The clusters of at least `share` of the hits (and `min_hits`) are the groups; a smaller cluster joins the group
    whose hits it resembles most (mean similarity). Groups are lists of hit indices, in the order of `clusters`."""
    n = sum(len(c) for c in clusters)
    major = [i for i, c in enumerate(clusters) if len(c) >= max(min_hits, share * n)] or [0]
    groups = {i: list(clusters[i]) for i in major}
    for i, c in enumerate(clusters):
        if i not in groups:
            best = max(major, key=lambda g: float(sim[np.ix_(c, clusters[g])].mean()))
            groups[best] += c
    return [sorted(groups[i]) for i in major]


def hit_pitch(x, sr, onsets, window=F0_WINDOW, lo=40.0, hi=300.0, nfft=1 << 16):
    """The spectral peak (Hz) in `lo`..`hi` of `window` after every onset, clipped at the next onset (at least 30 ms)."""
    f = np.fft.rfftfreq(nfft, 1 / sr)
    sel = (f >= lo) & (f <= hi)
    out = []
    for k, t in enumerate(onsets):
        end = t + window[1]
        if k + 1 < len(onsets):
            end = max(t + window[0] + 0.030, min(end, onsets[k + 1]))
        seg = np.asarray(x[int((t + window[0]) * sr):int(end * sr)], float)
        if len(seg) < 16:
            out.append(float("nan"))
            continue
        p = np.abs(np.fft.rfft(seg * np.hanning(len(seg)), nfft)) ** 2
        out.append(float(f[sel][int(np.argmax(p[sel]))]))
    return np.asarray(out)


def isolated(onsets, after=ISOLATED_S):
    """True for an onset with no next onset within `after` (its pitch window is not cut short)."""
    on = np.asarray(onsets, float)
    return (np.append(on[1:], np.inf) - on) >= after


def group_pitch(group, pitch, iso):
    """Median pitch (Hz) of a group's isolated hits (all hits when none is isolated) and the p10, p90 of all."""
    g = np.asarray(group)
    use = g[iso[g]] if iso[g].any() else g
    return float(np.nanmedian(pitch[use])), [float(v) for v in np.nanpercentile(pitch[g], [10, 90])]


def vote_names(groups, loudest):
    """[(mic, share)] per group: the tom mic that is loudest at most of the group's hits (a name for the whole group;
    several groups may take one mic: a tom with several samples or velocity layers)."""
    out = []
    for g in groups:
        cnt = {}
        for i in g:
            cnt[loudest[i]] = cnt.get(loudest[i], 0) + 1
        m = max(sorted(cnt), key=lambda k: cnt[k])
        out.append((m, cnt[m] / len(g)))
    return out


def split_hits(chans, sr, onsets, loudest):
    """The split of a combined tom reference: ({mic: [t]}, detail). The groups come from the sample alone
    (`average_linkage` of the waveforms, `major_groups`); `loudest` (one mic per hit) only names each whole group."""
    onsets = np.asarray(onsets, float)
    W, n = waveforms(chans, sr, onsets)
    sim = similarity(W, n, len(chans), int(round(0.001 * sr)))
    clusters = average_linkage(sim)
    groups = major_groups(clusters, sim)
    names = vote_names(groups, loudest)
    pitch = hit_pitch(np.mean(chans, axis=0), sr, onsets)
    iso = isolated(onsets)
    out = {m: [] for m in TOMS4}
    for (m, _), g in zip(names, groups):
        out[m] += onsets[g].tolist()
    detail = {"hits": len(onsets), "clusters": [len(c) for c in clusters], "groups": []}
    for (m, share), g in zip(names, groups):
        hz, p1090 = group_pitch(g, pitch, iso)
        others = [h for h in range(len(onsets)) if h not in set(g)]
        detail["groups"].append({
            "mic": m, "vote": round(share, 3), "hits": len(g), "pitch_hz": round(hz, 1),
            "pitch_p10_p90": [round(v, 1) for v in p1090], "within": round(float(sim[np.ix_(g, g)].mean()), 3),
            "to_others": round(float(sim[np.ix_(g, others)].mean()), 3) if others else None})
    return {m: sorted(v) for m, v in out.items()}, detail


# ---------------------------------------------------------------- steps

def loudest_mics(sess, onsets, pre=0.005, post=0.025):
    """The tom mic with the highest level at each onset: the loudest 1 ms frame in [t - pre, t + post], against the
    mic's own 99.9th percentile frame (the P4-6b level; a tom mic with few hits of its own keeps a fair scale). Used
    only to name a whole group of the split, never to split."""
    lv = {}
    for m in TOMS4:
        x, sr = tb.read_mono(tb.audio_path(sess, m))
        e, step = le.frame_db(x, sr)
        loud = float(np.percentile(e, 99.9))
        lv[m] = [float(e[max(0, int((t - pre) / step)):int((t + post) / step) + 1].max()) - loud for t in onsets]
    return [max(TOMS4, key=lambda m: lv[m][i]) for i in range(len(onsets))]


def do_refs(out, log=print):
    out = Path(out)
    tb.use_sessions(SESSIONS)
    refs, details = {}, {}
    for s, cfg in SESSIONS.items():
        tr = cfg["toms_ref"]
        if "per_tom" in tr:
            refs[s] = {}
            for m, stems in tr["per_tom"].items():
                x, sr = le.read_mono(ref_paths(s, stems))
                refs[s][m] = le.ref_onsets(x, sr).tolist()
            details[s] = {"per_tom": {m: len(v) for m, v in refs[s].items()}}
        else:
            chans, sr = read_channels(ref_paths(s, tr["combined"]))
            on = le.ref_onsets(np.mean(chans, axis=0), sr)
            split, det = split_hits(chans, sr, on, loudest_mics(s, on))
            refs[s] = {m: split.get(m, []) for m in TOMS4}
            refs[s]["toms_combined"] = on.tolist()
            details[s] = det
        log(f"refs {s}: " + ", ".join(f"{m} {len(refs[s][m])}" for m in TOMS4))
    (out / "refs.json").write_text(json.dumps(refs))
    (out / "split.json").write_text(json.dumps(details))
    return refs, details


def split_accuracy(truth, split, tol=0.005):
    """{true mic: {named mic: hits}} for a split against per-tom truth (a hit is the same within `tol`)."""
    tab = {}
    for m, v in truth.items():
        row = {}
        for t in v:
            name = None
            for k, w in split.items():
                if len(w) and float(np.min(np.abs(np.asarray(w) - t))) <= tol:
                    name = k
                    break
            row[name] = row.get(name, 0) + 1
        tab[m] = row
    return tab


def cluster_purity(clusters, onsets, truth, tol=0.005):
    """(hits in the majority tom of their cluster, hits with a true tom within `tol`) over all clusters."""
    lab = []
    for t in onsets:
        near = [(float(np.min(np.abs(np.asarray(v) - t))), m) for m, v in truth.items() if len(v)]
        d, m = min(near) if near else (np.inf, None)
        lab.append(m if d <= tol else None)
    major = total = 0
    for c in clusters:
        cnt = {}
        for i in c:
            if lab[i] is not None:
                cnt[lab[i]] = cnt.get(lab[i], 0) + 1
        major += max(cnt.values()) if cnt else 0
        total += sum(cnt.values())
    return major, total


def do_validate(out, log=print):
    """The split of Session F's four per-tom sample tracks summed, against their own onsets."""
    out = Path(out)
    tb.use_sessions(SESSIONS)
    s = "Session F"
    truth, mix, sr = {}, None, None
    for m, stems in SESSIONS[s]["toms_ref"]["per_tom"].items():
        chans, sr = read_channels(ref_paths(s, stems))
        truth[m] = le.ref_onsets(np.mean(chans, axis=0), sr).tolist()
        mix = chans if mix is None else [a + b for a, b in zip(mix, chans)]
    on = le.ref_onsets(np.mean(mix, axis=0), sr)
    split, det = split_hits(mix, sr, on, loudest_mics(s, on))
    tab = split_accuracy(truth, split)
    W, n = waveforms(mix, sr, on)
    clusters = average_linkage(similarity(W, n, len(mix), int(round(0.001 * sr))))
    res = {"purity": cluster_purity(clusters, on, truth),"truth": {m: len(v) for m, v in truth.items()}, "onsets": len(on), "table": tab, "detail": det,
           "right": sum(r.get(m, 0) for m, r in tab.items()), "total": sum(len(v) for v in truth.values())}
    (out / "validate.json").write_text(json.dumps(res))
    log(f"validate: clusters {res['purity'][0]} of {res['purity'][1]} pure; {res['right']} of {res['total']} hits to "
        f"the right tom; {tab}")
    return res


def do_features(out, log=print):
    out = Path(out)
    tb.use_sessions(SESSIONS)
    refs = json.loads((out / "refs.json").read_text())
    rows = []
    for s in SESSIONS:
        rows += tb.session_features(out, s, refs, {}, log)
    (out / "features.json").write_text(json.dumps(rows))
    return rows


def union_check(rows, refs, rule, tol=rr.TOL):
    """Per session: (combined reference hits, found by some tom mic before, of them kept by some tom mic after `rule`).
    The offset is the median of the per-mic offsets."""
    res = {}
    for s in sorted({r["session"] for r in rows}):
        sel = [r for r in rows if r["session"] == s]
        comb = refs[s].get("toms_combined") or sorted(t for m in TOMS4 for t in refs[s].get(m, []))
        off = float(np.median([r["offset_ms"] for r in sel])) / 1000 if sel else 0.0
        ref = np.asarray(comb) + off
        before = np.asarray(sorted(r["t"] for r in sel))
        after = np.asarray(sorted(r["t"] for r in sel if not tb.removes(rule, r)))

        def found(est):
            return {i for i, t in enumerate(ref) if len(est) and float(np.min(np.abs(est - t))) <= tol}

        fb, fa = found(before), found(after)
        res[s] = {"refs": len(ref), "found_before": len(fb), "kept_after": len(fb & fa), "lost": len(fb - fa)}
    return res


def do_score(out, log=print):
    out = Path(out)
    tb.use_sessions(SESSIONS)
    refs = json.loads((out / "refs.json").read_text())
    rows = do_features(out, log=lambda *_: None)
    sessions = list(SESSIONS)

    def keep_rule(r):
        return not tb.removes(RULE_B, r)

    per = {}
    for name, keep in (("base", lambda r: True), ("p26", lambda r: not r["p26"]), ("rule", keep_rule)):
        per[name] = tb.evaluate(rows, refs, keep, sessions)
    by_track = tb.by_track(rows)
    head = ["session", "tom mic", "refs", "base found/false/F", "P2-6 removed found/false/F", "rule B found/false/F",
            "real lost", "false removed"]
    body = []
    for k in per["base"][0]:
        tr = by_track[k]
        body.append([k[0], k[1], len(refs[k[0]][k[1]]), tb.fmt(per["base"][0][k]), tb.fmt(per["p26"][0][k]),
                     tb.fmt(per["rule"][0][k]), sum(1 for r in tr if r["real"] and tb.removes(RULE_B, r)),
                     sum(1 for r in tr if r["real"] is False and tb.removes(RULE_B, r))])
    for s in sessions:
        sel = [r for r in rows if r["session"] == s]
        body.append([f"**{s}**", "**toms**", sum(len(refs[s][m]) for m in TOMS4), tb.fmt(per["base"][1][s]),
                     tb.fmt(per["p26"][1][s]), tb.fmt(per["rule"][1][s]),
                     sum(1 for r in sel if r["real"] and tb.removes(RULE_B, r)),
                     sum(1 for r in sel if r["real"] is False and tb.removes(RULE_B, r))])
    n_real = sum(1 for r in rows if r["real"])
    lost = sum(1 for r in rows if r["real"] and tb.removes(RULE_B, r))
    body.append(["**all**", "**toms**", sum(len(refs[s][m]) for s in sessions for m in TOMS4), tb.fmt(per["base"][2]),
                 tb.fmt(per["p26"][2]), tb.fmt(per["rule"][2]), lost,
                 sum(1 for r in rows if r["real"] is False and tb.removes(RULE_B, r))])
    uni = union_check(rows, refs, RULE_B)
    vp = out / "validate.json"
    val = json.loads(vp.read_text()) if vp.is_file() else None
    lines = [f"### Rule B on the new sessions ({tb.rule_text(RULE_B)}, fixed from stage 1)", "",
             mc.md_table(head, body), "",
             f"Real tom hits lost: {lost} of {n_real} ({100 * lost / max(1, n_real):.1f} %).", "",
             tb.stroke_line(rows, refs, RULE_B), "",
             "Split-free check (a combined reference hit that some tom mic found before rule B and none keeps after):",
             "", mc.md_table(["session", "tom reference hits", "found before", "kept after", "lost"],
                             [[s, u["refs"], u["found_before"], u["kept_after"], u["lost"]] for s, u in uni.items()]),
             ""]
    if val:
        lines += [f"Split check on Session F (its four per-tom tracks summed and split): waveform clusters "
                  f"{val['purity'][0]} of {val['purity'][1]} pure; {val['right']} of "
                  f"{val['total']} hits named by their own tom; per true tom: {json.dumps(val['table'])}.", ""]
    res = {"base": per["base"][2], "p26": per["p26"][2], "rule": per["rule"][2], "lost": lost, "real": n_real,
           "sessions": {s: {"base": per["base"][1][s], "p26": per["p26"][1][s], "rule": per["rule"][1][s]}
                        for s in sessions}, "union": uni, "tracks": len(by_track)}
    (out / "scores.json").write_text(json.dumps(res))
    (out / "report.md").write_text("\n".join(lines))
    log("\n".join(lines))
    return res


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["run", "refs", "validate", "attribute", "score", "all"])
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--jobs", type=int, default=3)
    a = ap.parse_args(argv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    tb.use_sessions(SESSIONS)
    if a.cmd in ("run", "all"):
        tb.run_all(out, a.jobs)
    if a.cmd in ("refs", "all"):
        do_refs(out)
    if a.cmd in ("validate", "all"):
        do_validate(out)
    if a.cmd in ("attribute", "all"):
        tb.run_attribute(out)
    if a.cmd in ("score", "all"):
        do_score(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
