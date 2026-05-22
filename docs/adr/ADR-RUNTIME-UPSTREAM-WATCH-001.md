# ADR-RUNTIME-UPSTREAM-WATCH-001 — Upstream watch daemon

- Status: Accepted
- Date: 2026-04-28
- Authors: Cave Runtime team
- Component: `crates/cave-upstream`
- Charter article: "self-improving"

## Context

Cave Runtime reimplements 66 upstream OSS projects in Rust+eBPF. The
"self-improving" article of the runtime Charter requires that whenever
an upstream ships a new release, the Qwen TDD pump is automatically
notified, so the port can converge against the new surface. Before this
ADR the only mechanism was a weekly GitHub Actions cron
(`.github/workflows/upstream-tracker.yml`) that ran `gh api … releases/latest`
against a hand-maintained 26-repo bash array — drifted from the registry
in `cave-upstream::projects`, no state, no pump trigger.

We need to:

1. Detect new upstream releases within minutes, not days.
2. Drive the Qwen pump *automatically* — no human-in-the-loop.
3. Stay well below the GitHub API rate limit even at 66 repos.
4. Survive crashes — never miss a release because of a transient error.

## Decision

Add a new daemon binary `cave-upstream-watchd` (in `crates/cave-upstream`)
that runs continuously, polls the GitHub Releases API per-project at a
**tiered cadence**, and writes a JSON payload to the Qwen pump queue dir
on every detected tag transition. Source of truth for which repos to
poll is `crate::projects::TRACKED_PROJECTS`.

### Tiered cadence

- **15 minutes** for the 12 high-priority kernel modules listed in
  `cave_upstream::HIGH_PRIORITY_MODULES` — `cave-apiserver`, `cave-etcd`,
  `cave-scheduler`, `cave-cri`, `cave-net`, `cave-mesh`, `cave-streams`,
  `cave-pg`, `cave-docdb`, `cave-vault`, `cave-cache`, `cave-registry`.
- **60 minutes** for the remaining 54 repos.

#### Rationale

A new upstream release is typically observable on GitHub Releases
within 1–3 hours of being cut. A 15-minute window is the practical
upper bound for "we noticed effectively immediately"; a tighter window
is wasted budget. Normal-priority repos getting a 60-minute cadence
keeps the API-call budget low (264 calls/h vs 4× that with uniform
15-min) while still being 168× faster than the previous weekly cron.

#### Budget

- High-priority: 12 repos × 4 polls/h = **48 calls/h**
- Normal: 54 repos × 1 poll/h = **54 calls/h**
- Total: **~102 calls/h**, **2% of 5,000/h authenticated limit**

Conditional requests (ETag / `If-Modified-Since`) reduce this further
in steady state — 304 responses do not count against the limit.

### Detection ≠ port — payload contract is the only coupling

The daemon's responsibility ends at writing a JSON file to
`~/Library/Application Support/cave-qwen-pump/queue/upstream-port-<ms>-<slug>.json`
matching the [`PumpPayload`](../../crates/cave-upstream/src/pump.rs)
schema. The pump consumes the file and decides what to do (TDD port
job, batched merge overnight, etc.). This decoupling means we can
swap either side independently.

### Auto-PR draft, never auto-merge

When the pump produces a port branch, it opens a *draft* PR and
requests human review. Auto-merging upstream API changes into Cave
without review is a safety hazard — drift in semver-minor releases
of, say, `etcd-io/etcd` could ship behavior we don't want.

### Single source of truth: `projects::TRACKED_PROJECTS`

The hardcoded 26-repo bash array in the GHA workflow is deleted. The
workflow now invokes `cave-upstream-watchd --once` which reads the
registry. Anyone adding a new upstream edits one file
(`crates/cave-upstream/src/projects.rs`) and both the local LaunchAgent
and the GHA workflow pick it up.

### Persistence

