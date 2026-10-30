# Source provenance

The TUI is licensed under Apache-2.0.
The Ghostty build crate and terminal wrapper reference herdr-app revision `af2bd72c176df7b852df84a026c684dd6586f010`, paths `crates/ghostty-sys` and `src/vt.rs`.
The wrapper adapts cell extraction and input encoding without GPUI, app settings, or app services.
Ghostty is a pinned upstream submodule at `291fd050b9e9c293c8adb6a140742649b7e42d3a`; its own license remains in the submodule.
The library path dependency is a local development override.
A distributable dependency pin is required before release.
No original source checkout is modified.
