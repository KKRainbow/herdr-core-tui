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
