# ADR-030: OPA Gatekeeper + OPAL for Policy Enforcement

**Status:** Accepted

**Scope:** Universal

**Category:** Security / Policy

**Related ADRs:** 089, 131, 057

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

CAVE needs policy enforcement across the full stack:

- **K8s admission:** ValidatingWebhook blocks non-compliant workloads at deployment time
- **CI pipeline:** Conftest gates on Helm charts, Crossplane compositions, Dockerfiles (stages 5, 17, 52)
- **IaC validation:** Policy enforcement on Terraform/OpenTofu specs before apply
- **Runtime governance:** Policy drift detection via OPAL → real-time sync when policies change

Single policy language (Rego) reusable across all contexts reduces maintenance vs. multiple policy syntaxes (Kyverno YAML for K8s, separate tool for CI, etc.).

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

## Implementation Reference

**Implementation Status:** Production

- **cave-policy** crate: OPA policy bundles, Conftest integration in CI stages 5+17
- **cave-admission** crate: Gatekeeper ConstraintTemplate + Constraint CRDs
- **Integration:** OPAL distributes policies in real-time from Git (cave-policy-config repo)

## Rejected Options

### Kyverno — Not Primary

**Reasons:**
1. **K8s-only:** Cannot reuse YAML policies in CI pipeline (stage 5 schema validation, stage 17 IaC scan) or Terraform/OpenTofu validation. Separate tool needed for CI.
2. **No signed bundles:** Kyverno policies loaded from cluster storage. No cosign signature verification (ADR-089).
3. **No OPAL equivalent:** OPAL provides real-time policy sync + external data injection (tenant metadata, access lists). Kyverno requires ArgoCD sync + API calls inside policy (slow).

### Kubewarden / Cedar — Not Recommended

Kubewarden: Wasm-based, smaller ecosystem. Cedar: AWS-specific, not portable.

## Consequences

### Positive

- **Universal policy language:** Rego used in K8s admission, CI pipeline (Conftest), IaC validation (OpenTofu), runtime enforcement. Single syntax → single policy review/audit process.
- **Signed bundles:** cosign-signed OPA policy bundles (ADR-089) provide supply chain integrity. Bundle signature verified before Gatekeeper loads policies.
- **OPAL real-time sync:** External data (FQDN allowlists, tenant metadata, compliance rules) pushed to Gatekeeper in real-time. No ArgoCD sync latency.
- **Audit trails:** Gatekeeper audit mode analyzes all existing workloads against policies. Audit reports feed compliance assessments.

### Negative

- **Rego learning curve:** More complex than Kyverno YAML. Training required. Documentation must be extensive.
- **Mutation limitations:** OPA mutation webhooks less elegant than Kyverno's native mutation. Workaround: mutate in Buildah stage instead.
- **OPAL complexity:** Adds component + failure mode. OPAL-to-Gatekeeper data sync must be reliable. Fallback: ArgoCD manages policy bundles if OPAL down.
- **Fail-closed risk:** Misconfigured policy can block all deployments. Mitigation: audit mode validates policies before enforce mode.

### Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Buggy policy blocks all workload deployments | Low | High | Audit mode validates policies before enforce. Staging tests all policies. P1 runbook for quick rollback. |
| OPAL sync failure causes policy staleness | Low | Medium | Fallback: ArgoCD manages bundle. Health check monitors OPAL-to-Gatekeeper sync lag. Alert if lag >1min. |
| Rego syntax errors in policy bundles | Medium | Low | Code review on all policy changes. Rego linting in CI stage 5. Unit tests (conftest -t). |

## License

**OPA Gatekeeper:** Apache 2.0 (https://github.com/open-policy-agent/gatekeeper/blob/master/LICENSE)

## Compliance Mapping

**SOC2 CC6.1:** Access controls — policies enforce RBAC, tenant isolation, resource quotas.
**ISO/IEC 27001 A.5.1:** Information security policy — formal policies for compliance.
**NIS2 Directive Article 21:** Configuration management — policy-as-code enforces secure configuration baselines.
