# cave-controller-manager parity — 2026-05-12 measured audit

**Upstream pin:** `kubernetes/kubernetes` `pkg/controller/*` (v1.31.x; manifest's pinned v1.36.0 claim preserved).

## Why this exists

The 2026-05-01 audit placed cave-controller-manager at **tier B, parity_ratio = 0.25**. That was the wave3 metric (10 of 10 files declared, no other dimensions mapped). It told us nothing about which upstream controllers are actually ported.

This pass replaces it with a measured `fill_ratio` enumerated against `pkg/controller/*` sub-packages.

## Methodology

Same as cave-etcd / cave-apiserver / cave-scheduler. Each entry is `[[mapped]]` / `[[skipped]]` (with enumerated reason) / `[[unmapped]]`.

## Counts

| Bucket | Count |
|---|---:|
| `[[mapped]]` | **24** (was 23) |
| `[[skipped]]` | 10 |
| `[[unmapped]]` | **11** (was 12) |
| **Total** | **45** |
| **fill_ratio** | **0.7556** (was 0.7333) |

The previous self-reported `parity_ratio = 0.25` is replaced by `fill_ratio = 0.7556`. (Note: the numbers measure different things — 0.25 was "10/10 files declared in [[files]]" via the old schema; 0.7556 is `(mapped + skipped) / total` over enumerated upstream sub-packages. The new metric is the comparable one going forward.)

### 2026-05-13 k8s-core push update

`pkg/controller/resourceclaim/` (DRA, KEP-4381) — the biggest unmapped
gap in the 2026-05-12 audit — has been ported. `src/resourceclaim.rs`
implements the deterministic state machine:

* `AddFinalizer` → `Allocate` (Immediate or WaitForFirstConsumer with
  scheduler candidate) → `AddReservation` / `RemoveReservation` for
  consumer pod lifecycle → on delete, `AwaitConsumerDrain` →
  `RequestDeallocation` → `RemoveFinalizer` → `AwaitDeletion`.
* `apply_reservation_diff` helper that handles the add/remove
  diffing against the current `reservedFor[]`.
* Tenant gate (`check_tenant`) for cross-tenant isolation.
* `reconcile_outcome` mapping into the `crate::types::Reconcile`
  enum so the existing manager loop (`runtime.rs`) drives it.

23 deterministic tests cover every transition + a full-cycle audit
test that walks one claim through every state. Device-fitness
matching remains in `cave-scheduler/src/dra.rs` (the *candidate*
node + devices triple is passed into this reconciler as input).

## Mapped highlights — the 23 controllers cave ships

Workload: Deployment / ReplicaSet / StatefulSet / DaemonSet / Job / CronJob.
Scaling: HPA (v2). Disruption: PDB.
Service plane: EndpointSlice (with multiport + topology), Service.
Resource governance: ResourceQuota, RBAC (ClusterRoleAggregation), Namespace controller.
Identity: ServiceAccount, root-CA publisher, CertificateSigningRequest signer + auto-approver.
Garbage collection: GarbageCollector (orphan owner-reference reaper) + TTL.
Node lifecycle: NodeLifecycle + Node leases.
Storage: PV binder (PVC-side protection).
Bootstrap: Bootstrap-token signer.

Each maps to one or more `crates/cave-controller-manager/src/` files. 20 KB total + 78 .rs files.

## Skipped (10)

- `cmd/kube-controller-manager/`, `pkg/controller/apis/config/` — Go bootstrap + config types (cave uses serde structs).
- `pkg/controller/cloud/`, `pkg/controller/nodeipam/` — parallel track in cave-cloud-controller-manager.
- `pkg/controller/metrics/`, `pkg/controller/testutil/`, `pkg/controller/history/` — stdlib analogs / test harness / folded.
- `pkg/controller/podgc/`, `pkg/controller/volume/{expand,attachdetach}/` — covered by other crates.

## Unmapped (11 — real gaps as of 2026-05-13)

The big ones:
1. ~~**DRA `resourceclaim/` controller**~~ — **CLOSED 2026-05-13**, see
   the k8s-core push update above. `src/resourceclaim.rs` ships the
   state machine.
2. **`tainteviction/`** — per-pod toleration-timer eviction. Node-level NoExecute works; per-pod grace timing does not.
3. **`cidrallocator/`** — on-the-fly node CIDR allocation when running without a cloud provider. Cave-net is currently pre-provisioned.
4. **`validatingadmissionpolicystatus/`** — type-check status reconciler for VAP. cave-apiserver validates at write time but the steady-state status loop is missing.
5. **`storageversionmigrator/`** + worker — re-encode etcd-stored objects after serializer changes. cave-apiserver has the trigger surface; the reconciler is missing.
6. **`storageversiongarbagecollector/`** — clean up StorageVersion objects when their owning APIService is gone.
7. **`legacyserviceaccounttokencleaner/`** — upgrade-time cleanup of pre-v1.24 SA secrets. Cave never created them, but the migration-window job that walks existing clusters is not implemented.

The "less load-bearing today" four:
8. **`pkg/controller/endpoint/`** — legacy v1 Endpoints controller. cave only writes EndpointSlice and relies on apiserver mirroring for older clients.
9. **`pkg/controller/replication/`** — legacy ReplicationController; superseded by ReplicaSet.
10. **`pkg/controller/volume/pvprotection/`** — PV-side finalizer (PVC-side already covered).
11. **`pkg/controller/volume/ephemeral/`** — generic-ephemeral-volume controller (inline emptyDir+downwardAPI already in cave-kubelet).
12. **`storageversionmigrator/migrator/`** — inner worker, listed separately because v1.32 split it.

## Out of scope for this audit

The 12 unmapped controllers vary from ~300 LOC (legacyserviceaccounttokencleaner) to ~2K LOC (resourceclaim) of upstream code. Picking them up is straightforward sweep-able work; the audit gives the prioritised list.
