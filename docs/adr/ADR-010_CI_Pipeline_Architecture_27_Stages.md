# ADR-010: CI Pipeline Architecture — 27 Stages

**Status:** Accepted

**Scope:** Universal

**Category:** CI/CD

**Related ADRs:** 005, 026, 032, 077, 101, 115, 116

## Context

CAVE needs a standardized CI/CD pipeline that enforces security scanning, supply chain provenance, compliance validation, and progressive delivery across all tenant workloads and platform components. The pipeline must be language-agnostic, support multiple build targets (container, Helm, Crossplane), and produce attestable evidence for every build.

## Candidates

| Criteria | GitHub Actions + ARC | GitLab CI | Jenkins | Tekton |
|---|---|---|---|---|
| Self-hosted runners | ✅ ARC (K8s native, ephemeral) | ✅ K8s runner | ✅ K8s agents | ✅ K8s native |
| OIDC token exchange | ✅ Native (ADR-115) | ✅ | ⚠️ Plugin-based | ⚠️ |
| Reusable workflows | ✅ Composite actions + reusable workflows | ✅ includes | ⚠️ Shared libraries | ✅ Tasks |
| Marketplace/ecosystem | ✅ Largest (GitHub Actions marketplace) | ✅ Large | ✅ Plugins | ❌ Small |
| Matrix builds | ✅ Native matrix strategy | ✅ | ⚠️ | ⚠️ |
| SLSA provenance | ✅ slsa-github-generator (native) | ⚠️ Custom | ⚠️ Custom | ⚠️ |
| Backstage integration | ✅ GitHub Actions plugin | ⚠️ | ⚠️ | ❌ |
| GitOps trigger | ✅ ArgoCD webhook / OCI push to Harbor | ✅ | ✅ | ✅ |

## Decision

**GitHub Actions** with **Actions Runner Controller (ARC)** for K8s-native ephemeral runners. 27-stage pipeline standardized across all workloads. Stages grouped into: pre-build (1-7), build (8-16), validate (17-21), deploy (22-24), promote (25), report (26), cleanup (27).

### 27-Stage Pipeline

| # | Stage | Tool | Gate Level | Evidence |
|---|---|---|---|---|
| 1 | Checkout | Git | — | — |
| 2 | Secret scan | gitleaks | BLOCK on any finding | DefectDojo |
| 3 | SAST (SonarQube) | SonarQube | BLOCK on critical/high | DefectDojo |
| 4 | SAST (Semgrep) | Semgrep | BLOCK on OWASP findings | DefectDojo |
| 5 | License compliance | REUSE lint | BLOCK if non-compliant | — |
| 6 | Schema migration validation | Flyway/Alembic dry-run | BLOCK on failed rollback test | — |
| 7 | API contract validation | Spectral/Pact | WARN on breaking change | — |
| 8 | Build (container) | Buildah --no-network (hermetic) | BLOCK on build failure | — |
| 9 | SBOM generation | CycloneDX | — | DTrack |
| 10 | Dependency vulnerability | DTrack/Grype | BLOCK on critical unfixed | DTrack |
| 11 | Dockerfile lint | Hadolint | WARN | — |
| 12 | Container build | Buildah push | — | — |
| 13 | Harbor push | Harbor OCI | — | — |
| 14 | Image sign | cosign (keyless OIDC) | — | Sovereign Ledger |
| 15 | SLSA provenance | cosign attest (Provenance v1) | — | Sovereign Ledger |
| 16 | Image scan | Trivy | BLOCK on critical unfixed | DefectDojo |
| 17 | IaC policy (Conftest) | Conftest (Rego) | BLOCK on policy violation | — |
| 18 | IaC scan (Checkov) | Checkov | BLOCK on high findings | DefectDojo |
| 19 | Compliance scan | Kubescape (CIS/NSA) | WARN (Phase 1), BLOCK (Phase 3+) | DefectDojo |
| 20 | Deprecation check | Pluto/kubent | BLOCK on deprecated APIs in prod | — |
| 21 | Report aggregation | DefectDojo API | — | DefectDojo |
| 22 | Deploy to staging | ArgoCD sync (vcluster on prod, namespace on dev/staging) | BLOCK on sync failure | — |
| 23 | DAST (ZAP) | ZAP baseline/full scan | BLOCK on high findings | DefectDojo |
| 24 | Integration tests | k6/custom | BLOCK on test failure | — |
| 25 | Promote | Argo Rollouts canary (prod) / rolling update (dev/staging) | BLOCK on SLO regression | Sovereign Ledger |
| 26 | DORA event | DevLake webhook | — | DevLake |
| 27 | Cleanup | vcluster destroy, temp resources | — | — |

