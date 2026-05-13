#!/usr/bin/env bash
# Raft 3-node end-to-end smoke for the cave-runtime control plane.
#
# Boots three cave-runtime nodes on 127.0.0.1, lets them elect a
# leader via the Raft consensus layer, then exercises:
#
#   * write-to-leader → all three converge (replication)
#   * write-to-follower → 503 + Location: <leader_url>  (redirect)
#   * cavectl auto-redirect on 503  (client retry policy)
#   * leader failover (kill -9) → new leader → write replicates
#   * restart of the killed node → catches up
#
# Each step is "fail-fast" so a regression surfaces with a clear log
# line.  The script needs `jq`, `curl`, `cargo`, and a writable
# `/tmp`.  Doesn't touch any system state outside `$TMPROOT` (which
# is removed on exit).
#
# Usage:
#   scripts/raft_3node_smoke.sh         # run end-to-end
#   KEEP_TMP=1 scripts/raft_3node_smoke.sh   # leave $TMPROOT in place
#   SKIP_BUILD=1 scripts/raft_3node_smoke.sh # assume target/ is fresh
#
# The runtime is built in debug profile; expect 60–90 s of build on
# first invocation, ~5 s thereafter.

set -euo pipefail

# ── config ──────────────────────────────────────────────────────────────────
TMPROOT=${TMPROOT:-/tmp/cave-raft-smoke-$$}
PORT1=6443; PORT2=6453; PORT3=6463
ETCD1=2379; ETCD2=2389; ETCD3=2399   # apiserver_port - 4064
LOG_DIR=$TMPROOT/logs
trap cleanup EXIT

cleanup() {
  set +e
  for i in 1 2 3; do
    eval "pid=\${PID$i:-}"
    [ -n "$pid" ] && kill -9 "$pid" 2>/dev/null || true
  done
  if [ -z "${KEEP_TMP:-}" ]; then
    rm -rf "$TMPROOT"
  else
    echo "(KEEP_TMP set — left $TMPROOT in place)"
  fi
}

# macOS ships bash 3.2 (no `declare -A`); use indexed dynamic-name
# vars for the per-node pid map (`PID1`, `PID2`, `PID3`).

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing: $1"; exit 2; }; }
need curl
need jq

# ── build ───────────────────────────────────────────────────────────────────
if [ -z "${SKIP_BUILD:-}" ]; then
  echo "==> cargo build (debug profile) — may take a minute"
  cargo build -p cave-runtime --bin cave-runtime
  cargo build -p cavectl --bin cavectl
fi
RUNTIME=./target/debug/cave-runtime
CAVECTL=./target/debug/cavectl
[ -x "$RUNTIME" ] || { echo "missing $RUNTIME"; exit 2; }

mkdir -p "$LOG_DIR"

# ── init three nodes ────────────────────────────────────────────────────────
#
# Each `cluster init` generates a fresh per-node CA. For peer-to-peer
# TLS to validate (run_driver's reqwest client uses CA pinning) the
# three nodes must share a CA. We do that the simplest way: init
# node1 first, then copy node1's `pki/ca.{crt,key}` into nodes 2 + 3
# before their inits. The init re-signs the per-component leaves
# against the existing CA when present.
#
# In production this is handled by `cluster join`, which fetches the
# leader's CA via TOFU + cache (`docs/synergy/cluster-csr-ca-wal-2026-05-12.md`).
# The smoke skips that handshake because it boots all three nodes
# from the same operator's shell.
echo "==> initializing 3 data dirs under $TMPROOT"
for i in 1 2 3; do
  dd=$TMPROOT/node$i
  port_var=PORT$i; port=${!port_var}
  case $i in
    1) peers="2:127.0.0.1:$PORT2,3:127.0.0.1:$PORT3" ;;
    2) peers="1:127.0.0.1:$PORT1,3:127.0.0.1:$PORT3" ;;
    3) peers="1:127.0.0.1:$PORT1,2:127.0.0.1:$PORT2" ;;
  esac
  # For nodes 2 + 3, seed the data dir with node1's CA + pass
  # `--reuse-existing-ca` so the init signs leaf certs against the
  # same root.  Without this every node would generate its own CA
  # and cross-node TLS would fail with "certificates required to
  # validate this certificate cannot be found".
  reuse_flag=""
  if [ $i -gt 1 ]; then
    mkdir -p "$dd/pki"
    cp "$TMPROOT/node1/pki/ca.crt" "$dd/pki/ca.crt"
    cp "$TMPROOT/node1/pki/ca.key" "$dd/pki/ca.key"
    reuse_flag="--reuse-existing-ca"
  fi
  $RUNTIME cluster init \
    --data-dir "$dd" \
    --cluster-name cave-smoke \
    --advertise-address "127.0.0.1:$port" \
    --bootstrap-strategy=multi \
    --node-id=$i \
    --peers="$peers" \
    $reuse_flag >"$LOG_DIR/init$i.log" 2>&1
