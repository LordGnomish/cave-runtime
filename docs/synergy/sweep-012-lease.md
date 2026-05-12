# Sweep-012 — `cave_kernel::lease`

**Author:** 2026-05-12 close-out
**Branch:** `claude/gracious-banach-9be8eb`
**Status:** **Landed (primitive + tests).** Live adoption deferred —
recon clarifies the shape mismatch (see §3).

## What landed

`crates/cave-kernel/src/lease.rs` — 207 LOC + 14 unit tests.

`LeaseManager` provides etcd-style leader-election leases:

* **`acquire(name, holder, ttl, now)`** — succeed if the lease is
  free OR currently held by the same `holder` (re-acquisition
  slides the expiry forward, matching etcd's `KeepAlive`
  semantics).
* **`renew(name, holder, ttl, now)`** — slide expiry forward;
  refuses non-holders. Does NOT bump the revision (lease takeover
  detection works via `revision`).
* **`revoke(name, holder)`** — explicit revoke; only the holder
  may.
* **`get(name)`** / **`list()`** — read-side.
* **`sweep_expired(now)`** — sweeps expired leases out of memory;
  returns the count removed.

`LeaseInfo` carries `name`, `holder`, `expires_at_unix`, and a
monotonically increasing `revision` (etcd's primitive for detecting
takeovers across acquires).

The manager is `Clone` so a single store can be shared across
spawned background tasks without `Arc` ceremony — `LeaseManager`
internally wraps `Arc<RwLock<Inner>>`.

## What is NOT in scope

* Single-node MVP only. Leases live in memory; a crash loses them
  all. Multi-node Raft-backed storage is Paket C's territory.
* No `KeepAlive` stream like etcd's gRPC server-streaming response.
  Callers tick `renew()` on their own cadence.

## Adoption — landed: controller-manager leader election

New `crates/cave-controller-manager/src/leader_election.rs` (~140
LOC + 13 unit tests) wraps `cave_kernel::lease::LeaseManager` in a
controller-manager-shaped `LeaderElector` handle. Public surface:

* `LeaderElector::default_for_replica(manager, replica_id)` —
  constructs an elector with the upstream-default
  `kube-controller-manager` lease name and 15s TTL (matches
  `LeaderElectionConfiguration.LeaseDuration` in
  `pkg/leaderelection`).
* `acquire(now) -> Role` — promotes to `Leader` if the lease is
  free OR this replica already holds it; returns `Standby`
  otherwise.
* `renew(now)` — slides the expiry forward; fails if the replica
  no longer holds the lease (signal to step down and stop
  reconcilers immediately).
* `release()` — voluntary release on graceful shutdown so a
  standby can take over without waiting for expiry. Idempotent.
* `status(now) -> ElectionStatus` — snapshot of role + holder +
  expiry + revision, useful for the admin UI.

Distinct from `node_lease.rs` (per-kubelet liveness — what this
controller *watches*); leader_election is the controller-manager's
*own* lease (what decides which replica drives reconciliation).

Single-node MVP — `LeaseManager` lives in-process. Multi-node
Raft-backed storage lands with Paket C's consensus layer; the
adopter's API stays the same.

13/13 leader_election tests pass.

## Adoption — deferred: cave-rdbms-operator

CloudNativePG's primary-election uses a Postgres-side
`pg_advisory_lock` rather than an etcd lease. Mirroring that
faithfully is outside the kernel lease's scope. cave-rdbms-operator
keeps its existing fencing logic; the kernel lease could plausibly
serve as a secondary safeguard but that's a feature add, not a
swap.

## Tests

`cargo test -p cave-kernel --lib lease::` — 14 passed.
`cargo test -p cave-controller-manager --lib leader_election::` —
13 passed.
