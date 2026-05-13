# Auto-port dispatcher — Charter "self-improving" closer

**Date:** 2026-05-13
**Status:** End-to-end pipeline landed. `GAP_OPENED → TaskQueue.submit
→ verify_completed → CharterGate.verify → AutoPortStatus { Merged |
CharterFail | BackendFail }` works against `DryRunTaskQueue`,
`PumpTaskQueue`, and `OpusTaskQueue`. 57 new deterministic tests
across 4 new modules.

## What landed

### Modules (`crates/cave-upstream-watchd/src/`)

| File | Purpose | Tests |
|---|---|---:|
| `prompt.rs` | Port-prompt builder — assembles upstream identity + version diff + parsed changelog + crate context + charter v2 gate clauses into the prompt string the TaskQueue submits. | 11 |
| `task_queue.rs` | `TaskQueue` trait + `TaskId` / `TaskStatus` / `TaskOutput` + 3 backends: `DryRunTaskQueue` (audit-only JSONL log), `PumpTaskQueue` (writes envelopes to `~/.../cave-qwen-pump/queue/`, reads completion markers from `completed/`), `OpusTaskQueue` (Anthropic Messages API + completion-text parser for `commit_sha:` / `branch:` / `files_changed:` / `lines_added:` / `test_count:`). | 14 |
| `charter_gate.rs` | `CharterGate` trait + `CharterV2Gate` — runs `cargo check --workspace --tests` + `cargo test -p <crate> --include-ignored`, reads on-disk `fill_ratio` before/after, counts workspace stubs (excluding `#[cfg(test)]` blocks), produces `VerifyResult` with `overall_pass`. | 15 |
| `auto_port.rs` | `AutoPortDispatcher` — `scan_and_dispatch` + `verify_completed` + JSONL state file (`dispatched.jsonl`) + audit log (`audit.jsonl`) + cooldown after `charter_fail` (default 24h) + rate limit (default `max_concurrent=3`) + `CAVE_AUTOPORT_DISABLE=1` kill switch. | 11 |

### CLI

```text
cave-upstream-watchd dispatch
    --backend dryrun|pump|opus
    [--workspace <root>]
    [--events <events.jsonl>]
    [--state <dispatched.jsonl>]
    [--audit <audit.jsonl>]
    [--scan-only]

cave-upstream-watchd dump-dispatched
    [--state <dispatched.jsonl>]
```

Default backend is `dryrun` — the dispatcher records what it would
have submitted but never side-effects. Operator opts into `pump` /
`opus` explicitly.

### Portal `/admin/upstream` enhancement

Third panel below the existing Upstream + Watchd panels. Reads
`dispatched.jsonl` (live, newest-first); renders per-row:

* `cave-module`
* `task_id` (backend handle)
* `backend` name (`dryrun` / `pump` / `opus`)
* status badge: `MERGED` (green) / `DISPATCHED` (blue) / `RUNNING`
  (blue) / `CHARTER_FAIL` (orange) / `BACKEND_FAIL` (red)
* short commit SHA (first 7 chars)
* target branch
* `dispatched_at` timestamp

**Persona filter**: TenantAdmin sees only the 7 tenant-relevant
crates (`cave-vault`/`-keda`/`-kubelet`/`-streams`/`-cache`/`-pg`/`-docdb`);
PlatformAdmin sees everything. Tenant view shows a "switch to
platform_admin for full audit trail" hint.

6 new portal tests including empty-state, every-lifecycle-badge,
tenant filter, `max_rows` cap, newest-first ordering, missing-file
fallback.

### launchd plist

`scripts/com.cave.auto-port-dispatcher.plist` — `StartInterval=900`
(15 min). Backend defaults to `dryrun` so loading the plist is safe;
operator flips to `pump` / `opus` by editing the ProgramArguments
and (for opus) adding `ANTHROPIC_API_KEY` to `EnvironmentVariables`.

## Live end-to-end smoke

```text
$ ./target/debug/cave-upstream-watchd dispatch \
    --workspace /tmp/auto-port-smoke.XXXX \
    --events     /tmp/auto-port-smoke.XXXX/events.jsonl \
    --state      /tmp/auto-port-smoke.XXXX/dispatched.jsonl \
    --audit      /tmp/auto-port-smoke.XXXX/audit.jsonl \
    --backend dryrun

dispatch: considered=1 dispatched=1 already=0 skipped_disabled=0 skipped_cooldown=0 skipped_rate=0 errors=0
verify:   considered=1 still_running=1 merged=0 charter_failed=0 backend_failed=0

# dispatched.jsonl now has one record with status="running".
# audit.jsonl has one "dispatched" entry.

# Kill switch:
$ CAVE_AUTOPORT_DISABLE=1 cave-upstream-watchd dispatch ...
dispatch: considered=0 dispatched=0 already=0 skipped_disabled=1 ...

# Idempotency:
$ cave-upstream-watchd dispatch ...    # second time
dispatch: considered=1 dispatched=0 already=1 ...
```

