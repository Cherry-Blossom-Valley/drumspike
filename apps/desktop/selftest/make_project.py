"""Write a Drum Spike project (format 1) for the app tests (P3-7a). No server, no ffmpeg.

As a module: `build_project(parent, name, tracks)` with `tracks` = [(track name, wav path,
role, note)]; `mode` is "referenced" (the document points at the file) or "copied" (the
file goes into `Audio Files/`). As a script (used by `self-test.sh`):

    uv run python apps/desktop/selftest/make_project.py --out DIR --name NAME \
        --from "<session folder>" [--tracks N]

picks the first N of the standard drum tracks (k in, k out, k sub, s top, s bot, t2, t3,
oh, hh, r far) from `<session folder>/Audio Files/` and prints the project folder. Only the
Python standard library is used (P3-12: the writer below follows docs/migration/project-format.md
and writes what the retired server's project.py wrote); the WAV header gives rate and length.
With `--wavs DIR` instead of `--from`, the first N WAV files of DIR become the tracks (the
30-lane synthetic session of `tools/parity/make_session.py`, scenario `render`).
"""
import argparse
import hashlib
import json
import os
import shutil
import struct
import sys
import time
from pathlib import Path

FORMAT = 1
SUBDIRS = ("Audio Files", "Analysis", "Exports", "Backups")

ROLE_COLORS = {"kick": "#e05555", "snare": "#e0a13c", "tom": "#57b8e0", "hat": "#9be055",
               "cymbal": "#c9a0e0", "other": "#8a93a6"}
# (file prefix, track name, role, note): the naming scheme of the real sessions
STANDARD = [("k in", "k in", "kick", 36), ("k out", "k out", "kick", 36), ("k sub", "k sub", "kick", 36),
            ("s top", "s top", "snare", 38), ("s bot", "s bot", "snare", 38), ("t2", "t2", "tom", 45),
            ("t3", "t3", "tom", 43), ("oh", "oh", "cymbal", 49), ("hh", "hh", "hat", 42),
            ("r far", "r far", "cymbal", 51)]


def wav_info(path):
    """(sample rate, frames) from the RIFF header: 16/24-bit PCM, float and extensible files."""
    with open(path, "rb") as f:
        head = f.read(12)
        if head[:4] != b"RIFF" or head[8:12] != b"WAVE":
            raise ValueError(f"not a WAV file: {path}")
        sr = block = None
        while True:
            h = f.read(8)
            if len(h) < 8:
                raise ValueError(f"no data chunk: {path}")
            cid, size = h[:4], int.from_bytes(h[4:], "little")
            if cid == b"fmt ":
                fmt = f.read(size + (size & 1))
                sr = int.from_bytes(fmt[4:8], "little")
                block = int.from_bytes(fmt[12:14], "little")
            elif cid == b"data":
                if sr is None or not block:
                    raise ValueError(f"data before fmt: {path}")
                if size == 0xFFFFFFFF:
                    size = os.path.getsize(path) - f.tell()
                return sr, size // block
            else:
                f.seek(size + (size & 1), 1)


def free_name(folder, base):
    """<base> or <stem> (2).<ext> ... when the name is taken."""
    stem, ext = os.path.splitext(base)
    cand, k = base, 1
    while os.path.exists(os.path.join(folder, cand)):
        k += 1
        cand = f"{stem} ({k}){ext}"
    return cand


def bext_origination(path):
    """OriginationDate + OriginationTime of a Broadcast Wave bext chunk in the first MiB, or None."""
    with open(path, "rb") as f:
        head = f.read(1 << 20)
    pos = 12
    while head[:4] == b"RIFF" and pos + 8 <= len(head):
        cid, size = head[pos:pos + 4], struct.unpack("<I", head[pos + 4:pos + 8])[0]
        if cid == b"bext":
            body = head[pos + 8:pos + 8 + size]
            if len(body) < 338:
                return None
            date = body[320:330].split(b"\0")[0].decode("ascii", "replace")
            tm = body[330:338].split(b"\0")[0].decode("ascii", "replace")
            return f"{date}T{tm}Z" if date else None
        pos += 8 + size + (size & 1)
    return None


def source_record(path, project_dir, mode):
    """The source record of format 1: mode, absolute path, project-relative path, size, mtime, SHA-1 of the first MiB."""
    path = os.path.abspath(path)
    st = os.stat(path)
    with open(path, "rb") as f:
        head = hashlib.sha1(f.read(1 << 20)).hexdigest()
    rec = {"mode": mode, "path": path, "relative": os.path.relpath(path, os.path.abspath(project_dir)).replace(os.sep, "/"),
           "size": st.st_size, "mtime": int(st.st_mtime), "head_sha1": head}
    bo = bext_origination(path)
    if bo:
        rec["bext_origination"] = bo
    return rec


