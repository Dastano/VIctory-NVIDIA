#!/bin/bash
# citycar-run.sh <label> [switches]: Stadt-Autofahrt mit 70%-Save, Route nach Weltstart
cd /c/Users/hinze/gx-workers
: > /c/Users/hinze/vbench/live-keys.txt
(bash route.sh "$1" /c/Users/hinze/gx-workers/citycar-route.txt 50 > /c/Users/hinze/vbench/route-$1.out 2>&1 &)
powershell -NoProfile -ExecutionPolicy Bypass -File queue55.ps1 -Label "$1" -Seconds 600 -ShotEvery ${SHOT:-15} -KeysUntil 140 -LiveFile 'C:\Users\hinze\vbench\live-keys.txt' -Switches "${2:-}" > /dev/null 2>&1
diff -rq "$LOCALAPPDATA/victory/data/7952987A/saves" /c/Users/hinze/vbench/save-backup-20261128/saves >/dev/null && echo "$1 save ok" || echo "$1 SAVE DIFF"
tr -d '\0' < /c/Users/hinze/vbench/$1/summary.txt | cut -c1-150
