# MASTER build on an RTX 4090 (foxhound base)

Machine: RTX 4090 24 GB (driver 621.14), Ryzen 9 7950X3D, 32 GB DDR5, Windows 11. Title 1.0.2.1.
Goal: a stable 30 fps in the first in-game scene (the jail cell) with no visual faults. **Reached in r39.**

## What `master` is

Base: [readmet3xt-lab/foxhound](https://github.com/readmet3xt-lab/foxhound) (DLSS SR + Frame Generation through
NVIDIA Streamline, renderer port 1, submitting-thread work, Belobug ports, Series X at 2560x1440). On top:

| Branch | From | What | Default |
|---|---|---|---|
| mimalloc (2 commits) | trustno1-coder/VIctory-NVIDIA#11 | Rust allocations through mimalloc instead of the process heap | on |
| `port/guard` | [yaak192/Victory-Lap-NVIDIA](https://github.com/yaak192/Victory-Lap-NVIDIA) play13, renderer port 6 | the write guards' lock shared by faults and refreshes, a claim per 64 KiB block; refresh planning on side threads | on in `play-foxhound.ps1` (`VICTORY_PORT_GUARD=1`) |
| `port/cbv` | Victory-Lap, renderer port 5 | constant buffers read as raw bytes in place (`VICTORY_PORT_CBV=1`) | off (slower here, r40) |
| `port/scarlett` | [Op-lite/victory-scarlett](https://github.com/Op-lite/victory-scarlett) 183ac80, bc264a4 | `VICTORY_PROBE_RAYS` (no violet vegetation/posters with ray tracing off), `VICTORY_PLANE_NOW` (tiling reads the current material-class plane) | on |
| `perf/lto` | [soldier162/Victory-nvidia-soldier16](https://github.com/soldier162/Victory-nvidia-soldier16) da7a1ab | thin LTO in the release profile | on (r42) |

Looked at and not taken: soldier16's Series S 1080p pools (Series S is no faster than Series X + DLSS here, r37),
its `TEXTURE_NO_WAIT` default (the launchers set it already); Victory-Lap's `PORT_DESCRIPTORS` (overlaps
foxhound's own heap-record work).

## Run it

```powershell
cd <your foxhound checkout>
scripts\play-foxhound.ps1 -Exe <dump>\game_scarlett_master.exe            # Series X 1440p, DLSS Quality
scripts\play-foxhound.ps1 -Exe <dump>\game_scarlett_master.exe -FrameGen  # + DLSS Frame Generation (~2x shown)
# after a new build: shaders and pipelines ahead, with the run's own switches
scripts\play-foxhound.ps1 -Exe <dump>\game_scarlett_master.exe -Shaders -Warm
```

Streamline SDK 2.14.16 in `C:\dev\sdk\streamline-sdk-v2.14.16` (the launcher's default path).
The NVIDIA driver keeps compiled pipelines per program path: always run `target\release\victory.exe`.

## Results

Same unattended route for every run (`gx-workers\bench.ps1`): Enter every 4 s through the start screens, the
intro and diner cutscene, the jail cell from ~190 s; camera turns (right stick, 4 s every 20 s) from 280 s;
420 s. "last 2 min" = the last 24 five-second fps lines in the world.

| Run | Build / switches | last 2 min fps mean / min | prep ms |
|---|---|---|---|
| r25 | old line (trustno1 + mimalloc), Series S 540p, release | 24.8 / 23.0 | 23.7 |
| r34b | old line + soldier16, Series S 540p | 30.0 / 29.8 | 23.9 |
| r35 | foxhound + mimalloc, Series X 1440p DLSS Q (pipeline library still filling) | 28.3 / 23.8 | 22.2 |
| r36 | r35 + Frame Generation (rendered fps; every frame shown twice) | 27.4 / 20.2 | 23.8 |
| r37 | r35 on Series S | 28.0 / 21.8 | 21.9 |
| r38 | r35 + port 6 (`VICTORY_PORT_GUARD=1`) | 29.7 / 22.3 | 21.9 |
| **r39** | **master** (foxhound + mimalloc + port 6 + scarlett fixes), warmed | **30.0 / 30.0** | 20.9 |
| r40 | master + `PORT_CONSTANTS=1` + `PORT_CBV=1` | 25.8 / 24.7 | 14.1 (card waited 19 ms) |
| r41 | master + `PORT_CONSTANTS=1` | 26.5 / 25.5 | 14.0 (card waited 8.6 ms) |
| **r42** | **master + thin LTO** (the build in `target\release`) | **30.0 / 30.0** | 20.1 |
| r43 | r42 built with `-C target-cpu=native` (Zen 4) | 30.0 / 30.0 (one 12 fps dip earlier) | 20.3 |

The title caps the frame at 30 fps; r42 has ~13 ms of the 33 ms frame left on the submitting thread.

Ports 1 and 5 take 7 ms off the CPU but the card then reads title memory across PCIe and waits longer than
that: off on this PC.

Picture (r39 screenshots): the diner cutscene shows every face whole; the jail cell has no violet patches
(r38, before the scarlett fixes, had them on a poster and the bunk); no faults while turning.

Missing faces / stutter while turning in the first foxhound runs came from an empty host pipeline library and
pipelines made live; `-Shaders -Warm` with the run's switches (now in the launcher) fixed both.
