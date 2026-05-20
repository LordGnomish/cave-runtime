# ADR-046: Kubernetes Backup — Velero

**Status:** Accepted

**Scope:** Universal

**Category:** DR

**Related ADRs:** 080, 088

## Context

CAVE needs cluster-level backup for K8s resources (Deployments, ConfigMaps, CRDs, Crossplane XRs) and persistent volumes. Database-level backup is handled by CNPG/Azure PG (ADR-047). Velero covers the K8s resource layer.


## Candidates

| Criteria | Velero | Kasten K10 | Stash/KubeStash | etcd snapshot only |
|---|---|---|---|---|
| K8s resource backup | ✅ Full (namespaced + cluster-scoped) | ✅ | ✅ | ✅ (raw etcd) |
| PV backup | ✅ Restic/Kopia + CSI snapshots | ✅ | ✅ | ❌ |
| Scheduled backups | ✅ CronSchedule CRD | ✅ | ✅ | ❌ Custom CronJob |
| Cross-cluster restore | ✅ (migration use case) | ✅ | ✅ | ⚠️ |
| Object storage backend | ✅ MinIO (Hz) / ADLS (Az) | ✅ | ✅ | ❌ |
| Multi-tenant filtering | ✅ Label selector, namespace filter | ✅ | ✅ | ❌ |
| License | Apache 2.0 | Proprietary (Veeam) | ⚠️ AppsCode license | N/A |


## Decision

**Velero** for K8s resource + PV backup. Restic/Kopia for file-level PV backup. MinIO (sovereign) / ADLS (Azure) as backup storage. Scheduled: daily full, hourly incremental for prod. Restore smoke test: weekly automated (ADR restore matrix). WORM-backed backup storage for prod (ADR-106).


## Rejected Options

- **Kasten K10 (Veeam):** Proprietary license. Cost scales with node count. SaaS component for management.
- **Stash/KubeStash:** AppsCode license (not pure Apache 2.0). Smaller community.
- **etcd snapshot only:** Backs up K8s resources but not PV data. No namespace-level filtering. No scheduled CRD.


## Consequences

**Positive:**
- Full K8s resource + PV backup with scheduled automation.
- MinIO/ADLS backend — same storage as other CAVE data.
- Namespace filtering enables per-tenant backup scoping.
- Weekly restore smoke test validates backup integrity.
- Apache 2.0 — no licensing concerns.

**Negative:**
- Restic/Kopia backup of large PVs can be slow (mitigated: CSI snapshots for supported volume types).
- Velero restore can conflict with ArgoCD reconciliation (ArgoCD tries to reconcile while Velero restores).
- Backup storage costs scale with cluster size and frequency.

## Notes

**Universal scope** — Platform tenant backup + Cave Runtime crate state'lerinin durability garantisi. **Runtime mirror REQUIRED**: cave-backup crate (Mirror-001 blanket scope, dual upstream Velero+Restic semantics tek crate'te, MinIO+ADLS dual backend) — sovereign deployment'larda CI-time velero CLI kırılırsa runtime kendi backup loop'unu koşar; cave-self-improver gece pump'ında her tenant namespace'inin backup integrity'sini doğrular, Reflex Engine restore-anomaly detection için load-bearing. WORM (ADR-106) zorunluluğu cave-backup'ta baştan içeride.

## Compliance Mapping

SOC2 CC7.5 (backup and recovery). SOC2 CC9.1 (risk mitigation — data protection). ISO A.8.13 (information backup). ISO A.5.29 (business continuity). NIS2 Art.21 (disaster recovery).

