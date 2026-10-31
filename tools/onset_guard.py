"""P4-15 stage 1: the onset guard against bleed (measure only; no product change).

    uv run python tools/onset_guard.py events     # refinement versions 2 and 3 on the real sessions (events.json)
    uv run python tools/onset_guard.py labels     # the labelled set: hand check plus the stated rule (labels.json)
    uv run python tools/onset_guard.py measure    # candidates A and B on the labelled set (real.json, real.md)
    uv run python tools/onset_guard.py kit        # candidates A and B on the P4-11 kit outputs (kit.json, kit.md)

Refinement version 3 (P4-12 stage 2, `refine.rs` `first_motion`) walks back from the version 2 time `s` to the foot of
the attack. On real snare and tom mics the walk can join bleed that starts less than 5 ms before the hit (a hat or
cymbal burst, a kick plateau). Both candidates run after version 3 and give a foot in [version 3, version 2]: they never
move a hit earlier than version 3 and never later than version 2.

A, cross-mic lead stop: the walk may not pass a stretch where another mic explains the own mic's motion. For each other
mic B (close mics of other drums, hat, overheads, ride; never a mic of the same drum) the transfer gain G(B->A) is
calibrated as in P2-6 (`attribution::calibrate_gain`): at B's strong hits where A has no own hit within 30 ms, the
`pct` percentile of A's band peak minus B's band peak, in B's band (`Role::band`). A sample i is explained when the
own mic's envelope at i is at most B's band envelope in [i - lead, i + lag] plus G plus `margin` dB, for some B. The
walk (the version 3 walk, ported below) stops at a run of explained samples of `run_ms` or more.

B, band walk: the version 3 walk on a zero-phase low-pass of the own mic (snare 2 kHz, the card's value; toms and kick
`--lp`), the result clipped to [version 3, version 2]: the walk may not pass the band walk's foot.

Real sessions: `~/.cache/drumspike/sessions`, the songs Session A, Session B and Session C (read only; the
verifier's hand check is on these three). Outputs in `~/.cache/drumspike/onset-guard/`; nothing goes into git.
"""
import argparse
import json
import os
import sys
import time
from pathlib import Path

import numpy as np
import soundfile as sf

sys.path.insert(0, str(Path(__file__).resolve().parent))
import onset_early as oe  # noqa: E402

OUT = Path.home() / ".cache" / "drumspike" / "onset-guard"
EARLY = Path.home() / ".cache" / "drumspike" / "onset-first" / "real" / "detected.json"
KIT_BEFORE = Path.home() / ".cache" / "drumspike" / "onset-first" / "kit-before"
KIT_AFTER = Path.home() / ".cache" / "drumspike" / "onset-first" / "kit-after"
SAMPLES = oe.SAMPLES
SESSIONS = oe.SESSIONS
LOCK = Path.home() / ".cache" / "drumspike" / "measure.lock"
MAX_LOAD = 8.0
MOVE_MS = 1.0            # the labelled set: hits version 3 moved earlier by more than this

# ---------------------------------------------------------------- the version 3 walk (refine.rs first_motion)

FM_PEAK_S = 0.005
FM_PEAK_DB = 33.0
FM_FLOOR_S = 0.050
FM_FLOOR_SKIP_S = 0.002
FM_BLOCK_S = 0.001
FM_FLOOR_DB = 6.0
FM_GAP_S = 0.0005
FM_SPAN_S = 0.010
FM_LONG_S = 0.005
FM_LONG_END_S = 0.0005
FM_LONG_DB = 6.0
LEAD_IN_MAX_S = 0.0015


def _n(s, sr):
    return int(round(s * sr))


def floor_before(x, s, frm, sr):
    """`refine.rs` `floor_before`: the median of the 1 ms block maxima of |x| in [frm, s - 2 ms), counted back."""
    end = min(s - _n(FM_FLOOR_SKIP_S, sr), len(x))
    b = max(1, _n(FM_BLOCK_S, sr))
    frm = max(0, frm)
    if end - frm < b:
        return 0.0
    nb = (end - frm) // b
    seg = np.abs(x[end - nb * b:end]).reshape(nb, b).max(axis=1)
    return float(np.median(seg))


def two_tap(x):
    """|x[i] + x[i-1]| / 2 for every i (x[-1] = 0)."""
    x = np.asarray(x, np.float64)
    p = np.concatenate([[0.0], x[:-1]])
    return np.abs(x + p) / 2.0


def _lead_in(tt, off, foot, lo, fl, thr, gap, sr):
    """`refine.rs` `lead_in` (#239): from the walk's foot on back over the samples at or above the floor + 6 dB (the
    same gap rule), at most `LEAD_IN_MAX_S` and never past `lo`; the first sample of that rise, else `foot`."""
    low = fl * 10 ** (FM_FLOOR_DB / 20)
    if low <= 0.0 or low >= thr:
        return foot
    lo = max(lo, foot - _n(LEAD_IN_MAX_S, sr))
    lead, i = foot, foot
    while i > lo:
        i -= 1
        if tt[i - off] >= low:
            lead = i
        elif lead - i >= gap:
            return lead
    return foot


