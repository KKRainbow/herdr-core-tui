# Validation

Validated on Linux on 2026-10-29 with hc 0.1.14 and unchanged installed herdr-app 0.1.8.
The app ran on a private headless Weston display with scratch app and core state.
The installed app matches the source baseline version; its exact build revision was not verified.
The library's earlier interoperability run also used an unchanged build of pinned app revision `af2bd72c176df7b852df84a026c684dd6586f010`.
Original Herdr, app, core, and Libertty source checkouts were not changed.

| Check | Result |
|---|---|
| Formatting, Clippy, fast behavior tests | Passed: nine tests |
| Rust 1.89.0 | All targets checked |
| PTY lifecycle | Repaint, repeated attach, detach, core restart, kept process, host restoration |
| Workspace workflows | Creation, split, resize, swap, stale close rejection, worktree creation/removal |
| Unchanged app | App-created terminal, TUI input, TUI-created tab, mouse rename, split, close |
| Independent views | App focus/zoom unchanged; two TUI instances retain separate view state |
| CLI | Headless shared operations, explicit live instance targeting, endpoint cleanup |
| Host errors | Attach failure and injected save error restore terminal settings |
| Agent reports | Synthetic OSC 7501 status and keyboard navigation without a pane |
| Unknown content | Mod placeholder and lossless unrelated edits preserve opaque properties |
| Completed terminal | Retained output; no repeated attach or redraw loop |
| Geometry | Two different host sizes, input transfers ownership, no idle resize contention |
| Remote machine | Unavailable layout, rejected offline mutation, refreshed offline edit, same process after reconnect |
| Performance | One and fifteen populated panes; idle, visible output, hidden output |
| Independent package | Vendored library and Ghostty; optimized build outside checkout; scratch-prefix install |
| Native macOS and Windows | Not run; not advertised as supported |

Terminal tests cover Unicode width, colors, cursor, alternate screen, resize, application cursor keys, bracketed paste, SGR mouse, control keys, and Kitty release events.
UI tests cover weighted layouts, stack selection, grapheme editing, retained-history search, frozen selection, wide cells, hyperlinks, and Kitty image clipping.
The final event-stream fix handles multiple notifications per frame and notifications split across frames.
Its library regression test also covers cancellation and exact credit return.

The pinned library tests cover CAS conflicts, spawn/save reconciliation, lost-kept-shell recovery, partial Git/terminal/layout outcomes, changed removal preconditions, remote Git, and bounded slow consumers.
The real-core library workflow suite was rerun after the notification decoder change.
See the library's `docs/validation.md` for those checks.

Remote availability is polled every two seconds when remote documents are watched.
Core does not send a document event for a transport disconnect.
A disconnected machine's layout becomes unavailable; cached terminal records are not treated as exited.
Private views use stable core identities and separate instance state files.

See [performance results](performance.md) for measurement scope, bounded retention, and raw results.
See [the Libertty integration map](libertty.md) for future consumer work.

## Limits

The local control endpoint uses Unix sockets; Windows requires a native transport.
Native macOS behavior is unvalidated.
Clipboard writes depend on host OSC 52 support.
Kitty graphics use clipped half-block cells; virtual placements and pixel-perfect rendering are unsupported.
Selection is limited to one retained viewport page at a time.
Completed terminals use core's retained text when no live stream is available.
Host restoration is guaranteed for handled exits and panics, not SIGKILL or a host terminal crash.
