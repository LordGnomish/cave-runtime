<!--
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR-088: Resurrection Protocol — Restore-from-Cold

## Status
Accepted

## Context
Cave Runtime stores tenant state in `cave-etcd` (Raft-replicated) and
configuration as code (operator's git). A working cluster can be
destroyed by hardware loss, ransomware, mis-operation, or hostile
takeover. The Runtime must be recoverable **from cold** — that is,
starting from nothing except (a) an offline secret-zero kit and
(b) snapshots that the operator placed in independent escrow.

This ADR specifies the Runtime-side contract that makes that recovery
possible. Where to store snapshots, which cloud region to escrow into,
which legal jurisdiction holds the keys — those are operator deployment
choices and live outside the Runtime.

## Decision
Cave Runtime's resurrection contract has five Runtime-owned phases. The
target is <4 hours from cold start to "apiserver healthy, tenants
admissible". Phases 6–10 (workload re-bind) are operator-driven and
not part of the Runtime contract.

| Phase | Runtime obligation | Target |
|-------|--------------------|--------|
| 1. Secret-zero | `cave-vault` accepts an externally reassembled root key (Shamir 3-of-5 or HSM unseal) — see ADR-079. | < 90 min |
| 2. Snapshot ingress | `cave-runtime serve` accepts a previously-exported etcd snapshot and CA bundle on first boot via `--restore-from`. | < 15 min |
| 3. etcd restore | `cave-etcd` rehydrates from the snapshot deterministically; WAL is rebuilt; cluster membership is rewritten to the new node list. | < 15 min |
| 4. Apiserver up | `cave-apiserver` comes up read-only until quorum, then writeable. Admission for tenant workloads is gated until phase 5. | < 5 min |
| 5. Tenant readiness | `cave-auth` re-issues tenant tokens against the restored identity store; admission opens. | < 30 min |

Phases 6 onward (GitOps reconcile, application-layer restore) are the
operator's responsibility and depend on the deployment profile.

## Requirements on the Runtime

1. **Idempotent restore.** `--restore-from <snapshot>` must succeed
   against an empty data dir, or refuse cleanly if the data dir is
   already populated. No silent overwrite.
2. **Deterministic rehydration.** A given snapshot must produce a
   byte-identical etcd state across two independent restores. This is
   the property exercised by the quarterly drill.
3. **Drill-friendly mode.** `cave-runtime cluster restore --dry-run`
   validates a snapshot + key bundle without committing — used by the
   operator's quarterly resurrection drill to keep restorability honest.
4. **No external dependency in the critical path.** The restore path
   only needs the local snapshot, the local key bundle and the binary
   itself. No SaaS, no cloud API, no DNS round-trip is required.

## Out of Scope
- Where the snapshot is stored (S3, MinIO, tape, etc.) — Platform.
- Cross-region escrow strategy — Platform.
- Multi-cloud failover topology — Platform.
- Application-layer restore (databases inside tenants) — tenant operator.

## Rejected Alternatives
- **No restore path.** Unacceptable for a multi-tenant Runtime.
- **Backup tooling only, no documented restore.** "Untested backups are
  not backups." The drill exists because restore is the contract, not
  backup.
- **Restore via re-bootstrap of a fresh cluster.** Would lose tenant
  identity continuity (token issuer changes, certs invalidated).
  Restore must preserve the identity root.

## Consequences

Positive
- The operator can answer "what does recovery look like?" without
  reference to any specific cloud or SaaS.
- The drill is small enough to exercise quarterly (it does not require
  rebuilding the whole platform — only the Runtime contract).

Negative
- The 4-hour budget pre-supposes the operator has the secret-zero kit
  reachable in <90 min. That is a deployment property, not a Runtime
  guarantee.
- Idempotent restore costs extra disk space during transition
  (snapshot + rehydrated state held briefly together).

## Related ADRs
- ADR-079 (Secret-Zero Bootstrap + Break-Glass Kit)
- ADR-RUNTIME-CERT-LIFECYCLE-001 (CA hierarchy survives restore)
- ADR-MULTI-TENANT-001 (why identity continuity matters)
