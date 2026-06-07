<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
<!-- Copyright 2026 Cave Runtime contributors -->

# cave-runtime-tracker — bootstrap handoff

**Date:** 2026-06-07
**Branch:** `feature/cave-runtime-tracker` (worktree `../cave-runtime-tracker-bootstrap`)
**Status:** ✅ shipped on the cave-runtime side. cave-home side: re-dispatch needed (see §3).

---

## 1. What landed (cave-runtime side)

A new, self-contained crate **`crates/ops/cave-runtime-tracker`** — the cave-runtime
sibling of `cave-llm-tracker`. It tracks the platform seat the way the LLM
tracker tracks the model seat: it polls the latest GitHub release/tag of every
upstream OSS project a `cave-*` crate reimplements, classifies drift against the
version we have pinned, and writes a daily markdown + JSON report.

| Surface | Detail |
|---------|--------|
| Library | `config` · `error` · `poll` · `registry` · `report` modules, `#![forbid(unsafe_code)]` |
| Registry | 81 curated upstreams across ~75 distinct repos (`registry::default_registry`) |
| Binary | `cave-runtime-tracker` with subcommands `poll`, `report`, `config` |
| Config | YAML (`cave-runtime-tracker.yaml`), per-upstream `pinned:` baselines, `~/` expansion |
| Report | `daily-<date>.{md,json}` + `latest.json`; markdown grouped by category, stable `schema_version=1` |
| Schedule | `deploy/launchd/com.caveruntime.runtime-tracker.plist` — **06:30 local, daily** (30 min after cave-home) |
| Tests | 29 (`27` unit + `2` integration), all offline/deterministic via a fake `ReleaseFetcher` |

**Phase 0 mandate:** report only. Drift is surfaced, never acted on
(`phase_0_no_auto_bump: true` in every report).

### Design notes

- **Why a fetcher trait.** `poll_all` is generic over `ReleaseFetcher`, so the
  whole pipeline runs offline and deterministically under test. The binary wires
  in the live `GithubFetcher` (`releases/latest` → `tags` fallback, honours
  `GITHUB_TOKEN`, degrades to `unresolved` on any transport/rate-limit failure
  rather than aborting the run).
- **Distinct-repo fetch.** Many cave crates share one upstream
  (`kubernetes/kubernetes` backs five). `config::distinct_repos` dedupes so each
  repo is fetched exactly once, then the tag fans back out to every module.
- **Honest drift.** Rows with no `pinned:` baseline report `unknown`, not a
  fabricated `in-sync`. The reference YAML ships every row `pinned: null`;
  operators pin as they confirm ported versions.

---

## 2. Acceptance — verified

| Criterion | Evidence |
|-----------|----------|
| Crate builds | `cargo build -p cave-runtime-tracker` ✔, `cargo clippy` 0 warnings |
| `tracker poll` works | `cave-runtime-tracker poll` prints summary JSON (verified with `--config` override) |
| Daily markdown report | `cave-runtime-tracker report` wrote `daily-2026-06-07.{md,json}` + `latest.json` (81 subsystems, category tables) |
| launchctl plist 06:30 | `com.caveruntime.runtime-tracker.plist` — `StartCalendarInterval` Hour=6 Minute=30; `plutil -lint` OK |
| Tests pass | `cargo test -p cave-runtime-tracker` → 29 passed, 0 failed |

---

## 3. cave-home side — re-dispatch recommended

At the time of this bootstrap the parallel **cave-home-tracker** dispatch had
**not** produced a generic, reusable binary (no tracker crate/branch/worktree
existed under `/Users/gnomish/Code/cave-home`). Per the brief, the cave-runtime
side was therefore built as its **own complete copy** rather than bootstrapped
from a shared generic — which is also what the **cave-runtime ↔ cave-home strict
isolation rule** requires regardless: one tracker binary must never serve both
platforms.

**Recommendation for the cave-home dispatch:** build `cave-home-tracker` as a
*separate copy* in the cave-home repo, modeled on this crate's shape
(`config`/`registry`/`poll`/`report`/`ReleaseFetcher`/daily md+JSON), but with:

- a **cave-home upstream registry** — Matter/`project-chip`, ESPHome,
  Zigbee2MQTT, Home Assistant Core, Philips Hue, UniFi/unpoller, free@home,
  Frigate NVR, Mosquitto/MQTT, and the K3s components already being ported
  (apiserver-transport, kine, kubelet-cri, coredns-server, flannel-net,
  scheduler-loop, traefik-proxy);
- a `com.cavehome.home-tracker.plist` firing at **06:00 local** (this
  cave-runtime tracker is intentionally 30 min later at 06:30 so the two never
  contend for the GitHub rate limit);
- the same `GITHUB_TOKEN` env convention.

Do **not** symlink, share, or depend on the cave-runtime binary from cave-home.
Two copies, two registries, two plists — isolation preserved.

The two crates may later converge on a shared *design doc* (this file), but never
a shared compiled artifact.

---

## 4. Install (operator)

```sh
cargo build --release -p cave-runtime-tracker --bin cave-runtime-tracker
mkdir -p ~/.local/bin && cp target/release/cave-runtime-tracker ~/.local/bin/
cp deploy/launchd/com.caveruntime.runtime-tracker.plist ~/Library/LaunchAgents/
launchctl load -w ~/Library/LaunchAgents/com.caveruntime.runtime-tracker.plist
launchctl start com.caveruntime.runtime-tracker      # run once now
open "$HOME/Library/Application Support/cave-runtime/runtime-tracker"
```

Set `GITHUB_TOKEN` (in the plist `EnvironmentVariables` or your shell) before the
first real run — the registry has ~70 distinct repos, over the 60 req/h
unauthenticated GitHub limit.
