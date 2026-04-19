# ADR-083: Automated Secret Rotation

**Status:** Accepted

**Scope:** Azure, Universal

**Category:** Security

**Related ADRs:** 020, 053

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Static credentials are a compliance liability and security risk. CAVE needs automated rotation for all credential types: database, API keys, TLS certificates, service accounts.

## Candidates

## | Credential Type | Rotation Method | Cycle | Mechanism |
|---|---|---|---|
| DB credentials (dynamic) | OpenBao/Key Vault lease | Per-session (1h default, 24h max) | ESO sync → K8s Secret → app reconnect |
| DB credentials (static) | OpenBao rotation plugin | 90 days | ESO detects change, syncs |
| TLS certificates | cert-manager ACME | 30 days | Auto-renewal via Let's Encrypt DNS-01 |
| API keys (external) | OpenBao/Key Vault rotation | 90 days | ESO sync |
| Service account tokens | K8s TokenRequest API | 1h (projected volume) | K8s auto-refresh |

## Decision

## Dynamic DB secrets via OpenBao/Key Vault (lease-based, per-session). Static secrets: 90-day rotation. TLS: 30-day rotation via cert-manager. ESO syncs within minutes. Zero-downtime rotation via connection pool drain + reconnect.

## Rejected

## - **Manual rotation:** Human error, missed deadlines, compliance gaps. SOC2 auditors flag manual rotation.
- **Annual rotation:** 365-day credential lifetime is too long for SOC2 CC6.7 compliance.
- **No rotation:** Unacceptable. Static credentials eventually leak or are compromised.

## Consequences

## **Positive:**
- No manual rotation required for any credential type.
- Minimal credential exposure window (per-session for dynamic, 90d for static).
- Compliance-ready: SOC2 CC6.7, ISO A.8.24 rotation requirements met automatically.

**Negative:**
- ESO sync latency: brief window during rotation where old credential is in K8s Secret.
- Application must handle credential refresh (connection pool drain + reconnect pattern).
- Lease expiry can cause brief connection failures if ESO sync is delayed (mitigated: ESO health monitoring, P2 alert).

## Compliance Mapping

## SOC2 CC6.7 (changes to credentials — automated rotation). ISO A.8.24 (key management lifecycle). GDPR Art.32 (security of processing).
