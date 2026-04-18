# ADR-017: Pre-Commit and CI Secret Scanning — gitleaks

**Status:** Accepted

**Category:** Security

**Related ADRs:** 010, 079

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Accidental secret commits (API keys, passwords, tokens) to Git repositories are a persistent security risk. Secrets must be detected before they reach the remote repository (pre-commit) and again in CI (defense-in-depth).

## Candidates

## | Criteria | gitleaks | TruffleHog | detect-secrets (Yelp) | GitGuardian |
|---|---|---|---|---|
| Pre-commit hook | ✅ Native | ✅ | ✅ | ✅ |
| CI integration | ✅ GitHub Action | ✅ | ⚠️ Manual | ✅ |
| Custom rules | ✅ TOML config | ✅ | ✅ | ✅ |
| False positive rate | Low (pattern + entropy) | Medium (entropy-heavy) | Low | Low |
| Performance | ✅ Fast (Go binary) | ⚠️ Slower (Python) | ⚠️ Slower | ✅ (SaaS) |
| License | MIT | AGPL-3.0 | Apache 2.0 | Proprietary (SaaS) |
| Self-hosted | ✅ | ✅ | ✅ | ❌ SaaS |

## Decision

## **gitleaks** for both pre-commit hook and CI stage 2. TOML config with custom patterns for CAVE-specific secrets (OpenBao tokens, Cloudflare tokens, Hetzner API keys). Findings auto-reported to DefectDojo as CRITICAL severity. Secret rotation triggered immediately via cave-ctl for any leaked credential.

## Rejected

## - **TruffleHog:** AGPL-3.0 license is more restrictive than MIT. Higher false-positive rate due to entropy-based detection. Slower (Python).
- **detect-secrets (Yelp):** Good baseline detector but less active community. No native GitHub Action.
- **GitGuardian:** SaaS-only for full features. Code sent to external service — contradicts sovereign profile.

## Consequences

## **Positive:**
- Secrets caught at pre-commit (before push) and at CI (defense-in-depth).
- Fast Go binary — minimal impact on developer workflow and CI time.
- MIT license — no restrictions.
- Custom patterns for CAVE-specific credential formats.
- DefectDojo integration provides finding lifecycle management.

**Negative:**
- Pre-commit hook can be bypassed (git commit --no-verify). CI stage 2 is the enforcement backstop.
- False negatives possible for non-standard secret formats. Custom TOML patterns required.
- gitleaks scans file content, not git history by default in CI. History scan optional but slower.

## Compliance Mapping

## SOC2 CC6.7 (credential lifecycle — prevent exposure). ISO A.8.4 (access to source code — no secrets in code). NIS2 Art.21 (secure development). GDPR Art.32 (security of processing).