def walk(x, s, sr, prev_mid=None, next_mid=None, stop=None, run=1, tt=None, lead_in=False):
    """The version 3 back-track from sample `s` (`first_motion`), on `x`; `prev_mid`/`next_mid` are the midpoints to
    the neighbouring hits (samples). With `lead_in` (the app's rule since #239) the foot goes on back over a rise over
    the floor, at most 1.5 ms. With `stop` = (offset, bool array from that sample on): a run of `run` or more stopped samples ends
    the walk (candidate A); a stopped sample never becomes the foot. Returns (foot, step) with step one of moved,
    kept, busy, separate, stopped."""
    n = len(x)
    if s <= 0 or s >= n:
        return s, "kept"
    pe = min(s + _n(FM_PEAK_S, sr), n)
    if next_mid is not None:
        pe = min(pe, next_mid)
    pk = float(np.abs(x[s:pe]).max()) if pe > s else 0.0
    if pk < 1e-4:
        return s, "kept"
    lo = max(0, s - _n(FM_SPAN_S, sr))
    frm = s - _n(FM_FLOOR_S, sr)
    if prev_mid is not None:
        lo, frm = max(lo, prev_mid), max(frm, prev_mid)
    fl = floor_before(x, s, frm, sr)
    thr = max(pk * 10 ** (-FM_PEAK_DB / 20), fl * 10 ** (FM_FLOOR_DB / 20))
    gap = max(1, _n(FM_GAP_S, sr))
    tt = two_tap(x[max(0, lo - 1):s + 1]) if tt is None else tt
    off = max(0, lo - 1)
    i, foot, ended, how, sr_run = s, s, False, "moved", 0
    s_off, s_arr = stop if stop is not None else (0, np.zeros(0, bool))
    while i > lo:
        i -= 1
        if 0 <= i - s_off < len(s_arr) and s_arr[i - s_off]:
            sr_run += 1
            if sr_run >= run:
                ended, how = True, "stopped"
                break
            if foot - i >= gap:
                ended = True
                break
            continue
        sr_run = 0
        if tt[i - off] >= thr:
            foot = i
        elif foot - i >= gap:
            ended = True
            break
    if not ended:
        return s, "busy"
    if s - foot > _n(FM_LONG_S, sr):
        e = max(1, _n(FM_LONG_END_S, sr))
        held = float(np.abs(x[foot:max(s - e, foot + 1)]).max())
        at = float(np.abs(x[s:s + e]).max())
        if held < at * 10 ** (-FM_LONG_DB / 20):
            return s, "separate"
    if lead_in and how != "stopped":
        foot = _lead_in(tt, off, foot, lo, fl, thr, gap, sr)
    if foot < s:
        return foot, how if how == "stopped" else "moved"
    return s, "kept" if how != "stopped" else "stopped"


def mids(times2, i, sr):
    """The midpoints to the neighbouring hits (samples), from the version 2 times (the coarse times are not kept)."""
    t = times2[i]
    pm = int((times2[i - 1] + t) / 2 * sr) if i > 0 else None
    nm = int((t + times2[i + 1]) / 2 * sr) if i + 1 < len(times2) else None
    return pm, nm


# ---------------------------------------------------------------- bands and envelopes

BANDS = {"kick": [("lp", 150.0)], "tom": [("hp", 50.0), ("lp", 400.0)], "snare": [("hp", 150.0), ("lp", 2500.0)],
         "high": [("hp", 5000.0)], "broad": []}


def rbj(kind, f, sr):
    """RBJ cookbook lowpass or highpass at `f`, Q = 1/sqrt(2) (`attribution::Biquad::new`): (b, a)."""
    w = 2 * np.pi * f / sr
    cw, sw = np.cos(w), np.sin(w)
    alpha = sw / (2 * np.sqrt(0.5))
    b = [(1 - cw) / 2, 1 - cw, (1 - cw) / 2] if kind == "lp" else [(1 + cw) / 2, -(1 + cw), (1 + cw) / 2]
    a0 = 1 + alpha
    return np.array(b) / a0, np.array([1.0, -2 * cw / a0, (1 - alpha) / a0])


def zero_phase(x, sr, stages):
    """`x` through the biquad `stages` [(kind, f)], each twice, forward and backward: the power response |H|^4 per
    stage applied in the frequency domain (zero phase; 0.25 s of zeros pad the edges)."""
    x = np.asarray(x, np.float64)
    if not stages:
        return x
    pad = int(0.25 * sr)
    nf = 1 << int(np.ceil(np.log2(len(x) + 2 * pad)))
    f = np.fft.rfftfreq(nf, 1.0 / sr)
    z = np.exp(-2j * np.pi * f / sr)
    g = np.ones(len(f))
    for kind, fc in stages:
        b, a = rbj(kind, fc, sr)
        h = (b[0] + b[1] * z + b[2] * z ** 2) / (a[0] + a[1] * z + a[2] * z ** 2)
        g *= np.abs(h) ** 4
    buf = np.zeros(nf)
    buf[pad:pad + len(x)] = x
    y = np.fft.irfft(np.fft.rfft(buf) * g, nf)
    return y[pad:pad + len(x)]


