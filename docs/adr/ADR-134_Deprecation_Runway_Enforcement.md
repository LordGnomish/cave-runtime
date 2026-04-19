# ADR-134: Deprecation Runway Enforcement

**Status:** Accepted

**Scope:** Runtime, Universal

**Category:** Platform Governance

**Related ADRs:** 085 (Upgrades), 099 (Pluto/kubent), 127 (Roadmap Scan), 132 (Version Channels), 133 (Compatibility Matrix)

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Kubernetes deprecates APIs with a documented timeline (typically 2-3 releases). CNCF projects and cloud providers similarly deprecate features. CAVE uses Pluto/kubent (ADR-099) to detect deprecated APIs, but lacks a formal policy for how quickly deprecations must be resolved and what happens when external components or managed services announce breaking changes.

With 73 components and quarterly upstream releases, multiple deprecations can be in-flight simultaneously. Without explicit runways, the platform risks hitting removal deadlines unprepared.

---

## Candidates

## | Approach | Category-based runway timelines (chosen) | Fixed runway for all (e.g., 6 months) | Immediate deprecation | Tenant-driven timeline |
|---|---|---|---|---|
| Risk-adjusted planning | ✅ Critical components longer runway | ❌ Same for everything | ❌ No planning | ⚠️ Tenant variance |
| Upstream reality | ✅ Matches upstream EOL patterns | ⚠️ May conflict with upstream | ❌ Crisis-driven | ⚠️ |
| Platform control | ✅ Platform owns timeline | ✅ | ✅ | ❌ Tenant drives |
| Tenant notification | ✅ Early warning per category | ⚠️ Uniform | ❌ Late warning | ✅ |
| Migration cost | ✅ Predictable | ⚠️ | ❌ Emergency cost | ⚠️ |

## Decision

## | Deprecation Category | Detection | Required Action Window | Escalation |
|---|---|---|---|
| **Kubernetes API deprecation** | Pluto/kubent in CI (stage 20) | Fix within 2 sprints of detection | Blocks prod promotion after deadline |
| **Critical component EOL < 6 months** | `cave-ctl roadmap scan` | Immediate ADR review + migration plan | Guardian review within 1 week |
| **Managed service breaking change** | Provider announcements + roadmap scan | ≤ 30 days migration plan | ADR required if architectural impact |
| **Archived/low-health OSS project** | Quarterly component health scoring | Quarterly exit review | Replace candidate identified within 90 days |
| **Security tool deprecation** | CVE feed + vendor announcements | ≤ 14 days assessment | P1 if no mitigation path |

### Enforcement

- CI stage 20 (Pluto/kubent) blocks promotion for deprecated APIs past runway deadline
- `cave-ctl roadmap scan` auto-opens Jira/GitHub issues with deprecation category and deadline
- Sovereign Ledger `Deprecation Acknowledged` attestation required for any extension beyond runway
- Compatibility matrix (ADR-133) marks tuples with deprecated components as `supported: false` after runway expiry

### Runway Extension Process

Extensions require: guardian approval, ADR documenting risk acceptance, compensating controls, hard deadline (max 1 additional quarter), Ledger attestation.

---

## Rejected

## - **No runway (immediate deprecation):** Upstream component EOL forces emergency migration. No planning time. Incident-driven upgrades.
- **Fixed runway for all components (e.g., always 6 months):** Different components have different risk profiles. Critical components need longer runway. Tooling components can be replaced faster.
- **Tenant-driven deprecation timeline:** Platform team, not tenants, must control component lifecycle. Tenants are notified but don't drive deprecation schedule.

## Consequences

## ### Positive
- Predictable deprecation handling across 73 components
- No surprise breakages from unaddressed upstream removals
- Audit trail for every deprecation response
- Forces proactive component health monitoring

### Negative
- Strict runways may create sprint pressure when multiple deprecations coincide
- Extension process adds governance overhead (justified by risk)

## Compliance Mapping

## SOC2 CC8.1 (change management — planned deprecation lifecycle). ISO A.8.8 (vulnerability management — EOL component replacement). NIS2 Art.21 (supply chain — upstream component lifecycle management).
