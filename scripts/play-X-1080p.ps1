# Series X, rendering at 1080p (render scale 0.5 of 4K output; native 1080p display mode with scale 1.0 hits a title startup breakpoint).
#   scripts\play-X-1080p.ps1 -Exe <path of your own copy's executable> [other play-nvidia.ps1 switches, e.g. -Quality]
& "$PSScriptRoot\play-nvidia.ps1" -SeriesX -Resolution 1080 @args
