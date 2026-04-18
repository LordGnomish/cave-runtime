# ADR-058: Kubernetes Compliance Scanning — Kubescape

**Status:** Accepted

**Category:** Security

**Related ADRs:** 010, 057

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE clusters must comply with CIS Kubernetes Benchmark and NSA/CISA Hardening Guide. Continuous compliance scanning detects configuration drift from security baselines.

## Candidates

## | Criteria | Kubescape | kube-bench | Polaris | Starboard |
|---|---|---|---|---|
| CIS Benchmark | ✅ | ✅ | ⚠️ Partial | ✅ |
| NSA/CISA Guide | ✅ | ❌ | ❌ | ⚠️ |
| MITRE ATT&CK | ✅ | ❌ | ❌ | ❌ |
| CI integration | ✅ CLI + GitHub Action | ✅ CLI | ✅ | ⚠️ |
| Continuous scanning | ✅ In-cluster operator | ❌ One-shot | ⚠️ | ✅ |
| DefectDojo export | ✅ JSON/SARIF | ⚠️ | ⚠️ | ⚠️ |
| License | Apache 2.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 |

## Decision

## **Kubescape** for CI stage 19 (compliance scanning) + in-cluster continuous scanning. CIS Benchmark, NSA/CISA, and MITRE ATT&CK frameworks. Phase 1: WARN gate. Phase 3+: BLOCK gate. Findings to DefectDojo.

## Rejected

## - **kube-bench:** CIS only, no NSA/CISA or MITRE. One-shot scanning (no continuous).
- **Polaris:** Partial CIS coverage. No NSA/CISA.
- **Starboard (Trivy Operator):** Merged into Trivy. Kubescape provides richer framework coverage.

## Consequences

## **Positive:**
- CIS + NSA/CISA + MITRE ATT&CK frameworks in one tool — broadest compliance coverage.
- Continuous in-cluster scanning catches configuration drift between CI runs.
- DefectDojo integration for unified finding lifecycle.
- Apache 2.0 — no licensing concerns.

**Negative:**
- Kubescape finding volume can be high (CIS + NSA + MITRE = hundreds of checks). Requires baseline tuning.
- Phase 1 WARN gate means findings are informational only — BLOCK enforced from Phase 3+.
- In-cluster operator requires RBAC to read all cluster resources (wide read access).

## Compliance Mapping

## SOC2 CC6.1 (configuration compliance). ISO A.8.8 (vulnerability management — configuration). NIS2 Art.21 (security baseline compliance). CIS Kubernetes Benchmark. NSA/CISA K8s Hardening Guide.
