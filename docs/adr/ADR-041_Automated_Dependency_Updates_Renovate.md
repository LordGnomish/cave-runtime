# ADR-041: Automated Dependency Updates — Renovate

**Status:** Accepted

**Category:** CI/CD

**Related ADRs:** 010, 108, 127

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE has 73 components, each with upstream dependencies (container images, Helm charts, Crossplane providers, npm/pip packages). Manual dependency tracking is unsustainable.

## Candidates

## | Criteria | Renovate | Dependabot | WhiteSource Renovate (Mend) | Manual |
|---|---|---|---|---|
| Self-hosted | ✅ Self-hosted runner | ⚠️ GitHub-hosted only | ✅ | N/A |
| Digest pinning | ✅ Native (ADR-108) | ⚠️ Limited | ✅ | ❌ |
| Auto-merge | ✅ Configurable per package | ✅ | ✅ | ❌ |
| Grouping | ✅ Group related updates | ❌ Individual PRs only | ✅ | ❌ |
| Helm/Docker/K8s | ✅ 60+ managers | ⚠️ Limited ecosystem support | ✅ | ❌ |
| License | AGPL-3.0 (self-hosted is fine) | MIT (GitHub service) | Commercial | N/A |

## Decision

## **Renovate** (self-hosted) for automated dependency updates across all repositories. Digest pinning enforced (ADR-108). PRs grouped by ecosystem (e.g., all Cilium-related updates in one PR). Auto-merge for patch versions with passing CI. Major/minor versions require human review. Integrated with roadmap intelligence (ADR-127).

## Rejected

## - **Dependabot:** GitHub-hosted only (no self-hosting). Limited ecosystem support (no Crossplane, limited Helm). No PR grouping — creates PR flood for 73 components.
- **Manual tracking:** Unsustainable at 73 components. Missed updates create vulnerability exposure.

## Consequences

## **Positive:**
- Automated, continuous dependency freshness across all components.
- Digest pinning prevents mutable tag attacks.
- Grouped PRs reduce review fatigue.
- Integration with CI pipeline validates every update.

**Negative:**
- Renovate PR volume can be high (mitigated: grouping, auto-merge for patches, schedule windows).
- AGPL-3.0 license (acceptable for self-hosted — AGPL only triggers if distributed as SaaS).
- Renovate config (`renovate.json`) per repo requires maintenance.

## Compliance Mapping

## SOC2 CC7.1 (vulnerability management — automated patching). ISO A.8.8 (technical vulnerability management). NIS2 Art.21 (supply chain security — dependency freshness).
