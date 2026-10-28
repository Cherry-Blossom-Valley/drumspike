"""Frozen references (P3-12): the outputs of the retired Python server, kept as golden files.

Each parity test compares the Rust side with a reference. Until P3-12 the reference was computed
live by the Python server code (engine.py, project.py, server.py). P3-12 step 2 ran the parity
tests once on each host with DRUMSPIKE_FROZEN=record and stored every reference in
tests/golden/frozen/<kind>.json; from step 3 on the tests read the stored value instead.

    ref = frozen.store("midi").ref(key, inputs, compute)

key      a stable, readable name of the case (never a temporary path).
inputs   what the reference depends on (events, a file hash, parameters); stored as its
         SHA-256. A case whose inputs changed does not match its stored value: that fails.
compute  a function without arguments that gives the reference (JSON values only), or None
         when the code that made it is gone (replay only).

Modes (DRUMSPIKE_FROZEN): "record" computes and stores the value for this host; "check" (the
default while compute is given) computes it and fails when it differs from a stored value;
"replay" (the default when compute is None) returns the stored value.

Hosts: the value is stored per CPU architecture (arm64 = arm64-mac, x86_64 = x86-mac). A key with the
same inputs and value on both hosts is kept once ("all"). Replay takes the value of the own host,
else the shared one; `canonical=True` takes the arm64 value on every host (the Rust decode equals
the fusing arm64 build on every host, so a decode reference is that of arm64).

Large values (more than LARGE bytes of JSON) keep only their SHA-256 and size in git; the full value
goes to DRUMSPIKE_FROZEN_EXTERNAL (default ~/.cache/drumspike-frozen/<kind>/). same() compares a
result with such a digest by hashing the result."""
import atexit
import hashlib
import json
import os
import platform
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DIR = ROOT / "tests" / "golden" / "frozen"
LARGE = 64 * 1024
CANONICAL_HOST = "arm64"
HOST_NAMES = {"arm64": "arm64-mac", "x86_64": "x86-mac"}


class FrozenError(AssertionError):
    """A frozen reference is missing, its inputs changed, or a live value differs from it."""


def host():
    m = platform.machine()
    return "arm64" if m in ("arm64", "aarch64") else m


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)


def digest(value):
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def is_digest(v):
    return isinstance(v, dict) and "$sha256" in v


def same(got, ref):
    """True when got equals the reference; a large reference is compared by its digest."""
    if is_digest(ref):
        return digest(got) == ref["$sha256"]
    return got == ref


def external_dir():
    return Path(os.environ.get("DRUMSPIKE_FROZEN_EXTERNAL") or Path.home() / ".cache" / "drumspike-frozen")


def git_sha():
    try:
        sha = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        dirty = subprocess.run(["git", "-C", str(ROOT), "status", "--porcelain", "--untracked-files=no"],
                               capture_output=True, text=True).stdout.strip()
        return sha + ("-dirty" if dirty else "")
    except OSError:
        return "unknown"


def mode(compute):
    m = os.environ.get("DRUMSPIKE_FROZEN") or ("check" if compute is not None else "replay")
    if m not in ("record", "check", "replay"):
        raise ValueError(f"DRUMSPIKE_FROZEN={m}: use record, check or replay")
    return m


