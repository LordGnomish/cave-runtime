# night-pump

Coordinates Cave Runtime worker batches against a YAML queue. Standalone Rust binary that lives outside the `cave-runtime` workspace (`tools/night-pump/`).

A worker (Qwen, Sonnet, Opus, or a human at the keyboard) polls `/api/next-batch?worker_id=...` to pick up the next dispatchable batch — highest priority, dependencies satisfied, under the in-flight cap. When the worker is done it `POST`s `/api/batch-complete` with the outcome; the row is appended to `contributions.jsonl` for after-the-fact accounting.

## Quickstart

```bash
cd tools/night-pump
cargo build --release
./start.sh                          # daemonised, pid in night-pump.pid
curl -s http://localhost:9090/api/heartbeat | jq
./stop.sh
```

Override port / cap with `NIGHT_PUMP_PORT` and `NIGHT_PUMP_MAX_PARALLEL` env vars.

## CLI

```
night-pump --queue queue.yaml \
           --state state.json \
           --contributions contributions.jsonl \
           --heartbeat-log log/heartbeat.log \
           --port 9090 \
           --max-parallel 8
```

## Files

| Path | Role |
|------|------|
| `queue.yaml` | source of truth — list of batches to dispatch (read at startup) |
| `state.json` | per-batch runtime state (status, dispatched_at, retry_count, worker_id) — written on every change |
| `contributions.jsonl` | append-only worker log (one JSON object per line) |
| `log/heartbeat.log` | one line per 60 s tick: timestamp, disk free, memory free, completed, in-flight |
| `log/stdout.log` | server stdout/stderr (`start.sh` redirects here) |
| `night-pump.pid` | running pid (`start.sh` writes, `stop.sh` reads) |

## API

### `GET /api/next-batch?worker_id=<id>`

Returns the next dispatchable batch as JSON, or `null` if nothing is ready.

A batch is dispatchable when:
- it is `Queued`, **or** `Failed` with `retry_count < retry_max`;
- every entry in its `dependency` list is `Completed`;
- the worker pool has fewer than `max_parallel` in-flight batches;
- dispatch is not paused (resource pressure).

When a batch is returned the server marks it `Dispatched`, records `dispatched_at` and `worker_id`, and persists `state.json`.

```bash
curl -s 'http://localhost:9090/api/next-batch?worker_id=qwen-3-coder-next' | jq
```

### `POST /api/batch-complete`

Body:
```json
{
  "worker_id":     "qwen-3-coder-next",
  "batch_id":      "cave-etcd-deeper-003",
  "status":        "completed",
  "commit_sha":    "abc123…",
  "test_delta":    47,
  "lines_added":   1200,
  "lines_removed": 50,
  "error":         null
}
```

`status` is `"completed"` or `"failed"`. On `failed`, `retry_count` is incremented and `last_error` set; the batch becomes dispatchable again until `retry_count == retry_max`.

A `Contribution` row is appended to `contributions.jsonl` regardless of outcome.

### `GET /api/heartbeat`

Returns aggregate stats:
```json
{
  "queued": 12,
  "dispatched": 3,
  "in_progress": 0,
  "completed": 28,
  "failed": 0,
  "total": 43,
  "max_parallel": 8,
  "dispatch_paused": false,
  "last_heartbeat": "2026-04-26T09:00:00Z"
}
```

### `GET /api/contributions?since=<RFC3339>`

Returns per-worker aggregates from `contributions.jsonl`, optionally filtered to entries with `completed_at >= since`:
```json
{
  "qwen-3-coder-next": {
    "batches": 21, "completed": 19, "failed": 2,
    "test_delta": 940, "lines_added": 28430, "lines_removed": 1120
  },
  "sonnet-4-6-A": { … },
  "claude-opus-4-7": { … },
  "manual-burak": { … }
}
```

## Worker IDs (convention)

`qwen-3-coder-next` · `sonnet-4-6-A` · `sonnet-4-6-B` · `sonnet-4-6-C` · `sonnet-4-6-D` · `sonnet-4-6-E` · `claude-opus-4-7` · `manual-burak`

The server treats `worker_id` as opaque — any non-empty string is accepted. The convention is for downstream attribution.

## Tick task (60 s)

Every minute the server:
1. Captures system snapshot via `sysinfo` (smallest available_space across mounted disks; available memory).
2. Writes one line to `log/heartbeat.log`: `<rfc3339> disk_free_gb=… mem_avail_gb=… completed=… in_flight=…`.
3. Updates `last_heartbeat` in `state.json`.
4. Pauses dispatch (`dispatch_paused = true`) if disk free `< 30 GB` or memory available `< 4 GB`. Pause clears automatically when both checks recover.

## Tests

```bash
cargo test
```

Covers: queue parse against the shipped `queue.yaml`, the dispatch state machine (`is_dispatchable`, `deps_satisfied`, retry bookkeeping), the four HTTP contracts, and the in-flight concurrency cap.
