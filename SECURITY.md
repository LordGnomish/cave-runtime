# Security Policy

## Reporting a vulnerability

**Do not open a public GitHub issue for security reports.** Use **[GitHub Security Advisories](https://github.com/cave-runtime/cave-runtime/security/advisories/new)** — the link opens a private form that only repository security maintainers can see. GHSA also gives you a CVE-tracking flow once the report is triaged.

If GHSA is unavailable to you (anonymous report, account locked, etc.), email **security@cave-runtime.org**. PGP-encrypted reports are preferred; the maintainer key fingerprint will be published at `https://cave-runtime.org/.well-known/security.txt` once the public site is live. Until then, request the current key in an unencrypted message and we will reply with it before any sensitive details are exchanged.

Include in your report:

- The affected crate and the Cave Runtime commit SHA you reproduced on.
- A minimal reproducer (preferably a failing `cargo test` row).
- The upstream project the module ports, if the same vulnerability exists upstream.
- Your proposed CVSS v3.1 score and vector string.
- Whether you are open to public credit on disclosure.

We will:

- Acknowledge your report within **48 hours**.
- Provide a triage and severity assessment within **5 business days**.
- Target a fix within **30 days** for high / critical issues, **90 days** for medium / low.
- Coordinate disclosure on a default **90-day window** from initial report — shorter if exploitation is observed in the wild, longer only by mutual agreement with the reporter.

## Supported versions

| Version line | Supported |
|--------------|-----------|
| `0.1.x` (initial public preview) | Yes — security fixes against the latest tagged release on the `0.1.x` line only |
| Older `0.1.x` tags | No — please upgrade |

Pre-1.0, only the latest tagged `0.1.x` release receives security updates. Once `1.0` ships, the policy expands to the current and previous minor lines.

## Security boundaries (current state, pre-1.0)

Cave Runtime makes specific security promises by construction. Knowing what is and is not in scope today helps you decide whether a finding is a security bug or a feature gap.

### In scope as security bugs

- **Cross-tenant leaks.** Cave Runtime is multi-tenant by construction — every persisted resource owns exactly one tenant, default-deny across tenants. Any path that lets tenant A read, write, observe, or affect tenant B's resources without an explicit policy grant is a critical security bug regardless of CVSS scoring.
- **Sovereign-isolation breaks.** Code paths that exfiltrate tenant data to a non-sovereign endpoint, leak tenant identifiers across boundaries, or bypass the "Linux 7.1+ only, no third-party SaaS dependency at the core" charter constraint.
- **Authentication / authorization bypass.** Anywhere `tenant_id` is omitted, can be spoofed, or is enforced only at the edge while internal callers can supply their own.
- **Unsafe / panicking code on attacker-controlled input.** A `panic!`, integer overflow, or `unsafe` UB reachable from the wire protocols (Postgres, MongoDB, etcd v3, gRPC, kube-apiserver, Istio Ambient, …) counts as a denial-of-service security bug.
- **Supply-chain compromises.** Tampered upstream pins, missing SBOM rows, missing signing on release artefacts, or any path that lets a third-party dependency reach `cargo build` without explicit human review.
- **Cryptographic regressions** — see the next section.

### Cryptographic baseline and PQC roadmap

Cave Runtime targets post-quantum-crypto readiness from the primitives layer up (charter binding). The current state is a hybrid baseline:

- **Today (v0.1.x baseline).** New code uses **Ed25519** for signatures and **X25519** for key exchange where the wire protocol forces a classical primitive. ML-DSA / ML-KEM / SLH-DSA paths are present in `cave-pki` and `cave-acme`, gated behind feature flags, and exercised by the certs / mesh / ledger crates. ADR-RUNTIME-CERT-LIFECYCLE-001 describes the per-tenant CA hierarchy and the hybrid cert chain plan.
- **Roadmap (v0.2 → v1.0).** Hybrid PQC + classical for every TLS handshake (X25519 + ML-KEM-768), hybrid signatures (Ed25519 + ML-DSA-65) for all internal endpoints, classical-only paths kept only on the public-facing opt-in Let's Encrypt path until browser PQC adoption catches up.
- **In scope as security bugs.** Any new code that introduces a classical-only crypto path without a hybrid pair, any TLS handshake that does not offer the hybrid key exchange where supported, any signing operation using classical primitives where the message is stored beyond 2030, and any logic that lets a peer negotiate down to classical-only on a connection that should have negotiated hybrid.

### Out of scope (today)

These are real concerns but currently tracked as roadmap items rather than security bugs:

- Hardware-key (HSM / TPM) root-of-trust for the per-tenant CA — phase 0.2 (see ADR-RUNTIME-CERT-LIFECYCLE-001).
- CRL / OCSP responder for revocation — phase 0.3.
- Side-channel resistance audits of the bundled PQC implementations — pending upstream PQC library maturity.
- Kernel-level mitigations (Tetragon behavioural baselines, runtime forensics WORM) — phase 0.2 deliverables, currently scaffold-only.

Findings in these areas are welcomed as feature requests through the public issue tracker.

## Charter reference

Cave Runtime's security promises are derived from the project charter (sovereign by default, multi-tenant by construction, post-quantum-crypto ready, no backward-compatibility port, line-by-line upstream parity). The full charter and the supporting ADRs (multi-tenancy invariant, PQC binding, zero-trust networking, cert lifecycle) are catalogued under `docs/adr/` — start with the index in [`docs/adr/README.md`](docs/adr/README.md) if you need the architectural context behind a finding.

## Responsible disclosure

If you discover a vulnerability while running Cave Runtime in production, please report before public discussion. We will not pursue legal action against good-faith researchers who follow this policy.
