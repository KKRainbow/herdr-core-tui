# Performance validation

Measured on Linux on 2026-10-29 with the optimized TUI and hc 0.1.14.
The host PTY was 180 columns by 50 rows.
Each fake terminal emitted 50 lines per second during output phases.
Each measurement lasted approximately five seconds after startup or view transition.
Hidden-phase measurements selected a quiet tab while the original terminals continued output.
CPU is the TUI process time divided by elapsed time; 100% means one CPU core.
RSS excludes core and child processes.
RPC latency measures `terminal.list` through a separate connection to the same core.
These are local fixture measurements, not throughput guarantees.

| Panes | Phase | CPU % | RSS MiB | Redraws | Parsed bytes | RPC p95 ms |
|---|---|---:|---:|---:|---:|---:|
| 1 | idle | 0.0 | 6.67 | 0 | 0 | 1.95 |
| 1 | visible | 10.67 | 7.25 | 251 | 19829 | 2.04 |
| 1 | hidden | 0.0 | 7.64 | 0 | 0 | 1.86 |
| 15 | idle | 0.0 | 7.49 | 0 | 0 | 4.52 |
| 15 | visible | 17.55 | 10.4 | 313 | 294433 | 3.67 |
| 15 | hidden | 0.0 | 13.63 | 0 | 0 | 4.42 |

Idle and hidden-output phases produced no redraws or parsed terminal bytes.
The process used no CPU time measurable by the `/proc` tick sampler in those phases.
The fifteen-pane output run used about 18% of one CPU core after redraw coalescing, versus about 53% before it.
Output parsing and credit return continue between frames; rendering is capped near 60 Hz.

Each VT bounds history to 10,000 lines and 10 MB, and image storage to 16 MB.
Hidden streams detach; the cache prunes hidden surfaces when it exceeds 32 entries.
Visible panes can exceed that cache threshold.
The profile retains prior visible VTs during its hidden phase, so RSS need not fall when streams detach.
The library bounds queued stream bytes, frame size, pending calls, and control queues.
Its regression tests cover unread streams, overflow isolation, detach cycles, deadlines, and RPC progress.
No long-duration memory leak claim is made from this short profile.

Run `just profile` to repeat the measurements with isolated scratch cores.
Set `TUI_PERF_SECONDS` to extend each sample and `TUI_PERF_OUTPUT` to save JSON.
Use `TUI_PERF_PANES` to select a single pane count.
