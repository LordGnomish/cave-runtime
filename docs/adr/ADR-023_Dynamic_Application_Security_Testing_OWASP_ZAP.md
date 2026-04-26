# ADR-023: Dynamic Application Security Testing — OWASP ZAP

**Status:** Accepted

**Scope:** Universal

**Category:** Security/CI

**Related ADRs:** 010, 019, 035

## Context

Static analysis (ADR-019: SonarQube + Semgrep) catches code-level vulnerabilities before deployment. DAST (Dynamic Application Security Testing) complements by testing running applications for runtime vulnerabilities that only manifest during execution:

- XSS (Cross-Site Scripting) — client-side code injection
- CSRF (Cross-Site Request Forgery) — unauthorized state-changing requests
- Injection attacks (SQL, command, XML, NoSQL)
- Authentication bypass / session handling flaws
- Misconfigured security headers
- API endpoint vulnerabilities

DAST occurs in CI stages 23-24 (post-deployment to ephemeral staging vcluster). Findings inform deployment gate to production. CAVE's tenant isolation also tested via DAST (cross-tenant access attempts blocked).

## Candidates

| Criteria | OWASP ZAP | Burp Suite | Nikto | Nuclei |
|---|---|---|---|---|
| Automated CI scan | ✅ Baseline + full scan modes | ⚠️ Enterprise only (Pro SaaS) | ✅ | ✅ |
| API scanning | ✅ OpenAPI/Swagger import | ✅ (Enterprise) | ❌ | ⚠️ Template-based |
| Authentication support | ✅ Form, JWT, OAuth2, MTLS | ✅ | ❌ | ⚠️ Basic auth |
| CI integration | ✅ Docker, GitHub Action, CLI, helm | ⚠️ Docker plugin only (Pro) | ✅ CLI | ✅ CLI, Docker |
| Scan performance | ⚠️ 5-15 min (baseline), 1-2h (full) | ✅ Faster | ✅ Fast (~5 min) | ✅ Fast (known CVEs) |
| Self-hosted | ✅ Full OSS | ⚠️ SaaS only (Pro) | ✅ | ✅ |
| License | Apache 2.0 | Proprietary (Community ~free, Pro commercial) | GPL | MIT |
| OWASP Top 10 coverage | ✅ Comprehensive (all A1-A10) | ✅ | ⚠️ Limited | ⚠️ Known patterns |
| Community | OWASP Project, large | Large (Portswigger) | Unmaintained (~20y old) | Emerging (ProjectDiscovery) |

## Decision

**OWASP ZAP** (Apache 2.0) for CI pipeline stages 23-24 (ADR-010):
- **Stage 23:** Baseline scan on ephemeral staging vcluster (all tenant workloads + CAVE platform services)
- **Stage 24:** Integration tests + ZAP baseline in parallel. BLOCK gate on HIGH severity findings.
- **OpenAPI import:** APIs publish OpenAPI 3.0 specs (ADR-057 requirement). ZAP imports for targeted endpoint testing.
- **Authentication:** JWT token injection for authenticated tenant API testing. OAuth2 flow testing for SSO paths.
- **Output:** SARIF + JSON → DefectDojo API (ADR-035) for finding lifecycle.
- **SLA:** 5 min baseline (stages 23-24 budget 10 min for both scan + integration tests).

## Implementation Reference

**Implementation Status:** Production

- **cave-dast** crate: ZAP orchestration, OpenAPI spec discovery, result parsing + DefectDojo push
- **ZAP container:** Deployed as sidecar in staging vcluster during stage 23. Input: tenant workload URLs (discovered via Kubernetes Service endpoints).
- **OpenAPI specs:** Collected from workload swagger.json endpoints (convention: `:8080/swagger.json` or `:3000/openapi.json`). Specs versioned in cave-portal crate.
- **Artifact:** SARIF report uploaded to DefectDojo + Sovereign Ledger attestation.

## Rejected Options

### Burp Suite — Not Recommended