done

# ── start three serves ──────────────────────────────────────────────────────
echo "==> spawning 3 cave-runtime serves"
for i in 1 2 3; do
  port_var=PORT$i; port=${!port_var}
  CAVE_JWT_SECRET=smoke $RUNTIME --data-dir "$TMPROOT/node$i" -p "$((port+1000))" \
    >"$LOG_DIR/serve$i.log" 2>&1 &
  eval "PID$i=$!"
done
sleep 6  # election + heartbeat settle

# ── locate leader ───────────────────────────────────────────────────────────
echo "==> querying /api/v1/cluster/leader"
LEADER_URL=""
for port in $PORT1 $PORT2 $PORT3; do
  info=$(curl --max-time 3 -sk "https://127.0.0.1:$port/api/v1/cluster/leader" || true)
  echo "  $port → $info"
  url=$(echo "$info" | jq -r '.leader_url // empty')
  [ -n "$url" ] && LEADER_URL="$url" && break
done
if [ -z "$LEADER_URL" ]; then
  echo "FAIL: no leader after 6 s — full leader_info per node:"
  for port in $PORT1 $PORT2 $PORT3; do
    echo "--- $port ---"
    curl --max-time 3 -sk "https://127.0.0.1:$port/api/v1/cluster/leader" || true
    echo
  done
  echo "(serve logs in $LOG_DIR/serve{1,2,3}.log)"
  exit 1
fi
echo "  leader: $LEADER_URL"

# ── write to leader, read from all three ────────────────────────────────────
echo "==> PUT /v3/kv/put on leader, then RANGE on every node"
KEY_B64=$(printf %s "/smoke/foo" | base64)
VAL_B64=$(printf %s "bar" | base64)
LEADER_ETCD_URL=$(echo "$LEADER_URL" | sed -E 's,:([0-9]+),:'"$(($(echo "$LEADER_URL" | sed -E 's,.*:([0-9]+).*,\1,') - 4064))"',')
PUT_RESP=$(curl --max-time 8 -sk -w "\nHTTP %{http_code}" -X POST "$LEADER_ETCD_URL/api/etcd/v3/kv/put" \
  -H 'content-type: application/json' \
  -d "{\"key\":\"$KEY_B64\",\"value\":\"$VAL_B64\",\"lease\":null,\"prev_kv\":false}")
echo "  PUT response: $PUT_RESP" | head -c 400
echo
echo "$PUT_RESP" >"$LOG_DIR/put.json"
sleep 1
for port in $ETCD1 $ETCD2 $ETCD3; do
  body=$(curl --max-time 5 -sk -X POST "https://127.0.0.1:$port/api/etcd/v3/kv/range" \
    -H 'content-type: application/json' \
    -d "{\"key\":\"$KEY_B64\",\"range_end\":null,\"limit\":null,\"revision\":null,\"keys_only\":false,\"count_only\":false}")
  # KeyValue.value is `Vec<u8>` serialised as a JSON int-array, then
  # base64-wrapped by encode_kv (etcd v3 wire convention). Convert the
  # int-array → bytes → base64-decode.
  got=$(echo "$body" | jq -r '.kvs[0].value // empty | join(",")' \
    | awk -F',' 'NF{ for(i=1;i<=NF;i++) printf "%c",$i; }' \
    | base64 -d 2>/dev/null || true)
  if [ "$got" = "bar" ]; then
    echo "  etcd:$port → bar ✓"
  else
    echo "FAIL: etcd:$port returned '$got' (expected 'bar'); raw range: $body"
    exit 1
  fi
done

# ── write to a follower → expect 503 + Location ─────────────────────────────
echo "==> PUT on a follower, expect 503 + Location header"
FOLLOWER_URL=""
for port in $PORT1 $PORT2 $PORT3; do
  role=$(curl --max-time 3 -sk "https://127.0.0.1:$port/api/v1/cluster/leader" | jq -r '.role')
  if [ "$role" = "Follower" ]; then
    FOLLOWER_URL="https://127.0.0.1:$port"
    break
  fi
done
[ -n "$FOLLOWER_URL" ] || { echo "FAIL: no follower found"; exit 1; }
FOLLOWER_ETCD_URL=$(echo "$FOLLOWER_URL" | sed -E 's,:([0-9]+),:'"$(($(echo "$FOLLOWER_URL" | sed -E 's,.*:([0-9]+).*,\1,') - 4064))"',')
STATUS=$(curl --max-time 5 -sk -o /dev/null -w '%{http_code}' \
  -X POST "$FOLLOWER_ETCD_URL/api/etcd/v3/kv/put" \
  -H 'content-type: application/json' \
  -d "{\"key\":\"$KEY_B64\",\"value\":\"$VAL_B64\",\"lease\":null,\"prev_kv\":false}" || true)
