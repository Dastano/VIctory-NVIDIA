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
| r02 | `VICTORY_FILL_TITLE=1` | 23.1 / 20.5 | 23.4 | 29.5 | 13.8 | 10.5 | 7.9 |
| r03 | `VICTORY_CARD_WINDOWS_MB=4096` | 23.8 / 12.0 | 24.3 | 32.1 | 10.0 | 11.7 | 0.2 |
| r04 | `VICTORY_INPLACE=1` | 23.6 / 20.8 | 23.8 | 30.2 | 9.8 | 13.2 | 2.2 |
| r05 | `VICTORY_LAND_SKIP=1`, `VICTORY_WIDE_SPIN=50` | 24.2 / 14.2 | 24.7 | 30.2 | 9.4 | 12.6 | 0.1 |

| r07 | mimalloc (branch `perf/mimalloc`, PR trustno1-coder/VIctory-NVIDIA#11) | 26.4 / 16.7 | **26.9 (min 26.0)** | 26.9 | 10.0 | 10.7 | 0.7 |
| r08 | mimalloc + `VICTORY_UNDER_THREADS=12`, `VICTORY_LAND_THREADS=8` | 26.4 / 15.4 | 26.9 (min 25.6) | 27.3 | 10.2 | 10.2 | 0.3 |

| r09 | mimalloc + profile | 26.2 / 15.1 | 26.9 (min 26.0) | 26.6 | 9.5 | 10.9 | 0.8 |
| r10 | mimalloc, process pinned to CCD0 (V-cache, CPUs 0-15) | 26.2 / 13.9 | 26.8 (min 26.2) | 28.5 | 8.9 | 13.9 | 0.9 |
| r11 | mimalloc, pinned to CCD1 (frequency, CPUs 16-31) | 23.4 / 18.2 | 22.9 (min 18.2) | 31.6 | 9.2 | 16.3 | 2.2 |
| r12 | mimalloc + `VICTORY_JOIN_PARTS=16`, `VICTORY_JOIN_LISTS=96` | 26.6 / 15.9 | 27.2 (min 26.6) | 26.7 | 9.7 | 10.8 | 1.1 |
| r13 | mimalloc + `-Quality` (RELAY=1, CARD_STEPS=0, TEXTURE_NO_WAIT=0) | 17.7 / 8.9 | 18.2 (min 17.5) | 26.4 | 14.6 | 7.8 | 27.4 |
| r14 | mimalloc + `VICTORY_CARD_STEPS=1` | 22.3 / 16.7 | 22.7 (min 21.4) | 27.1 | 12.4 | 9.5 | 13.2 |
| r15 | mimalloc + refresh_front spans split at 1 MiB (experiment, reverted) | title crashed at 151 s (write to 0 in title code) | – | | | | |
| r16 | mimalloc + split at 256 KiB (reverted) | 24.1 / 15.2 | 24.7 (min 24.2) | 33.5 | 17.6 | 11.2 | 0.2 |

| r17 | mimalloc + `VICTORY_FAST_CORES=3` | 26.6 / 15.1 | 27.2 (min 26.2) | 27.1 | 10.3 | 10.7 | 0.5 |
| r18 | mimalloc + `VICTORY_TITLE_PRIORITIES=1` | 26.5 / 21.4 | 27.0 (min 25.8) | 26.7 | 9.8 | 10.6 | 1.0 |
| r19 | mimalloc + `VICTORY_WIDE_SPIN=200` | 25.9 / 4.9 | 25.9 (min 4.9) | 27.5 | 9.9 | 11.1 | 0.5 |
| r20 | mimalloc + `VICTORY_LATE_WAIT_LISTS=1024` | 23.2 / 0.8 | 25.2 (min 18.9) | 28.6 | 9.6 | 12.7 | 1.5 |

From r21 on the runs also turn the camera in the jail scene (right stick, 4 s every 20 s from 280 s), where the fps drop.

| r21 | mimalloc + `VICTORY_REFRESH_FEW=1000000` (look for "copies nothing" before spreading), camera turns | 25.5 / 15.7 | 25.4 (min 23.7) | 28.1 | 11.0 | 10.9 | 1.4 |
| r22 | mimalloc, camera turns (the base with turns) | 25.3 / 14.9 | 25.0 (min 22.6) | 26.3 | 9.2 | 10.8 | 2.3 |
| r23 | mimalloc + `VICTORY_TEXTURE_ROOM_PERCENT=96`, `VICTORY_TEXTURE_STAGE_MB=768`, turns | 25.9 / 22.6 | 25.3 (min 23.6) | 27.1 | 9.6 | 11.1 | 1.7 |
| r24 | mimalloc + `VICTORY_MAP_THREADS=4`, turns | 25.1 / 12.4 | 25.1 (min 24.1) | 29.6 | 10.7 | 11.6 | 0.5 |

| r25 | release build (no diagnostics) + mimalloc, skip list and keys enabled in release (branch `local/rtx4090`), turns | 25.6 / 23.0 | 24.8 (min 23.0) | 23.7 | 8.4 | 10.8 | 5.3 |
| r26 | r25 + `VICTORY_FILL_TITLE=1` | 24.6 / 16.6 | 24.7 (min 22.7) | 26.1 | 11.4 | 10.4 | 8.5 |
| r27 | r25 + `VICTORY_REFRESH_FEW=1000000` | 25.6 / 16.3 | 25.2 (min 23.0) | 25.7 | 9.7 | 10.7 | 3.3 |

Release vs diag: preparing 3.4 ms shorter, but the submitting thread then waits for the card (5.3 ms, "title memory read" settles 5.1 ms): the frame stays ~39 ms. In r25 the submit lane worked 35.2 ms a frame; title threads waited 9.9 ms a frame at presents for it.

None of r02-r05 beats r01 beyond run-to-run noise (~1 fps); FILL_TITLE is worse here (the card is waited for 7.9 ms a frame instead of 0.2).

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

### Profile of the submitting thread (r06, VICTORY_PROFILE, 30 s, 14,823 samples)

Inclusive: shadow::wide / refresh_front 24 % (12 % Condvar waits for its helpers), stale_under_flights 14 % (90 % of it merge_under_writes, half of that waiting in land::Pool::each), heap (ntdll under realloc/alloc) 15 %, take_steps 12 %, write-watch askings ~8 %, contended mutexes 5 %; ntdll self 51 %, NVIDIA driver 11 %. The heap share led to mimalloc (r07: -3 ms preparing, +2 fps). More under-/land threads (r08) change nothing.

### Visual check

- r00 (pipelines made live): in the diner cutscene the heads were missing (teeth and eyes
  floating). r01: that cutscene was skipped by the scripted Enter; the jail scene is clean.
  Open: whether the heads are drawn once pipelines are warm.


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