def build_project(parent, name, tracks, mode="referenced", mix_mode="average"):
    """Create <parent>/<name>/ with one track per entry; returns the project folder. An entry is
    (name, wav, role, note[, events[, extra track keys]]); `mix_mode` is every track's mix mode."""
    pdir = os.path.join(os.path.abspath(parent), name)
    doc_path = os.path.join(pdir, name + ".drumspike")
    if os.path.exists(doc_path):
        raise SystemExit(f"project already exists: {pdir}")
    for d in SUBDIRS:
        os.makedirs(os.path.join(pdir, d), exist_ok=True)
    now = time.time()
    doc = {"format": FORMAT, "name": name, "created": now, "modified": now, "settings": {"mix_mode": "average"},
           "order": [], "tracks": {}, "view": {}}
    for tname, wav, role, note, *rest in tracks:   # optional: the events [{t, v, id}, ...], extra keys (params)
        src = Path(wav)
        if mode == "copied":
            dst = Path(pdir) / "Audio Files" / free_name(str(Path(pdir) / "Audio Files"), src.name)
            shutil.copyfile(src, dst)
            src = dst
        sr, n = wav_info(src)
        if tname in doc["tracks"]:
            raise SystemExit(f"track exists: {tname}")
        doc["tracks"][tname] = {"role": role, "note": int(note), "mix_mode": mix_mode,
                                "source": source_record(src, pdir, mode), "events": (rest[0] if rest and rest[0] else []),
                                "sr": sr, "n_samples": n, "duration": n / sr, "color": ROLE_COLORS.get(role, "#8a93a6"),
                                **(rest[1] if len(rest) > 1 else {})}
        doc["order"].append(tname)
    with open(doc_path + ".tmp", "w") as f:
        json.dump(doc, f, indent=1)
    os.replace(doc_path + ".tmp", doc_path)
    return pdir


def standard_tracks(session, n):
    """The first n standard tracks found in <session>/Audio Files/ (file name = prefix + _NN.wav)."""
    audio = Path(session) / "Audio Files"
    out = []
    for prefix, tname, role, note in STANDARD:
        hits = sorted(audio.glob(f"{prefix}_[0-9][0-9].wav"))
        if hits:
            out.append((tname, hits[0], role, note))
    if len(out) < n:
        raise SystemExit(f"only {len(out)} of {n} standard tracks in {audio}")
    return out[:n]


def wav_folder_tracks(folder, n):
    """The first n WAV files of a folder (sorted by name) as tracks: the 30-lane session of
    tools/parity/make_session.py (t01.wav ... t30.wav). Roles cycle through kick, snare, tom."""
    roles = [("kick", 36), ("snare", 38), ("tom", 45)]
    files = sorted(Path(folder).glob("*.wav"))
    if len(files) < n:
        raise SystemExit(f"only {len(files)} of {n} WAV files in {folder}")
    return [(f.stem, f, *roles[k % 3]) for k, f in enumerate(files[:n])]


def synthetic_tracks(folder, n=None):
    """The tracks of a session written by tools/parity/make_session.py (session.json and t01.wav ...):
    names t01 ..., roles cycling through the standard drum roles, the known hits as events."""
    folder = Path(folder)
    manifest = json.loads((folder / "session.json").read_text())
    roles = [("kick", 36), ("snare", 38), ("tom", 45), ("hat", 42), ("cymbal", 49), ("other", 40)]
    out = []
    for i, t in enumerate(manifest["tracks"][:n]):
        role, note = roles[i % len(roles)]
        events = [{"t": s, "v": 100, "id": k + 1} for k, s in enumerate(t["hit_times_s"])]
        out.append((Path(t["file"]).stem, folder / t["file"], role, note, events))
    return out


DROP_FILES = ["k in", "k out", "k sub", "s top", "s bot", "t1 hh", "t2", "t3", "t4 ri", "hh"]


def drop_folder(session, out):
    """Scenario `tracks` (P3-4c): a folder of 10 copies of the real drum tracks of <session>/Audio Files/ with the
    owner's names (k in_02.wav ...), the folder a user drops on the empty window. The originals stay untouched."""
    audio = Path(session) / "Audio Files"
    os.makedirs(out, exist_ok=True)
    for prefix in DROP_FILES:
        hits = sorted(audio.glob(f"{prefix}_[0-9][0-9].wav"))
        if not hits:
            raise SystemExit(f"no {prefix}_NN.wav in {audio}")
        shutil.copyfile(hits[0], Path(out) / hits[0].name)
    return Path(out)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", required=True, help="parent folder of the project")
    ap.add_argument("--name", default="Self Test")
    ap.add_argument("--from", dest="session", help="session folder with Audio Files/")
    ap.add_argument("--wavs", help="a folder of WAV files instead (the synthetic 30-lane session)")
    ap.add_argument("--synthetic", help="a make_session.py folder (session.json, t01.wav ...) instead of --from; "
                                        "all its tracks (or --tracks N), with their hits as events")
    ap.add_argument("--tracks", type=int, default=10)
    ap.add_argument("--mode", default="referenced", choices=["referenced", "copied"])
    ap.add_argument("--drop", help="with --from: write the 10 drum tracks as plain files into this folder (no project)")
    a = ap.parse_args(argv)
    os.makedirs(a.out, exist_ok=True)
    if a.drop:
        if not a.session:
            raise SystemExit("--drop needs --from <session folder>")
        print(drop_folder(a.session, a.drop))
        return
    if a.synthetic:
        tracks = synthetic_tracks(a.synthetic, a.tracks if "--tracks" in (argv or sys.argv) else None)
    elif a.wavs:
        tracks = wav_folder_tracks(a.wavs, a.tracks)
    elif a.session:
        tracks = standard_tracks(a.session, a.tracks)
    else:
        raise SystemExit("give --from <session folder>, --wavs <folder> or --synthetic <make_session folder>")
    print(build_project(a.out, a.name, tracks, a.mode))


if __name__ == "__main__":
    main()