def band(x, sr, name):
    """`x` through the P2-6 band `name` (each stage twice), zero phase."""
    return zero_phase(x, sr, BANDS[name])


def lowpass(x, sr, f):
    """A zero-phase low-pass at `f` Hz (RBJ, twice, forward and backward)."""
    return zero_phase(x, sr, [("lp", f)])


def window_max(a, before, after):
    """Exact max of a[i - before .. i + after] (inclusive) for every i, zero outside."""
    a = np.asarray(a, np.float64)
    n = len(a)
    pad = np.concatenate([np.zeros(before), a, np.zeros(after)])
    from numpy.lib.stride_tricks import sliding_window_view
    return sliding_window_view(pad, before + after + 1).max(axis=1)[:n]


# ---------------------------------------------------------------- transfer gains (P2-6 calibration)

def strong_peaks(env, sr, min_gap_s=0.050, db=15.0):
    """Peaks of an envelope (one per `min_gap_s`, the largest) within `db` of its 99.5th percentile: B's strong hits."""
    b = max(1, int(min_gap_s * sr))
    nb = len(env) // b
    if nb == 0:
        return np.zeros(0, int)
    blk = env[:nb * b].reshape(nb, b)
    arg = blk.argmax(axis=1) + np.arange(nb) * b
    val = blk.max(axis=1)
    ref = np.percentile(val, 99.5)
    keep = val >= ref * 10 ** (-db / 20)
    return arg[keep]


def calibrate_gain(env_a, env_b, peaks, own_times, sr, pct=10.0, min_hits=5, quiet_s=0.030, win=(-0.0005, 0.008)):
    """G(B->A) in dB: the `pct` percentile of A's band peak minus B's band peak at B's strong hits (`peaks`, samples),
    over those where A has no own hit within `quiet_s` when there are `min_hits` of them, else over all; None with
    fewer than `min_hits` (P2-6, `attribution::calibrate_gain`)."""
    if len(peaks) < min_hits:
        return None
    own = np.sort(np.asarray(own_times, float))
    vals, quiet = [], []
    for p in peaks:
        a, b = max(0, p + int(win[0] * sr)), min(len(env_a), p + int(win[1] * sr))
        if b <= a:
            continue
        la = 20 * np.log10(env_a[a:b].max() + 1e-9)
        lb = 20 * np.log10(env_b[a:b].max() + 1e-9)
        if lb < -100:
            continue
        vals.append(la - lb)
        t = p / sr
        k = np.searchsorted(own, t)
        near = any(0 <= j < len(own) and abs(own[j] - t) <= quiet_s for j in (k - 1, k))
        quiet.append(not near)
    vals = np.asarray(vals)
    q = np.asarray(quiet, bool)
    use = vals[q] if q.sum() >= min_hits else vals
    return float(np.percentile(use, pct)) if len(use) >= min_hits else None


def explained(own_env, sources, sr, margin_db, lead_s=0.002, lag_s=0.00025):
    """Bool array: own_env[i] <= max over sources of (B's envelope max over [i - lead, i + lag]) * 10^((G + margin)/20).
    `sources` is [(env_b, gain_db)]."""
    pred = np.zeros(len(own_env))
    for env_b, g in sources:
        if g is None:
            continue
        pred = np.maximum(pred, window_max(env_b, int(lead_s * sr), int(lag_s * sr)) * 10 ** ((g + margin_db) / 20))
    return own_env <= pred


def envelope(y, sr, half_s=0.000125):
    """max |y| over i +- half_s."""
    h = max(0, int(round(half_s * sr)))
    return window_max(np.abs(y), h, h)


# ---------------------------------------------------------------- the real sessions

def wait_for_room(log=print):
    while LOCK.exists() or os.getloadavg()[0] >= MAX_LOAD:
        log(f"waiting: lock {LOCK.exists()}, load {os.getloadavg()[0]:.1f}")
        time.sleep(30)


def do_events(out, detected=EARLY, log=print):
    """Refinement versions 2 and 3 (`core-analyze`) on the model's cached coarse times of the close mics."""
    rows = [r for r in json.loads(Path(detected).read_text()) if r["session"] in SESSIONS and r["detector"] == "model"]
    if not isinstance(rows, list):
        raise SystemExit("detected.json is not a list")
    res = []
    for r in rows:
        jobs = [{"wav": r["path"], "role": r["role"], "params": {}, "model": None, "detected": r["detected"], "rule": k}
                for k in (2, 3)]
        a2, a3 = oe.core_analyze(jobs)
        t2, t3 = [e["t"] for e in a2["events"]], [e["t"] for e in a3["events"]]
        if len(t2) != len(t3):
            raise SystemExit(f"{r['name']}: versions 2 and 3 differ in count")
        res.append({"session": r["session"], "name": r["name"], "path": r["path"], "role": r["role"], "t2": t2, "t3": t3})
        log(f"{r['session']} / {r['name']}: {len(t2)} hits, {sum(a - b > MOVE_MS / 1000 for a, b in zip(t2, t3))} moved")
    Path(out).mkdir(parents=True, exist_ok=True)
    (Path(out) / "events.json").write_text(json.dumps(res))
    return res


