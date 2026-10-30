fmt:
    cargo fmt --all

lint:
    cargo fmt --all -- --check
    cargo clippy --all-targets --locked -- -D warnings

test:
    cargo test --all-targets --locked

check: lint test

build:
    cargo build --locked

integration: build
    python3 tests/integration_pty.py
