# Validation

## Single-terminal viewer

Validated on Linux with installed `hc` 0.1.14 and unchanged installed `herdr-app` 0.1.8.
The app test ran on a private headless Weston display with scratch app and core state.
The installed app version matches the source baseline version; its exact build revision was not verified.
The TUI passes formatting, lint, and three terminal behavior tests.
The tests cover Unicode width, true color, cursor, alternate screen, resize, application cursor keys, bracketed paste, SGR mouse encoding, control keys, and Kitty release events.
The isolated PTY check covers repaint, detach, repeated attach, host settings, and reconnect after core restart with a kept terminal.
The app check covers an app-created terminal, TUI input to the same process, and app retention after TUI detach.
Host cleanup uses a drop guard for returned errors and a panic hook.
Attach failure and a fault injected after terminal setup both restore the host terminal.

## Remaining validation

Workspace interaction, CLI targeting, packaging, and performance checks follow their implementation milestones.
Native macOS and Windows validation is pending.

## Workspace interface

Eight fast behavior tests pass, including weighted layouts, stack selection, Unicode editing, history search, frozen selection, hyperlinks, and Kitty image clipping.
The core workflow check passes creation, split, resize, swap, stale close rejection, worktree creation, and safe removal.
The unchanged app check passes TUI tab creation, mouse-driven rename, split, explicit close, independent app focus/zoom, and host restoration.
A synthetic OSC 7501 fixture passes status display and keyboard navigation for an agent without a pane.
The interface uses bounded scrollback and detaches hidden viewer streams.

## CLI and independent packaging

Rust 1.89 checks all TUI targets successfully.
The source bundle builds with its vendored pinned library and Ghostty outside the checkout.
The optimized installed binary passes the CLI integration test from a scratch prefix.
Headless structural commands work without a TUI.
Two live instances retain different focus and zoom, require explicit targeting, and remove their control endpoints on exit.
An injected save error after terminal setup restores the host settings and leaves shared terminals alive.
Rust dependency notices and license texts are included in the source bundle.
The local control transport currently uses Unix sockets; Windows needs a native transport before support can be advertised.
