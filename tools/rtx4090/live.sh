#!/bin/bash
# live.sh "<key> [hold] [after]" ... : Zeilen an live-keys anhängen, warten, neuesten Screenshot nennen
f=/c/Users/hinze/vbench/live-keys.txt
for l in "$@"; do printf '%s\n' "$l" >> "$f"; done
sleep "${WAIT:-5}"
ls -t /c/Users/hinze/vbench/${RUN:-r122-city70-live}/shot-*.jpg | head -1
