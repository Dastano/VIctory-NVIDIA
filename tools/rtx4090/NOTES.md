# RTX 4090 performance notes

Machine: RTX 4090 24 GB (driver 621.14), Ryzen 9 7950X3D, Windows 11. Title 1.0.2.1.
Goal: a stable 30 fps in the first in-game scene with no visual faults.

All runs: `tools/rtx4090/bench.ps1` (unattended; Enter every 4 s from 60 to 200 s through the
start screens, window-only screenshots every 30 s, 420 s). "world" = 5-s fps lines with more than
1,000 draws a frame, the first 12 left out; "last 2 min" = the last 24 such lines (the jail scene).
Pipelines warmed with `victory warm` before the runs (r01 on).

## Results

| Run | Change on top of `play-nvidia.ps1` defaults (Series S, half scale) | world fps mean / min | last 2 min fps | prep ms | copy ms | back ms | GPU wait ms |
|---|---|---|---|---|---|---|---|
| r00 | (pipelines made live, book written) | 25.7 / 19.7 | – | 28.3 | 9.9 | 11.1 | 0.5 |
| r01 | none (warm cache) | 24.6 / 17.6 | 24.7 (min 23.8) | 30.1 | 10.6 | 11.8 | 0.2 |

The card waits 0.2-0.5 ms a frame: the frame is bound by the CPU's preparing.

Per frame (copystat, r00): 16 submissions; ~21,900 refreshes over 16.6 GB of ranges; 316 write-watch
askings, 14.5 ms, 8.1 GB checked of which 8.1 GB unchanged; moves 6.2 ms.

### Where the frame goes (r01, jail scene, per frame)

- The critical path is the one thread that submits: 16.0 submissions a frame, inside them 22.9 ms
  user + 5.7 ms kernel, **5.2 ms waiting for the submission lock**, 3.7 ms between them: ~37 of
  the ~40 ms frame.
- The 8 recording threads: ~25 ms of list calls each, side by side (not the bound).
- "lists reset in 12.4 ms"; 5 ms from the last submission to the present call.
- ~21,500 refreshes over ~16.9 GB of ranges; ~320 write-watch / guard askings (14-17 ms, all
  threads) over ~8.3 GB of which 99.5 % unchanged; moves ~6 ms.

Note on the "5.2 ms waiting for the submission lock": it is measured from the call of
`Host::submit` (host.rs:5458) to `STARTED` (host.rs:5535), which includes `card_steps` and
`settle` / `relay_wait` for flights whose writes the submission reads (host.rs:5476-5494). So it
is largely waiting for shader writes to land, not contention. `VICTORY_FILL_TITLE=1` targets
exactly such waits (another fork measured 15-24 ms a frame less): first in the queue.

So 30 fps needs about 7 ms off the submitting thread: the lock wait and the refresh checks are
the targets, not the card (0.2 ms of waiting) and not shader translation.

### Visual check

- r00 (pipelines made live): in the diner cutscene the heads were missing (teeth and eyes
  floating). r01: that cutscene was skipped by the scripted Enter; the jail scene is clean.
  Open: whether the heads are drawn once pipelines are warm.

### Open runs (not done: the queue was stopped for low system memory; 32 GB RAM, the title
takes 18-36 GB)

| Run | Switch |
|---|---|
| r02 | `VICTORY_FILL_TITLE=1` |
| r03 | `VICTORY_CARD_WINDOWS_MB=4096` |
| r04 | `VICTORY_INPLACE=1` |
| r05 | `VICTORY_LAND_SKIP=1`, `VICTORY_WIDE_SPIN=50` |

## CUDA probe (`cuda_probe.c`, driver API from nvcuda.dll, no toolkit)

| Case | cuMemHostRegister | DMA 64 MiB host->device |
|---|---|---|
| VirtualAlloc, committed | ok | 2.5 ms (26 GB/s) |
| section, SEC_COMMIT view | ok | 2.5 ms |
| section, SEC_RESERVE view, pages committed (how the page pool is made) | ok (D3D12 refuses this) | 2.6 ms |
| pages already PAGE_READONLY (the view guards) | fails (rc 2) | – |
| a second view of the SEC_RESERVE section registered; the first view then guarded read-only | ok, VirtualProtect on the first view still works, DMA sees the first view's later writes (16,384 of 16,384 pages) | 2.6 ms |
| CPU memcpy, for comparison | – | 2.7 ms |

So CUDA can read the page pool by DMA through a view of our own, beside the title's guarded view.
It is not faster than memcpy; what it could buy is CPU time, if the copies are bandwidth and not
bookkeeping. The measurements above say the bookkeeping (write-watch askings over unchanged
memory) is the larger part, so that comes first.
