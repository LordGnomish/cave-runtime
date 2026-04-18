# ADR-127: Roadmap Intelligence Automation

**Status:** Accepted

**Category:** Governance

**Related ADRs:** ## Context

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## 73 components each have upstream release cycles, deprecation schedules, and EOL dates. Manual tracking is unsustainable.

## Candidates

## | Approach | Automated scan (cave-ctl roadmap scan) | Manual tracking | Vendor dashboards only |
|---|---|---|---|
| Coverage | ✅ All 73 components | ❌ Human bandwidth limited | ⚠️ Per-vendor |
| Forward visibility | ✅ 24-month lookahead | ⚠️ Reactive (EOL surprises) | ⚠️ Vendor-controlled |
| Integration | ✅ Auto-opens backlog issues | ❌ | ❌ |

## Decision

## `cave-ctl roadmap scan --months 24 --profile <p>` queries: GitHub Releases API, CNCF project status, vendor deprecation announcements, K8s API deprecation guide. Weekly CI job produces `roadmap-findings.md` and auto-opens backlog issues (severity based on deprecation timeline). Integration with ADR-134 (deprecation runway) and ADR-133 (compatibility matrix).

## Rejected

## - **Manual tracking only:** Unsustainable with 73 components. Deprecation deadlines missed. Surprise EOL forces emergency upgrades.
- **Vendor dashboards only:** Each vendor has different dashboard. No aggregation. No automated issue creation. Not sovereign — vendor controls visibility.

## Consequences

## **Positive:**
- 24-month forward visibility for all component lifecycle changes.
- Automated issue creation prevents missed deprecation deadlines.
- Integration with deprecation runway (ADR-134) triggers policy enforcement.
- Compatibility matrix (ADR-133) updates driven by scan findings.

**Negative:**
- Upstream API changes (GitHub rate limits, page structure changes) may break scans.
- False positives: pre-release announcements may not result in actual deprecation.
- Weekly scan frequency may miss fast-moving deprecation (mitigated: critical components scanned daily).

## Compliance Mapping

## SOC2 CC7.1 (technology monitoring). ISO A.8.8 (management of technical vulnerabilities). NIS2 Art.21 (risk management — proactive vulnerability management).
