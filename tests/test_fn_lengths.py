"""tools/fn_lengths.py on a small synthetic Rust text (no repository files)."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import fn_lengths  # noqa: E402

SRC = '''
/// doc { not code }
fn short() -> u8 { 1 }

pub fn long(a: u8) -> String {
    let s = "a } brace in a string {";
    let c = '}';
    // a { comment
    let f = |x: u8| {
        x + 1
    };
    if a > 0 {
        s.to_string()
    } else {
        format!("{c}{}", f(a))
    }
}

trait T { fn declared(&self); }

#[cfg(test)]
mod tests {
    fn in_tests() {
        let a = 1;
        let b = 2;
    }
}
'''


def test_lengths_and_exclusions():
    found = {name: (line, n) for name, line, n in fn_lengths.functions(SRC)}
    assert found["short"] == (3, 1)
    assert found["long"] == (5, 13)
    assert "declared" not in found          # no body
    assert "in_tests" not in found          # test module left out


def test_braces_in_strings_and_comments_do_not_count():
    src = 'fn a() {\n    let _ = "}}}";\n    // }\n}\nfn b() {\n}\n'
    assert [(n, ln, c) for n, ln, c in fn_lengths.functions(src)] == [("a", 1, 4), ("b", 5, 2)]


def test_is_test_file():
    assert fn_lengths.is_test_file("crates/app/src/x/tests.rs")
    assert fn_lengths.is_test_file("crates/core/tests/a.rs")
    assert not fn_lengths.is_test_file("crates/core/src/detect/onset.rs")