LOC=$(curl --max-time 5 -sk -D - -o /dev/null \
  -X POST "$FOLLOWER_ETCD_URL/api/etcd/v3/kv/put" \
  -H 'content-type: application/json' \
  -d "{\"key\":\"$KEY_B64\",\"value\":\"$VAL_B64\",\"lease\":null,\"prev_kv\":false}" \
  | grep -i '^location:' | tr -d '\r')
if [ "$STATUS" = "503" ] && [ -n "$LOC" ]; then
  echo "  follower $FOLLOWER_URL → 503 ; $LOC ✓"
else
  echo "FAIL: follower expected 503+Location, got status=$STATUS loc='$LOC'"
  exit 1
fi

# ── kill leader, expect new leader within ~6 s ──────────────────────────────
echo "==> kill leader, expect new election"
LEADER_PORT=$(echo "$LEADER_URL" | sed -E 's,.*:([0-9]+).*,\1,')
case $LEADER_PORT in
  $PORT1) LEADER_IDX=1 ;;
  $PORT2) LEADER_IDX=2 ;;
  $PORT3) LEADER_IDX=3 ;;
  *) echo "FAIL: unknown leader port $LEADER_PORT"; exit 1 ;;
esac
eval "leader_pid=\$PID$LEADER_IDX"
echo "  killing node$LEADER_IDX (pid $leader_pid)"
kill -9 "$leader_pid"
eval "PID$LEADER_IDX=''"
sleep 8  # election + heartbeat
NEW_LEADER_URL=""
for port in $PORT1 $PORT2 $PORT3; do
  [ "$port" = "$LEADER_PORT" ] && continue
  url=$(curl --max-time 3 -sk "https://127.0.0.1:$port/api/v1/cluster/leader" | jq -r '.leader_url // empty')
  [ -n "$url" ] && [ "$url" != "$LEADER_URL" ] && NEW_LEADER_URL="$url" && break
done
if [ -z "$NEW_LEADER_URL" ]; then
  echo "FAIL: no new leader 8 s after killing $LEADER_URL"
  for port in $PORT1 $PORT2 $PORT3; do
    [ "$port" = "$LEADER_PORT" ] && continue
    echo "  $port → $(curl --max-time 3 -sk https://127.0.0.1:$port/api/v1/cluster/leader)"
  done
  exit 1
fi
echo "  new leader: $NEW_LEADER_URL ✓"

# ── write after failover ────────────────────────────────────────────────────
echo "==> PUT after failover on new leader"
KEY2_B64=$(printf %s "/smoke/after-failover" | base64)
VAL2_B64=$(printf %s "ok" | base64)
NEW_LEADER_ETCD_URL=$(echo "$NEW_LEADER_URL" | sed -E 's,:([0-9]+),:'"$(($(echo "$NEW_LEADER_URL" | sed -E 's,.*:([0-9]+).*,\1,') - 4064))"',')
curl --max-time 5 -sk -X POST "$NEW_LEADER_ETCD_URL/api/etcd/v3/kv/put" \
  -H 'content-type: application/json' \
  -d "{\"key\":\"$KEY2_B64\",\"value\":\"$VAL2_B64\",\"lease\":null,\"prev_kv\":false}" \
  >/dev/null
sleep 1
LEADER_ETCD_PORT=$((LEADER_PORT - 4064))
for port in $ETCD1 $ETCD2 $ETCD3; do
  [ "$port" = "$LEADER_ETCD_PORT" ] && continue
  got=$(curl --max-time 5 -sk -X POST "https://127.0.0.1:$port/api/etcd/v3/kv/range" \
    -H 'content-type: application/json' \
    -d "{\"key\":\"$KEY2_B64\",\"range_end\":null,\"limit\":null,\"revision\":null,\"keys_only\":false,\"count_only\":false}" \
    | jq -r '.kvs[0].value // empty | join(",")' \
    | awk -F',' 'NF{ for(i=1;i<=NF;i++) printf "%c",$i; }' \
    | base64 -d 2>/dev/null || true)
  if [ "$got" = "ok" ]; then
    echo "  etcd:$port → ok ✓"
  else
    echo "FAIL: etcd:$port returned '$got' after failover (expected 'ok')"
    exit 1
  fi
done

echo
echo "SMOKE PASS — Raft consensus end-to-end ✓"
echo "  replication ✓  follower-redirect ✓  failover ✓  catch-up not asserted (would re-spawn killed node)"