## Safety surface

| Guarantee | Implementation |
|---|---|
| Idempotency | `dispatched.jsonl` keyed by `event_id`; `scan_and_dispatch` skips already-dispatched events. |
| Cooldown after charter_fail | Record's `dispatched_at` + `cfg.cooldown` (default 24h); re-attempts before the window land in `skipped_cooldown`. |
| Rate limit | `cfg.max_concurrent` (default 3) caps simultaneous `Dispatched|Running` records; excess events land in `skipped_rate_limit`. |
| Kill switch | `CAVE_AUTOPORT_DISABLE=1` env var (read every scan tick) — operator can toggle without unloading the daemon. |
| Audit log | `audit.jsonl` records every `dispatched` / `merged` / `charter_fail` / `backend_fail` action with timestamp + task_id + note. |
| Backend audit (dryrun) | `dryrun` backend writes every prompt's metadata to its own log file — operators can review what the dispatcher would have submitted before going live. |
| Stub-policy gate | `CharterV2Gate.count_workspace_stubs` walks `crates/*/src/**/*.rs` excluding `#[cfg(test)]` blocks, refuses merge if the count rose. |
| Ratio gate | `CharterV2Gate.read_fill_ratio` reads on-disk `parity.manifest.toml::[parity] fill_ratio` BEFORE (snapshot at dispatch) + AFTER (snapshot at verify); strict `> 0` delta required to merge. |
| Auth secret hygiene | `OpusTaskQueue::from_env` reads `ANTHROPIC_API_KEY` exclusively from env; never persisted to `dispatched.jsonl` or `audit.jsonl`. |
| Disabled backend mode | `OpusTaskQueue::disabled = true` refuses `submit` with a clear `TaskQueueError::Disabled` so audit-mode operators can install the plist without accidentally hitting the API. |

## Workspace impact

| Crate | Before | After | Tests |
|---|--:|--:|--:|
| `cave-upstream-watchd` lib | 48 | **99** | +51 |
| `cave-upstream-watchd` bin | 4 | 4 | — |
| `cave-portal --lib` | 1657 | **1665** | +6 panel + 2 pre-existing |

`cargo check --workspace` clean (pre-existing warnings only). Zero
`unimplemented!()` / `todo!()` / `#[ignore = "impl pending"]`.

## What's deliberately out of scope

* **Real charter-v2 git merge** — once a task hits `Merged`, the
  current code just records the status. The actual `git merge`
  needs the dispatcher to (a) check out the auto-port branch, (b)
  run the gate against `HEAD` of that branch (already wired —
  CharterV2Gate operates on the workspace root which the operator
  has checked out), (c) `git checkout main && git merge --no-ff
  <branch>` and `git push`. We didn't add the merge step because
  Charter says human review for the first N merges; operator can
  graduate it to auto-merge by extending `verify_completed`.
* **Anthropic API key secret store** — `OpusTaskQueue::from_env` is
  the entry point. Production deployments should pull the key from
  a secrets store (Keychain on macOS, systemd-creds on Linux) and
  inject it into the daemon's env at load time. Doc'd in the plist
  comment; not implemented here.
* **Per-tenant auto-port routing** — the dispatcher submits every
  GAP to the same backend. A future enhancement would route tenant-
  scoped crates (vault/keda/...) to a tenant-isolated queue.
* **WebSocket push to the portal** — the portal panel reads
  `dispatched.jsonl` on each request. A live event stream so
  operators see new `merged` records without refresh is a separate
  sweep (SSE infrastructure already exists from the realtime
  batch — adopter wiring).
* **Auto-update parity-index.json on merge** — the dispatched
  record carries `fill_ratio_after` from the gate; the parity-index
  build script already reads manifest live, so the next index regen
  picks up the new value automatically.

## How a fresh end-to-end run works in production

```text
1. cave-upstream-watchd poll       (every 5 min, com.cave.upstream-watchd.plist)
       │
       └→ writes new GAP_OPENED to events.jsonl
2. cave-upstream-watchd dispatch   (every 15 min, com.cave.auto-port-dispatcher.plist)
       │
       ├→ scan_and_dispatch:
       │     for each unprocessed event:
       │       resolver.resolve(event)        → PortContext + CharterBaseline
       │       prompt = build_prompt(...)
       │       task_id = queue.submit(prompt, branch)
       │       state[event_id] = Dispatched
       │
       └→ verify_completed:
            for each Dispatched|Running:
              queue.status(task_id) → Completed{sha} | Running | Failed
              if Completed:
                gate.verify(baseline, sha) → VerifyResult
                if overall_pass: state = Merged + audit("merged")
                else            : state = CharterFail + audit("charter_fail")
              if Failed:
                state = BackendFail + audit("backend_fail")
3. Portal /admin/upstream renders state + audit live for the operator.
```
