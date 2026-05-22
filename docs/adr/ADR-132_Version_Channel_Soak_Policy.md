# ADR-132: Version Channel & Soak Policy

**Status:** Accepted

**Scope:** Hyperscaler, Sovereign, Runtime, Universal

**Category:** Platform Governance

**Related ADRs:** 085 (Rolling Upgrades), 099 (Deprecation Guardrails), 127 (Roadmap Intelligence), 133 (Compatibility Matrix)

## Context

CAVE runs ~73 distinct components across 7 profiles. Each component has its own release cadence — Kubernetes targets 3 releases/year, ArgoCD and Crossplane release monthly, Cilium quarterly, Istio quarterly. Without a formal channel policy, prod environments risk running unvalidated versions, and dev environments may lag behind, missing integration issues early.

Key upstream cadence realities:
- **Kubernetes:** 3 minor releases/year, N-2 patch support. Skew policy: kubelet within 1 minor of API server. Talos pins to specific K8s minor.
- **ArgoCD:** Monthly minor releases. Current: v3.2.9. v3.3+ introduces server-side apply as default.
- **Crossplane:** Monthly releases. v2 namespace-first model is current.
- **Istio:** Quarterly releases. Ambient mode reaching parity with sidecar. Sidecar→ambient migration path active 2025-2026.
- **Cilium:** Quarterly releases. CNCF Graduated.
- **KEDA:** Regular release rhythm. Event-driven autoscaling core for Reflex Engine.

---

## Candidates

| Approach | Multi-channel soak windows (chosen) | Single channel (everyone gets latest) | LTS only (6-12 month lag) | No soak (immediate prod) |
|---|---|---|---|---|
| Risk containment | ✅ Staging absorbs regressions | ❌ Prod is the test | ✅ | ❌ |
| Feature velocity | ✅ Fast channel available | ✅ Fastest | ❌ Slow | ✅ Fastest |
| Prod stability | ✅ Soaked versions | ❌ Live testing in prod | ✅ Very stable | ❌ Unstable |
| Version tuple governance | ✅ Compatibility matrix enforced | ⚠️ Implicit | ✅ Simple | ❌ None |
| Emergency patch path | ✅ Documented skip-soak | ✅ Trivial | ⚠️ Breaks LTS model | ✅ |

## Decision

## Three release channels with enforced soak windows and promotion gates.

### Channel Definitions

| Channel | Profiles | Adoption Speed | Purpose |
|---|---|---|---|
| **fast** | local, dev | Latest stable (not RC/alpha/beta) within 7 days of release | Integration testing, early issue detection |
| **stable** | staging | Promoted from fast after dev soak | Pre-production validation |
| **production-delayed** | prod | Promoted from stable after staging soak | Risk-minimized production |

### Soak Windows

| Component Class | Dev Soak (fast→stable) | Staging Soak (stable→prod) | Total Pipeline |
|---|---|---|---|
| Core control plane (K8s, ArgoCD, Crossplane, Cilium, Istio) | 7 days | 14 days | 21 days |
| Data plane (CNPG, Strimzi, Valkey, OpenSearch, Qdrant) | 5 days | 10 days | 15 days |
| Security controls (OPA, Sigstore, Tetragon, cert-manager) | 7 days | 14 days | 21 days |
| Developer tooling (Backstage, Harbor, DevLake) | 3 days | 7 days | 10 days |
| Managed service provider versions (Azure PG, Confluent) | N/A (provider-managed) | 7 days (post-provider rollout) | 7 days |

### Promotion Entry Requirements

Every stable→prod promotion requires:
1. Compatibility matrix pass (`cave-ctl upgrade check`)
2. All Crossplane kuttl tests pass
3. Policy bundle verification (signed OPA bundles, ADR-089)
4. Rollback rehearsal (verified in staging)
5. Canary evidence (Argo Rollouts metrics)
6. Provider parity tests pass (ADR-135)
7. Sovereign Ledger `Upgrade Safe` attestation

### Forbidden in Prod

- Release candidates (RC), alpha, beta builds
- Versions outside vendor/community support window
- Versions violating Kubernetes skew policy
- Versions with known CVE severity ≥ HIGH (without explicit risk acceptance ADR)
- Versions not present in compatibility matrix

---

## Support Posture

| Posture | Rule |
|---|---|
| **Kubernetes** | Prod within N-1 of latest stable. Never N-0 (too fresh) or N-2 (approaching EOL). |
| **Talos** | Pinned to Kubernetes minor. Upgraded atomically with K8s (destroy/recreate). |
| **ArgoCD** | Prod within 2 minor versions of latest. |
| **Crossplane** | Prod within 2 minor versions. Provider versions pinned via digest (ADR-108). |
| **Cilium / Istio** | Prod within 1 minor of latest stable. |
| **KEDA** | Prod within 2 minor versions. |

`cave-ctl roadmap scan` validates support posture weekly and opens P2 tickets for versions approaching EOL.

---

## Rejected

## ### 4.1 Single Channel (everything on latest) — Rejected
Prod on latest stable = no soak time. Breaking changes hit production unvalidated. With 73 components, probability of at least one breaking interaction per month is near certain.

### 4.2 LTS-only — Rejected
Many CNCF projects don't offer formal LTS. Waiting for LTS would leave CAVE multiple versions behind, missing security patches and features. "Production-delayed" channel provides LTS-like stability without depending on upstream LTS programs.

### 4.3 Per-component channels — Rejected
Too much operational overhead. Channel assignment by component class (control plane, data plane, security, tooling) provides sufficient granularity without per-component tracking.

---

## Consequences

## ### Positive
- 21-day minimum pipeline before any control-plane change hits prod
- Kubernetes skew policy compliance enforced automatically
- RC/alpha/beta guaranteed absent from prod
- Soak windows catch interaction bugs between components

### Negative
- Prod is always 21+ days behind latest for control plane components
- Emergency security patches may need expedited promotion (bypass with guardian approval + Ledger attestation)
- Fast channel on dev may occasionally break (accepted — that's what dev is for)

### Risks

| Risk | Prob | Impact | Mitigation |
|---|---|---|---|
| Critical CVE requires immediate prod patch | Low | High | Emergency promotion path: guardian approves, skip soak, mandatory rollback rehearsal, Ledger `Emergency Upgrade` attestation |
| Soak window delays feature availability | Medium | Low | 21 days is acceptable for enterprise platform. Tenants don't see infrastructure version changes. |

## Compliance Mapping

SOC2 CC8.1 (change management — soak windows validate before promotion). ISO A.14.2.9 (system acceptance testing — soak as extended validation). ISO A.8.8 (vulnerability management — controlled upgrade cadence). NIS2 Art.21 (change management — graduated promotion).
