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

## Adoption — recon

The brief named `cave-controller-manager` and
`cave-rdbms-operator` as adopters. Per-crate recon:

### cave-controller-manager

`src/node_lease.rs` exists (230 LOC) and shapes a `Lease` type +
helpers. BUT — that `Lease` represents the **kube-node-lease**
Kubernetes resource (the per-node liveness signal kubelets renew),
not controller-manager leader-election. The shapes look similar
but their lifecycles are different:

* `kube-node-lease` is **per-node**, written by the kubelet,
  watched by the controller-manager.
* The controller-manager's own leader election would be a
  **per-controller** lease (one `kube-controller-manager` lease in
  the `kube-system` namespace).

`cave-controller-manager` does NOT currently have controller-self
leader election; introducing it would be a feature add, not an
adoption. The primitive is the prerequisite; the feature ticket can
land separately.

### cave-rdbms-operator

CloudNativePG's primary-election uses a Postgres-side
`pg_advisory_lock` rather than an etcd lease. Mirroring that
faithfully is outside the kernel lease's scope. The cave-side
fencing logic in `ha.rs` could plausibly use a kernel lease as a
secondary safeguard, but cave-rdbms-operator's Cargo doesn't
depend on `cave-kernel` yet — adding the dep + the integration
is a coordinated change with the operator's existing primary-election
state machine.

Both adoptions stay deferred; the primitive lands so the
follow-up work has something to consume.

## Tests

`cargo test -p cave-kernel --lib lease::` — 14 passed.
