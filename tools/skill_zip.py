"""#371: pack the drumspike-finetune skill as the release asset.

    uv run python tools/skill_zip.py [--out drumspike-finetune-skill.zip]

Zips `skills/drumspike-finetune/` with the folder name as the top entry (unpacked into ~/.claude/skills/ it gives
`drumspike-finetune/SKILL.md`). The entries are sorted and carry a fixed time, so the same folder packs to the same bytes.
"""
import argparse
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SKILL = ROOT / "skills" / "drumspike-finetune"
DEFAULT_OUT = "drumspike-finetune-skill.zip"
FIXED_TIME = (2026, 1, 1, 0, 0, 0)


def pack(out, skill=SKILL):
    """Write the zip of the skill folder to `out`; returns the entry names."""
    files = sorted(p for p in skill.rglob("*") if p.is_file() and "__pycache__" not in p.parts and p.name != ".DS_Store")
    if not (skill / "SKILL.md").is_file():
        raise SystemExit(f"{skill} has no SKILL.md")
    names = []
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for p in files:
            name = f"{skill.name}/{p.relative_to(skill).as_posix()}"
            info = zipfile.ZipInfo(name, FIXED_TIME)
            info.external_attr = 0o644 << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, p.read_bytes())
            names.append(name)
    return names


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", default=DEFAULT_OUT)
    args = ap.parse_args(argv)
    names = pack(Path(args.out))
    print(f"{args.out}: {len(names)} files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
