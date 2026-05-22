# ADR-019: Static Application Security Testing — SonarQube + Semgrep

**Status:** Accepted

**Scope:** Universal

**Category:** Security

**Related ADRs:** 010

## Context

CAVE needs static analysis to detect code quality issues, security vulnerabilities (OWASP Top 10), and anti-patterns before code reaches runtime. Two complementary tools provide defense-in-depth.

## Candidates

| Criteria | SonarQube + Semgrep | SonarQube only | Semgrep only | CodeQL | Checkmarx |
|---|---|---|---|---|---|
| Code quality | ✅ SonarQube (best-in-class) | ✅ | ⚠️ Limited | ⚠️ | ✅ |
| OWASP security rules | ✅ Semgrep (OWASP rulesets) | ⚠️ Basic security rules | ✅ | ✅ | ✅ |
| Custom rules | ✅ Both support custom | ✅ SonarQube rules | ✅ YAML-based rules | ✅ QL | ✅ |
| Language support | ✅ 30+ (SQ) + 30+ (Semgrep) | ✅ 30+ | ✅ 30+ | ⚠️ Fewer | ✅ |
| Self-hosted | ✅ SQ Community, Semgrep OSS | ✅ | ✅ | ⚠️ GitHub-hosted | ❌ SaaS |
| CI integration | ✅ Both have GitHub Actions | ✅ | ✅ | ✅ GitHub native | ✅ |
| License | SQ Community: LGPL. Semgrep: LGPL | LGPL | LGPL | MIT | Proprietary |

## Decision

**SonarQube Community** (CI stage 3) for code quality + basic security. **Semgrep OSS** (CI stage 4) for OWASP-focused security rules. Complementary: SonarQube catches code quality and maintainability issues; Semgrep catches security-specific patterns (injection, auth bypass, crypto misuse).

## Rejected Options

### SonarQube Only — Rejected

**Primary:** Insufficient OWASP coverage. SonarQube Community Edition has basic security rules but lacks the depth of Semgrep's OWASP-focused rulesets. Enterprise Edition has better security rules but costs €15K+/year. Semgrep OSS fills the security gap at zero cost.

**Secondary:** No taint analysis in Community Edition. SonarQube Enterprise can track data flow (source → sink) for injection detection. Community cannot. Semgrep's pattern matching catches many of the same injection patterns without requiring taint tracking.

### Semgrep Only — Rejected

**Primary:** No code quality analysis. Semgrep matches patterns but does not analyze complexity, duplication, cognitive load, or maintainability. SonarQube's quality gates (“no new code below B rating”) enforce team-wide code quality standards that Semgrep cannot provide.

**Secondary:** No SonarQube-equivalent dashboard. SonarQube's UI shows quality trends over time, technical debt estimates, and per-project quality gates. Semgrep outputs findings but has no integrated quality tracking dashboard.

### CodeQL — Rejected (but Watch)

**Primary:** GitHub-hosted execution model. CodeQL analysis runs on GitHub Actions compute (github.com-hosted) or requires GitHub Advanced Security license for self-hosted runners. CAVE's Gitea fallback (Phase 4) would lose CodeQL capability.

**Secondary:** Smaller rule library than Semgrep for non-GitHub-native languages. CodeQL excels at C/C++/Java but has fewer rules for Python/TypeScript compared to Semgrep Registry.

**Watch:** CodeQL is MIT-licensed and increasingly self-hostable. If CAVE remains GitHub-only (no Gitea fallback), CodeQL could replace Semgrep for security analysis. Re-evaluate annually.

### Checkmarx — Rejected

**Primary:** Proprietary, SaaS-focused. €50K+/year licensing. No self-hosted option. Contradicts CAVE's OSS-first principle and sovereign hosting requirement.

### Licensing Watch Note

**SonarSource trend:** SonarQube Community Edition is progressively losing features to paid editions (branch analysis removed 2024). If Community becomes too limited, evaluate **SonarQube Community Branch Plugin** (third-party) or migrate quality gates to **MegaLinter** (MIT, aggregates multiple linters).

**Semgrep license shift:** Semgrep's registry rules are moving toward restricted access. CAVE should vendor (snapshot) the OWASP rule pack and maintain independently if registry access is restricted. Semgrep OSS engine remains LGPL.

## Consequences

**Positive:**
- Complementary coverage: code quality (SonarQube) + security patterns (Semgrep).
- Both tools free/open source for CAVE's use case.
- Both export findings to DefectDojo for unified finding management.
- OWASP Top 10 coverage via Semgrep community rulesets (continuously updated).
- Custom rules for CAVE-specific patterns (e.g., detecting hardcoded cave_uid, direct kubectl usage).

**Negative:**
- Two SAST tools = two configurations to maintain, two finding streams to triage.
- SonarQube server requires PostgreSQL backend + ~2GB RAM.
- False positives from both tools require triage discipline.
- SonarQube Community edition lacks some Enterprise features (branch analysis, portfolio management) — acceptable for CAVE.

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| SonarQube Community Edition feature removal | Medium | Medium | Track SonarQube release notes. Semgrep covers OWASP rules independently — SonarQube is complementary. |
| False positive fatigue from dual SAST | Medium | Medium | Tune rule sets. Suppress known false positives. Weekly triage rotation. |
| Semgrep rule maintenance (custom rules) | Low | Low | Use Semgrep Registry (community-maintained). Custom rules only for CAVE-specific patterns. |
| SonarQube resource overhead (~2GB RAM) | Low | Medium | Dedicated node or shared infra node. Monitor via OpenCost. |

## Compliance Mapping

SOC2 CC8.1 (secure development — static analysis). ISO A.8.25-28 (secure development lifecycle). NIS2 Art.21 (secure development practices). OWASP Top 10 coverage.
