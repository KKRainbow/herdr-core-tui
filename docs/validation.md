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
Attach failure is tested; a fault injected after terminal setup remains a separate validation task.

## Remaining validation

Workspace interaction, CLI targeting, packaging, and performance checks follow their implementation milestones.
Native macOS and Windows validation is pending.

## Workspace interface

Eight fast behavior tests pass, including weighted layouts, stack selection, Unicode editing, history search, frozen selection, hyperlinks, and Kitty image clipping.
The core workflow check passes creation, split, resize, swap, stale close rejection, worktree creation, and safe removal.
The unchanged app check passes TUI tab creation, mouse-driven rename, split, explicit close, independent app focus/zoom, and host restoration.
A synthetic OSC 7501 fixture passes status display and keyboard navigation for an agent without a pane.
The interface uses bounded scrollback and detaches hidden viewer streams.
