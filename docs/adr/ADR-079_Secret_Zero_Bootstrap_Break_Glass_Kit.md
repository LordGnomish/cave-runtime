# ADR-079: Secret Zero Bootstrap + Break-Glass Kit

**Status:** Accepted

**Category:** Security

**Related ADRs:** 020, 088

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Bootstrapping CAVE requires initial credentials (cloud provider API token, DNS token, vault unseal keys). These bootstrap credentials must not persist. Additionally, a total cluster destruction scenario requires offline recovery materials.

## Candidates

## | Approach | Local .env + Break-glass Kit | Cloud KMS only | Single master key | Pre-shared secrets |
|---|---|---|---|---|
| Bootstrap | ✅ Local .env, rotated immediately | ❌ Requires cloud access first | ❌ SPOF | ❌ Key distribution problem |
| Resurrection | ✅ Offline Kit (Shamir + WORM) | ❌ Circular dependency (need cloud to access cloud KMS) | ❌ Single compromise = total loss | ❌ Shared secrets = shared risk |
| HSM fast-path | ✅ Remote Shamir (<90min) | N/A | N/A | N/A |
| Pre-staged envelope | ✅ 2-of-5 triage access (<30min) | N/A | N/A | N/A |

## Decision

## Day 0: local `.env` with initial credentials → immediately rotated into OpenBao/Key Vault → `.env` destroyed, never committed to Git. Offline Break-glass Kit: Shamir 3-of-5 split of vault unseal keys, Cloudflare root token, cloud SP credentials, WORM access keys, static etcd decryption key. Primary: physical safe. HSM fast-path: 2 remote HSM + 1 physical = <90min reassembly. Pre-staged envelope: 2-of-5 triage-only access (<30min, weaker security).

## Rejected

## - **Cloud KMS only:** During total cloud loss, KMS is inaccessible. Circular dependency: need cloud to access KMS, need KMS to access cloud credentials. Kit must be independently accessible.
- **Single master key:** Single point of failure. One key compromise = total platform compromise. Shamir splitting distributes trust.
- **No offline backup:** Total cloud destruction = permanent loss. Unacceptable for platform claiming <4h RTO.

## Consequences

## **Positive:**
- No persistent bootstrap credentials after Day 0.
- Platform recoverable from total destruction via offline Kit.
- HSM fast-path enables remote Shamir reassembly — geographic distribution doesn't block RTO.
- Pre-staged envelope provides rapid triage access during disasters.

**Negative:**
- Key holder coordination required (minimum 3 of 5 for full recovery).
- Physical safe dependency (location, access control, auditing).
- HSM hardware distribution and verification overhead (quarterly validation during resurrection drills).
- Pre-staged envelope provides weaker security (2-of-5 vs 3-of-5) — documented trade-off.

## Compliance Mapping

## SOC2 CC6.1 (access controls for critical credentials). ISO A.8.24 (cryptographic key management). ISO A.5.29 (information security during disruption). NIS2 Art.21 (incident response, business continuity).
