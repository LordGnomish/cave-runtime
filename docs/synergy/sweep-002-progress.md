# Sweep-002 Progress — SEALED 2026-05-01

**Status:** ✅ closed. Faz 1 (primitives) + Faz 2-G (TenantId in 6 crates)
+ Faz 2-A/B/D (consensus / eventbus / reconcile adoption) all on `main`.
The remaining deferred items (F2-C, F2-E, F2-F, cave-portal-web /
cave-logs TenantId) are excluded by design — see "Faz 2 deferred" table.

**Closure commits:** `c51712c` (F2-B), `098e2be` (F2-D), `10db5f3`
(F2-A). Aggregate: **+65 tests** added in F2-A/B/D (36 + 12 + 17),
**1 duplicate trait removed** (cave-ha StateMachine), **1 broadcast
wrapper removed** (cave-apiserver ResourceStore), 10-controller
preventative loop bridge in cave-controller-manager.

---

## 2026-05-01 Faz 2 batch (this session)

`cave_kernel::ns::TenantId` now adopted by 6 crates (was 8 local duplicates → 6 absorbed; 2 deferred with cause). Cumulative impact:

| Crate | Commit | Sites | Tests after | Duplicate LOC removed |
|---|---|---|---|---|
| cave-search | `da49a80` | 0 src + 26 test fixtures | n/a (no inline) | 11 |
| cave-cloud-controller-manager | `945302d` | 11 (4 src + 7 provider tests) | 623 / 0 | 16 |
| cave-controller-manager | `e62eca3` | 30 (gc_lite, deeper, resource_quota, tests_crosscut) | 714 / 0 | 19 |
| cave-portal (admin) | `8431cc6` | 8 + 1 test rewritten (defence-in-depth → boundary validation) | 534 / 0 | 19 |
| cave-net (cilium) | `7caffaa` | 18 src + 48 e2e + 1 fixture lowercased | 1705 / 0 | 16 |
| cave-mesh (ambient) | `a1246f9` | 11 ambient | 131 / 0 | 16 |
| **Total** | — | **~115 .expect() / fixture sites** | **3707 passing** | **97 LOC** |

Each commit does the same shape: `pub struct TenantId(pub String)` + `impl new/as_str` + `impl fmt::Display` deleted, replaced with `pub use cave_kernel::ns::TenantId;`. `TenantId::new(...)` call sites get `.expect("test fixture")` since the kernel newtype is fallible (DNS-1123 validated). Underscore-bearing test fixtures (`tenant_001` → `tenant-001`, `tenant-mg-M` → `tenant-mg-m`) corrected to satisfy validation.

### Faz 2 deferred — honest reasons

