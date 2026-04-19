# ADR-057: Application Security Testing Strategy — Defense-in-Depth

**Status:** Accepted

**Scope:** Azure, Universal

**Category:** Security

**Related ADRs:** 010, 017, 018, 019, 023, 035, 058

## Context

CAVE's security testing must span the entire software lifecycle — from pre-commit through runtime. No single tool catches all vulnerability types. A layered strategy ensures coverage across secrets, code quality, dependencies, containers, infrastructure, compliance, and runtime behavior.

## Candidates

| Approach | Multi-layer (chosen) | Single SAST tool | SaaS scanner | Manual review only |
|---|---|---|---|---|
| Coverage | ✅ 7 layers | ⚠️ Code only | ⚠️ Vendor-dependent | ❌ Human bandwidth limited |
| Automation | ✅ CI-integrated | ✅ | ✅ | ❌ |
| False negative rate | ✅ Low (complementary tools) | ❌ High (single perspective) | ⚠️ | ❌ High |

## Decision

**Seven-layer defense-in-depth security testing integrated into 27-stage CI pipeline (ADR-010):**

| Layer | Tool | CI Stage | Gate | Finding Destination |
|---|---|---|---|---|
| Secrets | gitleaks (ADR-017) | Pre-commit + Stage 2 | BLOCK | DefectDojo |
| SAST | SonarQube + Semgrep (ADR-019) | Stages 3-4 | BLOCK critical/high | DefectDojo |
| SCA/SBOM | CycloneDX + DTrack | Stages 9-10 | BLOCK critical unfixed | DTrack + DefectDojo |
| Container | Trivy (ADR-018) | Stage 16 | BLOCK critical unfixed | DefectDojo |
| IaC | Conftest + Checkov | Stages 17-18 | BLOCK policy violations | DefectDojo |
| Compliance | Kubescape (ADR-058) | Stage 19 | WARN → BLOCK (Phase 3+) | DefectDojo |
| DAST | OWASP ZAP (ADR-023) | Stages 23-24 | BLOCK high findings | DefectDojo |

All findings aggregated in DefectDojo (ADR-035) for unified lifecycle management. Per-severity SLA: Critical 7d, High 30d, Medium 90d, Low 180d.

## Rejected

- **Single SAST tool only:** Catches code-level issues but misses dependencies, containers, infrastructure, runtime. Single tool = single perspective = blind spots.
- **SaaS security scanner (Snyk, Checkmarx):** Proprietary, per-scan pricing, code sent to external service. Contradicts sovereign profile.
- **Manual security review only:** Doesn't scale for 27-stage pipeline across multiple tenants. Human review complements but cannot replace automated scanning.

## Consequences

**Positive:**
- Seven layers of automated security testing — comprehensive coverage.
- CI-integrated — no manual steps between development and deployment.
- All findings in one dashboard (DefectDojo) — unified triage and SLA tracking.
- Complementary tools reduce false negatives (what SonarQube misses, Semgrep catches).

**Negative:**
- Seven tools to maintain (version updates, rule tuning, false positive management).
- CI pipeline duration increases (~5-15 min from security stages).
- Finding volume can be high — triage discipline required.
- DefectDojo deduplication is good but not perfect — some cross-tool duplicates need manual triage.

## Compliance Mapping

SOC2 CC7.1 (multi-layer vulnerability management). ISO A.8.25-28 (complete secure development lifecycle). OWASP Top 10 (covered by SAST + DAST). NIS2 Art.21 (comprehensive vulnerability management). SLSA Level 3 (supply chain scanning).
