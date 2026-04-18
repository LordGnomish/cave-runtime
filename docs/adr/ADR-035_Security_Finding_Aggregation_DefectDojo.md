# ADR-035: Security Finding Aggregation — DefectDojo

**Status:** Accepted

**Category:** Security

**Related ADRs:** 010, 017, 018, 019, 023

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE's 27-stage CI pipeline produces security findings from 7+ tools (gitleaks, SonarQube, Semgrep, DTrack, Trivy, Checkov, ZAP). Without aggregation, findings are scattered across tool-specific dashboards. Need a single pane for finding lifecycle management: deduplication, triage, risk acceptance, SLA tracking.

## Candidates

## | Criteria | DefectDojo | Dependency-Track (DTrack) | Jira (manual) | SonarQube (as aggregator) |
|---|---|---|---|---|
| Multi-tool import | ✅ 150+ parser plugins (Trivy, ZAP, Semgrep, gitleaks, Checkov, etc.) | ⚠️ SBOM-focused only | ❌ Manual entry | ❌ SonarQube findings only |
| Deduplication | ✅ Cross-tool deduplication | ⚠️ SBOM-level only | ❌ | ❌ |
| Finding lifecycle | ✅ Active → Verified → Mitigated → Closed → Risk Accepted | ❌ | ✅ | ⚠️ |
| SLA tracking | ✅ Per-severity SLA (Critical: 7d, High: 30d, Medium: 90d) | ❌ | ⚠️ | ❌ |
| API | ✅ Full REST API (CI integration) | ✅ | ✅ | ✅ |
| Tenant scoping | ✅ Product-per-tenant | ⚠️ | N/A | N/A |
| License | BSD 3-Clause | Apache 2.0 | Proprietary | LGPL (Community) |

## Decision

## **DefectDojo** for security finding aggregation and lifecycle management. **DTrack** (Dependency-Track) as complementary SBOM vulnerability tracker. Both deployed per-profile. DefectDojo Product-per-tenant for multi-tenant isolation. DTrack provides continuous SBOM monitoring; DefectDojo aggregates all finding types.

## Rejected

## - **DTrack alone:** SBOM/dependency vulnerabilities only. Cannot import SAST, DAST, IaC scan, or secret scan findings.
- **Jira as finding tracker:** No deduplication, no multi-tool import, no SLA tracking, no severity auto-classification. Manual overhead.
- **SonarQube as aggregator:** Only aggregates its own findings. Cannot import Trivy, ZAP, gitleaks results.

## Consequences

## **Positive:**
- Single dashboard for all security findings across 7+ tools.
- Cross-tool deduplication prevents duplicate triage effort.
- Per-severity SLA tracking (Critical: 7d fix, High: 30d, Medium: 90d, Low: 180d).
- Product-per-tenant isolation — tenants see only their findings.
- API-driven: CI pipeline auto-imports findings, no manual upload.

**Negative:**
- DefectDojo server requires PostgreSQL + ~1GB RAM.
- Finding volume can be high (hundreds per pipeline run across all tools) — requires triage discipline.
- Two tools (DefectDojo + DTrack) for comprehensive coverage — DTrack handles continuous SBOM monitoring between pipeline runs.

## Compliance Mapping

## SOC2 CC7.1 (vulnerability management lifecycle). ISO A.8.8 (technical vulnerability management). NIS2 Art.21 (vulnerability management). GDPR Art.32 (security assessment evidence).

**Absorbed Decisions:** The following tool-level decisions are absorbed into this ADR for traceability

**ADR-034**

Dependency-Track for SBOM/SCA

**Decision:** Dependency-Track for continuous SBOM/SCA monitoring, feeds DefectDojo aggregation. Rejection: Grype (scan-only, no continuous tracking, no policy engine).

**ADR-069**

DefectDojo as Single Source of Truth

**Decision:** DefectDojo is the single source of truth for ALL security findings, including pre-commit findings logged via webhook (auto-closed for audit trail). No parallel finding stores.
