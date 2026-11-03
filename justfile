# Task runner for drumspike. Commands are copied from README.md, docs/DEVELOPING.md and
# .github/workflows/ci.yml. `just --list` shows the recipes. P3-12: the Python server is retired;
# Python runs the tests only.

default:
    @just --list

# The Rust tools the Python tests call (parity, service_tool) and the test bridge
tools:
    cargo build --release -p drumspike-devtools
    cargo build -p drumspike-devtools --bin ds-bridge

# Fast loop: the golden and other tests without the browser tests
fast: tools
    uv run pytest -m "not e2e_app"

# G4: app e2e tests through the test bridge (P3-7a); fails without the binary
e2e-app:
    cargo build -p drumspike-devtools --bin ds-bridge
    DRUMSPIKE_AUDIO=null DRUMSPIKE_REQUIRE_APP=1 uv run pytest -m e2e_app --no-cov

# G2: the full suite (one mode since P3-12; the JS coverage gate included)
test: tools
    DRUMSPIKE_AUDIO=null DRUMSPIKE_REQUIRE_APP=1 DRUMSPIKE_REQUIRE_CORE=1 uv run pytest -q

# Rust tests
rust:
    cargo test --workspace

# Format and lint, as in CI
lint:
    cargo fmt --all -- --check
    cargo clippy --all-targets -- -D warnings

# Benchmark: 30 tracks, 5 min, 48 kHz
bench:
    cargo build --release -p drumspike-devtools
    ./target/release/bench

# Everything CI runs, in order
ci: lint rust tools
    uv sync
    uv run playwright install chromium
    DRUMSPIKE_AUDIO=null DRUMSPIKE_REQUIRE_APP=1 DRUMSPIKE_REQUIRE_CORE=1 uv run pytest -q
