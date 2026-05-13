# Fix-A (manifest-primary parity-index) + Fix-C (cave-upstream-watchd scaffold)

**Date:** 2026-05-13
**Status:** Both paketler landed. Fix-A makes the parity-index live
from each crate's `parity.manifest.toml::[parity] fill_ratio`; Fix-C
introduces a dedicated `cave-upstream-watchd` workspace crate with
GitHub release polling, semver diff, structured changelog parser,
JSONL event sink, and a Portal `/admin/upstream` panel that reads
the events live.

## Fix-A — parity-index manifest-primary

### What changed

* `scripts/build-parity-index.py`
  * Manifest is now the **primary** source for `parity_ratio`. The
    audit doc remains the source for tier, upstream identity, and
    notes — everything that doesn't drift between commits.
  * Every entry carries a new `parity_ratio_source` field:
    `"manifest"` (live `parity.manifest.toml::fill_ratio`),
    `"audit"` (2026-05-01 snapshot), or `"none"` (never measured).
  * When the on-disk manifest carries a `fill_ratio`/`ratio`, it
    wins unconditionally — including when newer than the doc and
    when the disk value is honest-zero. Removed the date-gated
    "newer audit wins" branching that previously left some 0.25
    audit-doc values shadowing 0.7556 measurements.
  * The build script also propagates `last_audit_disk` so the
    dashboard can render "measured YYYY-MM-DD" alongside the ratio.
* `crates/cave-portal/src/admin/compliance.rs`
  * `ParityIndexEntry` gained `parity_ratio_source` +
    `last_audit_disk` fields.
  * `CrateCompliance` gained `parity_ratio_source` +
    `parity_ratio_last_audit` fields.
  * `attach_parity_index` now runs a second pass that re-reads
    each crate's on-disk `[parity] fill_ratio` live, so the
    dashboard reflects the newest measured value even when the JSON
    index lags. The fallback (audit-doc snapshot) still applies
    when no manifest exists on disk.

### Verification

`python3 scripts/build-parity-index.py`:

| Source bucket | Count |
|---|---:|
| `parity_ratio_source = "manifest"` | **94** |
| `parity_ratio_source = "audit"` | 9 |
| `parity_ratio_source = "none"` | 5 |
| **Total** | **108** |

Spot checks (Burak's expected values):

| Crate | parity_ratio | source |
|---|--:|---|
| cave-cri | 0.9118 | manifest |
| cave-cache | 0.8684 | manifest |
| cave-rdbms-operator | 0.84 | manifest |
| cave-controller-manager | 0.7556 | manifest |
| cave-kubelet | 0.8158 | manifest |

`cargo test -p cave-portal --lib`: **1396 pass** (was 1387; +9 new
watchd-panel tests). `cargo check --workspace`: clean.

## Fix-C — cave-upstream-watchd crate

### New crate: `crates/cave-upstream-watchd/`

Dedicated workspace member (split out of the in-place daemon that
lived under `crates/cave-upstream/src/{daemon,state,delta}.rs`).
Burak's self-healing/self-improving mandate; this scaffold is the
foundation — it **detects + publishes** gaps. The auto-port
dispatcher that consumes the events is BACKLOG and out of scope.

| File | Purpose | Tests |
|---|---|---:|
| `Cargo.toml` | workspace member; reqwest + clap + semver | — |
| `src/lib.rs` | module wiring + re-exports | — |
| `src/tracked.rs` | walks `crates/*/parity.manifest.toml` → `TrackedProject` list with priority derived from a 12-module high-priority allow-list | 8 |
| `src/persistence.rs` | `state.json` atomic write (tempfile+rename); `ETag` / `Last-Modified` cache; `consecutive_errors` counter | 5 |
| `src/poller.rs` | GitHub `/releases/latest` fetch with `If-None-Match` + `If-Modified-Since`; 304 / 404 / rate-limit branches; `httpmock`-tested | 5 |
| `src/diff.rs` | `semver::Version`-based compare; `Severity { Major, Minor, Patch, None, Unknown }`; pre-release suffix trim; 2-part-version padding | 10 |
| `src/changelog.rs` | Keep-a-Changelog parser → `Changelog { entries: Vec<ChangelogEntry { kind, description, breaking }> }`; supports `##`/`###` + `**Heading**`; inline `BREAKING` keyword promotes any bullet | 12 |
| `src/event.rs` | `GapEvent` struct + `GapEventSink` trait + `JsonlSink` (append-only, fsync); `read_events` returns newest-first | 6 |
| `src/bin/main.rs` | CLI: `poll`, `list`, `dump-events`; honours `GITHUB_TOKEN` + `CAVE_WATCHD_*` env vars | 4 |

**Test totals:** **48 lib + 4 bin = 52 deterministic tests.**

### Launchd plist

`scripts/com.cave.upstream-watchd.plist` —
`StartInterval = 300` (5-minute cadence), stdout/stderr to
`~/Library/Application Support/cave-runtime/watchd/watchd.{log,err}`,
optional `GITHUB_TOKEN` env entry commented out (operator must
populate locally; do not commit a real token). Linux systemd
equivalent intentionally deferred to the production deploy work.

### Portal `/admin/upstream` enhancement

`crates/cave-portal/src/admin/upstream.rs` —
new `render_watchd_panel_in(ctx, events_path, max_rows)` reads
`<data_dir>/watchd/events.jsonl` and renders the most recent
`GAP_OPENED` events with:

* Severity badge (`MAJOR`=red, `MINOR`=orange, `PATCH`=yellow,
  `UNKNOWN`=grey).
* Gap-age formatted as `s/m/h/d`.
* Newest-first ordering.
* **Persona filter**: PlatformAdmin sees everything; TenantAdmin
  sees only the 7 tenant-relevant modules (vault, keda, kubelet,
  streams, cache, pg, docdb). A "Tenant view" note explains the
  filter.

9 new tests, all green:
* empty-events render shows a friendly placeholder
* events lex-newest-first
* tenant persona filters out non-relevant
* platform persona sees everything
* severity badges + classes rendered
* `max_rows` cap respected
* `Unknown` severity gets grey badge
* `format_age` bucketing covers s/m/h/d
* `TENANT_RELEVANT_MODULES` audit list matches the charter

## What's deliberately NOT in this batch

* **Auto-port dispatcher** — consuming `GAP_OPENED` events and
  drafting Qwen/Opus port prompts. Charter v2 gate (real-run test,
  no stub) is needed for safety; this is a separate ~1.5k-LOC
  sweep.
* **Live workspace-level cron / systemd timer** — only macOS
  launchd shipped today. Linux systemd unit can mirror the same
  CLI; deferred.
* **HTTP webhook / NATS sinks** — `GapEventSink` trait exists, but
  only the `JsonlSink` impl ships. Future sinks drop in without
  touching the daemon.
* **Watchd UI badges in `/admin/compliance`** — the dashboard
  already shows `parity_ratio_source`; surfacing pending GAP events
  alongside is a follow-up.

## Workspace test impact

| Crate | Before | After |
|---|--:|--:|
| cave-portal lib | 1387 | **1396** |
| cave-upstream-watchd lib | n/a | **48** (new) |
| cave-upstream-watchd bin | n/a | **4** (new) |

`cargo check --workspace` clean (pre-existing warnings only).

## Stub policy honored

Zero `unimplemented!()` / `todo!()` / `#[ignore = "impl pending"]`.
Every code path covered by a deterministic test using `httpmock` for
the HTTP boundary.
