"""Function lengths of the production Rust code (refactor plan section 7.2: longest function, functions over 100 lines).

    uv run --no-project python tools/fn_lengths.py <min lines> [path prefix ...]

Prints `lines file function first-line` for every function of at least <min lines>, longest first. The default
prefixes are `crates/` and `apps/`. A function is counted from its `fn` keyword to the matching closing brace
(signature, body, nested closures included; doc comments not). Strings, character literals and comments are blanked
first, so braces in them do not count. Production code only: files named `tests.rs` or `*_tests.rs`, `tests/`
folders and the top-level `#[cfg(test)] mod` at the end of a file are left out. It is a brace counter, not a
parser: raw strings with braces and macros that build `fn` items are not understood.
"""
import re
import subprocess
import sys


def blank(src):
    """`src` without comments, with strings and character literals reduced to a placeholder (newlines kept
    inside comments only; the line numbers of the code stay right because strings keep their newlines)."""
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            while i < n and src[i] != "\n":
                i += 1
        elif src.startswith("/*", i):
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append("\n" * src.count("\n", i, j))
            i = j
        elif c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            out.append('""' + "\n" * src.count("\n", i, j))
            i = j + 1
        elif c == "'" and (m := re.match(r"'(\\.|[^\\'])'", src[i:i + 4])):
            out.append("' '")
            i += m.end()
        else:
            out.append(c)
            i += 1
    return "".join(out)


def production(raw):
    """The text before the top-level test module (the last `#[cfg(test)] mod` at column 0)."""
    m = re.search(r"^#\[cfg\(test\)\]\s*\n(pub )?mod\s", raw, re.M)
    return raw[:m.start()] if m else raw


def functions(raw):
    """[(name, first line, length in lines)] of the functions with a body in the production part of `raw`."""
    src = blank(production(raw))
    out = []
    for m in re.finditer(r"\bfn\s+(\w+)", src):
        j, paren = m.end(), 0
        while j < len(src):                      # the signature ends at the body brace or at a `;`
            ch = src[j]
            if ch == "(":
                paren += 1
            elif ch == ")":
                paren -= 1
            elif paren == 0 and ch in "{;":
                break
            j += 1
        if j >= len(src) or src[j] == ";":
            continue
        depth, k = 0, j
        while k < len(src):
            if src[k] == "{":
                depth += 1
            elif src[k] == "}":
                depth -= 1
                if depth == 0:
                    break
            k += 1
        out.append((m.group(1), src.count("\n", 0, m.start()) + 1, src.count("\n", m.start(), k) + 1))
    return out


def is_test_file(path):
    return re.search(r"(^|/)tests?(/|\.rs$)|_tests?\.rs$", path) is not None


def main(argv):
    if not argv or not argv[0].isdigit():
        print(__doc__)
        return 2
    lo, prefixes = int(argv[0]), tuple(argv[1:]) or ("crates/", "apps/")
    files = subprocess.run(["git", "ls-files", "*.rs"], capture_output=True, text=True, check=True).stdout.split("\n")
    rows = []
    for f in files:
        if not f or not f.startswith(prefixes) or is_test_file(f):
            continue
        with open(f) as fh:
            rows += [(n, f, name, line) for name, line, n in functions(fh.read()) if n >= lo]
    for n, f, name, line in sorted(rows, reverse=True):
        print(n, f, name, line)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
