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

r73: the same as r72 with the game window moved onto the display the RTX 4090 drives (the run's window had opened
on the primary display, a Parsec virtual display): 27.7 rendered fps mean, 31 presents / 61 frames shown a second,
7 ms in the present. So the 15/30 lock came from presenting on a display not on the 4090. Play with the window on
a display connected to the 4090 (and the Parsec virtual display off or not primary).

r74 (walking, DLAA, no frame generation, window on the 4090's display): 29.30 fps mean, over 50 ms 342, longest
186 ms: the same as r70 on the Parsec display (29.33). Without frame generation the display does not matter;
the remaining gap to 30 is the submitting thread's work in heavy frames.

r75 (`VICTORY_CC_TAKE_BESIDE`: a submission's sends copied on a thread beside the list recording): 29.34 fps,
over 50 ms 363, longest 143 ms: no change from r74 (29.30). The submitting thread's preparing is now 18.6 ms on
average; the frames over 40 ms are no longer explained by it alone: the card's clock line puts much of the idle
time "while the title had not submitted", i.e. the title's own threads (its code plus our write guards' faults,
~350 a frame) set those frames. Next lever, if any: the title threads' fault cost, not the submitting thread.

r76 (`VICTORY_COPY_BESIDE=1`, `VICTORY_STORE_BESIDE=1`: the title's writes into guarded blocks done beside the
guards): 29.42 fps (r75: 29.34), preparing 18.1 ms, over 50 ms 355: within noise; left off (they emulate the
title's stores, a fault there would corrupt title data for ~0.1 fps).

## Round 2 of fork updates and the water fixed (r78-r80)

master now also has trustno1 main up to d12eb1b (BC7 rotation, violet probe garbage, raw view clamp, helper
priority, skip-bink, F1 mouse, Vice-Tay's group-counter lanes), foxhound 9f8838b (static-sampler order, cloud tile
sort: VICTORY_LDS_LOOP_ATOMICS / VICTORY_LDS_BOUNDS, buffer pool) and foxhound 454ffea's water fixes
(VICTORY_LOOPS_WAVE, VICTORY_LDS_FIRST_INIT, a skip kept before a loop that would never end); the water's tiled
light runs by default again. r79 (walking, DLAA): 29.17 fps, preparing 16.5-19.5 ms in play. r80 (the same,
shots every 8 s, window on the primary display): the open water around the boat is lit (turquoise) with no black
blocks anywhere in the top-down shots; 29.1 fps over the last 2 minutes. Not taken: soldier16's two-stage submit
pipelining (a possible race; tied to its other changes).

r81 (walking, DLAA, `-Fps 1000`, i.e. no cap): 34.8 fps mean, preparing 16.3 ms, lowest line 19.4, longest frame
201 ms; 20 of 114 fps lines under 29.5 (capped r79: 21 of 132). So the build has headroom on average (35 fps
walking), and the dips under 30 sit at the same places with or without the cap: heavy moments of the title's own
work, not the cap. `-Fps 30` (default) gives the evener frame times; `-Fps 1000` the higher average.

r82 (walking, DLSS Quality, no cap): 39.4 fps mean, 14 of 115 lines under 29.5 (7 under 27), over 50 ms 300,
longest 224 ms; the card 88-90 % busy. With DLAA and no cap (r81) the card was 90-95 % busy at 34.8 fps: DLAA
plus the water light make the card part of the limit once the cap is off. Best measured setup for frame rate:
DLSS Quality, no cap.

r83 (walking, DLSS Balanced, no cap): 40.35 fps mean, 12 of 115 lines under 29.5 (7 under 27): little better
than Quality (r82: 39.4, 14): the remaining dips are not the card's; they are spikes of the title's and our CPU
work at heavy moments of the mission. Recommended: DLSS Quality, no cap (or -Fps 30 for even frame times).

## Texture pass budget (r84-r85)

A log analysis of r82 tied the frames over 50 ms to bursts of first-fill texture uploads (~3,000 a second; the
texture pass ran 2.2-2.7 s of every 5 s in those stretches). `VICTORY_TEXSPK_PASS_MS=8` (perf/texspikes) caps a
frame's texture pass at 8 ms, the rest a frame later. r84 (walking, DLSS Quality, no cap, window on the 4090's
display): 47.0 fps mean (r82, the same without the cap: 39.4), 14 lines under 29.5, 5 under 27. r85 (the same on
the Parsec primary display, shots every 7 s): 39.7 fps; textures sharp, water lit, no faults in the shots. On in
the launcher. The Parsec virtual display costs ~7 fps uncapped.

r86 (as r84 with `VICTORY_TEXSPK_PASS_MS=4`): 44.7 fps, the same 14 lines under 29.5 (5 under 27): 8 stays.
The remaining dips (about 12 % of the seconds while walking through the mission) sit in the heaviest moments
(5,000-9,000 draws, the submitting thread 30-40 ms, the title waiting at presents) and in mission loads.

## The water's volumetric programs on 32-lane hosts (r88)

Three water programs did not translate for 32-lane waves and were dropped: CS_TransmissionVolumetrics
(6871d6e4b3ac9a10) and _VolumePortalsOnly (a10179ab7eda0dc3) were missing from the digests that try the
wave-wide branch form; CS_TransmissionVolumetricsSubmerged (926c46bab3b86bd6) hit a regression where the loop
form dropped the list of blocks that need every lane (`VICTORY_WAVE64_WATER3`, on; also fixes
vfx_triangleClip:CS_DefragHeap_ComputeRelocations). The user's game had shown a black box on the water around a
floating package. r88 (walking, DLSS Quality, no cap, shots every 6 s): water lit, no black boxes in any shot,
including the bridge view; 38.75 fps mean (Parsec display), 13 of 90 lines under 29.5.

r89 (r88 + `VICTORY_EARLY_ALL=1`, every title pipeline started at creation): 39.1 fps (r88: 38.75), 10 of 89 lines
under 29.5 (r88: 13 of 90), over 50 ms 260 (257), longest 667 ms: within noise; left off. `warm --used` found all
83,821 booked pipelines already made: the first-use bursts at area loads (1,561 shaders in 5 s in a user session)
are loads, not compiles.

## Diagnostics run and the last knobs (r90-r92)

r90 (the diagnostics build at the release path, walking, DLSS Quality, no cap; 39.0 fps): in slow seconds versus
fast ones the card is idle 12.7 vs 2.4 ms a frame (half behind our submitting thread, half while the title had
not submitted), the title waits 13.3 vs 5.9 ms at presents, the write trackers' locks are waited on 19.1 vs 2.0
ms (all threads; page faults 2,591 vs 377 a frame), flights are waited for over upload-memory fills the card
steps refused (2.0 vs 0.18 ms, peaks of 15), and the copy-back relay lands 13.8 vs 5.6 ms. The draws are 5,369
vs 1,654: about half the extra frame time is real extra content.
r91 (`VICTORY_SUBMIT_FRAMES=2`): 39.1 fps, 11 lines under 29.5: no change. r92 (`VICTORY_PORT_GUARD=0`,
`VICTORY_FAULT_UNLOCKED=1`): 37.7 fps, 16 under 29.5: worse; port 6 stays.

## Frame generation on the ASUS monitor (r93-r94)

The 3440x1440 monitor is now on the RTX 4090 (59 Hz). r93: the window moved at 3440x1440 by a DPI-unaware
script spanned two displays: 15 rendered / 30 shown again. r94 (1600x900 window inside the ASUS): 31 presents /
62 frames shown a second once in the world, 28.5 rendered fps mean (34 of 92 lines under 29.5), over 50 ms 207.
Frame generation shows at most the monitor's refresh: on 59 Hz it holds the rendered rate at ~30. Without it
the build renders 38-47 fps. On this monitor: no frame generation for the most responsive game, frame generation
for 60 shown at 30 rendered. Keep the game window fully on the 4090's monitor (F1 FULLSCREEN there).

r95 (`VICTORY_GUARD_SHARDS=1`: the guards' lock split into 64 region locks, refresh holes noted under claims;
perf/guard-shards, merged, off by default): 38.4 fps, 13 of 89 lines under 29.5, over 50 ms 277, longest 315 ms:
no change from r88/r89; no deadlock or fault. The lock waits were not what set the slow seconds. Its diagnostics
split of lock waits by holder stays in for the next diagnostics run.

## Driving the boat (r96)

At the user's suggestion the route now drives: RT held (key C) from 240 s, left stick left/right every 16 s,
camera turns (`gx-workers\boatdrive-keys.txt`). r96 (DLSS Quality, no cap): the boat runs at speed through the
bay after the plane, once onto the shore road (reset prompt). Picture clean, water lit with wake, no black boxes.
32.3 fps mean, 44 of 97 lines under 29.5 (26 under 27), over 50 ms 615, longest 345 ms: driving streams the
world much faster than walking and is the harder benchmark from now on.
New in other forks (20:37, summarised by the GX Cube): trustno1's MFG Unlocker (frame generation for RTX 20/30;
nothing for an RTX 40), Victory-Lap play29-31 (its biggest win, VICTORY_BVH_LOG_ONCE, concerns its BVH stand-in
we do not run; constants in card memory saves card time, not our CPU limit): not taken.

