# herder-tui

An independent terminal client for Herdr Core.
This first slice attaches to one existing terminal through `herdr-client-lib`.
It does not start or stop terminal programs.

Build with Rust, libclang, and Zig 0.16.
Initialize the pinned Ghostty submodule with `git submodule update --init`.
Keep `herdr-client-lib` beside this repository for the current development dependency.
Set `ZIG` if Zig is not on PATH.
Run `just check` and `just build`.

Run `target/debug/herder-tui --session SESSION TERMINAL`.
The session is required.
Set `HC_BINARY` to select a core executable.
Press Ctrl+Q to detach.
Ctrl+C is sent to the terminal program.
Mouse wheel scrolls local history when the program does not use mouse reporting.

This is an initial implementation of PRD step 7.
Automatic reconnect, shared geometry updates, enhanced host keyboard negotiation, and graphical app interoperability validation remain open.
A stream closure exits the viewer; run it again to attach to the same address.
Spaces, tabs, panes, dialogs, configuration, and release packaging follow in later steps.
Linux is the development platform; macOS and Windows runtime checks are pending.

See [source provenance](docs/provenance.md).

Run `just integration` with `hc` on PATH for the isolated Linux PTY check.
The check creates scratch core state and stops only its test session.