# ---------------------------------------------------------------- the labelled set

# The P4-12 verifier's hand check (model events, hit index in the track's version 3 events, 0-based: every index lies
# in the set of hits moved by more than 1 ms). Bleed hits carry the own start in ms after the version 3 time: my
# reading of the log envelopes (broadband, 2 kHz low-pass, P2-6 bands of the other mics), the start of the own drum's
# main rise, +-0.15 ms. CNGE "t4 #25" of the verifier's note is no moved hit (t4 #25 moves under 1 ms); t2 #25 is the
# nearest candidate, but its lead is snare bleed and its own start is unclear: it stays out.
HAND = {
    ("Session B", "s top_01.wav"): {
        "own_quiet": [56, 226], "unclear": [78, 82, 149],
        "bleed": {79: 1.40, 87: 2.60, 98: 1.30, 119: 0.85, 140: 2.60, 242: 3.35}},
    ("Session C", "s top_02.wav"): {"unclear": [113, 179], "bleed": {104: 1.10, 266: 2.60}},
    ("Session A", "s top_02.wav"): {"own_quiet": [23, 72], "bleed": {135: 2.70, 214: 0.80}},
    ("Session B", "t3_01.wav"): {"bleed": {73: 4.05}},
    ("Session B", "t4 Ri_01.wav"): {"bleed": {9: 3.10}},
    ("Session A", "t3_02.wav"): {"bleed": {28: 3.45}},
    ("Session A", "t2_02.wav"): {"bleed": {43: 6.55}},
    ("Session A", "k in_02.wav"): {"bleed": {301: 2.40}},
}
HAND_LOUD = ("s top",)   # on a hand-checked snare track the other moved hits start with their own loud attack

# The other mics of each session: (file, drum, band). Two mics of one drum never explain each other.
MICS = {
    "Session B": [("k in_01.wav", "kick", "kick"), ("s top_01.wav", "snare", "snare"),
                                    ("t1 HH_01.wav", "t1", "tom"), ("t2_01.wav", "t2", "tom"), ("t3_01.wav", "t3", "tom"),
                                    ("t4 Ri_01.wav", "t4", "tom"), ("hh_01.wav", "hat", "high"),
                                    ("OH_01.wav", "oh", "high"), ("ride_01.wav", "ride", "high")],
    "Session C": [("k in_02.wav", "kick", "kick"), ("s top_02.wav", "snare", "snare"),
                                ("t 1_03.wav", "t1", "tom"), ("t 2_03.wav", "t2", "tom"), ("t 3_03.wav", "t3", "tom"),
                                ("t 4_03.wav", "t4", "tom"), ("hh_03.wav", "hat", "high"), ("oh_03.L.wav", "oh", "high"),
                                ("oh_03.R.wav", "oh", "high"), ("ri_03.wav", "ride", "high")],
    "Session A": [("k in_02.wav", "kick", "kick"), ("s top_02.wav", "snare", "snare"),
                         ("t1 hh_02.wav", "t1", "tom"), ("t2_02.wav", "t2", "tom"), ("t3_02.wav", "t3", "tom"),
                         ("t4 ri_02.wav", "t4", "tom"), ("hh_03.wav", "hat", "high"), ("oh_01.wav", "oh", "high")],
}

RULE_RISE_DB = 10.0      # extension rule: another mic's band envelope rises this much ...
RULE_BEFORE_S = 0.0015   # ... in [t3 - this, t3 + RULE_AFTER_S]
RULE_AFTER_S = 0.0005
RULE_QUIET_DB = 12.0     # ... and the own lead [t3, t3 + RULE_LEAD_S] lies this far under the own level in [t2, t2 + 5 ms]
RULE_LEAD_S = 0.0005
RULE_OWN_S = 0.002       # own: no other mic rises in [t3 - this, t3 + RULE_AFTER_S]


def drum_of(session, name):
    for f, d, _ in MICS[session]:
        if f == name:
            return d
    return None


def rises(env, at, before, after, sr, rise_db):
    """True when `env` rises by `rise_db` inside [at - before, at + after]: its max there over its min before the max."""
    a, b = max(0, at - int(before * sr)), min(len(env), at + int(after * sr) + 1)
    if b - a < 2:
        return False
    w = env[a:b]
    k = int(np.argmax(w))
    lo = w[:k + 1].min()
    return 20 * np.log10(w[k] + 1e-9) - 20 * np.log10(lo + 1e-9) >= rise_db


