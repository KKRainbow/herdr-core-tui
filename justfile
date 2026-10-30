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

package output:
    python3 scripts/package.py {{quote(output)}}

release:
    cargo build --release --locked

install prefix: release
    install -Dm755 target/release/herder-tui {{quote(prefix)}}/bin/herder-tui

cli-integration: build
    python3 tests/cli.py

msrv:
    cargo +1.89.0 check --all-targets --locked

notices:
    python3 scripts/notices.py

remote-integration: release
    python3 tests/remote.py

geometry-integration: release
    python3 tests/geometry.py

profile: release
    python3 tests/performance.py

compatibility-integration: release
    python3 tests/compatibility.py
