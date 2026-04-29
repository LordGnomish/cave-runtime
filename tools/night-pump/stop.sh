#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

if [ ! -f "night-pump.pid" ]; then
  echo "no night-pump.pid found" >&2
  exit 0
fi

PID=$(cat night-pump.pid)
if kill -0 "$PID" 2>/dev/null; then
  kill "$PID"
  for _ in $(seq 1 20); do
    if ! kill -0 "$PID" 2>/dev/null; then break; fi
    sleep 0.25
  done
  if kill -0 "$PID" 2>/dev/null; then
    kill -9 "$PID"
  fi
  echo "stopped pid=$PID"
else
  echo "pid=$PID was not running"
fi
rm -f night-pump.pid
