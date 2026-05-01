# Sweep-002 Progress — Faz 1 Landed + Faz 2 Partial

**Status:** Faz 1 cherry-picked + Faz 2 partial adoption (TenantId across 6 crates) — main on 2026-05-01.

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
| **F2-B cave-apiserver::watch_cache** → EventBus | watch_cache.rs uses a tenant-scoped ring buffer, not `broadcast::Sender`. EventBus<T> is a different abstraction; "adoption" would be a redesign, not a refactor. Defer to a focused PR that picks one of the two contracts. |
| **F2-D cave-controller-manager reconcile** → run_reconciler | 10+ controller files, each with its own bespoke loop. Each adoption is a real test-and-port; doing all 10 in one batch breaks the "small reviewable diff" principle. Faz 2 leaves it as-is for per-controller adoption. |
| **F2-A cave-ha::raft → consensus** | HIGH risk per author's own note; cave-ha doesn't yet depend on cave-kernel. Adopt by `impl LogStore for cave_ha::raft::Log; impl StateMachine for cave_ha::raft::Node` in a focused PR with extra review on the consensus path. |

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
