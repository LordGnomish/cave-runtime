#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

if [ -f "night-pump.pid" ] && kill -0 "$(cat night-pump.pid)" 2>/dev/null; then
  echo "night-pump already running with pid $(cat night-pump.pid)" >&2
  exit 1
fi

if [ ! -x "target/release/night-pump" ]; then
  echo "binary missing — running cargo build --release"
  cargo build --release --quiet
fi

mkdir -p log
RUST_LOG="${RUST_LOG:-info}" nohup ./target/release/night-pump \
  --queue ./queue.yaml \
  --state ./state.json \
  --contributions ./contributions.jsonl \
  --heartbeat-log ./log/heartbeat.log \
  --port "${NIGHT_PUMP_PORT:-9090}" \
  --max-parallel "${NIGHT_PUMP_MAX_PARALLEL:-8}" \
  > log/stdout.log 2>&1 &
echo $! > night-pump.pid
echo "started night-pump pid=$(cat night-pump.pid) port=${NIGHT_PUMP_PORT:-9090}"
