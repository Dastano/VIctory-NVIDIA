# Starting the MASTER build (RTX 4090 PC)

```powershell
cd C:\Users\hinze\foxhound
scripts\play-foxhound.ps1 -Exe F:\gta\gta6\game_scarlett_master.exe
```

## Launcher options (`scripts\play-foxhound.ps1`)

| Option | Values | Default | What |
|---|---|---|---|
| `-Exe` | path | – (required) | the title's `game_scarlett_master.exe` |
| `-Dlss` | `off` `dlaa` `quality` `balanced` `performance` | `quality` | DLSS mode (DLAA: native 1440p, sharper, more GPU) |
| `-Fps` | 1-1000 | `30` | rendered frames a second; `1000` = no cap (higher average, less even) |
| `-FrameGen` | switch | off | DLSS Frame Generation (needs the window on a display connected to the RTX 4090) |
| `-FrameGenX` | 2-6 | 2 | frames shown per rendered one (an RTX 40 does 2) |
| `-FrameGenReady` | switch | off | make frame generation ready so the F1 menu can switch it live (~1 fps) |
| `-Res` | `1440` `1080to1440` `4k` | `1440` | output / scene size |
| `-SeriesS` | switch | off | Series S profile (no faster here, softer) |
| `-Build` | `release` `diag` | `release` | diagnostics build for captures / timelines |
| `-More` | `@{VICTORY_X='1'}` | – | add or override any switch |
| `-Shaders` `-Warm` `-Used` | switches | – | translate the shaders / make the pipelines ahead with the run's own switches (after a new build) |

## F1 menu

In the game F1 opens a settings menu. It saves to `victory-settings.txt` in this folder, applied at every start
**over** the options above (e.g. its DLSS mode wins over `-Dlss`). Delete the file to go back to the launcher's
options. Current file on this PC: DLSS Quality, frame generation off, no fps cap.

## Recommended on this PC

- **Smoothest frame rate:** DLSS Quality, `-Fps 1000` (walking ~40-47 fps, dips under 30 in ~12 % of seconds).
- **Even frame times:** DLSS Quality or DLAA, `-Fps 30`.
- **Frame generation:** `-FrameGen -Fps 1000` with the game window on the monitor connected to the RTX 4090.
  On the Parsec virtual display it locks at 15 rendered / 30 shown (artifacts).
- The monitor on the RTX 4090 as the primary display: the game opens there; ~7 fps more than on the Parsec display.

## Switches on in the launcher (our additions)

`VICTORY_PORT_GUARD=1` (shared guard lock), `VICTORY_PASSES_HOT=1` (refresh helpers awake),
`VICTORY_TEXSPK_PASS_MS=8` (texture pass budget per frame). Code defaults on: mimalloc, thin LTO,
`VICTORY_PROBE_RAYS`, `VICTORY_PLANE_NOW`, `VICTORY_REFRESH_SKIP`, `VICTORY_REFRESH_WIDE`,
`VICTORY_CARD_PLAN_FAST`, `VICTORY_HEAVY_CARD_RUNS`, `VICTORY_HEAVY2_SLOT_BATCH`, `VICTORY_TEXREF_PARTIAL`,
`VICTORY_CC_TAKE_BESIDE`, the water fixes (`VICTORY_LOOPS_WAVE`, `VICTORY_LDS_FIRST_INIT`, `VICTORY_WAVE64_FIX`;
water light on). Measurements: `NOTES-master.md`.

## Known issues

- A black box can still appear on the water around a floating object (water light); a fix is in progress.
  `VICTORY_WATER_LIGHT=0` (or F1 → WATER LIGHT off) removes it, with darker water.
- Memory grows over long sessions; restart after about an hour.
