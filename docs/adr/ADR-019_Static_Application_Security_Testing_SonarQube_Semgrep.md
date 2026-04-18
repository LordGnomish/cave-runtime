# ADR-019: Static Application Security Testing — SonarQube + Semgrep

**Status:** Accepted

**Category:** Security

**Related ADRs:** 010

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE needs static analysis to detect code quality issues, security vulnerabilities (OWASP Top 10), and anti-patterns before code reaches runtime. Two complementary tools provide defense-in-depth.

## Candidates

## | Criteria | SonarQube + Semgrep | SonarQube only | Semgrep only | CodeQL | Checkmarx |
|---|---|---|---|---|---|
| Code quality | ✅ SonarQube (best-in-class) | ✅ | ⚠️ Limited | ⚠️ | ✅ |
| OWASP security rules | ✅ Semgrep (OWASP rulesets) | ⚠️ Basic security rules | ✅ | ✅ | ✅ |
| Custom rules | ✅ Both support custom | ✅ SonarQube rules | ✅ YAML-based rules | ✅ QL | ✅ |
| Language support | ✅ 30+ (SQ) + 30+ (Semgrep) | ✅ 30+ | ✅ 30+ | ⚠️ Fewer | ✅ |
| Self-hosted | ✅ SQ Community, Semgrep OSS | ✅ | ✅ | ⚠️ GitHub-hosted | ❌ SaaS |
| CI integration | ✅ Both have GitHub Actions | ✅ | ✅ | ✅ GitHub native | ✅ |
| License | SQ Community: LGPL. Semgrep: LGPL | LGPL | LGPL | MIT | Proprietary |

## Decision

## **SonarQube Community** (CI stage 3) for code quality + basic security. **Semgrep OSS** (CI stage 4) for OWASP-focused security rules. Complementary: SonarQube catches code quality and maintainability issues; Semgrep catches security-specific patterns (injection, auth bypass, crypto misuse).

## Rejected

## - **SonarQube only:** SonarQube Community has limited security rules compared to Enterprise edition. Semgrep fills the OWASP gap at zero cost.
- **Semgrep only:** Semgrep focuses on pattern matching (security). SonarQube provides deeper code quality analysis (complexity, duplication, maintainability).
- **CodeQL:** GitHub-hosted analysis. Cannot run on self-hosted Gitea. Less flexible for custom rules.
- **Checkmarx:** Proprietary. Expensive. SaaS-focused.

## Consequences

## **Positive:**
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

## Compliance Mapping

## SOC2 CC8.1 (secure development — static analysis). ISO A.8.25-28 (secure development lifecycle). NIS2 Art.21 (secure development practices). OWASP Top 10 coverage.
