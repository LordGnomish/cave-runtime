# ADR-030: OPA Gatekeeper + OPAL

**Status:** Accepted

**Category:** Security

**Related ADRs:** 089, 131

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE needs policy enforcement at multiple points: K8s admission, IaC validation (Conftest), CI pipeline gates, and runtime governance. Policy language must be reusable across all these contexts.

## Candidates

## | Criteria | OPA Gatekeeper + OPAL | Kyverno | Kubewarden | Cedar (AWS) |
|---|---|---|---|---|
| Policy language | Rego (universal, used in CI + admission + IaC) | YAML/Kyverno policy (K8s-only) | Wasm modules | Cedar (AWS-specific) |
| K8s admission | ✅ Webhook | ✅ Webhook | ✅ Webhook | ❌ |
| CI pipeline (Conftest) | ✅ Same Rego policies | ❌ Different tool needed | ❌ | ❌ |
| IaC validation | ✅ Conftest (Rego on Helm/Crossplane) | ❌ K8s resources only | ❌ | ❌ |
| Signed bundles | ✅ cosign-signed OPA bundles (ADR-089) | ❌ No bundle signing | ❌ | ❌ |
| Real-time data | ✅ OPAL pushes external data (Keycloak, tenant metadata) | ⚠️ API calls in policy (slow) | ❌ | ❌ |
| Mutation | ⚠️ Via mutation webhooks (less elegant) | ✅ Native mutation (cleaner syntax) | ⚠️ | ❌ |
| Audit mode | ✅ Constraint audit (dry-run existing resources) | ✅ Policy reports | ⚠️ | ❌ |
| License | Apache 2.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 |
| Community | Very large (CNCF Graduated — OPA) | Large (CNCF Incubating) | Small | AWS-specific |

## Decision

## **OPA Gatekeeper** for admission + audit. **Conftest** for CI + IaC (same Rego). **OPAL** for real-time data distribution (ADR-131). Git is sole policy source of truth.

## Rejected

## - **Kyverno:** K8s admission only — cannot reuse policies in CI pipeline (Conftest) or IaC validation. No signed bundle provenance. No OPAL-equivalent real-time external data integration. Kyverno's YAML policy syntax is simpler for basic admission but Rego's universality across CI + admission + IaC is a stronger architectural choice for CAVE's 27-stage pipeline. Kyverno's mutation is cleaner — acknowledged trade-off.
- **Kubewarden:** Wasm-based, smaller community, less mature. No CI/IaC reuse.
- **Cedar:** AWS-specific. Not portable.

## Consequences

## (+) Single policy language (Rego) across all enforcement points. Signed bundle provenance for supply chain security. OPAL real-time data eliminates ArgoCD sync delay for policy-critical data. Cross-stack policy reuse reduces maintenance.
(-) Rego learning curve steeper than Kyverno YAML. OPA mutation less elegant than Kyverno. OPAL adds component + failure mode. Gatekeeper fail-closed can block all deployments if misconfigured.

## Compliance Mapping

## SOC2 CC6.1 (access controls), ISO A.5.1 (policies for information security), NIS2 Art.21 (security policies).
