# ADR-099: Deprecation Guardrails in CI — Pluto + kubent

**Status:** Accepted

**Scope:** Universal

**Category:** CI/CD

**Related ADRs:** 010, 127, 134

## Context

Kubernetes deprecates APIs across minor versions. Helm charts and manifests using deprecated APIs will break after upgrade. Must detect deprecated APIs before they reach production.

## Candidates

| Criteria | Pluto + kubent | kubent only | Pluto only | Manual review |
|---|---|---|---|---|
| Deprecated K8s APIs | ✅ Both detect | ✅ | ✅ | ❌ |
| Helm chart scanning | ✅ Pluto scans Helm releases | ❌ | ✅ | ❌ |
| In-cluster scanning | ✅ kubent scans running resources | ✅ | ❌ | ❌ |
| CI integration | ✅ CLI (exit code for CI gate) | ✅ | ✅ | ❌ |
| Target K8s version | ✅ Configurable per profile | ✅ | ✅ | ❌ |

## Decision

**Pluto** (CI stage 20) for pre-deployment deprecation scanning of Helm charts and manifests. **kubent** for in-cluster scanning of running resources. BLOCK gate in CI on deprecated APIs for prod profile. WARN for dev/staging. Integration with deprecation runway (ADR-134) for migration planning.

## Rejected

- **Manual review:** Unsustainable. K8s deprecates APIs silently across minor versions. Engineers miss deprecation warnings.
- **Single tool only:** Pluto catches pre-deployment (CI). kubent catches in-cluster (drift). Both needed for complete coverage.

## Consequences

**Positive:**
- Deprecated K8s APIs caught before production deployment.
- Pluto (pre-deployment) + kubent (in-cluster) provides complete coverage.
- BLOCK gate prevents deprecated APIs from reaching prod.
- Integration with deprecation runway (ADR-134) for planned migration.

**Negative:**
- Pluto/kubent must be updated when new K8s versions introduce deprecations.
- BLOCK gate can slow deployments if Helm charts use deprecated APIs that haven't been updated upstream.
- In-cluster kubent scanning requires RBAC to read all resources.

## Compliance Mapping

SOC2 CC8.1 (change management — API compatibility validation). ISO A.14.2 (secure development — compatibility testing).
