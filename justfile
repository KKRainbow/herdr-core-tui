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

workflows-integration:
    cargo test workspace_and_worktree_workflows -- --ignored

app-integration: build
    python3 tests/app_terminal.py
    python3 tests/workspace_app.py
    python3 tests/agents_app.py
