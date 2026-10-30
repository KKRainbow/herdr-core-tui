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
Add a terminal address to use the single-terminal viewer; Ctrl+Q quits it.
Set `HC_BINARY` to select a core executable.
Detaching leaves shared terminal programs running.

## Interface

The interface follows Herdr: a sidebar with spaces above and agents below, a tab bar above the panes, and Herdr's menus, prompts, and mode bar.
Spaces show their state dot, name, and Git branch.
Linked worktrees appear under their main checkout; click the arrow to collapse the group.
The agents list shows terminals that report OSC 7501 status, including ones without a pane.
Click `grouped` to sort agents by priority instead.
Panes have borders only when a tab shows several; the focused pane's border uses the accent color.

## Controls

Herdr's default keymap applies.
Press Ctrl+B, then a key.
Press prefix+? for the full keybind list and prefix+q to detach.

| Key | Action |
|---|---|
| shift+n / shift+w / shift+d | New / rename / close workspace |
| shift+g | New worktree |
| c / shift+t / shift+x | New / rename / close tab |
| n / p / 1..9 | Next / previous / numbered tab |
| v / minus | Split vertical / horizontal |
| h j k l | Focus an adjacent pane |
| shift+h j k l | Swap with an adjacent pane |
| tab / shift+tab | Cycle panes |
| x / shift+p / z | Close / rename / zoom pane |
| r | Resize mode |
| [ | Copy mode |
| w | Workspace navigation |
| g | Go to any space, pane, agent, or terminal without a pane |
| b | Collapse the sidebar |

Click spaces, tabs, and agents to navigate.
Click `new` for a workspace, `menu` for keybinds, reload, and detach, and `+` for a tab.
Right-click a space, tab, or pane for its menu.
Space menus offer worktree creation, opening, and removal in Git repositories.
Pane menus also park the terminal without ending it.
Right-click an agent without a pane to keep it in a new tab.
Shift+right-click sends the click to the program.
Drag pane borders, the sidebar edge, or the line above the agents to resize.
Drag in a pane to select text when the program does not use the mouse; release copies it with OSC 52.
Shift+drag selects in programs that use the mouse.
Ctrl+click opens an HTTP or HTTPS link with `xdg-open` on Linux.
The mouse wheel scrolls history when the program does not use mouse reporting.

In copy mode, use h/j/k/l and w/b/e to move, v or Space to select, and y or Enter to copy.
Use PageUp and PageDown for history, / to search retained history, n to repeat, and q or Esc to leave.
The host terminal must allow OSC 52 for clipboard writes.
Paste through the host terminal's paste command.

Closing a workspace asks for confirmation.
Closing a tab or pane asks only when it would close the workspace.
Worktree removal revalidates the checkout before it deletes it.

## Configuration

Read preferences from `$XDG_CONFIG_HOME/herder-tui/config.json`, or `~/.config/herder-tui/config.json`.
Set `theme` to a Herdr theme name such as `catppuccin`, `tokyo-night`, `dracula`, `nord`, or `gruvbox`.
Set `status_indicators` to `dots` or `symbols`, `agent_panel_sort` to `spaces` or `priority`, and `sidebar_width`.
Set `keys` to map Herdr `[keys]` action names to bindings, for example `{"next_agent": "prefix+a"}`.
An empty binding disables an action.
Omitted settings use Herdr's defaults.
Use the menu's reload config to apply changes.
View state and sidebar layout use the matching XDG state directory under `herder-tui`.
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
`just discovery-integration` verifies hc session defaults and terminal discovery.
`just chrome-integration` verifies the Herdr chrome and mouse workflows.
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