def rule_label(x_env, srcs, s2, f3, sr):
    """The extension rule for a moved hit outside the hand check: `bleed` when another mic's band envelope rises by
    RULE_RISE_DB in [f3 - 1.5 ms, f3 + 0.5 ms] and the own lead [f3, f3 + 0.5 ms] is RULE_QUIET_DB under the own
    level in [s2, s2 + 5 ms]; `own` when no other mic rises in [f3 - 2 ms, f3 + 0.5 ms]; else `unclear`."""
    lead_end = max(f3 + 1, min(s2, f3 + int(RULE_LEAD_S * sr)))
    lead = 20 * np.log10(x_env[f3:lead_end].max() + 1e-9)
    body = 20 * np.log10(x_env[s2:s2 + int(0.005 * sr)].max() + 1e-9)
    near = [rises(e, f3, RULE_BEFORE_S, RULE_AFTER_S, sr, RULE_RISE_DB) for e, _ in srcs]
    wide = [rises(e, f3, RULE_OWN_S, RULE_AFTER_S, sr, RULE_RISE_DB) for e, _ in srcs]
    if any(near) and body - lead >= RULE_QUIET_DB:
        return "bleed"
    if not any(wide):
        return "own"
    return "unclear"


# ---------------------------------------------------------------- the candidates on a track

class Track:
    """One close mic of a session with its version 2 and 3 times, its envelope and the other mics' band envelopes
    and transfer gains G(B->A) (computed on first use)."""

    def __init__(self, session, name, role, t2, t3, x=None, sr=None, sources=None):
        self.session, self.name, self.role, self.t2, self.t3 = session, name, role, t2, t3
        if x is None:
            x, sr = oe.read_mono(SAMPLES / session / "Audio Files" / name)
        self.x, self.sr = np.asarray(x, np.float64), sr
        self.env = envelope(self.x, sr)
        self._lp = {}
        self._src = sources
        self._gain = {}

    def sources(self):
        """[(B's band envelope, its file)] of the other mics of other drums."""
        if self._src is None:
            own = drum_of(self.session, self.name)
            self._src = []
            for f, d, bd in MICS[self.session]:
                if d == own or f == self.name:
                    continue
                y, sr = oe.read_mono(SAMPLES / self.session / "Audio Files" / f)
                if sr != self.sr or len(y) != len(self.x):
                    y = np.resize(y, len(self.x)) if sr == self.sr else None
                if y is None:
                    continue
                self._src.append((envelope(band(y, sr, bd), sr), f))
        return self._src

    def gains(self, pct):
        if pct not in self._gain:
            g = []
            for env_b, f in self.sources():
                g.append(calibrate_gain(self.env, env_b, strong_peaks(env_b, self.sr), self.t3, self.sr, pct=pct))
            self._gain[pct] = g
        return self._gain[pct]

    def lp(self, fc):
        if fc not in self._lp:
            self._lp[fc] = lowpass(self.x, self.sr, fc)
        return self._lp[fc]

    def samples(self, i):
        return int(round(self.t2[i] * self.sr)), int(round(self.t3[i] * self.sr))

    def cand_a(self, i, margin_db=6.0, pct=10.0, run_s=0.00025, lead_s=0.002, lag_s=0.00025):
        """Candidate A for hit i: (foot, step)."""
        s2, f3 = self.samples(i)
        if f3 >= s2:
            return f3, "kept"
        sr = self.sr
        pm, nm = mids(self.t2, i, sr)
        a = max(0, s2 - int(FM_SPAN_S * sr) - 2)
        b = s2 + 1
        pa, pb = max(0, a - int(lead_s * sr)), min(len(self.x), b + int(lag_s * sr))
        srcs = [(e[pa:pb], g) for (e, _), g in zip(self.sources(), self.gains(pct))]
        ex = explained(self.env[pa:pb], srcs, sr, margin_db, lead_s, lag_s)[a - pa:b - pa]
        foot, step = walk(self.x, s2, sr, pm, nm, stop=(a, ex), run=max(1, int(round(run_s * sr))))
        return min(s2, max(f3, foot)), step

    def cand_b(self, i, fc):
        """Candidate B for hit i: the walk on the low-pass at `fc`, clipped to [version 3, version 2]."""
        s2, f3 = self.samples(i)
        if f3 >= s2:
            return f3, "kept"
        pm, nm = mids(self.t2, i, self.sr)
        foot, step = walk(self.lp(fc), s2, self.sr, pm, nm)
        return min(s2, max(f3, foot)), step


LP = {"snare": 2000.0, "tom": 2000.0, "kick": 2000.0}
A_PARAMS = {"margin_db": 3.0, "pct": 10.0, "lead_s": 0.001, "run_s": 0.00025}   # the best of the hand-set grid
A_ALT = {"margin_db": 6.0, "pct": 10.0, "lead_s": 0.0005, "run_s": 0.0005}      # the other one of the grid's best
B_FC = 2000.0
OWN_TOL_MS = 0.3         # done criterion: a bleed hit within this of the own start
STILL_MS = 0.05          # an own hit "does not move": the candidate stays within this of version 3


