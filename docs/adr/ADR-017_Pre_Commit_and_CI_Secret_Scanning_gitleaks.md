# ADR-017: Pre-Commit and CI Secret Scanning — gitleaks

**Status:** Accepted

**Scope:** Universal

**Category:** Security

**Related ADRs:** 010, 079

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

Accidental secret commits (API keys, passwords, tokens, certificates) to Git repositories are a persistent security risk across all software supply chains. Secrets must be detected before they reach the remote repository (pre-commit defense) and again in CI (defense-in-depth principle). CAVE's distributed architecture requires consistent secret detection across:

- Developer pre-commit hooks (catch secrets before push)
- CI pipeline stage 2 (enforce at repository boundary)
- Both Hetzner and Azure profiles (identical behavior)
- Support for CAVE-specific credential formats (OpenBao tokens, Cloudflare API keys, Hetzner credentials)

The cost of a leaked credential is high: unauthorized API access, privilege escalation, data exfiltration, supply chain compromise. Detection must be fast enough to not degrade developer experience and accurate enough to minimize false positives (which erode developer trust in the tool).

## Candidates

| Criteria | gitleaks | TruffleHog | detect-secrets (Yelp) | GitGuardian |
|---|---|---|---|---|
| Pre-commit hook | ✅ Native | ✅ | ✅ | ✅ |
| CI integration | ✅ GitHub Action | ✅ | ⚠️ Manual | ✅ |
| Custom rules | ✅ TOML config | ✅ | ✅ | ✅ |
| False positive rate | Low (pattern + entropy) | Medium (entropy-heavy) | Low | Low |
| Performance | ✅ Fast (Go binary) | ⚠️ Slower (Python) | ⚠️ Slower | ✅ (SaaS) |
| License | MIT | AGPL-3.0 | Apache 2.0 | Proprietary (SaaS) |
| Self-hosted | ✅ | ✅ | ✅ | ❌ SaaS |
| Language | Go (single binary) | Python (requires runtime) | Python (requires runtime) | Cloud service |
| Community | Very active (Zack Rice, CNCF context) | Active (Dino Prugh, GitHub Inc) | Maintained (Yelp) | Proprietary |
| Git history scanning | ✅ `gitleaks detect --source=git` | ✅ Full history | ⚠️ Historical support limited | ✅ |
| Serverless/CI friendly | ✅ Single binary | ⚠️ Python runtime overhead | ❌ Runtime + lib dependencies | ✅ API-based |

## Decision

**gitleaks** (MIT license) for both pre-commit hook (developer machines) and CI pipeline stage 2 (GitHub Actions). TOML configuration with custom patterns for CAVE-specific credentials: OpenBao AppRole IDs, Cloudflare API tokens, Hetzner API keys, Azure service principal secrets. Findings auto-reported to DefectDojo (ADR-035) as CRITICAL severity. Secret rotation workflow (ADR-083) triggered immediately via cave-ctl for any detected leaked credential.

## Implementation Reference

**Implementation Status:** Production

- **cave-secrets** crate: Integration with `cave-ctl secret-rotate` workflow
- **Location:** CI stage 2 (gitleaks GitHub Action), pre-commit hook (managed via cave-ctl init)
- **TOML config:** Stored in `.gitleaks.toml` at repository root. CAVE-specific patterns versioned in cave-runtime config.
- **DefectDojo integration:** gitleaks JSON output → DefectDojo API (ADR-035) with automated finding lifecycle

## Rejected Options

### TruffleHog — Not Recommended

**Reasons:**
1. **Licensing:** AGPL-3.0 is copy-left — requires derivative works (CAVE + TruffleHog integration) to be open source. CAVE's proprietary platform components would be affected. MIT (gitleaks) avoids this constraint.
2. **Entropy detection:** TruffleHog's primary method is entropy analysis — high false-positive rate for random strings in configuration, test vectors, or mock credentials. Requires manual allow-listing. gitleaks combines pattern matching (lower false positives) + entropy (catches variants).
3. **Performance:** Python-based. Pre-commit hook on developer machines adds noticeable latency (~3-5s for TruffleHog vs ~1s for gitleaks on typical repo).
4. **Community maturity:** Active but less widely deployed in large enterprises compared to gitleaks.

