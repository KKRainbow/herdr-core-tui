# CPU optimization measurements

Measured on 2026-11-02 at 180 columns by 50 rows.
CPU is TUI process time divided by elapsed time; 100% is one core.
Core, proxy, and workload processes are excluded.
Each visible sample lasts eight seconds after one second of output warmup.
Each pane emits output at 50 Hz.
Scroll appends lines; repaint changes a fixed screen row.
Baseline and candidate run consecutively for each workload.
Short samples vary with host load and CPU frequency; these are local measurements, not guarantees.

## 1. Retain terminal rows

Cache text rows with shared ownership and extract only Ghostty dirty rows.
Keep image overlays separate and use stack storage for short graphemes.

| Workload | Panes | Baseline CPU % | Candidate CPU % | Relative saving |
|---|---:|---:|---:|---:|
| scroll | 1 | 14.21 | 9.28 | 34.7% |
| scroll | 15 | 30.18 | 27.74 | 8.1% |
| repaint | 1 | 11.25 | 6.92 | 38.5% |
| repaint | 15 | 24.81 | 23.16 | 6.7% |

Validation: formatting, Clippy, 62 unit tests, selection, 400 streaming resizes, and two-viewer geometry passed.
One existing workflow test requires a separate integration environment and is ignored by the unit suite.
Idle and hidden-output checks reported no redraws or parsed bytes.

Run `TUI_PERF_VISIBLE_ONLY=1 TUI_PERF_SECONDS=8 TUI_PERF_MODE=repaint python3 tests/performance.py` for visible repaint measurements.
Omit `TUI_PERF_VISIBLE_ONLY` to include idle and hidden-output checks.
Set `TUI_TEST_BINARY` to compare saved release binaries and `TUI_PERF_OUTPUT` to save JSON.

## 2. Invalidate layout and snapshots by event

Terminal bytes now invalidate terminal presentation only, except when alternate-screen changes require layout updates.

Compared with item1.
Negative savings indicate a higher sample, which can include measurement noise.

| Workload | Panes | Previous CPU % | Candidate CPU % | Relative saving |
|---|---:|---:|---:|---:|
| scroll | 1 | 11.49 | 11.85 | -3.1% |
| scroll | 15 | 27.36 | 19.27 | 29.6% |
| repaint | 1 | 7.17 | 6.18 | 13.8% |
| repaint | 15 | 26.83 | 16.84 | 37.2% |

Validation: formatting, Clippy, 62 unit tests, slow Git, pane exit, 400 streaming resizes, and two-viewer geometry passed.
The pane-exit fixture now uses a controlled shell and waits for its prompt before EOF.
Visible samples performed four or five snapshot/layout rebuilds across about 400 to 500 renders.

## 3. Borrow documents for reads

Add `Session::document_ref` and use it for TUI projection, layout, availability checks, and library read-only commands.
Keep `Session::document` as an owned copy for edits and existing consumers.
Validation: both repositories passed formatting, Clippy, and all enabled unit tests.
The library's isolated core integration suite also passed, including document conflicts and worktree operations.
The new test checks shared storage, unavailable documents, private edits, and unknown-field preservation.
CPU samples were inconclusive because unrelated host compilation raised load near 29 and changed redraw rates and RPC latency.
No incremental CPU saving is claimed for this step.

## 4. Batch framed transport I/O

Buffer reads and writes, encode each header together, and drain queued frames in bounded batches.
Merge adjacent credits for the same channel without adding a timer.
Preserve control priority and flush even a single credit immediately.
Validation: both check suites and the isolated core integration passed.
New transport tests verify one underlying write for a small batch, exact credit totals, ordering, bounded queue draining, overflow handling, and immediate sparse flushes.
The one-pane repaint samples were 5.59% before and 4.86% after, with 387 and 394 renders.
Other samples had materially different render counts under continued compiler load, so no reliable incremental CPU saving is claimed.