def candidates(tr, i):
    """{candidate: ms after version 3} for hit i of a track."""
    s2, f3 = tr.samples(i)
    out = {}
    for name, p in (("A", A_PARAMS), ("A'", A_ALT)):
        out[name] = (tr.cand_a(i, **p)[0] - f3) * 1000 / tr.sr
    out["B"] = (tr.cand_b(i, B_FC)[0] - f3) * 1000 / tr.sr
    return out


def score(rows, cands=("A", "A'", "B")):
    """Per candidate: bleed hits within OWN_TOL_MS of the own start, own hits moved by more than STILL_MS, unclear
    hits moved, and for the rule labels the same counts (no own start: bleed counts a move later by > STILL_MS)."""
    res = {}
    for c in ("v2",) + tuple(cands):
        val = (lambda r: r["v2_ms"]) if c == "v2" else (lambda r, c=c: r[c])
        d = {}
        for src in ("hand", "rule"):
            sel = [r for r in rows if r["src"] == src]
            bl = [r for r in sel if r["label"] == "bleed"]
            d[src] = {
                "bleed": len(bl),
                "bleed_on_own": sum(abs(val(r) - r["own_ms"]) <= OWN_TOL_MS for r in bl if r["own_ms"] is not None),
                "bleed_moved": sum(val(r) > STILL_MS for r in bl),
                "own": sum(r["label"].startswith("own") for r in sel),
                "own_moved": sum(val(r) > STILL_MS for r in sel if r["label"].startswith("own")),
                "own_quiet_moved": sum(val(r) > STILL_MS for r in sel if r["label"] == "own_quiet"),
                "unclear": sum(r["label"] == "unclear" for r in sel),
                "unclear_moved": sum(val(r) > STILL_MS for r in sel if r["label"] == "unclear"),
            }
        res[c] = d
    return res


def do_measure(out, log=print):
    """The candidates on every moved hit (> MOVE_MS) of the three songs: hand labels where the verifier looked, the
    extension rule elsewhere."""
    ev = json.loads((Path(out) / "events.json").read_text())
    if not isinstance(ev, list):
        raise SystemExit("events.json is not a list")
    rows = []
    for r in ev:
        moved = [i for i in range(len(r["t2"])) if r["t2"][i] - r["t3"][i] > MOVE_MS / 1000]
        if not moved:
            continue
        wait_for_room(log)
        tr = Track(r["session"], r["name"], r["role"], r["t2"], r["t3"])
        hand = HAND.get((r["session"], r["name"]))
        for i in moved:
            s2, f3 = tr.samples(i)
            if hand is not None:
                src = "hand"
                if i in hand.get("bleed", {}):
                    lab, own = "bleed", hand["bleed"][i]
                elif i in hand.get("own_quiet", []):
                    lab, own = "own_quiet", 0.0
                elif i in hand.get("unclear", []):
                    lab, own = "unclear", None
                elif r["name"].startswith(HAND_LOUD):
                    lab, own = "own_loud", 0.0
                else:
                    src = None
            else:
                src = None
            if src is None:
                # kick: no rule (the verifier found one kick bleed hit; version 3 moves most kick hits about 2 ms by
                # design, onto the foot of the first half cycle): the kick hits only count the candidates' moves
                lab = "unclear" if r["role"] == "kick" else rule_label(tr.env, tr.sources(), s2, f3, tr.sr)
                src, own = "rule", None
                own = 0.0 if lab == "own" else None
            row = {"session": r["session"], "name": r["name"], "role": r["role"], "i": i, "t3": r["t3"][i],
                   "src": src, "label": lab, "own_ms": own, "v2_ms": (s2 - f3) * 1000 / tr.sr}
            row.update(candidates(tr, i))
            rows.append(row)
        log(f"{r['session']} / {r['name']}: {len(moved)} moved hits")
    res = {"rows": rows, "score": score(rows),
           "by_role": {role: score([x for x in rows if x["role"] == role]) for role in ("kick", "snare", "tom")},
           "gains": {}}
    (Path(out) / "real.json").write_text(json.dumps(res, indent=1))
    return res


# ---------------------------------------------------------------- the P4-11 kit

KIT_BAND = {"kick": "kick", "snare": "snare", "t1": "tom", "t2": "tom", "t3": "tom", "t4": "tom"}


def kit_files(before=KIT_BEFORE, after=KIT_AFTER):
    """[(kind, cond, shell, sr, detector, before json, after json)] of the kit runs with both versions."""
    out = []
    for jf in sorted((Path(after) / "res").glob("*/*/*__*.json")):
        kind, cond = jf.parent.parent.name, jf.parent.name
        stem, det = jf.stem.rsplit("__", 1)
        shell, sr = stem.split("_")[0], int(stem.split("_")[1])
        bf = Path(before) / "res" / kind / cond / jf.name
        if bf.is_file():
            out.append((kind, cond, shell, sr, det, bf, jf))
    return out


