#!/bin/bash
# route.sh <runlabel> <routefile> [delay]: wartet auf die ersten Weltframes im run.log, dann + delay s die Route ins Live-Keyfile
log=/c/Users/hinze/vbench/$1/run.log; f=/c/Users/hinze/vbench/live-keys.txt
for i in $(seq 1 400); do [ -f "$log" ] && sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -qE "host: [0-9.]+ fps.* ([0-9]{4,}) draws/frame" && break; sleep 2; done
sleep "${3:-25}"
cat "$2" >> "$f"; echo "route sent $(date +%T)"
