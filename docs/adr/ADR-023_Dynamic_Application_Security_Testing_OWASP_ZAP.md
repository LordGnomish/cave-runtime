# ADR-023: Dynamic Application Security Testing — OWASP ZAP

**Status:** Accepted

**Category:** Security/CI

**Related ADRs:** 010

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Static analysis (ADR-019) catches code-level vulnerabilities. DAST complements by testing running applications for runtime vulnerabilities (XSS, CSRF, injection, auth bypass) that only manifest when the application is deployed.

## Candidates

## | Criteria | OWASP ZAP | Burp Suite | Nikto | Nuclei |
|---|---|---|---|---|
| Automated CI scan | ✅ Baseline + full scan modes | ⚠️ Enterprise only | ✅ | ✅ |
| API scanning | ✅ OpenAPI import | ✅ | ❌ | ⚠️ Templates |
| Authentication support | ✅ Form, JWT, OAuth2 | ✅ | ❌ | ⚠️ |
| CI integration | ✅ Docker-based, GitHub Action | ⚠️ | ✅ CLI | ✅ CLI |
| Self-hosted | ✅ | ⚠️ License server | ✅ | ✅ |
| License | Apache 2.0 | Proprietary (commercial) | GPL | MIT |

## Decision

## **OWASP ZAP** for CI stages 23-24. Baseline scan in dev/staging, full scan before prod promotion. OpenAPI spec import for targeted API scanning. Findings exported to DefectDojo. BLOCK gate on high severity findings.

## Rejected

## - **Burp Suite:** Proprietary. Enterprise license expensive. Not designed for automated CI pipeline integration.
- **Nikto:** Web server scanner only. No API scanning. No OpenAPI import. No authentication support.
- **Nuclei:** Template-based vulnerability scanner. Good for known CVE detection but less comprehensive for custom application testing than ZAP.

## Consequences

## **Positive:**
- Runtime vulnerability detection complementing SAST (defense-in-depth).
- OpenAPI import enables targeted API-specific testing.
- Apache 2.0 — no licensing cost.
- DefectDojo integration for unified finding management.

**Negative:**
- DAST scans take 5-15 minutes (adds to CI pipeline time).
- False positives require triage.
- ZAP requires a running application (staging/vcluster deployment must complete before DAST).

## Compliance Mapping

## SOC2 CC7.1 (vulnerability detection — runtime testing). ISO A.8.28 (secure coding — dynamic testing). NIS2 Art.21 (vulnerability management). OWASP Top 10 runtime validation.
