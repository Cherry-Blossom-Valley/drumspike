"""#402: the audio-spike snippet rule of crates/core/src/spike.rs in Python, for explainer 05
and the guide figure. `low_component` and `snippet_end` mirror the Rust code (AUDIO_LOW_HZ,
AUDIO_MAX_S, the counted-lobe walk): the demo session's real hits, the app's real lengths.
"""
import numpy as np

RATE = 48000  # the demo session's sample rate
AUDIO_LOW_HZ = 800.0  # spike.rs: the lowpass that isolates the body
AUDIO_MIN_S = 0.002
AUDIO_MAX_S = 0.060
AUDIO_DEFAULT_S = 0.010


def _rbj_lowpass(fc, sr):
    """One RBJ lowpass biquad (the coefficients of crates/core Biquad, FilterKind::LowPass,
    Q = 1/sqrt(2) as in attribution.rs)."""
    w0 = 2.0 * np.pi * fc / sr
    cosw0 = np.cos(w0)
    alpha = np.sin(w0) / np.sqrt(2.0)  # Q = 1/sqrt(2): the Butterworth-style stage
    b0 = (1.0 - cosw0) / 2.0
    b1 = 1.0 - cosw0
    b2 = (1.0 - cosw0) / 2.0
    a0 = 1.0 + alpha
    a1 = -2.0 * cosw0
    a2 = 1.0 - alpha
    return (b0 / a0, b1 / a0, b2 / a0), (a1 / a0, a2 / a0)


def _lowpass_once(x, sr):
    """Two cascaded RBJ lowpass stages over all of `x` (float64 state, like lowpass_once)."""
    (b0, b1, b2), (a1, a2) = _rbj_lowpass(AUDIO_LOW_HZ, sr)
    z = [[0.0, 0.0], [0.0, 0.0]]
    out = np.empty(len(x))
    for i, v in enumerate(x):
        y = float(v)
        for zs in z:
            o = b0 * y + zs[0]
            zs[0] = b1 * y - a1 * o + zs[1]
            zs[1] = b2 * y - a2 * o
            y = o
        out[i] = y
    return out


def low_component(x, sr=RATE):
    """The dominant low component: the lowpass twice, forward then backward (zero-phase)."""
    rev = _lowpass_once(np.asarray(x, dtype=np.float64), sr)[::-1]
    return _lowpass_once(rev, sr)[::-1]


def _crossings(smooth, start, scan_to):
    """The sample indices in (start, scan_to] where the smoothed signal changes sign."""
    s = np.signbit(smooth[start:scan_to + 1])
    return (start + 1 + np.nonzero(s[1:] != s[:-1])[0]).tolist()


def _third_significant_lobe_end(smooth, start, scan_to, crossings):
    """The end of counted lobe 3 of the smoothed signal (spike.rs, the same walk)."""
    bounds = crossings + [scan_to]
    peaks, signs = [], []
    lo = start
    for c in bounds:
        seg = np.abs(smooth[lo:c])
        j = int(np.argmax(seg)) if len(seg) else 0
        v = float(seg[j]) if len(seg) else 0.0
        mid = smooth[lo + j] if len(seg) else 0.0
        peaks.append(v)
        signs.append(1 if mid > 0 else -1)
        lo = c
    big = max(peaks[:3])
    if big <= 0.0:
        return None
    threshold = 0.25 * big
    count, last_sign, end, last_sig_end = 0, 0, None, None
    for j, p in enumerate(peaks):
        if p < threshold:
            continue  # an insignificant lobe never extends anything
        boundary = bounds[j]
        last_sig_end = boundary
        if count > 0 and signs[j] == last_sign:
            if count == 3:
                end = boundary
        else:
            count += 1
            last_sign = signs[j]
            if count == 3:
                end = boundary
    return end if count >= 3 else last_sig_end


def counted_lobes(x, smooth, start, sr=RATE):
    """The counted lobes of the snippet walk at `start`, for the explainer's pictures:
    (counted, small) where counted is [(lo, hi)] per counted lobe in order (the same walk as
    spike.rs third_significant_lobe_end) and small is the (lo, hi, peak) of the first
    insignificant lobe in the scan range or None."""
    min_len = max(round(AUDIO_MIN_S * sr), 2)
    max_len = max(round(AUDIO_MAX_S * sr), min_len)
    scan_to = min(len(x), start + max_len + 1)
    crossings = _crossings(smooth, start + 1, scan_to)
    bounds = crossings + [scan_to]
    peaks, signs = [], []
    lo = start
    for c in bounds:
        seg = np.abs(smooth[lo:c])
        j = int(np.argmax(seg)) if len(seg) else 0
        v = float(seg[j]) if len(seg) else 0.0
        mid = smooth[lo + j] if len(seg) else 0.0
        peaks.append(v)
        signs.append(1 if mid > 0 else -1)
        lo = c
    big = max(peaks[:3])
    if big <= 0.0:
        return [], None
    threshold = 0.25 * big
    counted, counted_spans, small = 0, [], None
    last_sign = 0
    for j, p in enumerate(peaks):
        lo_b = start if j == 0 else bounds[j - 1]
        hi_b = bounds[j]
        if p < threshold:
            if small is None:
                small = (lo_b, hi_b, p)
            continue
        if counted > 0 and signs[j] == last_sign:
            if counted_spans:
                counted_spans[-1] = (counted_spans[-1][0], hi_b)  # extends
        else:
            counted += 1
            last_sign = signs[j]
            if counted <= 3:
                counted_spans.append((lo_b, hi_b))
    return counted_spans[:3], small


def snippet_end(x, smooth, start, sr=RATE):
    """The (end, found) pair of spike.rs snippet_end: the counted-lobe end snapped to the
    nearest raw crossing within +-1 ms, else the 10 ms default."""
    min_len = max(round(AUDIO_MIN_S * sr), 2)
    max_len = max(round(AUDIO_MAX_S * sr), min_len)
    scan_to = min(len(x), start + max_len + 1)
    end, found = None, False
    if len(smooth) > start:
        crossings = _crossings(smooth, start + 1, scan_to)
        target = (_third_significant_lobe_end(smooth, start, scan_to, crossings)
                  if len(crossings) >= 3 else None)
        if target is not None:
            found = True
            tol = max(round(0.001 * sr), 2)
            best, best_d = target, tol + 1
            s = np.signbit(x[start:scan_to + 1])
            for j in np.nonzero(s[1:] != s[:-1])[0]:
                d = abs(int(j) + start + 1 - target)
                if d < best_d:
                    best_d, best = d, int(j) + start + 1
            end = best if best_d <= tol else target
        else:
            end = start + round(AUDIO_DEFAULT_S * sr)
    else:
        end = start + round(AUDIO_DEFAULT_S * sr)
    end = max(end, start + min_len)
    return min(end, start + max_len), found
