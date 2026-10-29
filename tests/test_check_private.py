"""#104: tests for the private-name text guard. Fixtures use invented words only
(ZUMQUUX, FLOOB-FRAME), never a real private name."""
import subprocess
import sys
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1] / "tools"
sys.path.insert(0, str(TOOLS))

import check_private  # noqa: E402

REPO = Path(__file__).resolve().parents[1]
INVENTED = "ZUMQUUX"
INVENTED_PAT = r"[a-z]+-flooob-frame\.npz"


def run(argv, env_extra):
    import os
    env = {k: v for k, v in os.environ.items() if not k.startswith("DRUMSPIKE_PRIVATE_")}
    env.update(env_extra)
    return subprocess.run([sys.executable, str(TOOLS / "check_private.py"), *argv],
                          capture_output=True, text=True, env=env, cwd=REPO)


def test_planted_word_found(tmp_path):
    f = tmp_path / "draft.md"
    f.write_text(f"clean line\nthe take is {INVENTED} take 7\n", encoding="utf-8")
    r = run(["--text-file", str(f)], {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp,zumquux"})
    assert r.returncode == 1
    assert "draft.md:2" in r.stderr
    assert INVENTED.lower() in r.stderr.lower()


def test_planted_pattern_found(tmp_path):
    f = tmp_path / "body.md"
    f.write_text("dataset: some-flooob-frame.npz here\n", encoding="utf-8")
    r = run(["--text-file", str(f)],
            {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp", "DRUMSPIKE_PRIVATE_PATTERNS": INVENTED_PAT})
    assert r.returncode == 1
    assert "body.md:1" in r.stderr
    assert "flooob" in r.stderr


def test_unset_list_is_loud():
    r = run(["--diff", "origin/main"], {})
    assert r.returncode == 2
    assert "unset" in r.stderr.lower()


def test_allow_empty_passes_unset():
    r = run(["--diff", "origin/main", "--allow-empty"], {})
    assert r.returncode == 0


def test_base_word_flagged_in_file(tmp_path):
    """A base-list word (/Users) in a plain file is flagged."""
    f = tmp_path / "x.txt"
    f.write_text("/Users/someone/x\n", encoding="utf-8")
    r = run([str(f)], {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp"})
    assert r.returncode == 1
    assert "/Users" in r.stderr


def test_clean_text_file_passes(tmp_path):
    f = tmp_path / "ok.md"
    f.write_text("Nothing private here, all invented words only.\n", encoding="utf-8")
    r = run([str(f)], {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp,zumquux"})
    assert r.returncode == 0


def test_diff_clean_on_branch_head():
    """--diff origin/main rc=0 on this PR itself (with a test list): the PR text carries
    no private name and no planted word leaks into its own diff."""
    r = run(["--diff", "origin/main"], {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp,zumquux",
                                        "DRUMSPIKE_PRIVATE_PATTERNS": INVENTED_PAT})
    assert r.returncode == 0, r.stderr


def test_shared_definition_with_check_paths():
    """One definition, not two copies: check_private's private_words is the function of
    the loaded check_paths module, not a re-defined tuple."""
    assert check_private._check_paths.__file__ == str(TOOLS / "media" / "check_paths.py")
    assert check_private.private_words is check_private._check_paths.private_words
    assert check_private.BANNED_BASE == check_paths_module().BANNED_BASE


def check_paths_module():
    import importlib.util
    spec = importlib.util.spec_from_file_location(
        "check_paths", TOOLS / "media" / "check_paths.py")
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_bad_pattern_regex_is_loud(tmp_path):
    f = tmp_path / "x.md"
    f.write_text("text\n", encoding="utf-8")
    r = run([str(f)], {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp",
                       "DRUMSPIKE_PRIVATE_PATTERNS": "(unclosed"})
    assert r.returncode == 2
