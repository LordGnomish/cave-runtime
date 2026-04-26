# ADR-133: Compatibility Matrix as Code

**Status:** Accepted

**Scope:** Azure, Hetzner, Runtime, Universal

**Category:** Platform Governance

**Related ADRs:** 085 (Rolling Upgrades), 099 (Deprecation Guardrails), 127 (Roadmap Intelligence), 132 (Version Channels)

## Context

CAVE's ~73 components have interdependencies that constrain which versions can run together. Examples of breaking interactions:

- Cilium CNI version must be compatible with Kubernetes minor (kernel eBPF requirements)
- Istio ambient ztunnel version must match Cilium CNI for L4/L7 handoff
- ArgoCD v3.3+ server-side apply behavior changed — affects Crossplane XR reconciliation
- Crossplane provider versions must match Crossplane core version (API compatibility)
- KEDA CRDs must be compatible with Kubernetes API server version
- Talos Linux images bundle a specific Kubernetes minor — cannot be mixed
- cert-manager webhook compatibility with Kubernetes admission API

Without a machine-readable matrix, `cave-ctl upgrade check` cannot reliably block incompatible promotions.

---

## Candidates

| Approach | Compatibility matrix (version tuples) — chosen | Latest-of-everything | Component-pinned (no coordination) | Vendor bundles only |
|---|---|---|---|---|
| Dependency validation | ✅ Tuple-level (CI-enforced) | ❌ Assumes compatibility | ⚠️ Per-component only | ⚠️ Vendor-determined |
| Upgrade safety | ✅ Tested combinations before promotion | ❌ Break-on-upgrade | ⚠️ Individual comps tested | ✅ |
| Freshness policy | ✅ 90-day tuple age limit | N/A | ⚠️ | ⚠️ |
| CI gate | ✅ cave-ctl upgrade check | ❌ None | ❌ None | ⚠️ |
| Flexibility | ✅ Can pin or roll any component | ✅ Max flexibility | ✅ Max flexibility | ❌ Bundle-locked |

## Decision

Maintain `docs/compatibility-matrix.yaml` as a **constitutional artifact** (requires multi-sig for changes). `cave-ctl upgrade check --profile <p>` validates all tuples before any promotion.

### Matrix Schema

```yaml
# docs/compatibility-matrix.yaml
version: "2026-04"
tuples:
  - id: "k8s-1.36-talos-1.12"
    kubernetes: "1.36.x"
    talos: "1.12.x"
    aks: "1.36.x"
    argocd: "3.2.x - 3.4.x"
    crossplane: "2.2.x - 2.4.x"
    cilium: "1.16.x - 1.17.x"
    istio_ambient: "1.24.x - 1.26.x"
    keda: "2.15.x - 2.17.x"
    cert_manager: "1.16.x - 1.17.x"
    gatekeeper: "3.17.x - 3.18.x"
    eso: "0.12.x - 0.14.x"
    supported: true
    profiles: ["dev-hetzner", "staging-hetzner", "prod-hetzner"]
    caveats:
      - "ArgoCD 3.4+ requires Crossplane 2.1+ for SSA drift detection fix"
      - "Cilium 1.17 requires kernel 6.x+ (Talos 1.12 provides 6.18; kernel 7.0 pending — ADR-014)"
    rollback_target: "k8s-1.35-talos-1.11"
    last_verified: "2026-04-01"
    ledger_hash: "sha256:abc123..."
```

### Validation Rules

| Rule | Enforcement |
|---|---|
| Tuple must be `supported: true` | CI blocks promotion |
| `last_verified` must be < 90 days old | CI warns at 60d, blocks at 90d |
| After upstream major/minor release, tuple must be re-verified | `cave-ctl roadmap scan` opens verification task |
| Rollback target must also be a supported tuple | CI validates |
| All profiles in deployment must use tuples from same matrix version | CI validates cross-profile consistency |

### Matrix Update Process

1. Upstream release detected by `cave-ctl roadmap scan`
2. New tuple proposed (PR to `docs/compatibility-matrix.yaml`)
3. Dev profile validates new tuple (fast channel)
4. Staging validates (stable channel, soak window)
5. Tuple marked `supported: true`, `last_verified` updated
6. Ledger `Compatibility Verified` attestation
7. Prod promotion eligible

---

## Rejected

## ### 3.1 Manual Version Tracking (wiki/spreadsheet) — Rejected
Not machine-readable. Cannot be enforced in CI. Drifts from reality. No audit trail.

### 3.2 Per-Component Version Pinning Only (no matrix) — Rejected
Pins individual versions but doesn't validate inter-component compatibility. ArgoCD 3.4 might be individually valid but incompatible with Crossplane 2.0 — only a matrix catches this.

### 3.3 Vendor-Provided Compatibility (rely on upstream docs) — Rejected
Upstream projects document their own compatibility but not cross-project interactions. Cilium docs don't cover Istio ambient compatibility. Only CAVE knows its specific stack combination.

---

## Consequences

## ### Positive
- Machine-enforceable compatibility guarantees
- No more "works on my cluster" version mismatches between profiles
- Clear rollback target for every supported combination
- Audit trail via Ledger for every compatibility verification
- `cave-ctl upgrade check` becomes deterministic

### Negative
- Matrix maintenance overhead (mitigated by `cave-ctl roadmap scan` automation)
- Constitutional artifact status means matrix changes require multi-sig
- May delay adoption of new versions while matrix is being validated

## Compliance Mapping

SOC2 CC8.1 (change management — compatibility validation before deployment). ISO A.8.9 (configuration management — version tuple tracking). ISO A.14.2 (secure development — dependency compatibility). NIS2 Art.21 (risk management — preventing incompatible deployments).
