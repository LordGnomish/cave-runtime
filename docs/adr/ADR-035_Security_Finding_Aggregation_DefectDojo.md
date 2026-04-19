# ADR-035: Security Finding Aggregation — DefectDojo

**Status:** Accepted

**Scope:** Universal

**Category:** Security / Finding Management

**Related ADRs:** 010, 017, 018, 019, 023, 034, 069

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

## Implementation Reference

**Implementation Status:** Production

- **cave-security** crate: DefectDojo + DTrack deployment, finding import pipelines
- **Integration:** CI stages 2, 3, 4, 10, 16, 18, 21, 23 push findings via REST API
- **SLA policy:** Critical findings 7d fix deadline, High 30d, Medium 90d, Low 180d (tracked in DefectDojo, alerts to on-call)

## Consequences

### Positive

- **Single pane of glass:** All 7+ tool findings (gitleaks, SonarQube, Semgrep, DTrack, Trivy, Checkov, ZAP, Kubescape) in one dashboard.
- **Cross-tool deduplication:** Same vuln found by Trivy + Checkov = deduplicated, triaged once.
- **Per-severity SLA tracking:** Critical 7d fix deadline, High 30d, Medium 90d tracked with automated escalation.
- **Multi-tenancy:** Product-per-tenant isolation. Tenant A cannot see tenant B's findings.
- **API-driven automation:** CI pipeline auto-imports findings on completion. No manual upload.
- **Risk acceptance workflow:** Security team approves risk waivers (ADR-140) through DefectDojo UI.

### Negative

- **Infrastructure overhead:** DefectDojo + DTrack = 2 services, 2 DBs, ~2GB RAM combined.
- **Finding volume:** 100+ findings per pipeline run common (especially in SAST stages). Triage fatigue requires discipline.
- **Two tools for complete coverage:** DTrack continuously monitors SBOM, DefectDojo aggregates all types. Operational model split.

### Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Critical finding missed due to dedup failure | Low | High | Audit findings in DefectDojo per severity. Weekly review of high-severity backlog. |
| SLA breach (critical not fixed in 7d) | Medium | Medium | Automated P1 alert when approaching deadline. Escalation to DevOps lead. Waiver process for unavoidable delays (ADR-140). |
| DefectDojo database corruption | Low | High | PostgreSQL HA via CNPG (ADR-105). Daily automated backups. Restore test monthly. |

## License

**DefectDojo:** BSD 3-Clause (https://github.com/DefectDojo/defectdojo/blob/dev/LICENSE)

## Compliance Mapping

**SOC2 CC7.1:** Vulnerability management lifecycle — DefectDojo tracks finding from discovery to remediation to closure.
**ISO/IEC 27001 A.8.8:** Technical vulnerability management — systematic tracking of all security findings.
**NIS2 Directive Article 21:** Vulnerability and incident management — documented SLA-driven remediation.
**GDPR Article 32:** Security of processing — evidence of vulnerability assessments and remediation tracked in DefectDojo.

**Absorbed Decisions:** The following tool-level decisions are absorbed into this ADR for traceability

**ADR-034**

Dependency-Track for SBOM/SCA

**Decision:** Dependency-Track for continuous SBOM/SCA monitoring, feeds DefectDojo aggregation. Rejection: Grype (scan-only, no continuous tracking, no policy engine).

**ADR-069**

DefectDojo as Single Source of Truth

**Decision:** DefectDojo is the single source of truth for ALL security findings, including pre-commit findings logged via webhook (auto-closed for audit trail). No parallel finding stores.
