# Source provenance

The TUI is licensed under Apache-2.0.
The Ghostty build crate and terminal wrapper reference herdr-app revision `af2bd72c176df7b852df84a026c684dd6586f010`, paths `crates/ghostty-sys` and `src/vt.rs`.
The wrapper adapts cell extraction and input encoding without GPUI, app settings, or app services.
Ghostty is a pinned upstream submodule at `291fd050b9e9c293c8adb6a140742649b7e42d3a`; its own license remains in the submodule.
The library path dependency is a local development override.
A distributable dependency pin is required before release.
No original source checkout is modified.
The workspace editor adapts Herdr `src/client/shell/text_editor.rs` at `ee5bfb5041bb3e4a49d591c72308aae3c81ecdde`.
The sidebar, pane navigation, and selection interactions reference that revision's `src/ui/sidebar.rs`, `src/client/shell/workspace_navigation.rs`, and `src/selection.rs`.
The new interface uses library documents and private views instead of the original server snapshots.
Kitty image extraction, PNG decoding, and hyperlink lookup adapt the pinned app's `src/vt.rs`.
Images use clipped half-block cells rather than app GPU textures.