class Store:
    def __init__(self, kind, about=""):
        self.kind, self.about = kind, about
        self.path = DIR / f"{kind}.json"
        self.doc = json.loads(self.path.read_text()) if self.path.exists() else {
            "kind": kind, "about": about, "generated": {}, "entries": {}}
        self.dirty = False

    def entry_for(self, key, h=None, canonical_host=False):
        """(inputs digest, value) stored for key on host h (default: this host), or None."""
        e = self.doc["entries"].get(key)
        if e is None:
            return None
        h = CANONICAL_HOST if canonical_host else (h or host())
        if "all" in e:
            return e["all"]["inputs"], e["all"]["value"]
        got = e.get("by_host", {}).get(h)
        return None if got is None else (got["inputs"], got["value"])

    def _put(self, key, inputs_d, value):
        if len(canonical(value)) > LARGE:
            ext = external_dir() / self.kind / (hashlib.sha256(key.encode()).hexdigest()[:24] + ".json")
            ext.parent.mkdir(parents=True, exist_ok=True)
            ext.write_text(canonical({"key": key, "value": value}))
            value = {"$sha256": digest(value), "$bytes": len(canonical(value)), "$file": str(ext.relative_to(external_dir()))}
        e = self.doc["entries"].get(key, {})
        hosts = dict(e.get("by_host", {}))
        if "all" in e:
            for h in e["hosts"]:
                hosts[h] = e["all"]
        hosts[host()] = {"inputs": inputs_d, "value": value}
        vals = list(hosts.values())
        if len(hosts) > 1 and all(v == vals[0] for v in vals):
            self.doc["entries"][key] = {"all": vals[0], "hosts": sorted(hosts)}
        else:
            self.doc["entries"][key] = {"by_host": dict(sorted(hosts.items()))}
        gen = self.doc.setdefault("generated", {})
        gen[host()] = {"sha": git_sha(), "host": HOST_NAMES.get(host(), platform.node()),
                       "date": time.strftime("%Y-%m-%d")}
        if not self.dirty:
            self.dirty = True
            _DIRTY.append(self)

    def full_value(self, value):
        """The full value of a large reference when its external file exists, else the digest itself."""
        if not is_digest(value):
            return value
        p = external_dir() / value["$file"]
        return json.loads(p.read_text())["value"] if p.exists() else value

    def ref(self, key, inputs, compute=None, canonical_host=False):
        m = mode(compute)
        inputs_d = digest(inputs)
        if m == "replay":
            tries = [self.entry_for(key, canonical_host=True)] if canonical_host else []
            tries.append(self.entry_for(key))
            if all(g is None for g in tries):
                raise FrozenError(f"no frozen {self.kind} reference for {key!r} on {host()} "
                                  f"({self.path.relative_to(ROOT)})")
            for g in tries:
                if g is not None and g[0] == inputs_d:
                    return g[1]
            raise FrozenError(f"the inputs of {self.kind} {key!r} differ from the frozen run: the case changed")
        value = compute()
        if m == "record":
            self._put(key, inputs_d, value)
        else:
            got = self.entry_for(key)            # a live value is checked against the value of its own host
            if got is not None:
                if got[0] != inputs_d:
                    raise FrozenError(f"the inputs of {self.kind} {key!r} differ from the frozen run")
                if not same(value, got[1]):
                    raise FrozenError(f"the live {self.kind} reference for {key!r} differs from the frozen one")
        if canonical_host and host() != CANONICAL_HOST:
            # the comparison uses the arm64 value here too, once it is stored (see the module text)
            got = self.entry_for(key, canonical_host=True)
            if got is not None and got[0] == inputs_d:
                return got[1]
        return value

    def save(self):
        DIR.mkdir(parents=True, exist_ok=True)
        self.doc["entries"] = dict(sorted(self.doc["entries"].items()))
        self.path.write_text(dumps_store(self.doc))
        self.dirty = False


def dumps_store(doc):
    """The text of a store file: the head indented, then one line per entry (a diff names the changed key)."""
    head = {k: v for k, v in doc.items() if k != "entries"}
    lines = json.dumps(head, indent=1).rstrip("}").rstrip() + ',\n "entries": {\n'
    items = [f" {json.dumps(k)}: {json.dumps(v, separators=(',', ':'), allow_nan=False)}" for k, v in doc["entries"].items()]
    return lines + ",\n".join(items) + "\n }\n}\n"


_STORES = {}
_DIRTY = []


def store(kind, about=""):
    if kind not in _STORES:
        _STORES[kind] = Store(kind, about)
    s = _STORES[kind]
    if about and not s.doc.get("about"):
        s.doc["about"] = about
    return s


@atexit.register
def _save_all():
    for s in list(_DIRTY):
        s.save()


def wav_summary(data):
    """A WAV file as a small JSON value (P3-12: the frozen spike files): every byte before and after
    the sample data as hex, the sample format, the frame count, the non-zero samples as
    [index, value] (the integer for PCM, the float for float32) and the SHA-256 of the whole file.
    A changed byte anywhere changes the summary."""
    import struct

    import numpy as np
    pos, fmt, start, size = 12, None, None, None
    while pos + 8 <= len(data):
        cid, n = data[pos:pos + 4], struct.unpack("<I", data[pos + 4:pos + 8])[0]
        if cid == b"fmt ":
            tag, ch, _sr, _, _block, bits = struct.unpack("<HHIIHH", data[pos + 8:pos + 24])
            if tag == 0xFFFE:
                tag = struct.unpack("<H", data[pos + 32:pos + 34])[0]
            fmt = (tag, ch, bits)
        elif cid == b"data":
            start, size = pos + 8, min(n, len(data) - pos - 8)
            break
        pos += 8 + n + (n & 1)
    if fmt is None or start is None:
        raise ValueError("no fmt or data chunk")
    tag, ch, bits = fmt
    raw = data[start:start + size]
    if tag == 3 and bits == 32:
        x, kind = np.frombuffer(raw, "<f4"), "float32"
    elif bits == 16:
        x, kind = np.frombuffer(raw, "<i2"), "pcm16"
    elif bits == 32:
        x, kind = np.frombuffer(raw, "<i4"), "pcm32"
    elif bits == 24:
        b = np.frombuffer(raw[:len(raw) // 3 * 3], np.uint8).reshape(-1, 3).astype(np.int32)
        x, kind = ((b[:, 0] | (b[:, 1] << 8) | (b[:, 2] << 16)) << 8) >> 8, "pcm24"
    else:
        raise ValueError(f"format tag {tag}, {bits} bits")
    nz = np.flatnonzero(x)
    vals = [float(v) for v in x[nz]] if kind == "float32" else [int(v) for v in x[nz]]
    return {"format": kind, "channels": ch, "samples": int(len(x)), "head": data[:start].hex(),
            "tail": data[start + size:].hex(), "nonzero": [[int(i), v] for i, v in zip(nz, vals)],
            "sha256": hashlib.sha256(data).hexdigest()}
