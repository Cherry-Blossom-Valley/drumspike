#!/usr/bin/env -S uv run --no-project --python 3.12
"""Make apps/desktop/THIRD_PARTY.md from `cargo metadata` (card P3-8a).

The list holds every crate that the Drum Spike app links: the normal dependencies of `drumspike-desktop`
for the targets aarch64-apple-darwin and x86_64-apple-darwin (P3-11; `cargo tree -e normal` per target, the
union; dev and build-only crates are not linked into the app), without the four workspace crates. `cargo metadata` gives the licence data. Each crate has its version, its licence
expression, the licence that the app uses it under, its source and its licence files.
MPL-2.0 crates are used unchanged; their notice and the MPL-2.0 text are in a section of their own.

  uv run --no-project --python 3.12 apps/desktop/scripts/gen_third_party.py            # write the file
  uv run --no-project --python 3.12 apps/desktop/scripts/gen_third_party.py --check    # exit 1 when the file is stale
  uv run --no-project --python 3.12 apps/desktop/scripts/gen_third_party.py --count    # print the crate count

Licence rule (owner decision B5): MIT, Apache-2.0, BSD, ISC, Zlib, Unicode and MPL-2.0 (unchanged)
are approved. A crate with any other licence stops the script (exit 2) and is named.
The standard library only; no third-party Python package.
"""

import hashlib
import json
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "apps" / "desktop" / "THIRD_PARTY.md"
# P3-11: one list for both app builds; the column "Targets" names a crate that only one of them links.
TARGETS = {"aarch64-apple-darwin": "arm64", "x86_64-apple-darwin": "x86_64"}
ROOT_CRATE = "drumspike-desktop"
APPROVED = {
    "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib",
    "Unicode-3.0", "Unicode-DFS-2016", "MPL-2.0",
    # owner decision 2026-10-11: the Unlicense (a public domain dedication; midly) is approved
    "Unlicense",
}
# Licences that are not on the owner's list but belong to a dependency that was already in the repository.
# The script lists them as "owner decision pending" and says so on stderr. Empty since 2026-10-11.
PENDING: set[str] = set()
# preference when an expression offers a choice
PREFER = ["MIT", "Apache-2.0", "BSD-3-Clause", "BSD-2-Clause", "ISC", "Zlib", "Unicode-3.0", "Unicode-DFS-2016", "MPL-2.0", "Unlicense"]
LICENSE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)([-_.].*)?$", re.I)


def metadata():
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out)


