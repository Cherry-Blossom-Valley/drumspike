"""#577 (Gate line 6): look at the files of the plugin's user defaults, so a test can prove that the standalone app never reads or writes them.

`snapshot(root)` lists every file and folder below `root` with size, mtime and the sha256 of the content. `changed(before, after)` names what differs: a
changed byte, a changed mtime, a new or a missing entry. The unit tests of this module (tests/test_standalone_plugin_files.py) prove that `changed`
fires on each of them, and plugins/ara/check.py uses it as the positive control (a plugin run that saves a default must make it fire)."""
import hashlib
from pathlib import Path

FOLDER = "DrumSpike ARA"


def plugin_folder(home):
    return Path(home) / "Library" / "Application Support" / FOLDER


def snapshot(root):
    out = {}
    root = Path(root)
    if not root.exists():
        return out
    for p in sorted([root, *root.rglob("*")]):
        st = p.lstat()
        digest = hashlib.sha256(p.read_bytes()).hexdigest() if p.is_file() and not p.is_symlink() else ""
        out[str(p.relative_to(root))] = (st.st_size if p.is_file() else -1, st.st_mtime_ns, digest)
    return out


def changed(before, after):
    names = []
    for k in sorted(set(before) | set(after)):
        if k not in before:
            names.append(f"new: {k}")
        elif k not in after:
            names.append(f"missing: {k}")
        elif before[k][0] != after[k][0] or before[k][2] != after[k][2]:
            names.append(f"content: {k}")
        elif before[k][1] != after[k][1]:
            names.append(f"mtime: {k}")
    return names