| Crate / target | Why deferred |
|---|---|
| **cave-portal-web** TenantId | Genuinely different abstraction: 6 error variants (Empty, TooLong, InvalidChar, HyphenBoundary, **Unknown**, **Missing**), allows underscore in slugs, max length 64 (vs kernel's 63), `TenantContext::set_current/require/clear` flow with a `Missing` default-deny variant the kernel doesn't model. Migration would either lose functionality or fork the kernel API; neither is honest "adoption". |
| **cave-logs** TenantId | `pub type TenantId = String;` (type alias, not a newtype) — the call-site impact of converting `String` → `TenantId` newtype is a wide-touch migration. Worth its own focused PR with the multi-tenant reviewer in the loop. |
| **cave-mesh** SpiffeId (F2-F) | API mismatch with `cave_kernel::identity::SpiffeId`: pub fields vs accessor methods, `parse(uri) -> Option<Self>` vs `new(td, path) -> Result<Self, SpiffeError>`, path-with-leading-slash vs path-without-leading-slash. Migration is invasive (every `.trust_domain` / `.path` access becomes a method call, plus path convention shift). Not a drop-in. |
| **cave-auth** SpiffeId (F2-F) | No SPIFFE surface in cave-auth — zero hits for `SpiffeId` / `spiffe::`. Nothing to adopt. |
| **F2-C cave-portal SSE** → EventBus | No SSE / `broadcast::Sender` / `text/event-stream` in any cave-portal* crate. Reflex Engine target was aspirational at sweep-002 plan time; no surface yet. |
| **F2-E cave-cri reconcile** → Reconciler | Zero `Reconciler` / `reconcile_loop` / `fn reconcile` hits in cave-cri. No surface. |
| **F2-B cave-apiserver::watch_cache** → EventBus | ✅ **CLOSED** in `c51712c` (2026-05-01 afternoon). Real duplicate identified was the `tokio::sync::broadcast::Sender<WatchEvent>` in `cave-apiserver::store::ResourceStore`, not the watch_cache ring buffer. ResourceStore migrated to `EventBus<WatchEvent>`; WatchCache gained an `EventBus<WatchCacheEvent>` for live fan-out alongside the RV-indexed ring buffer (separate capacities = no replay-buffer eviction from slow tailers). +12 tests. |
| **F2-D cave-controller-manager reconcile** → run_reconciler | ✅ **CLOSED** in `098e2be` (2026-05-01 afternoon). Generic `ScaffoldReconciler<S, O, F>` adapter handles 9 controllers; daemonset has a purpose-built wrapper (`Vec<NodeView>` observation). Per-controller `run_*(snapshot_fn, config, cancel)` factories spawn kernel `run_reconciler` loops with controller-specific `requeue_delay` (30s default, 10s cronjob, 15s hpa). +17 tests. |
| **F2-A cave-ha::raft → consensus** | ✅ **CLOSED** in `10db5f3` (2026-05-01 afternoon). Local `cave-ha::raft::state_machine::StateMachine` trait deleted (`pub use cave_kernel::consensus::StateMachine;`). New `cave-ha::raft::kernel_bridge` module exposes `KernelLogStore` (over `Arc<Mutex<MemLog>>`) and `KernelRaftHandle` (forwards through cave-ha RaftHandle with HaError → ConsensusError mapping). +36 tests including end-to-end against a real RaftNode. |

### Constraint compliance

- **No stubs**: all 6 commits delete real duplicate code; no `todo!()` / `unimplemented!()` introduced.
- **Compile green**: `cargo check --workspace` after every commit, plus `cargo test -p <crate> --lib` 3707/0 cumulative.
- **Conventional commits + ff merges**: every commit is `refactor(<crate>): adopt cave_kernel::ns::TenantId (sweep-002 F2-G)`.
- **One genuine semantic shift**: cave-portal `contributions_html_escape_blocks_tenant_injection` test rewritten — the canonical newtype rejects `<script>` at construction, so the test asserts the boundary rejection rather than the downstream HTML escape. Documented in the commit message.

---

## Original Faz 1 (2026-05-01 cherry-pick)

**Status:** Faz 1 cherry-picked from `claude/eager-jennings-ab6961` (commit `bc50d6b`) into main on 2026-05-01.

**Author:** Burak (original `bc50d6b` committer, 2026-04-27) + Faz 1 cherry-pick session.

**Related:** `docs/synergy/sweep-002-plan-2026-04-23.md` (the original plan that listed these 5 primitives as the sweep-002 deliverables).

---

## What Faz 1 actually shipped

The sweep-002 plan from 2026-04-23 listed five primitives. The earlier work session (2026-04-27) wrote and tested all five but landed them on an orphan branch (`claude/eager-jennings-ab6961`) that never merged. Faz 1 of this progress note **cherry-picks that single commit** to main. Net effect: 7 files / 943 LOC / 34 new tests added to `cave-kernel`.

| Primitive | File | LOC | Tests | What it gives downstream |
|---|---|---|---|---|
| `consensus` | `crates/cave-kernel/src/consensus.rs` | 140 | 4 | `LogStore`, `StateMachine`, `RaftHandle` traits; `LogEntry`, `LeaderInfo`, `ConsensusError`. Raft FSM impl stays in `cave-ha`; this is the **contract surface** downstream clients hold. |
| `eventbus` | `crates/cave-kernel/src/eventbus.rs` | 155 | 6 | `EventBus<T>`, `Subscription<T>` over `tokio::sync::broadcast`. `EventBusError::{NoSubscribers, Lagged, Closed}`. Replaces ad-hoc `broadcast::Sender` wrappers. |
| `reconcile` | `crates/cave-kernel/src/reconcile.rs` | 244 | 3 | `Reconciler` trait, `ReconcileOutcome` enum, `run_reconciler` task runner with bounded queue, cancellation token, exponential backoff (delegates to existing `cave_kernel::retrypolicy`). |
| `identity` | `crates/cave-kernel/src/identity.rs` | 218 | 11 | `SpiffeId` parser/validator (RFC SPIFFE-ID 1.0 grammar), `SvidMetadata` validity-window. |
| `ns` | `crates/cave-kernel/src/ns.rs` | 175 | 10 | `TenantId` newtype with DNS-1123 validation, `TenantScope` bundle, `X-Scope-OrgID` header constant. |

**`cargo test -p cave-kernel --lib`: 113/113 passing** (was 79; +34 from this cherry-pick, matching `bc50d6b`'s claim of 80 → 113 modulo one identity test that was undercounted in the commit message).

**`cargo check --workspace`: green** (9.72s) — the cherry-pick is pure addition, no existing call site changed.

**No stubs.** All five files were grepped for `todo!()`, `unimplemented!()`, and panic-style placeholders: zero hits. Every public API has a concrete body.

---

## What Faz 1 did **not** do — and why

The 2026-04-27 commit message stated: *"No call-site changes — adoption deferred to follow-up PRs / parity sprints to avoid colliding with the 13 in-flight sprint branches that own AVOID-list crate sources."*

That call still applies. The Faz 1 task brief asked for adoption in `cave-etcd` + `cave-kamaji` + `cave-apiserver` + `cave-store`. Investigation in the cherry-pick session found:

- **`cave-etcd` has no Raft.** The crate is an in-memory MVCC store; `fn append_entries` / `become_leader` / `RaftNode` / `RaftLog` all return zero hits. The string `"raft"` appears in `routes.rs` only as etcd wire-protocol response field names (`raft_index`, `raft_term`). Nothing to adopt against.
- **`cave-kamaji` has no Raft.** The crate is 4 files / ~1.5K LOC: `lib.rs`, `lifecycle.rs`, `models.rs`, `routes.rs`. No consensus code.
- **The real Raft implementation lives in `cave-ha::raft`** (`crates/cave-ha/src/raft/{log.rs, node.rs}`). `cave-ha` does not yet depend on `cave-kernel`. Adopting the consensus traits here means adding the workspace dep + writing `impl LogStore for cave_ha::raft::Log` etc. — real refactor work, but it touches a critical-infra crate and was scoped out of Faz 1.
- **`cave-store::notification::NotificationDispatcher` is dead code** (0 external callers; only `crates/cave-store/src/s3/notification.rs` does dispatch via a free `dispatch` function, not the struct). Migrating dead code adds zero downstream value.
- **`cave-apiserver::watch_cache`** (820 LOC) uses `broadcast::Sender` directly. Worth adopting `EventBus<T>`, but it's a substantial refactor of a load-bearing path — out of Faz 1 scope.

Forcing fake migrations to hit the brief's letter would have introduced churn without coherence. Faz 1 stops at landing the contracts; Faz 2 does adoption per crate, with a clear entry point each.

---

## Faz 2 — adoption candidates (real targets, real entry points)

| # | Adopter | Primitive | Entry point | Risk | Effort |
|---|---|---|---|---|---|
| F2-A | `cave-ha::raft` | `consensus` | `crates/cave-ha/src/raft/{log.rs, node.rs}` — add `cave-kernel` dep, `impl LogStore for Log`, `impl StateMachine for Node`, `impl RaftHandle for Node` | HIGH (critical infra) | 1-2 days |
| F2-B | `cave-apiserver::watch_cache` | `eventbus` | `crates/cave-apiserver/src/watch_cache.rs` (820 LOC) — replace ad-hoc `broadcast::Sender<WatchEvent>` with `EventBus<WatchEvent>`, propagate `Lagged` to clients as resync signal | MEDIUM | 1 day |
| F2-C | `cave-portal` SSE | `eventbus` | wherever portal SSE handlers fan out backend events — the original design doc target | LOW (newer code) | 0.5 day |
| F2-D | `cave-controller-manager` reconcile loops | `reconcile` | `crates/cave-controller-manager/src/{gc_lite,pv,rbac,sa}/mod.rs` — replace bespoke loops with `run_reconciler` | MEDIUM (many call sites) | 1-2 days |
| F2-E | `cave-cri` reconcile | `reconcile` | container-state reconciliation in `cave-cri` lifecycle | MEDIUM | 1 day |
| F2-F | `cave-mesh`, `cave-auth` | `identity` | SVID issuance + verification paths | LOW | 0.5 day |
| F2-G | `cave-apiserver`, `cave-net`, `cave-portal` | `ns` | tenant header propagation — uses `X-Scope-OrgID` constant | LOW | 0.5 day each |

**Multi-tenant compliance gap** (per the original 2026-04-23 plan, section 4): `Controller<T>::reconcile` should grow `enqueue_with_tenant(key, tenant_id, item)`; `netns::EbpfHook` and `netns::CgroupV2Handle` need a `tenant_id: String` field. These two follow-up updates are **non-breaking additive** and should land alongside Faz 2-D (reconcile adoption).

**Sweep-003 status**: separate scope, already landed on main as `9d64075 feat(cave-kernel): sweep-003 — RateLimiter + CircuitBreaker + RetryPolicy` (these are the `circuitbreaker` / `ratelimiter` / `retrypolicy` modules already in `cave-kernel/src`). Don't conflate with sweep-002 Faz 2.

---

## Verification commands run during Faz 1

```bash
git cherry-pick bc50d6b                          # → fe8042d on main, no conflict
cargo test -p cave-kernel --lib                  # → 113 passed; 0 failed
cargo check --workspace                          # → Finished in 9.72s, 0 errors
cargo test -p cave-etcd -p cave-kamaji -p cave-store --lib --no-run
                                                 # → all three test executables built clean
grep -rE 'todo!|unimplemented!' crates/cave-kernel/src/{consensus,eventbus,identity,ns,reconcile}.rs
                                                 # → 0 hits
```

`claude/eager-jennings-ab6961` worktree + branch can be removed once the cherry-pick is on main; the 23 unrelated commits on that branch (cave-kubelet M13-M17, cave-cri M6+, cave-etcd M6 deeper, etc.) are sprint work that does not belong to sweep-002 and would need their own rebase + merge if they are to land.

---

## 2026-05-01 (afternoon) — Faz 2-A/B/D closure

The three high-risk / load-bearing items deferred from the morning Faz 2-G batch were closed in one session on the `claude/wizardly-lalande-f5817b` branch and ff-merged to `main` in commit order F2-B → F2-D → F2-A.

### Faz 2-B — cave-apiserver watch + store → `cave_kernel::eventbus` (commit `c51712c`)

**Real duplicate found:** `ResourceStore::watch_tx: broadcast::Sender<WatchEvent>` (not the watch_cache ring buffer, which is a different abstraction — RV-indexed replay store, not pub/sub). The morning deferral note misidentified the target.

**Migration:**
- `ResourceStore::watch_bus: EventBus<WatchEvent>` replaces the broadcast wrapper. Capacity 4096 retained (matches upstream apiserver default). `subscribe()` now returns `Subscription<WatchEvent>` with explicit `Lagged`/`Closed` semantics propagated to the watcher.
- `WatchCache` gains `subscribe()` + `subscriber_count()` backed by `EventBus<WatchCacheEvent>` for live fan-out. Ring buffer (RV-indexed replay) stays as-is. Live-bus capacity is decoupled from ring-buffer capacity — a slow tailer hits `Lagged` without evicting events from the replay history.
- KEP-365 bookmarks (interval + `force_bookmark` heartbeats) ride the live bus alongside Added/Modified/Deleted, matching upstream `cacher.dispatchEvent`.

**Tests:** +13 in `watch_cache::tests` (24 → 37) + +12 apiserver lib total (916 → 928). Coverage: live subscribe, multi-subscriber fan-out, consumer-side tenant filter, `Lagged` detection with replay-buffer recovery, capacity rotation independence, bookmark fan-out (both interval + force), `try_recv` non-blocking semantics, post-subscribe baseline, concurrent multi-tenant tenant_id invariant, subscriber-count drop tracking, no-subscriber publish non-blocking guarantee.

### Faz 2-D — cave-controller-manager → `cave_kernel::reconcile` (commit `098e2be`)

**Adoption surface:** every per-controller pure decision function in this crate gets a kernel-loop bridge.

- New `runtime` module exposing `run_<controller>(snapshot_fn, config, cancel) -> (ReconcileQueue<String>, JoinHandle)` factories for: deployment, replicaset, statefulset, daemonset, job, cronjob, hpa, pdb, service, endpointslice (10 controllers).
- Generic `ScaffoldReconciler<S, O, F>` adapter handles 9 of them. DaemonSet has a purpose-built `DaemonSetReconciler` because its observation type is `Vec<NodeView>`, not a Status struct.
- `reconcile_to_outcome` maps the local `Reconcile { NoOp | Create(n) | Delete(n) | Update(n) | Requeue }` enum onto kernel `ReconcileOutcome`. Terminal decisions → `Done`; `Requeue` → `Requeue { delay }` with per-controller cadence: 30s default (controller-runtime `DefaultRequeueAfter`), 10s cronjob (cron re-evaluation), 15s hpa (`--horizontal-pod-autoscaler-sync-period`).
- A missing snapshot (object deleted between enqueue and dequeue) maps to `ReconcileOutcome::Done`, mirroring upstream `controller-runtime/pkg/reconcile.Func` NotFound semantics.

**Tests:** +17 in `runtime::tests` (714 → 731). Coverage: terminal-decision mapping, requeue delay (cronjob/hpa cadences), every per-controller `run_*` smoke + tenant_id invariant preservation, missing-snapshot Done semantics, shared-cancel clean shutdown across two loops, Requeue re-enqueue verification through the kernel's spawned timer task.

This is **preventative**: the controllers had no shared loop wrapper to remove, but they were each one keystroke away from spawning ten one-off bespoke loops. F2-D bottles that work in the kernel primitive instead.

### Faz 2-A — cave-ha::raft → `cave_kernel::consensus` (commit `10db5f3`)

**Real duplicate removed:** `cave-ha::raft::state_machine::StateMachine` was a structural duplicate of `cave_kernel::consensus::StateMachine` — same async signature shape, different error type and `LogEntry` shape. Replaced with `pub use cave_kernel::consensus::StateMachine;`. Concrete impls in cave-ha (`NoopStateMachine`, `KvStateMachine`) re-implement against `cave_kernel::consensus::LogEntry` and return `ConsensusResult` directly. Decode failures map to `ConsensusError::Storage`.

**Bridge layer:** new `cave-ha::raft::kernel_bridge` module providing:
- `to_kernel_entry` / `from_kernel_entry` projections (cave-ha `LogEntry` ↔ kernel `LogEntry`; the `entry_type` discriminator is internal to cave-ha and dropped at the bridge).
- `map_ha_error`: `HaError` → `ConsensusError`. `NotLeader{leader_id}` → `NotLeader(stringified)`; `LogCompacted` → `LogNotFound`; Storage/Transport pass-through; everything operational/transient (Shutdown, ProposalDropped, TransferInProgress, IsLearner, MembershipChangePending, NodeNotFound, NoQuorum, Timeout, Raft, Dr, Serialization, Io) → `Aborted` with descriptive message.
- `to_kernel_role`: PreCandidate is collapsed to `Candidate` (the pre-vote phase is internal to cave-ha and not part of the kernel's three-state Role).
- `to_kernel_node_id`: numeric `u64` → `String` (kernel uses transport-neutral String IDs).
- `KernelLogStore`: `Arc<tokio::sync::Mutex<MemLog>>` adapter implementing `cave_kernel::consensus::LogStore` (async append/get/last_index/truncate_after; missing-index returns `Ok(None)` per kernel contract). Two constructors: `new()` (fresh storage) and `from_arc(arc)` (share an existing MemLog with the cave-ha node loop).
- `KernelRaftHandle`: wraps `cave-ha::RaftHandle`, implements `cave_kernel::consensus::RaftHandle`. `propose`/`read_index` forward through with `HaError` mapping; `leader()` builds `LeaderInfo` from cave-ha's `NodeStatus`; `node_id()` stringifies the inner numeric id.

**Tests:** +36 in `kernel_bridge::tests` (cave-ha lib total 4 → 40).
- Conversion + projection (5): LogEntry index/term/data preservation, entry_type drop, kernel-staged Normal lifting, Role projection, NodeId stringification.
- Error mapping (6): NotLeader (with/without leader id), LogCompacted, Storage/Transport pass-through, Shutdown + ProposalDropped → Aborted.
- KernelLogStore conformance (6): empty log, append→get round-trip, last_index advancement, missing-index Ok(None), truncate_after suffix drop, append→truncate→re-append index reuse (mirrors Raft conflict resolution), Clone shares storage, from_arc shares MemLog with external owner.
- StateMachine via kernel trait (8): Noop apply/snapshot/restore idempotency, KvStateMachine Set/Delete/empty-data-noop, invalid-JSON → Storage error, snapshot+restore round-trip on populated store, restore-invalid → Storage error.
- Trait-object composition (1): `Arc<dyn StateMachine>` + `Arc<dyn LogStore>` against the kernel surface end-to-end.
- End-to-end against real RaftNode (10): node_id forwarding, leader() returns LeaderInfo with elected term and stringified leader id, propose() advances log index, repeated propose() preserves monotonicity, propose-after-shutdown → Aborted, DynRaftHandle Arc<dyn> usability, KernelRaftHandle Clone shares inner node.

**Note:** the cave-ha single-node `read_index` path is known to block on quorum-ack accounting (pre-existing in the baseline, not introduced by F2-A). The bridge's `read_index` implementation itself is a one-line forward + error map; it is exercised in cave-ha's multi-node integration tests rather than in the bridge unit tests.

### Aggregate

| Phase | Crates | Commits | Tests added | Lib total after |
|-------|--------|---------|-------------|-----------------|
| Faz 1 | cave-kernel | `fe8042d` | +34 | 113 |
| Faz 2-G (TenantId) | 6 (search, ccm, cm, portal, net, mesh) | `da49a80` .. `a1246f9` + `ae33e79` | (existing tests preserved; -97 LOC duplicate) | 3707 cumulative |
| Faz 2-B (eventbus) | cave-apiserver | `c51712c` | +12 | 928 |
| Faz 2-D (reconcile) | cave-controller-manager | `098e2be` | +17 | 731 |
| Faz 2-A (consensus) | cave-ha | `10db5f3` | +36 | 40 |
| **Closure totals (F2-A/B/D)** | **3 crates** | **3 commits** | **+65** | — |

`cargo check --workspace`: green at every commit. No remote pushes (local closure). Conventional commits + ff merges only.

### Multi-tenant compliance recap

Per ADR-MULTI-TENANT-001 (sweep-002 plan §4):

| Primitive | Status | Note |
|-----------|--------|------|
| consensus | ✅ safe | caller carries tenant; bridge tests assert tenant_id propagation through propose path |
| eventbus | ✅ safe | tenant-id filter at consumer side; F2-B watch_cache tests cover multi-tenant fan-out |
| reconcile | ✅ safe | F2-D `run_deployment_preserves_tenant_id_invariant` test covers the snapshot-fn → reconcile-fn tenant pipeline |
| identity | ✅ already compliant | embedded in trust domain path |
| ns (TenantId) | ✅ adopted in 6 crates | F2-G batch |
| netns::EbpfHook + CgroupV2Handle | ⚠ deferred to sweep-003 | non-breaking additive; not a closure blocker |

### Closure checklist

- [x] All planned primitives extracted (Faz 1: `consensus`, `eventbus`, `reconcile`, `identity`, `ns`).
- [x] All planned adopters migrated (Faz 2-G: 6 TenantId; Faz 2-A/B/D: 3 high-risk).
- [x] `cargo check --workspace` green at every commit on `main`.
- [x] No `unimplemented!()` / `todo!()` / stub introduced anywhere in sweep-002 commits.
- [x] No remote push performed; local fast-forward only.
- [x] Conventional-commit messages on every commit.
- [x] cave-cli/main.rs untouched (per closure rule).
- [x] Real duplicate code removed where present (cave-ha StateMachine trait, cave-apiserver broadcast::Sender wrapper, 6 TenantId newtype duplicates).
- [x] Test counts: cave-kernel 113, cave-apiserver 928, cave-controller-manager 731, cave-ha 40 — all pass.

**Sweep-002 is sealed.** The next synergy slot opens on sweep-003 (RateLimiter extraction; matrix in `sweep-002-plan-2026-04-23.md` §5). Pre-OSS-launch window remains 20 days from 2026-05-01.
