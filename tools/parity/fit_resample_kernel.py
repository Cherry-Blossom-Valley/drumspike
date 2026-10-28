"""Where the resampler's kernel parameters come from: measure soxr_hq's kernel, fit the model.

    uv run --with numpy --with scipy --with soxr==1.1.0 \\
        python tools/parity/fit_resample_kernel.py [--starts 3] [--save kernel.npz]

Step 1 measures soxr 1.1.0's impulse response for 48000 -> 44100 (160:147) and 96000 -> 44100
(320:147) at 1/147 input-sample resolution: a unit impulse is placed at every input offset
s in 0..M and the outputs y[k] are kernel samples at t = k*M - (n0+s)*147 (units of 1/147
input samples); the union over s covers every grid point. Step 2 fits the model of
`crates/core/src/detect/resample.rs`, the convolution of two Kaiser-windowed sincs

    A(s)   = 2 fc_A sinc(2 fc_A s) kaiser(s / T_A, beta_A)
    B(tau) = 2 fc_B sinc(2 fc_B tau) kaiser(tau / T_B, beta_B)
    g      = gain * (A (*) B)

to the 48 kHz kernel by least squares (Nelder-Mead from a few starts) on every 7th grid
point (a 1.008 MHz grid, exact for a kernel band limited far below it), and reports the
white-noise SNR of the fit at both rates (10 log10(sum g_soxr^2 / sum (g_fit - g_soxr)^2)),
plus the frequency response of soxr, the fit and their difference. The six numbers in
resample.rs are the result of this script (SNR 133.6 dB at 48 kHz, 132.2 dB at 96 kHz); a
single Kaiser stage reaches 73 dB, which is why the design has two.
"""
import argparse
import sys

import numpy as np
import soxr
from scipy.optimize import minimize
from scipy.signal import fftconvolve
from scipy.special import i0

L = 147


def measure(sr_in, n=6000, n0=3000, window=1500):
    """soxr_hq kernel for sr_in -> 44100 on the 1/147 input-sample grid: (tau_seconds, g)."""
    m = sr_in * L // 44100
    pts = {}
    for s in range(m):
        x = np.zeros(n, dtype=np.float32)
        x[n0 + s] = 1.0
        y = soxr.resample(x, sr_in, 44100, quality="HQ")
        for k, v in enumerate(y):
            pts[k * m - (n0 + s) * L] = float(v)
    ts = np.array([t for t in sorted(pts) if abs(t) < window * L])
    assert np.all(np.diff(ts) == 1), "grid has holes"
    return ts / L / sr_in, np.array([pts[t] for t in ts])


def decimate(tau, g, d=7):
    i0_ = int(np.argmin(np.abs(tau)))
    idx = np.arange(i0_ % d, len(tau), d)
    return tau[idx], g[idx]


def ksinc(tau, fc, beta, t_h):
    w = np.zeros_like(tau)
    m = np.abs(tau) < t_h
    w[m] = i0(beta * np.sqrt(1 - (tau[m] / t_h) ** 2)) / i0(beta)
    return 2 * fc * np.sinc(2 * fc * tau) * w


def model(tau, sr_in, p, two=True):
    rate = 1.0 / (tau[1] - tau[0])
    fc_a, b_a, t_a, fc_b, b_b, t_b, gain = p
    k = ksinc(tau, fc_b, b_b, t_b)
    if two:
        k = fftconvolve(ksinc(tau, fc_a, b_a, t_a), k, mode="same") / rate
    return gain * k / sr_in


def snr(p, tau, g, sr_in, two=True):
    d = model(tau, sr_in, p, two) - g
    return 10 * np.log10(np.sum(g ** 2) / np.sum(d ** 2))


def response(tau, k, freqs):
    rate = 1.0 / (tau[1] - tau[0])
    nf = 1 << 18
    f = np.fft.rfftfreq(nf, d=1 / rate)
    h = np.abs(np.fft.rfft(k, n=nf))
    return np.array([h[np.searchsorted(f, fq)] for fq in freqs]), h[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--starts", type=int, default=3)
    ap.add_argument("--save", help="write the measured kernels and the fit to this .npz")
    args = ap.parse_args()
    print(f"soxr {soxr.__version__}; measuring the 48 kHz and 96 kHz kernels", flush=True)
    tau48, g48 = measure(48000)
    tau96, g96 = measure(96000)
    d48 = decimate(tau48, g48)
    d96 = decimate(tau96, g96)
    nz = np.abs(g48) > 1e-7
    print(f"48 kHz kernel support {tau48[nz][0]*1e3:.4f} .. {tau48[nz][-1]*1e3:.4f} ms, peak at t = {tau48[np.argmax(g48)]*1e6:.1f} us")

    single = [0, 0, 0, 21090.8, 15.75, 0.00248, 1.0]
    r1 = minimize(lambda p: -snr(p, *d48, 48000, two=False), single, method="Nelder-Mead",
                  options={"xatol": 1e-9, "fatol": 1e-7, "maxiter": 2000, "adaptive": True})
    print(f"single Kaiser stage: {-r1.fun:.2f} dB (fc {r1.x[3]:.1f} Hz, beta {r1.x[4]:.3f}, T_h {r1.x[5]*1e3:.4f} ms)", flush=True)

    starts = [[24000.0, 8.0, 0.00008], [24000.0, 12.0, 0.00015], [24000.0, 16.0, 0.0003]][: args.starts]
    best = None
    for fc_a, b_a, t_a in starts:
        p0 = [fc_a, b_a, t_a, 21090.8, 15.75, 0.00248, 1.0]
        r = minimize(lambda p: -snr(p, *d48, 48000), p0, method="Nelder-Mead",
                     options={"xatol": 1e-10, "fatol": 1e-8, "maxiter": 3000, "adaptive": True})
        print(f"  start beta_A={b_a} T_A={t_a*1e3:.2f} ms: {-r.fun:.2f} dB", flush=True)
        if best is None or r.fun < best.fun:
            best = r
    p = best.x
    print("two-stage fit:")
    print(f"  FC_A = {p[0]:.1f} Hz, BETA_A = {p[1]:.4f}, T_A = {p[2]*1e6:.3f} us")
    print(f"  FC_B = {p[3]:.2f} Hz, BETA_B = {p[4]:.4f}, T_B = {p[5]*1e3:.5f} ms, gain = {p[6]:.8f}")
    print(f"  white-noise SNR vs soxr_hq: 48 kHz {-best.fun:.2f} dB, 96 kHz {snr(p, *d96, 96000):.2f} dB")

    freqs = [1000, 10000, 15000, 18000, 19000, 20000, 20500, 21000, 21500, 22050, 23000, 24000]
    hs, h0 = response(d48[0], d48[1], freqs)
    hm, _ = response(d48[0], model(d48[0], 48000, p), freqs)
    hd, _ = response(d48[0], model(d48[0], 48000, p) - d48[1], freqs)
    db = lambda a: 20 * np.log10(np.maximum(a / h0, 1e-12))
    print(f"{'Hz':>6s} {'soxr dB':>9s} {'fit dB':>9s} {'|diff| dB':>9s}")
    for fq, a, b, c in zip(freqs, db(hs), db(hm), db(hd)):
        print(f"{fq:6d} {a:9.4f} {b:9.4f} {c:9.1f}")
    if args.save:
        np.savez(args.save, tau48=tau48, g48=g48, tau96=tau96, g96=g96, params=p)
    return 0


if __name__ == "__main__":
    sys.exit(main())
