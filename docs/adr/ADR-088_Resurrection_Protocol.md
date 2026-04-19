# ADR-088: Resurrection Protocol

**Status:** Accepted

**Scope:** Universal

**Category:** DR

**Related ADRs:** 079 | Absorbs: ADR-097

## Context

CAVE must be recoverable from complete cluster destruction (data center fire, catastrophic cloud failure, hostile takeover). Recovery must be independently achievable without depending on the destroyed infrastructure.


## Candidates

| Phase | Action | Dependency | Target Duration |
|---|---|---|---|
| 1. Key recovery | Shamir 3-of-5 reassembly (HSM fast-path) | Break-glass Kit (offline) | <90 min |
| 2. WORM access | Retrieve etcd snapshot, Talos configs, Git mirror | WORM escrow (cross-region) | <15 min |
| 3. etcd restore | Decrypt + restore etcd snapshot | KMS key from Kit | <15 min |
| 4. Node provision | Apply Talos machine configs to new nodes | Talos image in WORM | <30 min |
| 5. K8s API | Kubernetes API server comes up | etcd + nodes | <5 min |
| 6. ArgoCD bootstrap | Deploy ArgoCD, point to Git mirror | K8s API | <15 min |
| 7. Platform reconcile | ArgoCD reconciles all platform components | ArgoCD + Git | <30 min |
| 8. Crossplane reconcile | Crossplane reconciles infrastructure | Crossplane + providers | <30 min |
| 9. Tenant restore | Workloads restored from last known state | Platform services | <30 min |
| 10. Validation | SLO baseline comparison, smoke tests | All services | <15 min |


## Decision

10-phase resurrection from Break-glass Kit + WORM escrow. Total target: <4 hours from incident declaration to ArgoCD healthy. WORM escrow contents: Git mirror (hourly), etcd snapshots (5min prod), Talos machine configs, Sovereign Ledger replica. WORM escrow is cross-region, cross-account, independent of primary cluster. Quarterly simulation drill (phases 3-10). Annual full drill (all 10 phases including Shamir).


## Rejected Options

- **No DR plan:** Unacceptable for a platform hosting multi-tenant production workloads.
- **Backup-only (no tested restore):** Untested backups are not backups. Resurrection drill validates restorability.
- **Cloud-provider DR only:** Circular dependency during total cloud loss. Azure Backup/Hetzner snapshots are useful for node-level recovery but insufficient for total platform resurrection.
- **Active-active multi-region:** Phase 4 future capability. Too complex for initial delivery. Resurrection protocol covers the gap.


## Consequences

**Positive:**
- Platform fully recoverable from total destruction within 4 hours.
- WORM escrow is independent of primary cloud infrastructure.
- HSM fast-path enables remote key recovery without physical co-location.
- Quarterly/annual drills produce `GOT-Benchmark` attestations proving recoverability.

**Negative:**
- Key holder coordination required (3-of-5 for full recovery).
- WORM escrow storage cost.
- Git mirror and etcd snapshot frequency determine RPO (5min for etcd, hourly for Git — worst case 1h RPO for platform config).
- Resurrection drill is time-consuming (4h for full drill — planned quarterly/annually).

Compliance Mapping

SOC2 CC7.5 (disaster recovery). SOC2 CC9.1 (risk mitigation). ISO A.5.29 (information security during disruption). ISO A.5.30 (ICT readiness for business continuity). NIS2 Art.21 (business continuity, disaster recovery).

Absorbed Decisions:

The following tool-level decisions are absorbed into this ADR for traceability

Multi-Region Failover

Decision:

Multi-region failover via Cloudflare DNS health-check. Active-passive default (cost-efficient). Active-active for premium dedicated tenants. Cross-cloud out of scope (ADR-066).

