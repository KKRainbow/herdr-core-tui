# Future Libertty integration

Use `herdr-client-lib` as a Rust dependency behind Libertty's existing server boundary.
Retain browser rendering, authentication, SSH routing, dictation, and notification delivery.
Supply an authenticated transport to the client; keep one private `View` per browser connection and stable core identity.
Use `Session` snapshots and shared commands for spaces, tabs, panes, worktrees, and reported agent state.
Forward structured conflicts and partial outcomes instead of retrying side effects blindly.

Replace the old rendered-frame adapter with a core raw-stream adapter.
Return stream credit after the browser has consumed the bytes, and detach on cancellation.
Handle fresh repaint, core restart, and remote loss without relaunching agents.
Forward input, display facts, focus, and explicit resize activity through core's viewer protocol.
Validate mobile resize ownership and multiple viewers before migration.
This work requires new stream and browser integration; changing a dependency is not sufficient.
No Libertty source was changed for this project.
