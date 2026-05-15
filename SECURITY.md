# Security Policy

## Supported versions

Pre-v1. Until the 1.0 release, only `main` is supported. Post-1.0, the current and previous minor line are supported.

## Reporting a vulnerability

**Do not open a public GitHub issue for security reports.**

Email: security@cave-runtime.dev (GPG key fingerprint published at `/.well-known/security.txt` once the public site is live).

Include in the report:

- The affected module and commit SHA.
- A minimal reproducer.
- The upstream project the module is porting, if the vulnerability exists upstream as well.
- Your proposed CVSS score and vector string.
- Whether you are open to being publicly credited.

We will:

- Acknowledge within 48 hours.
- Triage and provide a severity assessment within 5 business days.
- Target a fix within 30 days for high/critical, 90 days for medium/low.
- Coordinate disclosure: a 90-day default window, shorter if exploitation is observed in the wild.

## Post-quantum cryptography

Cave Runtime is on a post-quantum-crypto migration path (see [ADR-GOLDEN-003](docs/adr/ADR-GOLDEN-003-no-backcompat-pqc.md)). Reports that identify:

- Classical-only paths in new code (RSA/ECDSA/Ed25519 without a PQC pair),
- TLS handshakes that do not offer a hybrid PQC + classical key exchange,
- Signing operations using classical primitives where the message is stored beyond 2030,

are in-scope security bugs. Treat with the same urgency as traditional vulnerabilities.

## Multi-tenancy boundary bugs

Cave Runtime is multi-tenant by construction (see [ADR-MULTI-TENANT-001](docs/adr/ADR-MULTI-TENANT-001.md)). Any cross-tenant leak (tenant A can read/write/observe/affect tenant B's resources without explicit policy) is a critical vulnerability regardless of CVSS scoring heuristics.

## Responsible disclosure

If you discover an issue while using Cave Runtime in production, please report before public discussion. We will not pursue legal action against good-faith researchers following this policy.
