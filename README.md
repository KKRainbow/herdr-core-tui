# herder-tui

An independent terminal client for Herdr Core.
Shared terminals and workspace layouts remain compatible with herdr-app.
The TUI keeps focus, zoom, selection, and preferences private.

## Build

Use Rust, libclang, and Zig 0.16.
Run `git submodule update --init` to obtain the pinned Ghostty source.
Keep `herdr-client-lib` beside this repository for development.
Set `ZIG` if Zig is not on PATH.
Run `just check` and `just build`.

## Run

Run `target/debug/herder-tui --session SESSION` for the workspace interface.
Add a terminal address to use the single-terminal viewer.
The core session is required.
Set `HC_BINARY` to select a core executable.
Quitting detaches viewers and leaves shared terminal programs running.

## Controls

Press Ctrl+B, then a command key.
Press Ctrl+Q to quit.
Ctrl+C goes to the focused program.

| Key | Action |
|---|---|
| s / t | New space / tab |
| % / " | Split right / down |
| h j k l or arrows | Focus an adjacent pane |
| p / n / S | Next pane / tab / space |
| a | Next reported agent, including one without a pane |
| z | Toggle private zoom |
| r / x | Rename / close pane |
| [ | Enter copy mode |
| w | Create a worktree |
| m | Open the actions menu |
| : | Open a named command |
| ? | Help |

Click spaces, tabs, and agents to navigate.
Right-click a pane for the actions menu.
Shift+right-click sends the click to the program.
Drag a pane's right border to change its weight.
Shift+drag selects text; release copies it with OSC 52.
Ctrl+click opens an HTTP or HTTPS link with `xdg-open` on Linux.
Mouse wheel scrolls history when the program does not use mouse reporting.

In copy mode, use arrows or hjkl to move, Space to anchor, and y to copy.
Use PageUp and PageDown for history, / to search retained history, n to repeat, and Esc to leave.
A selection freezes its displayed viewport while output continues.
Moving to another history page starts a new selection.
The host terminal must allow OSC 52 for clipboard writes.
Paste through the host terminal's paste command.

Dialogs support Tab, Shift+Tab, cursor keys, mouse field selection, and Unicode text editing.
Enter submits; Esc cancels.
Close and worktree removal dialogs require confirmation and revalidate the reviewed targets.
The actions menu includes pane move/swap/resize, space/tab ordering, terminal keep/park, and worktree discovery/open/removal.
Use Select machine to view a core-managed remote machine.

## Configuration

Read preferences from `$XDG_CONFIG_HOME/herder-tui/config.json`, or `~/.config/herder-tui/config.json`.
Set `sidebar_width` and a `keys` object mapping Ctrl+B command keys to action names.
Omitted settings use defaults.
View state uses the matching XDG state directory under `herder-tui`.
The TUI does not write the app's private view or preferences.

## Terminal support

The viewer follows core size events and reconnects after transport loss or kept-terminal recovery.
It does not relaunch an agent.
Ghostty handles colors, Unicode, alternate screens, keyboard modes, mouse reporting, and bracketed paste.
Kitty graphics use clipped half-block rendering, including decoded PNG images.
Virtual image placements and pixel-perfect graphics are not supported.
Scrollback is bounded to 10,000 lines and 10 MB per cached terminal; image storage is bounded to 16 MB.
The workspace keeps at most 32 hidden terminal caches and detaches hidden streams.
Linux is the validated platform; native macOS and Windows checks are pending.

## Checks

`just check` runs fast behavior tests, formatting, and lint.
`just integration` uses `hc` and an isolated scratch core for PTY checks.
`just workflows-integration` requires `HC_TUI_TEST_ROOT` under `/tmp` for Git and workspace workflows.
`just app-integration` requires an isolated running app fixture, `TUI_APP_TEST_ROOT`, `TUI_APP_TEST_TERMINAL`, and `HC_SESSION`.
Do not point integration tests at live work.
See [validation](docs/validation.md) and [provenance](docs/provenance.md).
