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

Run `target/debug/herder-tui` for the workspace interface on the same core as `hc`.
The session defaults to `HC_SESSION`, or `default` when that variable is unset or empty.
Use `--session SESSION` to select another core session and `--instance NAME` to name the viewer.
The instance name is optional for interactive use and required for live control.
Add a terminal address to use the single-terminal viewer.
The sidebar shows the selected session.
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
| e | Next live terminal, including one without a pane or reported status |
| z | Toggle private zoom |
| r / x | Rename / close pane |
| [ | Enter copy mode |
| w | Create a worktree |
| m | Open the actions menu |
| : | Open a named command |
| ? | Help |

Click spaces, tabs, and terminals to navigate.
The terminal list includes ordinary running programs that do not report agent status.
Opening an unplaced terminal previews it without changing the shared layout.
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
Linux is the supported platform for this release.
Native macOS is unvalidated; Windows also needs a native local control transport.

## Checks

`just check` runs fast behavior tests, formatting, and lint.
`just integration` uses `hc` and an isolated scratch core for PTY checks.
`just workflows-integration` requires `HC_TUI_TEST_ROOT` under `/tmp` for Git and workspace workflows.
`just app-integration` requires an isolated running app fixture, `TUI_APP_TEST_ROOT`, `TUI_APP_TEST_TERMINAL`, and `HC_SESSION`.
Do not point integration tests at live work.
`just remote-integration`, `just geometry-integration`, and `just compatibility-integration` use new scratch cores.
`just cli-integration` verifies headless operations and multiple live instances.
`just profile` measures one and fifteen output-producing panes.
See [validation](docs/validation.md), [performance](docs/performance.md), and [provenance](docs/provenance.md).

## CLI and packaging

Run `herder-tui --session SESSION do help` for shared command names and arguments.
Pass arguments as one JSON object.
Structural commands work while no TUI is open and require explicit target IDs.
For example, `herder-tui --session work do tab.rename '{"id":"tb123","name":"Tests"}'` changes the shared tab.
A close command first returns its affected terminals; submit again with `confirm: true` only when you intend to end them.
Keep the returned review in `expected` to reject changed confirmation targets.

Run `herder-tui --session SESSION instances` to list live viewers.
Run `herder-tui --session SESSION --instance NAME view state` to inspect one viewer.
Live view commands include `pane.focus`, `tab.focus`, `space.focus`, `terminal.focus`, `agent.focus`, `zoom`, `machine`, and `quit`.
Pane, tab, and space focus take an `id`; terminal and agent focus take a `terminal` address.
The control socket is private to the current user and disappears when the viewer exits.
Each named instance has its own saved view.
Use `hc` for terminal automation and machine management.

Run `just install /chosen/prefix` to build an optimized binary and install it under that prefix's `bin` directory.
This does not replace the installed app or core.
Run `just package /path/herder-tui-source.tar.gz` to export the staged source tree with pinned Ghostty and library sources.
The source bundle needs no sibling checkout and already includes its submodules.
Run `just package-check` to export the staged source, verify its library path, and build and test it in a temporary directory.
`distribution/client-revision` selects the library revision; `SOURCE-REVISIONS` records the bundle inputs.
The development checkout keeps its sibling path dependency.
Nothing is published or pushed by these recipes.
