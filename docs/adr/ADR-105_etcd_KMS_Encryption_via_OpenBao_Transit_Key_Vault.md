# ADR-105: etcd KMS Encryption via OpenBao Transit / Key Vault

**Status:** Accepted

**Scope:** Hyperscaler, Sovereign, Universal

**Category:** Security

**Related ADRs:** 020, 079

## Context

K8s Secrets are stored in etcd. Without encryption at rest, etcd dump exposes all secrets in plaintext.

## Candidates

| Approach | OpenBao Transit (Hz) + Key Vault KMS (Az) | Static key encryption | No encryption | AWS KMS |
|---|---|---|---|---|
| Key rotation | ✅ Automatic (OpenBao Transit / Key Vault auto-rotate) | ❌ Manual | N/A | ❌ AWS only |
| Envelope encryption | ✅ DEK encrypted by KEK | ❌ Single key | N/A | ✅ |
| Offline recovery | ✅ Static fallback key in Break-glass Kit | ❌ Key lost = data lost | N/A | ❌ |
| Provider portability | ✅ OpenBao (Hz) + Key Vault (Az) | ⚠️ Vendor-specific | N/A | ❌ |

## Decision

etcd encryption at rest mandatory on all profiles. Talos/Hetzner: OpenBao Transit engine (envelope encryption — KEK encrypts DEK, DEK encrypts etcd data). AKS/Azure: Key Vault KMS provider. Break-glass Kit includes static etcd decryption key for resurrection when OpenBao is unavailable (ADR-079).

## Rejected

- **Unencrypted etcd:** K8s Secrets visible in etcd dumps. Any etcd backup or snapshot contains plaintext secrets. Compliance failure.
- **Static key encryption only:** No automatic rotation. Manual key management. Key compromise requires re-encryption of all etcd data.
- **AWS KMS:** Not available on the sovereign profile. Cloud-specific lock-in.

## Consequences

**Positive:**
- All K8s Secrets encrypted at rest in etcd.
- Automatic key rotation via OpenBao/Key Vault.
- Envelope encryption: even if etcd data is exfiltrated, it's encrypted with a DEK that's encrypted by KEK.
- Static fallback key enables resurrection without running OpenBao.

**Negative:**
- OpenBao Transit dependency for Hetzner etcd encryption. OpenBao failure → new secret writes fail (existing encrypted data still readable).
- Static fallback key in Break-glass Kit is a security trade-off (documented, Shamir-split).
- Key rotation during OpenBao maintenance requires coordination.

## Compliance Mapping

SOC2 CC6.7 (encryption of stored credentials). ISO A.8.24 (cryptographic controls). GDPR Art.32 (security of processing — encryption at rest).
