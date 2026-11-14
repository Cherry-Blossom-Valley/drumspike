"""Spike check: make fixtures, run the ARA test host on a plugin, compare what the plugin read.
usage: check.py make | check.py verify <out.jsonl> <label>"""
import json, struct, sys, os
FIX = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures")
SPEC = {  # name: (rate, channels, seconds, hit sample positions)
    "kick_48k_mono.wav": (48000, 1, 3, [1000, 24001, 77777, 143999]),
    "snare_44k_stereo.wav": (44100, 2, 2, [0 + 5, 4410, 44099, 88199]),
    "tom_96k_mono.wav": (96000, 1, 2, [12345, 100000, 191999]),
    "hat_48k_mono.wav": (48000, 1, 1, [10, 24000, 47999]),
}
IXML = b"<BWFXML><IXML_VERSION>1.5</IXML_VERSION></BWFXML> "  # even length; the test host loader loops without an iXML chunk
assert len(IXML) % 2 == 0
def samples(rate, ch, secs, hits):
    n = rate * secs
    data = [[0.0] * n for _ in range(ch)]
    for c in range(ch):
        for i in range(0, n, 97):            # low noise floor so the hash sees every region
            data[c][i] = ((i * 2654435761 + c) % 1000) / 1e6
    for k, h in enumerate(hits):
        data[0][h] = 0.9 - 0.1 * k           # the click, channel 0 only
        if h + 1 < n: data[0][h + 1] = 0.0
    return data
def f32(x): return struct.unpack("<f", struct.pack("<f", x))[0]
def make():
    os.makedirs(FIX, exist_ok=True)
    for name, (rate, ch, secs, hits) in SPEC.items():
        d = samples(rate, ch, secs, hits); n = rate * secs
        body = b"".join(struct.pack("<f", d[c][i]) for i in range(n) for c in range(ch))
        fmt = struct.pack("<HHIIHH", 3, ch, rate, rate * ch * 4, ch * 4, 32)
        riff = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"fact" + struct.pack("<II", 4, n) + b"data" + struct.pack("<I", len(body)) + body + b"iXML" + struct.pack("<I", len(IXML)) + IXML
        open(os.path.join(FIX, name), "wb").write(b"RIFF" + struct.pack("<I", len(riff)) + riff)
    print("made", len(SPEC), "fixtures in", FIX)
def expected(name):
    rate, ch, secs, hits = SPEC[name]; d = samples(rate, ch, secs, hits); n = rate * secs
    h = 0xcbf29ce484222325
    for i in range(n):
        for c in range(ch):
            for b in struct.pack("<f", d[c][i]):
                h = ((h ^ b) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return dict(sample_count=n, sample_rate=float(rate), channels=ch, fnv64=f"{h:016x}", hits=sorted(hits))
def verify(path, label):
    rows = [json.loads(l) for l in open(path) if l.strip()]
    reads = {os.path.basename(r["name"]): r for r in rows if r["event"] == "read"}
    readers = [r for r in rows if r["event"] == "content_reader"]
    bad = 0; compared = 0
    for name in SPEC:
        e = expected(name); r = reads.get(name)
        if r is None:
            print(f"FAIL {label} {name}: the plugin read nothing"); bad += 1; continue
        compared += 1
        for k in ("sample_count", "sample_rate", "channels", "fnv64", "hits"):
            if r[k] != e[k]:
                print(f"FAIL {label} {name}: {k} plugin={r[k]} expected={e[k]}"); bad += 1
        if r["samples_read"] != e["sample_count"] or not r["read_ok"]:
            print(f"FAIL {label} {name}: read incomplete"); bad += 1
        if r["caller_thread"] == r["reader_thread"]:
            print(f"FAIL {label} {name}: read ran on the host's calling thread"); bad += 1
        print(f"ok   {label} {name}: {e['sample_count']} samples x {e['channels']} ch at {int(e['sample_rate'])} Hz, hash equal, {len(e['hits'])} hits at the exact samples, background thread")
    notes = sum(r["notes"] for r in readers)
    print(f"{label}: compared {compared} sources, host asked for notes {len(readers)} times ({notes} notes), events: {sorted(set(r['event'] for r in rows))}")
    if compared == 0 or bad or not readers:
        print(f"SPIKE CHECK {label}: FAIL"); sys.exit(1)
    print(f"SPIKE CHECK {label}: PASS")
def render(dump, label, names):
    """The host rendered the regions of `names` back to back at the rate of the first file."""
    start, rate, count, ch = open(dump + ".txt").read().split(); start = int(start); rate = float(rate); count = int(count)
    raw = open(dump, "rb").read(); assert len(raw) == 4 * count, (len(raw), count)
    got = [start + i for i, (v,) in enumerate(struct.iter_unpack("<f", raw)) if abs(v) >= 0.5]
    nonzero = sum(1 for (v,) in struct.iter_unpack("<f", raw) if v != 0.0)
    want = []; offset = 0
    for n in names:
        r, c, secs, hits = SPEC[n]; assert float(r) == rate, "the render check needs files at the render rate"
        want += [offset + h for h in hits]; offset += r * secs
    diffs = [g - w for g, w in zip(got, want)] if len(got) == len(want) else None
    print(f"{label}: host buffer {count} samples at {int(rate)} Hz, clicks found {len(got)}, expected {len(want)}, other non-zero samples {nonzero - len(got)}")
    if not want or diffs is None or any(diffs) or nonzero != len(got):
        print(f"  got  {got}\n  want {want}"); print(f"SPIKE RENDER CHECK {label}: FAIL"); sys.exit(1)
    print(f"  all {len(want)} clicks at 0 samples difference; SPIKE RENDER CHECK {label}: PASS")
if __name__ == "__main__":
    if sys.argv[1] == "render": render(sys.argv[2], sys.argv[3], sys.argv[4:]); sys.exit(0)
    make() if sys.argv[1] == "make" else verify(sys.argv[2], sys.argv[3])
