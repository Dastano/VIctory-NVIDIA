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
| r44 | r42 again (repeat) | 30.0 / 29.9 | 20.1 |
| r45 | r42 + `-FrameGen` | 30.0 / 29.6 rendered, **60 shown** (Streamline: 30 presents, 60 frames a second) | 20.4 |

The title caps the frame at 30 fps; r42 has ~13 ms of the 33 ms frame left on the submitting thread.

Ports 1 and 5 take 7 ms off the CPU but the card then reads title memory across PCIe and waits longer than
that: off on this PC.

Picture (r39 screenshots): the diner cutscene shows every face whole; the jail cell has no violet patches
(r38, before the scarlett fixes, had them on a poster and the bunk); no faults while turning.

Missing faces / stutter while turning in the first foxhound runs came from an empty host pipeline library and
pipelines made live; `-Shaders -Warm` with the run's switches (now in the launcher) fixed both.

## Walking, not only standing (r46-r48, with the frame-time line)

The 30.0 above is with the player at the cell window: the camera turns there are small (the scene
holds the view until she walks). r46 (12 min, 21 turns at the window, frame-time line per second): 9 minutes in
the cell, ~16,000 frames, every fps line 30.0, 4 frames over 40 ms, the longest 60 ms.

r47 walks (S/D/W/A, big right-stick turns, look up/down), which also plays on: out of the cell after the guard,
the corridors and the crowded common room (7,000-10,000 draws a frame), into the next cutscene and mission:

| Run | Switches | world fps mean / lowest line | frames over 40 ms (of ~16,900) | longest |
|---|---|---|---|---|
| r47 | master | 28.8 / 8.7 | 1,301 | 572 ms |
| r48 | master + `VICTORY_SUBMIT_FRAMES=2` | 28.6 / 0.2 | 1,063 | 4,862 ms |

No picture faults in the r47 shots (faces, heads, light fine). The drops follow the draw count: the submitting
thread does 27 ms a frame at the window and 30-39 ms in the common room (lists' steps 13-16 ms, of which
refreshes 7-10 ms where over 90 % of ~7,000-11,000 refreshes copy nothing; planning card steps 3-6 ms; waits for
the submission lock and flights up to 5 ms). Holding 30 fps while walking through the crowd needs that work
cut (the refresh design), not a switch.

## trustno1-coder/VIctory-NVIDIA merged (DualSense, F1 menu, F3 panel), refresh skip (r49-r51)

trustno1's fork moved onto the foxhound base; its main (661 commits: DualSense/DualSense Edge over HID, USB or
Bluetooth, on by default as `VICTORY_DUALSENSE` auto; F1 settings menu; frame generation 2x-6x; F3 stats;
window/input fixes; open water) merges into master with conflicts only in README and the launcher.
`VICTORY_REFRESH_SKIP` (perf/refresh): the "copies nothing" check by binary search over the asked ranges and
on the submitting thread before a flush wakes helpers; on by default (`=0`: as before).

Same walking route as r47, `-Fps 30`:

| Run | Build / launcher | fps mean / lowest line | frames over 40 ms | longest |
|---|---|---|---|---|
| r47 | master before the merge | 28.8 / 8.7 | 1,301 | 572 ms |
| r49 | + trustno1 (frame generation made ready at every start) | 27.7 / 2.5 | 1,735 | 1,119 ms |
| r50 | r49 with `-NoFrameGenReady` | 28.7 / 11.8 | 1,406 | 557 ms |
| r51 | r50 + `VICTORY_REFRESH_SKIP` | 28.9 / 16.9 | 1,320 | 341 ms |

So the launcher now makes frame generation ready only with `-FrameGen` (or `-FrameGenReady` for the F1 menu's
live switch). trustno1's launcher defaults to `-Fps 1000` (no cap); `-Fps 30` holds the title's own rate.

| r52 | final master, `-Fps 1000` (no cap), at the window | 33.6 / 31.6 (last 2 min) | - | - |

The launcher default is `-Fps 30` here (most even frame times); `-Fps 1000` uncaps.

## DLAA, frame generation pacing, the water (r56-r60)

