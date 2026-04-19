# ADR-041: Automated Dependency Updates — Renovate

**Status:** Accepted

**Scope:** Universal

**Category:** CI/CD / Dependency Management

**Related ADRs:** 010, 108, 127, 099

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

## Implementation Reference

**Implementation Status:** Production

- **cave-upstream** crate: Renovate configuration management, update policy enforcement
- **Self-hosted:** Renovate runner on Kubernetes (cron job), push results to GitHub Actions for CI validation
- **Policy:** Patch auto-merge (if CI passes). Minor/major version PRs require human review + runbook check (ADR-099).

## Consequences

### Positive

- **Automated freshness:** Continuous dependency updates across all 73 components without manual tracking.
- **Digest pinning:** Container image digests locked (ADR-108). Prevents mutable tag attacks (latest tag changing after build).
- **Grouped PRs:** Related updates batched (e.g., all Cilium updates in one PR). Reduces from 100s PRs to dozens per cycle.
- **CI-validated:** Every Renovate PR runs full 27-stage pipeline before merge. Integration failures caught early.
- **Roadmap intelligence:** Renovate findings feed roadmap (ADR-127). High-churn dependencies identified for strategic redesign.

### Negative

- **PR volume:** Even with grouping, 50-100 PRs per month from Renovate. Requires PR review SLA discipline.
- **AGPL-3.0 licensing:** Self-hosted is fine. Mend's Renovate SaaS is proprietary; self-hosted Renovate is AGPL. If CAVE ever offered Renovate-as-a-Service, would require code disclosure.
- **Config maintenance:** renovate.json per repo requires updates as new dependencies introduced.

### Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Major version update breaks application | Medium | High | Minor/major version PRs require human review + testing in staging. Auto-merge only for patches. |
| Renovate runner pod failure stops updates | Low | Medium | Renovate runner health check. Alert if >1 week without update PR. Manual trigger fallback. |
| Digest pinning prevents legitimate base image updates | Low | Low | Weekly digest refresh for base images (ubuntu:22.04, alpine). Script in cave-upstream automates. |

## License

**Renovate:** AGPL-3.0 for self-hosted (https://github.com/renovatebot/renovate/blob/main/LICENSE). AGPL copyleft applies only to distributed SaaS versions.

## Compliance Mapping

**SOC2 CC7.1:** Vulnerability management — automated patching ensures timely security updates.
**ISO/IEC 27001 A.8.8:** Technical vulnerability management — systematic dependency updating.
**NIS2 Directive Article 21:** Vulnerability and incident management — supply chain dependency freshness reduced attack surface.