def tree(target):
    """(name, version) of every crate that `cargo tree -e normal` names for the target."""
    out = subprocess.run(
        ["cargo", "tree", "-p", ROOT_CRATE, "--target", target, "-e", "normal", "--prefix", "none", "--locked",
         "--format", "{p}"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout
    want = set()
    for line in out.splitlines():
        mt = re.match(r"^(\S+) v(\S+)", line.strip())
        if mt:
            want.add((mt.group(1), mt.group(2)))
    return want


def reachable(m):
    """The crates that the app links: `cargo tree -e normal` for each target (it keeps the features of normal and
    build dependencies apart; `cargo metadata` unifies them and would add build-only crates such as `ureq`),
    the union of both targets. The licence data comes from `cargo metadata`. Workspace crates are left out.
    Each package gets the key `targets`: the short names of the targets that link it."""
    per = {short: tree(t) for t, short in TARGETS.items()}
    want = set().union(*per.values())
    ws = set(m["workspace_members"])
    have = {(p["name"], p["version"]): p for p in m["packages"] if p["id"] not in ws}
    missing = [k for k in want if k not in have and not k[0].startswith("drumspike-")]
    if missing:
        sys.exit(f"cargo tree names crates that cargo metadata lacks: {missing}")
    out = []
    for k in sorted(k for k in want if k in have):
        p = dict(have[k])
        p["targets"] = [short for short, ks in per.items() if k in ks]
        out.append(p)
    return out


# ---- licence expressions: A OR B, A AND B, A WITH exception, parentheses ----
def tokens(expr):
    return re.findall(r"\(|\)|[^\s()]+", expr.replace("/", " OR "))


def parse(toks):
    def alt():
        left = conj()
        while toks and toks[0].upper() == "OR":
            toks.pop(0)
            right = conj()
            left = ("or", left, right)
        return left

    def conj():
        left = atom()
        while toks and toks[0].upper() == "AND":
            toks.pop(0)
            right = atom()
            left = ("and", left, right)
        return left

    def atom():
        t = toks.pop(0)
        if t == "(":
            v = alt()
            toks.pop(0)
            return v
        if toks and toks[0].upper() == "WITH":
            toks.pop(0)
            toks.pop(0)
        return ("id", t.rstrip("+"))

    return alt()


def choose(node):
    """The approved licences that cover the expression, or None when it cannot be met."""
    kind = node[0]
    if kind == "id":
        return [node[1]] if node[1] in APPROVED | PENDING else None
    a, b = choose(node[1]), choose(node[2])
    if kind == "and":
        return None if a is None or b is None else a + b
    if a is None:
        return b
    if b is None:
        return a
    ra = min(PREFER.index(x) for x in a)
    rb = min(PREFER.index(x) for x in b)
    return a if ra <= rb else b


def used_under(pkg):
    expr = pkg.get("license")
    if not expr:
        return None
    return choose(parse(tokens(expr)))


def license_files(pkg):
    d = Path(pkg["manifest_path"]).parent
    files = []
    for f in sorted(d.iterdir()):
        if f.is_file() and LICENSE_FILE.match(f.name):
            files.append(f)
    for sub in ("licenses", "license", "LICENSES"):
        s = d / sub
        if s.is_dir():
            files += [f for f in sorted(s.iterdir()) if f.is_file()]
    return files


def norm(text):
    return "\n".join(line.rstrip() for line in text.replace("\r\n", "\n").strip().split("\n"))


def build():
    m = metadata()
    pkgs = reachable(m)
    bad = [f'{p["name"]} {p["version"]}: {p.get("license")!r}' for p in pkgs if not used_under(p)]
    if bad:
        print("licence stop (owner decision B5): not approved or unknown:\n  " + "\n  ".join(bad), file=sys.stderr)
        sys.exit(2)
    pend = [f'{p["name"]} {p["version"]} ({p["license"]})' for p in pkgs if set(used_under(p)) & PENDING]
    if pend:
        print("owner decision pending (licence not on the B5 list): " + "; ".join(pend), file=sys.stderr)
    texts = {}                       # hash -> [text, [crate labels]]
    nofile = []
    per_crate = {}
    for p in pkgs:
        label = f'{p["name"]} {p["version"]}'
        hs = []
        for f in license_files(p):
            try:
                t = norm(f.read_text(encoding="utf-8", errors="replace"))
            except OSError:
                continue
            if not t:
                continue
            h = hashlib.sha256(t.encode()).hexdigest()
            texts.setdefault(h, [t, []])[1].append(label)
            hs.append(h)
        per_crate[label] = hs
        if not hs:
            nofile.append(label)
    return pkgs, texts, nofile


def source_of(p):
    return p.get("repository") or p.get("homepage") or (f'https://crates.io/crates/{p["name"]}')


def render(pkgs, texts, nofile):
    n = len(pkgs)
    counts = defaultdict(int)
    for p in pkgs:
        counts[" + ".join(used_under(p))] += 1
    mpl = [p for p in pkgs if "MPL-2.0" in used_under(p)]
    L = []
    w = L.append
    w("# Third-party software in Drum Spike")
    w("")
    w("Generated by `apps/desktop/scripts/gen_third_party.py` from `cargo metadata` and `cargo tree` "
      f"(targets {', '.join(f'`{t}`' for t in TARGETS)}, root crate `{ROOT_CRATE}`, normal dependencies only). "
      "Do not edit by hand.")
    w("")
    w(f"The two builds of the app (Apple silicon `arm64` and Intel `x86_64`) link **{n} third-party Rust crates** "
      "together (the four crates of this repository are not counted); the column \"Targets\" names a crate that "
      "only one build links. "
      "Every crate below is used under a permissive licence or under MPL-2.0 without changes.")
    w("")
    w("- **npm**: the only npm package is the build tool `@tauri-apps/cli` 2.12.1 (MIT OR Apache-2.0). It runs at build "
      "time and ships nothing in the app. The front end (`static/`) has no npm runtime dependency.")
    w("")
    pend = [p for p in pkgs if set(used_under(p)) & PENDING]
    if pend:
        w("## Owner decision pending")
        w("")
        w("Licence rule B5 does not name the Unlicense (a public domain dedication). It applies to: " +
          ", ".join(f'`{p["name"]}` {p["version"]}' for p in pend) +
          ". The crate was already a dependency of `drumspike-core` before the rule. The owner decides.")
        w("")
    w("## Licences in use")
    w("")
    w("| Licence used | Crates |")
    w("|---|---|")
    for k, v in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0])):
        w(f"| {k} | {v} |")
    w("")
    w(f"## MPL-2.0 crates ({len(mpl)}, unchanged)")
    w("")
    w("These crates are used exactly as published on crates.io. Their source code is available at the address in the "
      "list and in the crates.io package of the given version. The text of the Mozilla Public License 2.0 follows "
      "in the last section. This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. "
      "If a copy of the MPL was not distributed with a file, you can obtain one at https://mozilla.org/MPL/2.0/.")
    w("")
    for p in mpl:
        w(f'- `{p["name"]}` {p["version"]}: {source_of(p)} (licence expression `{p["license"]}`)')
    w("")
    w(f"## Crates ({n})")
    w("")
    w("| Crate | Version | Licence expression | Used under | Source | Targets |")
    w("|---|---|---|---|---|---|")
    for p in pkgs:
        tg = "both" if len(p["targets"]) == len(TARGETS) else " + ".join(p["targets"])
        w(f'| {p["name"]} | {p["version"]} | {p["license"]} | {" + ".join(used_under(p))} | {source_of(p)} | {tg} |')
    w("")
    w("## Licence texts")
    w("")
    w("Each text is shown once. Crates with the same text are listed under it. "
      "The text is the licence file of the crate as published.")
    if nofile:
        w("")
        w(f"{len(nofile)} crate(s) have no licence file in their package; for them the licence expression above applies "
          "and the standard text of that licence (https://spdx.org/licenses/) is meant: " + ", ".join(nofile) + ".")
    w("")
    ordered = sorted(texts.items(), key=lambda kv: (kv[1][1][0].lower(), kv[0]))
    for i, (h, (t, who)) in enumerate(ordered, 1):
        w(f"### Text {i}")
        w("")
        w("Used by: " + ", ".join(sorted(set(who))))
        w("")
        w("~~~~text")
        w(t)
        w("~~~~")
        w("")
    if not any("Mozilla Public License" in t for t, _ in texts.values()):
        w("### MPL-2.0 text")
        w("")
        w("Not found in the crate packages; see https://www.mozilla.org/MPL/2.0/.")
        w("")
    return "\n".join(L).rstrip() + "\n"


def main():
    args = sys.argv[1:]
    pkgs, texts, nofile = build()
    if "--count" in args:
        print(len(pkgs))
        return
    text = render(pkgs, texts, nofile)
    if "--check" in args:
        if not OUT.exists() or OUT.read_text(encoding="utf-8") != text:
            print("THIRD_PARTY.md is stale: run gen_third_party.py", file=sys.stderr)
            sys.exit(1)
        print(f"THIRD_PARTY.md is current ({len(pkgs)} crates)")
        return
    OUT.write_text(text, encoding="utf-8")
    print(f"wrote {OUT} ({len(pkgs)} crates, {len(texts)} licence texts, {len(text)} bytes)")


if __name__ == "__main__":
    main()