### detect-secrets (Yelp) — Not Recommended

**Reasons:**
1. **Limited GitHub Actions support:** No native GitHub Action. Would require custom wrapper script or deprecated action. gitleaks has official GitHub Actions marketplace entry with 100K+ weekly downloads.
2. **Smaller community:** Maintained by Yelp but less adoption signal than gitleaks (which has 2K+ GitHub stars and backing from Cloud Native Computing Foundation context).
3. **Pre-commit hook maturity:** While detect-secrets has pre-commit support, gitleaks' hook experience is more polished (faster, fewer configuration gotchas).

### GitGuardian — Not Recommended

**Reasons:**
1. **SaaS-only model:** Code/commits transmitted to GitGuardian's cloud infrastructure. Violates CAVE's sovereign deployment requirement — some regulated customers cannot send code to external services.
2. **Vendor lock-in:** Proprietary service. No self-hosting option. Dependency on external vendor uptime.
3. **Cost:** SaaS pricing scales with repository and secret volume. OSS approach (gitleaks) eliminates licensing costs across all CAVE deployments.

## Consequences

### Positive

- **Two-layer detection:** Secrets caught at pre-commit (developer machine, immediate feedback) and at CI (repository boundary enforcement). Developer can fix before pushing; CI enforces no secret commits reach remote.
- **Fast execution:** gitleaks is a single Go binary (~20MB). Pre-commit hook executes in <1s on typical repos. CI stage 2 runs in <30s. No language runtime overhead.
- **MIT license:** Permissive. No restrictions on CAVE's commercial distribution or proprietary components.
- **CAVE-specific patterns:** TOML config supports regex patterns for OpenBao tokens, Cloudflare API keys, Azure service principal secrets, Hetzner credentials. Easy to add new patterns as CAVE evolves.
- **DefectDojo integration:** Gitleaks JSON output plugs directly into DefectDojo API. Finding lifecycle (open → triage → fix → verified-fix) provides audit trail for compliance.
- **Community trust:** Widely deployed (Kubernetes, HashiCorp, major tech companies). Continuous pattern updates.

### Negative

- **Pre-commit bypass:** Developers can override hook with `git commit --no-verify` flag. Not suitable as sole defense (theater). CI stage 2 is enforcement.
- **False negative risk:** Non-standard secret formats may not match patterns. Example: custom CAVE token format without pattern definition. Mitigation: TOML patterns reviewed on each new credential type introduced.
- **False positives:** Random hex strings in comments, test vectors, or configuration can trigger entropy-based rules. Mitigation: `.gitleaksignore` file allows exemptions with justification (ADR-057 audit requirement).
- **History scanning:** Full git history scan (`--source=git`) slower than just scanning staged changes. CI runs staged-only by default. Historical scan is optional, off-cycle maintenance task.
- **Maintenance burden:** TOML pattern maintenance required. New credential formats introduced in CAVE require pattern updates before deployment (documentation in cave-ctl runbook).

### Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Secret leaked despite gitleaks | Low | High | Secret rotation workflow (ADR-083) triggered immediately. Sovereign Ledger audit trail. |
| False positives frustrate developers | Medium | Low | Allow-listing via `.gitleaksignore` with justification. Quarterly pattern review. |
| gitleaks regex DoS on large commit | Very Low | Low | File size limits in CI. Timeout on gitleaks stage 2 (30s SLA). |
| New secret format not detected | Medium | High | CAVE requires credential type PR to include gitleaks pattern update. Code review enforces. |

## License

**gitleaks:** MIT License (https://github.com/gitleaks/gitleaks/blob/master/LICENSE)

## Compliance Mapping

**SOC2 CC6.7:** Credential lifecycle management — prevent secrets from reaching repositories.
**ISO/IEC 27001 A.8.4:** Access control to source code — secrets stored in code violates data access controls.
**NIS2 Directive Article 21:** Secure development practice — secret detection before supply chain exposure.
**GDPR Article 32:** Security of processing — prevents unauthorized access to API keys that could compromise customer data.