r97/r98 (boat drive; `VICTORY_CARD_FILLS`: fills around a copy at its place done on the card instead of waiting for
flights, perf/card-fills, on): on 32.9 fps, 42 of 99 lines under 29.5 (24 under 27), over 50 ms 1,012; off (r98,
same build) 32.9 fps, 48 under 29.5 (29 under 27), over 50 ms 625. A draw within the boat route's run-to-run
spread; left on (byte-identical by its test, fewer dips).

r99 (boat drive; `VICTORY_RELAY_TEX_WIDE`: a flight's texture read-backs land on the landing threads at once,
overlapping ones in order; perf/relay, on): 33.8 fps mean (r97/r98: 32.9), 35 of 98 lines under 29.5 (r97/r98:
42-48), 21 under 27 (24-29), lowest line 19.2 (6.8-7.7). The relay's texture read-backs: 14.0 a frame, all 14 at
once, 0.74 ms of "read-backs and lists" (about 4.5 ms before in slow seconds).

## trustno1 merge 0cef07e..f3534e6, relay round 2 off (r100-r102)

r100 (relay round 2 on: retired objects released on their own thread, the title told before flights are tidied)
32.4 fps / 44 of 99 lines under 29.5; r101 (same build, both off) 33.5 / 37: off in the launcher.
Merged trustno1-coder/VIctory-NVIDIA up to f3534e6 (placed views ignore padding words - the memory and slowdown
over a session, "confetti on faces"; upload room reserved at start - crash on 4090; arena storm; audio DSP
convolutions; FSR 3 frame generation; MFG unlocker opt-in; VICTORY_HOME). r102 (boat drive, DLSS Quality, no cap):
35.3 fps mean, 23 of 97 lines under 29.5 (10 under 27), over 50 ms 374, longest 191 ms - the best boat run so
far (r96-r101: 32.3-33.8 fps, 35-48 lines under 29.5). Picture clean: faces whole, water lit, no black boxes.
PR readmet3xt-lab/foxhound#13 was closed without merge or comment.

