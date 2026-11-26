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
