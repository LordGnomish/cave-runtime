# Security Policy

Cave Runtime takes security seriously. This document describes how to
report a vulnerability, what we support, how fast we respond, and what
classes of bug are explicitly in scope.

## Supported versions

| Version | Supported | Notes |
|---------|-----------|-------|
| `main` (HEAD) | yes — active development | Pre-1.0; the only supported line until 1.0 |
| `0.x` tagged pre-releases | best-effort | Reported on a case-by-case basis; please test against `main` first |
| Post-1.0 (future) | the current and previous minor line | The Charter v2 8-gate guarantees apply per minor |

## Reporting a vulnerability

**Do not open a public GitHub issue for security reports.**

Preferred channels, in order:

1. **GitHub Security Advisory** (private):
   <https://github.com/LordGnomish/cave-runtime/security/advisories/new>
2. **Email:** **security@cave-runtime.dev**
   (alias forwarded to the maintainer; PGP key fingerprint published at
   `/.well-known/security.txt` once the public site is live).

Please include in the report:

- The affected module/crate and the Cave commit SHA under test.
- A minimal reproducer (single shell snippet, single test, or single
  curl command preferred).
- The upstream project the module is porting, if the vulnerability
  exists upstream as well — this helps us coordinate with upstream
  maintainers.
- Your proposed CVSS 3.1 score and vector string.
- Whether you are open to being publicly credited in the advisory and
  the Hall of Fame.

## Response SLA

| Step | Target |
|------|--------|
| Acknowledge receipt | **3 calendar days** (often within 24 hours) |
| Triage + severity assessment | **5 business days** |
| Fix for High / Critical | **30 days** from triage |
| Fix for Medium / Low | **90 days** from triage |
| Coordinated public disclosure | **90 days** default, shorter if exploitation is observed in the wild |

We will keep you informed throughout. If a deadline slips because of
upstream coordination or a non-trivial root cause, we will communicate
the new target and the reason.

## What is explicitly in scope

Beyond the standard "memory safety / auth bypass / privilege escalation"
categories, the following classes are **always** in-scope security bugs
even if they look like architectural debt to a non-Cave reviewer:

### 1. Multi-tenancy boundary violations

Cave Runtime is multi-tenant by construction (see
[ADR-MULTI-TENANT-001](docs/adr/ADR-MULTI-TENANT-001.md)). **Any cross-tenant
leak** — tenant A can read, write, observe, signal, or otherwise affect
tenant B's resources without an explicit policy grant — is a **critical**
vulnerability regardless of CVSS scoring heuristics.

### 2. Post-quantum-crypto regressions

Cave is on a PQC migration path (see
[ADR-GOLDEN-003](docs/adr/ADR-GOLDEN-003-no-backcompat-pqc.md)). Reports that
identify:

- **Classical-only paths in new code** (RSA / ECDSA / Ed25519 without
  a PQC pair).
- **TLS handshakes that do not offer a hybrid PQC + classical key
  exchange** in PQC-required modes.
- **Signing operations using classical primitives where the message is
  stored beyond 2030** (long-lived audit logs, durable artifacts).

are in-scope. Treat with the same urgency as traditional
vulnerabilities.

### 3. Upstream parity divergence with security consequences

If Cave's reimplementation diverges from the upstream behaviour in a way
that creates a vulnerability (e.g. a missing CSRF check, a missing
admission webhook, a less strict default), this is **in scope** even if
the divergence is "merely" a parity gap. File via the Security Advisory
channel; do not open it as a parity-gap issue.

### 4. Supply-chain / build-time security

- Compromised dependencies surfaced via `cargo audit`,
  `cargo deny check advisories`, or `cargo about`.
- SPDX / NOTICE / `parity.manifest.toml` drift that misrepresents the
  upstream version under test (could lead operators to apply the wrong
  CVE patch).

## What is out of scope

- Theoretical issues with no reproducer.
- Findings from automated scanners without a path-to-impact (please run
  reproducer locally before submitting).
- Denial of service via plain resource exhaustion on a single
  operator-controlled node (use `cave-runtime` rate limits and tenant
  quotas; these are intended controls, not vulnerabilities).
- Social-engineering attacks against maintainers.

## Safe-harbour for good-faith research

If you discover an issue while using Cave Runtime, please report before
public discussion. We will not pursue legal action against good-faith
researchers following this policy. Specifically, you may:

- Test against your own deployments or local checkouts.
- Avoid privacy violations, service disruption, or destruction of data.
- Give us reasonable time to investigate and fix.

We will not initiate or support legal action against you for activity
that complies with this policy.

## Hall of Fame

Researchers who have responsibly disclosed vulnerabilities to Cave
Runtime are credited here once the advisory is public.

| Date | Researcher | Advisory | Severity |
|------|-----------|----------|----------|
| _none yet — be the first_ | | | |

Anonymous credit is offered on request. Pseudonymous credit is fine; we
will only require an identity for legally mandated CVE coordination
where applicable.

## Disclosure history

- 2026-05-19: initial Security Policy published as part of the OSS-launch
  hardening sweep.