**Reasons:**
1. **Licensing model:** Community edition has limited features. Professional edition requires SaaS subscription ($4K+/year). Enterprise deployments expensive.
2. **CI integration:** Community edition not designed for automation. Pro SaaS edition requires external API calls during pipeline.
3. **Sovereign requirement:** Pro SaaS version sends scan data to Portswigger cloud. Violates sovereign deployment requirement for regulated customers.
4. **Scanning speed:** Burp pro is faster but comes at licensing cost. ZAP baseline covers 95% of use cases at zero cost.

### Nikto — Not Recommended

**Reasons:**
1. **Limited scope:** Web server vulnerability scanner only. Designed for traditional web apps, not modern APIs.
2. **No API support:** No OpenAPI import. No JWT authentication. No GraphQL scanning. CAVE's APIs require these capabilities.
3. **Outdated:** Last major update ~20 years ago. Not actively maintained for modern attack patterns (OAuth2 bypass, JWT weaknesses, etc.).
4. **No CI integration:** CLI only. Manual result parsing required.

### Nuclei — Not Appropriate

**Reasons:**
1. **Different purpose:** Template-based scanner for known CVEs and specific vulnerability patterns. Excellent for detecting Log4j, Spring4Shell, etc. Not a general-purpose DAST tool.
2. **False sense of security:** Nuclei reports "no findings" if template doesn't match pattern. Custom/zero-day application vulnerabilities missed.
3. **Complementary, not primary:** Nuclei works well as secondary tool (post-DAST) for known CVE scanning. Not replacement for ZAP's comprehensive crawl + test methodology.

## Consequences

### Positive

- **Runtime vulnerability detection:** Complements SAST. Catches logic flaws, authentication failures, injection vulnerabilities only visible during execution.
- **Defense-in-depth:** SAST (static) + DAST (dynamic) + SBOM (dependencies) = three independent security perspectives.
- **OpenAPI-driven:** Automated discovery of API endpoints + test case generation from specs. Reduces manual testing burden.
- **Tenant isolation validation:** DAST tests cross-tenant access attempts (malicious tenant accessing another tenant's data). Verifies network policy enforcement.
- **Apache 2.0:** No licensing cost. Fully self-hosted. No external dependencies during scans.
- **OWASP Top 10:** Comprehensive coverage of A1-A10 vulnerabilities in running application context.

### Negative

- **Scan time:** Baseline scan 5 min, full scan 1-2h. Adds to CI pipeline duration. Mitigated: baseline only in stages 23-24 (parallel with integration tests).
- **False positives:** DAST generates noise. Example: reflected XSS in error message that's properly escaped in production. Triage required.
- **Requires running app:** Staging deployment must complete before DAST begins. Dependency chain: build → deploy vcluster → DAST.
- **Authentication complexity:** Testing authenticated endpoints requires token injection + session management. Custom scripts per tenant auth model.
- **Blind spots:** ZAP can't test features not exposed in baseline crawl. Requires API specification for full coverage.

### Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| DAST finds zero-day after deployment | Low | High | Canary deployment (Argo Rollouts). Production traffic 5% initially. Production monitoring catches issues. |
| False positive triggers false-negative (team ignores warnings) | Medium | High | Establish triage baseline. P3 alert for every new finding. Runbook for FP classification. |
| ZAP scan timeout on large applications | Low | Medium | Baseline-only in CI. Full scans off-cycle. URL scope limiting in ZAP config. |
| API spec out-of-sync with implementation | Medium | Medium | API spec as source of truth (code generation). CI validates spec matches OpenAPI validation stage 7. |

## License

**OWASP ZAP:** Apache 2.0 License (https://github.com/zaproxy/zaproxy/blob/main/LICENSE)

## Compliance Mapping

**SOC2 CC7.1:** Vulnerability detection — runtime application testing before production release.
**ISO/IEC 27001 A.8.28:** Secure coding — dynamic testing for logic flaws not detectable by static analysis.
**NIS2 Directive Article 21:** Vulnerability management — runtime vulnerability detection and remediation.
**OWASP Top 10:** Full coverage of A1-A10 vulnerabilities in running application context.