State is one JSON file (`upstream-state.json`) holding per-repo
`{last_known_tag, etag, last_modified, consecutive_errors, …}`. Atomic
write via `tempfile + rename`. Operationally this trumps sled / sqlite:
operators can `cat` and hand-edit it, and the data set is small.

### Backoff

Per-project consecutive-error counter. Effective cadence ×=
`2^consecutive_errors` capped at `max_backoff_ticks` (default 16). Any
successful round-trip (200 *or* 304) resets to 0. Rate-limit (403/429)
counts as an error so backed-off repos don't keep hammering during the
ratelimit window.

### Jitter

Tick interval has a uniform random `[0, +tick_jitter]` offset added to
the sleep so deployments across multiple boxes don't synchronise on
the wall-clock minute boundary.

## Consequences

### Positive
- New upstream releases reach the pump in ≤15 min for kernel modules,
  ≤60 min otherwise — vs ≤7 days previously.
- Single source of truth for the project registry.
- ETag-conditional polls keep us comfortably under rate limit.
- State file makes restart safe — no missed releases, no duplicate
  payloads.

### Negative / costs
- The LaunchAgent must be installed and kept running on each operator's
  machine (or a long-lived host) for *real-time* detection. CI provides
  a redundancy net but is rate-limit constrained per repo for
  unauthenticated runs and is not the source of truth.
- One always-running Rust process per operator.
- We are coupled to GitHub Releases as the release-signal source. Some
  upstreams tag without releasing — those will be caught by the
  Phase-2 source-level differ (see below) but not by Phase 1.

## Phase 2 — deferred work

This ADR explicitly defers the following to a separate effort:

1. **Source-level public-API surface diff.** Real diffs across Go
   (`go doc -all` / AST), Java (`javap`), TypeScript (`.d.ts`
   diff), and Rust (`cargo public-api`) require:
   - cloning each upstream at the new tag,
   - running language-specific toolchains in a sandbox,
   - caching by `(repo, tag, sha)`.
   This is multi-week work per ecosystem and is out of scope for
   this initial daemon. The
   [`SurfaceDiffer`](../../crates/cave-upstream/src/delta.rs) trait
   plugs Phase-2 differs in without changing the daemon loop. The
   Phase-1 default is `TagOnlyDiffer`, which emits an empty diff — the
   *tag transition itself* is the signal.

2. **Auto-PR generation from pump payloads.** Belongs in the Qwen
   pump, not here. The daemon writes payloads; the pump opens PRs.

3. **Slack / digest notification.** Belongs in a separate
   notification crate that watches the pump queue dir, not in
   `cave-upstream`.

## Alternatives considered

### Push-based webhooks instead of polling
GitHub supports per-repo `release` webhooks. Rejected because:
- Requires a publicly reachable webhook endpoint (more infra than the
  Charter currently mandates).
- Requires per-repo write access to install. We don't own those repos.
- Misses tag-only releases.

Polling with ETag conditional requests is *trivial* operationally and
costs almost nothing on the rate limit.

### sled / sqlite for state
Rejected — see "Persistence" above. The dataset is too small (≤66
records, hundreds of bytes each) to justify the operational complexity.

### Uniform 15-minute cadence for all 66 repos
Rejected — 4× the API budget for no win on the long tail of
slow-moving repos (e.g. `apache/incubator-devlake` cuts a release
every few months).

## References

- `crates/cave-upstream/src/projects.rs` — registry of all tracked repos
- `crates/cave-upstream/src/daemon.rs` — driver loop
- `crates/cave-upstream/src/delta.rs` — release detection + `SurfaceDiffer` trait
- `crates/cave-upstream/src/pump.rs` — pump payload contract
- `crates/cave-upstream/src/state.rs` — persistent state
- `deploy/launchd/com.btartan.cave-upstream-watchd.plist` — LaunchAgent
- `.github/workflows/upstream-tracker.yml` — CI redundancy
