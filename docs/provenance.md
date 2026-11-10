# Source provenance

The TUI is licensed under Apache-2.0.
The Ghostty build crate and terminal wrapper reference herdr-app revision `af2bd72c176df7b852df84a026c684dd6586f010`, paths `crates/ghostty-sys` and `src/vt.rs`.
The wrapper adapts cell extraction and input encoding without GPUI, app settings, or app services.
Ghostty is a pinned upstream submodule at `291fd050b9e9c293c8adb6a140742649b7e42d3a`; its own license remains in the submodule.
The library dependency pins a Git revision for ordinary builds.
The release bundle uses the library revision in `distribution/client-revision` and includes its sources.
No original source checkout is modified.
The workspace editor adapts Herdr `src/client/shell/text_editor.rs` at `ee5bfb5041bb3e4a49d591c72308aae3c81ecdde`.
The sidebar, pane navigation, and selection interactions reference that revision's `src/ui/sidebar.rs`, `src/client/shell/workspace_navigation.rs`, and `src/selection.rs`.
The new interface uses library documents and private views instead of the original server snapshots.
`src/shell` adapts Herdr's client shell at that revision.
The palette comes from `src/app/state.rs`; the keymap and keybind help from `src/config/model.rs` and `src/input/keybind_help.rs`.
The sidebar, agents panel, tab bar, mode bar, and list scrolling adapt `src/client/shell/{sidebar,agent_sidebar,tabs,render,scroll}.rs` and `src/ui/sidebar*.rs`.
Menus, prompts, keybind help, the navigator, and worktree dialogs adapt `src/client/shell/{overlays,worktree_overlays,context_menu,global_menu,aggregate_navigation}.rs`.
Pane borders, titles, scrollbars, and selection colors adapt `src/ui/panes.rs` and `src/ui/scrollbar.rs`.
Mode and overlay input adapt `src/client/shell/{input,overlay_input,mouse,actions}.rs`.
Space and tab drag reorder adapts the chrome drag in `src/client/shell/mouse.rs`.
Notification sounds adapt `src/sound.rs` and the state rules in `src/app/actions.rs`; `assets/sounds` holds Herdr's unchanged `done.mp3` and `request.mp3`.
Kitty image extraction, PNG decoding, and hyperlink lookup adapt the pinned app's `src/vt.rs`.
Images use clipped half-block cells rather than app GPU textures.