def kit_track(kind, cond, shell, sr, t2, t3, after=KIT_AFTER):
    """The kit file as a Track; its other mic is the clean mix of the shell that bleeds into it (calib_kit
    `BLEED_FROM`), in that shell's band."""
    import calib_kit as ck
    x, fsr = sf.read(str(Path(after) / "audio" / kind / cond / f"{shell}_{sr}.wav"), dtype="float32", always_2d=True)
    x = x.mean(axis=1)
    other = ck.BLEED_FROM[shell]
    y, _ = sf.read(str(Path(after) / "audio" / kind / "clean" / f"{other}_{sr}.wav"), dtype="float32", always_2d=True)
    y = np.resize(y.mean(axis=1), len(x)) if len(y) != len(x) else y.mean(axis=1)
    src = [(envelope(band(y, fsr, KIT_BAND[other]), fsr), f"{other} clean")]
    return Track(f"kit {kind} {cond}", f"{shell}_{sr}", ck.ROLE[shell], t2, t3, x=x, sr=fsr, sources=src)


def do_kit(out, before=KIT_BEFORE, after=KIT_AFTER, log=print):
    """Candidates A and B on the kit's version 3 events (version 2 from the run with it pinned): the error against
    the first motion (synthetic `abs`, library slices `aud`) per hit, version 3 and each candidate."""
    import calib_kit as ck
    rows = []
    for kind, cond, shell, sr, det, bf, af in kit_files(before, after):
        e2 = sorted(e["t"] for e in json.loads(bf.read_text())["events"]["events"])
        e3 = sorted(e["t"] for e in json.loads(af.read_text())["events"]["events"])
        if len(e2) != len(e3):
            log(f"{af}: versions 2 and 3 differ in count, skipped")
            continue
        tr = kit_track(kind, cond, shell, sr, e2, e3, after)
        times = {"v3": e3, "v2": e2}
        for c in ("A", "A'", "B"):
            times[c] = []
        for i in range(len(e3)):
            s2, f3 = tr.samples(i)
            for c, p in (("A", A_PARAMS), ("A'", A_ALT)):
                times[c].append(tr.cand_a(i, **p)[0] / tr.sr if f3 < s2 else e3[i])
            times["B"].append(tr.cand_b(i, B_FC)[0] / tr.sr if f3 < s2 else e3[i])
        refs = ck.read_csv(Path(after) / "ref" / kind / cond / f"{shell}_{sr}.csv")
        key = "err_aud" if kind == "acoustic" else "err_abs"
        per = {c: ck.compare_pair(refs, sorted(times[c]), sr)[0] for c in times}
        for k, ref in enumerate(refs):
            if not per["v3"][k].get("found"):
                continue
            row = {"kind": kind, "cond": cond, "shell": shell, "sr": sr, "detector": det,
                   "rise_ms": ref.get("rise_ms"), "group": ref.get("group")}
            for c in times:
                row[c] = per[c][k].get(key) if per[c][k].get("found") else None
            rows.append(row)
        log(f"kit {kind}/{cond}/{shell}_{sr} [{det}]: {len(e3)} events")
    res = {"rows": rows, "summary": kit_summary(rows),
           "pairs": kit_summary([dict(r, group="main") for r in rows if r.get("group") != "main"])}
    (Path(out) / "kit.json").write_text(json.dumps(res, indent=1))
    return res


def _stat(v):
    v = np.asarray([x for x in v if x is not None], float)
    if not len(v):
        return {"n": 0}
    return {"n": int(len(v)), "median": float(np.median(v)), "p5": float(np.percentile(v, 5)),
            "p95": float(np.percentile(v, 95))}


def kit_summary(rows, cands=("A", "A'", "B")):
    """Per kind, condition and candidate: hits whose error changed by more than 1 sample against version 3 (later and
    earlier), the median and p95; per acoustic shell the median and p95; sharp synthetic attacks (rise 0) outside
    +-1 sample."""
    out = {"sets": [], "acoustic": [], "sharp": {}}
    rows = [r for r in rows if r.get("group") == "main"]   # the main hits (P4-12's tables); the close pairs apart
    for kind in sorted({r["kind"] for r in rows}):
        for cond in ("clean", "noise", "bleed"):
            sel = [r for r in rows if r["kind"] == kind and r["cond"] == cond]
            if not sel:
                continue
            row = {"kind": kind, "cond": cond, "n": len(sel), "v3": _stat(r["v3"] for r in sel)}
            for c in cands:
                both = [r for r in sel if r[c] is not None and r["v3"] is not None]
                row[c] = {**_stat(r[c] for r in sel),
                          "later": sum(r[c] - r["v3"] > 1 for r in both), "earlier": sum(r[c] - r["v3"] < -1 for r in both),
                          "closer": sum(abs(r[c]) < abs(r["v3"]) - 1 for r in both),
                          "away": sum(abs(r[c]) > abs(r["v3"]) + 1 for r in both)}
            out["sets"].append(row)
    for shell in ("kick", "snare", "t1", "t2", "t3", "t4"):
        for cond in ("clean", "noise", "bleed"):
            sel = [r for r in rows if r["kind"] == "acoustic" and r["shell"] == shell and r["cond"] == cond]
            if sel:
                out["acoustic"].append({"shell": shell, "cond": cond, "v3": _stat(r["v3"] for r in sel),
                                        **{c: _stat(r[c] for r in sel) for c in cands}})
    sharp = [r for r in rows if r["kind"] == "syn" and r["rise_ms"] == 0.0]
    for c in ("v3",) + tuple(cands):
        both = [r for r in sharp if r[c] is not None and r["v3"] is not None]
        out["sharp"][c] = {"n": len(sharp), "outside": sum(r[c] is None or abs(r[c]) > 1 for r in sharp),
                           "clean_outside": sum(r[c] is None or abs(r[c]) > 1 for r in sharp if r["cond"] == "clean"),
                           "moved": sum(abs(r[c] - r["v3"]) > 1 for r in both),
                           "away": sum(abs(r[c]) > abs(r["v3"]) + 1 for r in both)}
    return out