| Run | Build / setup (walking route) | rendered fps mean | frames over 50 ms | note |
|---|---|---|---|---|
| r56 | master + `VICTORY_WATCH_FAST` (then default on), DLAA + FG 2x | 3.3 | – | the write-watch askings took 264-267 ms a frame: the switch is now off by default |
| r58 | DLAA + FG 2x, `-Fps 30` | 14.0 (28 shown) | 7,998 | the interval paced the frames shown, rendered and generated alike |
| r59 | DLAA, no FG | 28.8 | 405 | DLAA costs nothing here (the CPU sets the frame); r47 with DLSS Q: 642 |
| r60 | DLAA + FG 2x, launcher fix (`-Fps` counts rendered frames) | 27.3 (~55 shown) | 203 | |

The F1 menu's WATER LIGHT (`VICTORY_WATER_LIGHT=1`) runs the water's two tiled light programs in the wave-wide
form on 32-lane waves, which leaves whole tiles of open water black; off by default. Memory (r56): private bytes
9.9 GB at the start, ~35 GB after three minutes in the world, flat for the rest of the 11 minutes; a 50-minute play
session reached 59 GB (commit 84 of 89 GB, heavy paging), so it grows with places visited.

## Frame generation: open (r61-r65)

After the water fix (fix/water: one-thread LDS accumulator inits hoisted, `VICTORY_WAVE64_FIX`, on; the water's
tiled light runs on 32-lane hosts again) every frame-generation run at the cell window locked at 15 rendered /
30 shown, the presenting thread 40-50 ms inside Streamline's present: r61 (default), r62 (`-Fps 1000`), r63
(borderless fullscreen), r64 (`VICTORY_WAVE64_FIX=0`, water light off). r60 (walking route, build before the
water merge) ran 27 rendered / ~55 shown. r65 (walking route, current build) never reached the world: Streamline
raised an exception every ~225 ms (83 mini-dumps) and the title ended; no other run had one. Not resolved.
Without frame generation (r59: DLAA, walking) the build runs 28.8 fps with no such lock. Recommendation for now:
DLAA, frame generation off.

## Card-step runs and the water, again (r66-r67; the save is now at the first boat ride)

r66 (DLAA, no FG, walking keys; `VICTORY_HEAVY_CARD_RUNS`: constant-buffer reads that follow one another in
the ring copied in one `CopyBufferRegion`): 29.1 fps mean, 1,036 frames over 40 ms (r59: 28.8, 1,214).
But its screenshots show small black blocks scattered over the open water around the boat with the water's
tiled light running (`VICTORY_WAVE64_FIX` on): that light is left out again by default (`VICTORY_WATER_LIGHT=1`
runs it). r67 (the same, light left out, 7 min, cutscenes on the boat): 29.3 fps over the last 2 min (min
26.8), no black blocks in the shots; the top-down boat view was not reached in that run.

r68 (`VICTORY_HEAVY2_SLOT_BATCH`: the fills' slots retire the buffers they showed in one batch a thread, not one
global-lock hold each, ~5,800 a heavy frame): 29.1 fps mean, frames over 40 ms 1,002 (r66: 1,036), over 50 ms
296 (r66: 345). Next by size in heavy frames: the refresh passes' wall time (4.4 -> 6.8 ms in 23 passes).

r69 (perf/passes: copies flush only the refreshes they touch, the join spins 250 us, textures biggest first): 29.1
fps, over 50 ms 396, one 5-second hitch. r70 (+ `VICTORY_PASSES_HOT=1`, helpers awake through a submission):
29.3 fps, over 50 ms 302, longest frame 145 ms (r68: 525), lowest fps line 19.6: on in the launcher.

r71 (`VICTORY_TEXREF_PARTIAL`: textures detiled only in the rows whose bytes changed): 29.15 fps mean, over 50 ms
300, longest 464 ms; preparing 18.7 ms (r70: 19.0): within noise of r70. Its shots reach the top-down view of the
boat on open water: no black blocks with the water's tiled light left out (the default again); no texture faults.

r72 (`VICTORY_FG_PACING_FIX`: 3 buffers, tearing): still 15 rendered / 30 shown, 44-51 ms in the present. Likely
cause outside the layer: on this PC the 3440x1440 monitor is driven by the CPU's integrated AMD Radeon, not the
RTX 4090, and a Parsec virtual display is the primary display; every frame crosses adapters, and frame generation
(which paces presents on the display it is connected to) cannot pace across that. To check with the monitor on
the 4090.
