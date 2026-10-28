"""#471: the residuals of the published playhead pairs against a straight line.

    cargo run -p drumspike-core --release --example clock_probe -- --secs 20 --wav tone.wav --out pairs.txt
    uv run python tools/clock_residuals.py pairs.txt

Each input line is `frame host_time_ns` (one per published playhead). The least-squares line
host = a + b * frame is fitted over all pairs; the residual of a pair is its host time minus the
line, in microseconds. Prints the count, the fitted rate (frames per second), and the max and p99
of |residual|. Stdlib only.
"""
import sys


def fit(xs, ys):
    """Least-squares a, b for y = a + b * x."""
    n = len(xs)
    mx = sum(xs) / n
    my = sum(ys) / n
    sxx = sum((x - mx) ** 2 for x in xs)
    sxy = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
    b = sxy / sxx
    return my - b * mx, b


def p99(values):
    """The 99th percentile (nearest rank) of a non-empty list."""
    s = sorted(values)
    return s[min(len(s) - 1, int(round(0.99 * (len(s) - 1))))]


def main(path):
    frames, hosts = [], []
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            parts = line.split()
            if len(parts) != 2:
                continue
            frames.append(float(parts[0]))
            hosts.append(float(parts[1]))
    if len(frames) < 3:
        print(f"FAIL: {len(frames)} pairs, need at least 3", file=sys.stderr)
        return 2
    a, b = fit(frames, hosts)
    res_us = [abs(h - (a + b * f)) / 1000.0 for f, h in zip(frames, hosts)]
    rate = 1e9 / b
    print(f"pairs={len(frames)} fitted_rate_hz={rate:.3f} max_us={max(res_us):.1f} p99_us={p99(res_us):.1f}")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("usage: clock_residuals.py pairs.txt", file=sys.stderr)
        sys.exit(2)
    sys.exit(main(sys.argv[1]))