r103/r104 (boat drive; `VICTORY_FEWER_PAST_COPIES`: refreshes after a copy into title memory skipped when the
journal shows nothing new): on 32.6 fps / 36 lines under 29.5, off 33.9 / 30: off in the launcher.
Ported from readmet3xt-lab/foxhound 0.2.0-pre.4 (3d52ac8; it also carries our PR #13's two switches): a texture
copy let go is marked unused (long-session slowdown, their #12), a pool of list preludes (VICTORY_PRELUDE_POOL),
the card with the most dedicated memory (or VICTORY_ADAPTER). r105 (boat drive, DLSS Quality, no cap): 35.4 fps
mean, 21 of 99 lines under 29.5 (10 under 27), over 50 ms 539, longest 320 ms - the best boat run so far.

r106 (boat drive; `VICTORY_CARD_RUNS_SORTED`: card-step places sorted by host copy and address, read in runs): 1,502 places in 1,322 copies (few runs); 32.0 fps, 45 of 99 lines under 29.5 (r105: 35.4 / 21): off in the launcher.

r107: r105's build and settings again (boat drive): 33.5 fps, 37 of 99 lines under 29.5 (28 under 27), over
50 ms 693 - against r105's 35.4 / 21 / 10. The boat route spreads by about 2 fps and 16 dip lines between runs of
the same build (where the boat goes depends on its physics). So the single-run verdicts r100/r101, r103/r104 and
r106 (1-3 fps apart) are within that spread; a change needs several runs a side to be judged on this route.

r108/r109 (boat drive, `VICTORY_CARD_ALL=1`: every title buffer's copy gets a card side, changes found on the
card): 33.9 / 34.1 fps, 42 / 34 lines under 29.5 - inside the spread of the same build without it (r105/r107:
33.5-35.4 fps, 21-37 lines): no effect; left off.

## Redesign stage 1: frozen copies (r110-r112)

`VICTORY_FREEZE=1` (REDESIGN.md stages 0-1, d8744d6, off by default), three boat runs: 33.4 / 34.9 / 33.9 fps,
45 / 25 / 27 lines under 29.5 - inside the spread without it (r105/r107: 33.5-35.4, 21-37). Its counters: 49-64
frozen copies looked at per frame, 470-846 of ~7,000 refreshes not recorded: few copies stay unwritten for 30
frames, so the premise (most no-op refreshes belong to idle copies) does not hold; the no-op refreshes are of
copies the title keeps touching a little. Stage 1 stays off. Stage 2 (the constant ring as CPU-only) targets
card-step planning and card sends instead and stays a candidate.

Merged trustno1-coder/VIctory-NVIDIA up to 64d4931 (PS4/DualShock 4 pads, F1 menu saves at once, long-session
memory, memory switches off, VICTORY_ACTIVATE_RAYS on, lendable prelude pool and adapter pick superseding our
ports); shaders re-translated and pipelines re-warmed. r113 (boat drive): 33.9 fps, 35 of 98 lines under 29.5 -
inside the spread. One shot (boat driven into mangroves, camera inside the foliage) shows red-tinted leaves:
game effect or fault, to check.

## Redesign stage 2: the constant ring CPU-only (r114-r116)

`VICTORY_RING_CPU=1` (36f259a, off by default): the ring was learned and confirmed (28.3 MB inside the heap, no
GPU writes, 0 demotions); 5,872 / 1,824 fills a frame were served without card-step planning. Boat drive: r114
32.7 fps, 47 of 97 lines under 29.5; r115 34.5 fps, 26 of 98 - inside the spread (33.5-35.4). The skinning passes'
whole-heap view still reads the ring, so 11 MB of it are still sent a frame. No gain measured; stays off.
r116 never reached the world: the title's watchdog store (write to 0 at image+0x23cf683, the same site trustno1
notes for frames of many seconds) during loading, with the ring still learning (inactive). First such crash in
~20 boat runs; watch for it (new with the 64d4931 merge, or a rare load hitch).

r117 (current master, all redesign switches off, boat drive): 34.0 fps, 34 of 98 lines under 29.5, no crash: the
r116 watchdog crash did not repeat (a rare load hitch, not a regression so far).

## Where this stands (2026-11-28)

Stable 30 fps through the first mission is not reached. Cell window: 30.0 held. Walking uncapped: 38-47 fps.
Boat drive (the hardest route): 33.5-35.4 fps mean, 20-40 % of 5-s lines under 29.5. Redesign stages 1 (frozen
copies) and 2 (ring CPU-only) were built, tested and measured without gain; the premises behind them do not
hold for this title. Further gains need a different approach (e.g. the title's own frame pacing, or a much
deeper change of how title memory reaches the card).

## The city benchmark: the user's 70% save (r121-r126)

From now on the main benchmark is the user's 70% story save: it starts on a city street with many draws
(8,000-11,000 a frame, against 4,000-5,000 in the jail and 5,000 on the boat). `tools/rtx4090/queue55.ps1` swaps
it in for a run and puts the user's own saves back afterwards (checked against the backup after every run).
`bench.ps1` now captures the game window with PrintWindow (PW_RENDERFULLCONTENT): other windows in front of the
game no longer end up in the shots.

r120 (new story, legal screens accepted by keys, the first in-game scene, DLSS Quality, `-Fps 30`): 29.49 fps,
23 of 157 lines under 29.5, longest frame 1.3 s.
r121 (city, walking and turning, DLSS Quality, no cap): 27.5 fps mean, 56 of 72 lines under 29.5, preparing
23.4 ms. r123-r125 (city, standing at the spawn point: the scripted Y came before the world was there): 26.3 /
24.6 / 26.5 fps (mean 25.8), 68-75 of 86 lines under 29.5, stalls up to 3.4 s.
r122 (city, live keys through `VICTORY_KEYS_FILE` while watching the shots): Y gets in the SUV left of the spawn
point; RT/LT drive, crashes deform the car (wheel off, smoke, "Engine damaged") with no picture faults; 17-22 fps
in the alley behind the houses (10,000 draws).

Where the time goes in the city (sampling the threads from outside: `ripsample.ps1` suspends a thread, reads its
instruction pointer and scans its stack for return addresses into victory.exe; `symbolize.ps1` names them from
victory.pdb with dbghelp; no admin rights, so no ETW):
- The title's main thread is not the limit: it spends ~70% of its samples in `NtWaitForSingleObject` /
  `NtDelayExecution` from one polling loop (it waits at presents, as r90 found).
- Our submitting lane thread (`queue::behind::Lane::serve` running `Host::submit`) is 84% busy: 25% of its samples
  parked (`NtWaitForAlertByThreadId`: the lane waiting for work, `send::Own::wait`, the refresh workers' joins in
  `shadow::wide`, descriptor `SYNC_ROUNDS`), 8.7% in `NtGetWriteWatch`, 12% in the NVIDIA driver, ~36% our code
  (top: `shadow::wide`, `victory_safe_copy_at`, `watch::nothing_new`/`Shadow::nothing_new`, `Host::submit`,
  `Batch::copy_spans`, a BTreeMap range search, `Batch::add`).
- About 12 helper threads run at 40-50% each, a third of that parked.
The card is ~55% busy; per frame 16-18 ms of it idle, 4-6 ms of that behind our submitting thread.

## trustno1 to 63b902f merged (r126-r133, city standing, DLSS Quality, no cap)

r126 (`pin.ps1`: the title's main thread and our busiest submitting thread each pinned to a core of their own,
switched on and off every 40 s in one run): on 18.8 fps, off 19.6: pinning costs; dropped. (The busiest thread of
ours changes between the lanes, and CPU 0 also takes the system's interrupts.)
Merge of trustno1 main 8fc2254 (52 commits: tearing presents also without frame generation, put onto our
`SwapSetup` with trustno1's fallback for a refused present; division fixup with zero operands, lane masks in halves
mode, the underwater program translating again; descriptor slots of split submissions given back; -NoFrameGenReady)
and 63b902f (Series S rows in F1; the memory stack `VICTORY_SLOT_EVICT` / `ALLOCATOR_TRIM` / `BUDGET_PACK` on by
default in the launcher). Shaders re-translated (25,762) and pipelines re-warmed (88,837 in 155 s).
| Runs | Build | fps (bench summary) | draws |
|---|---|---|---|
| r123-r125 | master 388d9cb | 26.5 / 24.8 / 26.8 (26.0) | 7,450 |
| r128, r129, r131 | + trustno1 8fc2254 | 24.8 / 23.7 / 25.3 (24.6) | 7,750 |
| r130 | the same, `VICTORY_TEARING=0` | 24.6 | 7,960 |
| r132, r133 | + 63b902f, memory stack on | 25.7 / 24.6 (25.2) | 7,490 |
Within the city scene's spread (the standing scene drifts with traffic and light); kept for the fixes. A black
wall lamp without any shading showed in r127 (walking behind the houses): a picture fault to chase.
