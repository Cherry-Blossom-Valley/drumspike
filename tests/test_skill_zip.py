"""#371 (g): tools/skill_zip.py packs skills/drumspike-finetune as the release asset."""
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / "tools" / "skill_zip.py"


def run(out):
    return subprocess.run([sys.executable, "-I", str(TOOL), "--out", str(out)], capture_output=True, text=True, check=True)


def test_the_zip_lists_the_skill_under_its_folder_name(tmp_path):
    out = tmp_path / "drumspike-finetune-skill.zip"
    p = run(out)
    names = zipfile.ZipFile(out).namelist()
    assert "drumspike-finetune/SKILL.md" in names and "drumspike-finetune/README.md" in names
    assert all(n.startswith("drumspike-finetune/") for n in names)
    assert f"{len(names)} files" in p.stdout


def test_the_same_folder_packs_to_the_same_bytes(tmp_path):
    run(tmp_path / "a.zip")
    run(tmp_path / "b.zip")
    assert (tmp_path / "a.zip").read_bytes() == (tmp_path / "b.zip").read_bytes()


def test_a_folder_without_skill_md_is_refused(tmp_path):
    sys.path.insert(0, str(ROOT / "tools"))
    import skill_zip
    empty = tmp_path / "x"
    empty.mkdir()
    try:
        skill_zip.pack(tmp_path / "x.zip", empty)
    except SystemExit as e:
        assert "SKILL.md" in str(e)
    else:
        raise AssertionError("no refusal")