def report(real, kit):
    """The markdown tables of `real.json` and `kit.json`."""
    L = []
    if real:
        L += ["| candidate | hand: bleed on own start (+-0.3 ms) | hand: own moved (quiet) | hand: unclear moved |"
              " rule: bleed moved later | rule: own moved |", "|---|---|---|---|---|---|"]
        for c, d in real["score"].items():
            h, r = d["hand"], d["rule"]
            L.append(f"| {c} | {h['bleed_on_own']} of {h['bleed']} | {h['own_moved']} of {h['own']} "
                     f"({h['own_quiet_moved']} of 4) | {h['unclear_moved']} of {h['unclear']} | "
                     f"{r['bleed_moved']} of {r['bleed']} | {r['own_moved']} of {r['own']} |")
        L.append("")
        L += ["| role | candidate | rule: bleed moved | rule: own moved | rule: unclear moved |", "|---|---|---|---|---|"]
        for role, sc in real["by_role"].items():
            for c, d in sc.items():
                r = d["rule"]
                L.append(f"| {role} | {c} | {r['bleed_moved']} of {r['bleed']} | {r['own_moved']} of {r['own']} | "
                         f"{r['unclear_moved']} of {r['unclear']} |")
        L += ["", "| hit | label | own ms | v2 | A | A' | B |", "|---|---|---|---|---|---|---|"]
        for r in real["rows"]:
            if r["src"] == "hand" and r["label"] != "own_loud" or r["src"] == "hand" and max(r["A"], r["B"]) > STILL_MS:
                L.append(f"| {r['session'].split(' - ')[0]} {r['name']} #{r['i']} | {r['label']} | {r['own_ms']} | "
                         f"{r['v2_ms']:.2f} | {r['A']:.2f} | {r[chr(65) + chr(39)]:.2f} | {r['B']:.2f} |")
    if kit:
        s = kit_summary(kit["rows"])
        f = lambda st: f"{st['median']:+.1f} / {st['p95']:+.1f}" if st.get("n") else "-"
        L += ["", "| set | cond | n | v3 median / p95 | A median / p95 (later, earlier, closer, away) | A' | B |",
              "|---|---|---|---|---|---|---|"]
        for row in s["sets"]:
            cells = [f"{f(row[c])} ({row[c]['later']}, {row[c]['earlier']}, {row[c]['closer']}, {row[c]['away']})"
                     for c in ("A", "A'", "B")]
            L.append(f"| {row['kind']} | {row['cond']} | {row['n']} | {f(row['v3'])} | " + " | ".join(cells) + " |")
        L += ["", "| acoustic shell | cond | v3 | A | A' | B |", "|---|---|---|---|---|---|"]
        for row in s["acoustic"]:
            L.append(f"| {row['shell']} | {row['cond']} | " + " | ".join(f(row[c]) for c in ("v3", "A", "A'", "B")) + " |")
        L += ["", "| sharp synthetic (rise 0) | outside +-1 sample (clean) | moved > 1 sample from v3 | moved away |",
              "|---|---|---|---|"]
        for c, d in s["sharp"].items():
            L.append(f"| {c} | {d['outside']} of {d['n']} ({d['clean_outside']}) | {d['moved']} | {d['away']} |")
    return "\n".join(L) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["events", "measure", "kit", "all"])
    ap.add_argument("--out", default=str(OUT))
    a = ap.parse_args(argv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    real = kit = None
    if a.cmd in ("events", "all"):
        wait_for_room()
        do_events(out)
    if a.cmd in ("measure", "all"):
        real = do_measure(out)
    if a.cmd in ("kit", "all"):
        wait_for_room()
        kit = do_kit(out)
    if real is None and (out / "real.json").is_file():
        real = json.loads((out / "real.json").read_text())
    if kit is None and (out / "kit.json").is_file():
        kit = json.loads((out / "kit.json").read_text())
    (out / "guard.md").write_text(report(real, kit))
    print(report(real, kit))


if __name__ == "__main__":
    main()
