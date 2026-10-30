# Initial terminal slice validation

Validated on Linux with installed `hc` 0.1.14.
The library passes `just check`: 22 tests, formatting, and lint.
The TUI passes `just check`: two terminal behavior tests, formatting, and lint.
The tests cover Unicode cells, true color, cursor position, alternate screen, resize, application cursor keys, bracketed paste, and SGR mouse encoding.
The isolated PTY check covers initial repaint, normal detach, a second attach to the same live process, and host settings after normal exit and an attach error.

Step 7 is not complete.
Automatic reconnect, core geometry events, enhanced keyboard negotiation, and graphical app interoperability remain unverified or unimplemented.
Error cleanup after a failure during an active attachment needs a dedicated failure test.
Native macOS and Windows validation and performance profiling remain pending.