### Pipeline Evidence Chain

Every pipeline run produces a single `Pipeline Attestation` in Sovereign Ledger via Merkle tree aggregation: individual stage proofs collected in-memory during pipeline, bundled into one signed hash at completion. One Ledger write per pipeline run (not per stage).

## Rejected

- **GitLab CI:** Would require self-hosting GitLab (heavy infrastructure). GitHub Actions ecosystem is larger. OIDC token exchange and SLSA provenance more mature on GitHub Actions.
- **Jenkins:** Legacy. Plugin maintenance burden. No native OIDC. No native SLSA. Groovy pipeline syntax less maintainable than YAML.
- **Tekton:** K8s-native (good) but small ecosystem. No marketplace equivalent. Backstage integration less mature. Building 27 custom Tekton Tasks is more work than leveraging GitHub Actions marketplace.

## Consequences

**Positive:**
- Standardized 27-stage pipeline across all workloads. No per-team pipeline customization.
- SLSA Level 3 provenance for every build (ADR-101).
- Security scanning at multiple layers: secret (gitleaks), SAST (SonarQube + Semgrep), dependency (DTrack/Grype), container (Trivy), IaC (Conftest + Checkov), compliance (Kubescape), DAST (ZAP).
- Single Ledger attestation per pipeline run (Merkle aggregation).
- ARC runners are ephemeral — no persistent state, no stale credentials (ADR-115).
- Language-agnostic: same pipeline structure for Java, Python, Go, Node.js (build stage adapts per language).

**Negative:**
- 27 stages = longer pipeline execution time. Target: p95 < 15 min. Parallelization (stages 3+4, 9+10+11, 17+18+19) mitigates.
- GitHub Actions dependency — if GitHub is unavailable, CI stops. Mitigated: Gitea self-hosted mirror as fallback (Phase 4).
- DefectDojo finding volume can be high — triage discipline required.
- Each new language requires a build-stage adapter (Buildah + language-specific build commands).

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| GitHub outage blocks all CI | Low | High | Gitea self-hosted mirror as fallback (Phase 4). Critical hotfix path: manual Buildah + cosign on operator laptop with break-glass credentials. |
| Pipeline execution > 15 min SLO | Medium | Medium | Stages 3+4, 9+10+11, 17+18+19 parallelized. Cache Buildah layers in Harbor. Skip unchanged stages via path-filter. Monitor p95 via DORA metrics (ADR-042). |
| DefectDojo finding fatigue (too many alerts) | Medium | Medium | Severity-based gates (only CRITICAL/HIGH block). Weekly triage rotation. Auto-close findings resolved in next build. |
| GitHub Actions runner escape (ARC security) | Very Low | Critical | ARC runners ephemeral (pod destroyed after job). No Docker socket mount. Buildah rootless (ADR-005). Network policy isolates runner pods. |
| SLSA provenance spec breaking change | Low | Medium | Pin slsa-github-generator version. Staging validates before prod. SLSA v1.0 spec is stable. |
| Dagger.io disrupts CI landscape | Low (2027+) | Low | **Watch:** Dagger offers portable CI pipelines (write once, run on any CI engine). If Dagger matures and GitHub Actions lock-in becomes a concern, evaluate Dagger as pipeline abstraction layer. Does not replace GitHub Actions — sits on top. Annual review. |
| GitHub Enterprise licensing cost | Medium | Medium | ARC self-hosted runners avoid per-minute charges. Monitor GitHub plan vs Gitea+Woodpecker self-hosted alternative. Cost-benefit analysis annually. |

## Compliance Mapping

SOC2 CC8.1 (change management — automated pipeline enforces process). SOC2 CC7.1 (vulnerability detection — multi-layer scanning). ISO A.8.25-28 (secure development lifecycle). ISO A.8.8 (vulnerability management). SLSA Level 3 (hermetic build + signed provenance). NIS2 Art.21 (supply chain security, vulnerability management).
