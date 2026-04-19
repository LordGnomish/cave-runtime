# ADR-020: OpenBao for Secrets Management

**Status:** Accepted

**Scope:** Universal

**Category:** Security / Secrets Management

**Related ADRs:** 053, 083, 105, 115

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

CAVE manages multiple classes of secrets across all profiles and tenants:

- **Static secrets:** Database credentials, API keys, service account passwords
- **Short-lived tokens:** OIDC tokens (ADR-115), Kubernetes SA tokens
- **Encryption keys:** etcd encryption keys (ADR-105), KMS transit keys, WORM log signing keys
- **Certificates:** TLS certificates for platform services, tenant mutual TLS (ADR-015)
- **Rotation policies:** Automated secret rotation based on TTL or on-demand (ADR-083)

Secrets must be encrypted at rest, have fine-grained RBAC (tenant A cannot read tenant B secrets), and support audit logging for compliance.

## Candidates

| Criteria | OpenBao | Vault (HashiCorp) | External Secrets Operator (K8s-native) | AWS Secrets Manager |
|---|---|---|---|---|
| **Self-hosted** | ✅ Full self-hosting (Hetzner/Azure) | ✅ Self-hosting + enterprise SaaS | ✅ External secret integration | ❌ AWS-only |
| **License** | Apache 2.0 (community fork post-BSL) | BSL → MPL 2.0 (commercial restrictions) | Apache 2.0 | Proprietary |
| **Multi-tenancy** | ✅ Namespace-isolated secret engines | ⚠️ Auth method per org, complex RBAC | ✅ K8s namespace-scoped | ❌ AWS account-level |
| **Secret types** | ✅ Key-value, PKI, transit encryption, SSH | ✅ All (standard in industry) | ⚠️ Integration-focused, limited generation | ✅ |
| **KMS integration** | ✅ Transit engine for key management | ✅ Transit engine | ⚠️ External KMS integration | ✅ AWS KMS native |
| **Automation** | ✅ API + cli, Secret Operator (ESO) | ✅ CLI + API | ✅ CRDs for automation | ✅ API |
| **Compliance** | ✅ Audit logs + encryption at rest + HSM | ✅ Enterprise audit | ⚠️ Depends on backend | ✅ AWS compliance |
| **OIDC/SPIFFE** | ✅ Native OIDC auth + SPIFFE support | ✅ OIDC available | ✅ Via integrations | ❌ IAM role-based |

## Decision

**OpenBao** (Apache 2.0 fork of Vault community edition) as self-hosted secrets management platform. Configuration:

- **Storage:** PostgreSQL with encryption at rest via etcd KMS plugin (ADR-105). Backup to S3/Azure Blob encrypted.
- **Multi-tenancy:** K8s namespace isolation enforced. Tenant workloads in tenant-* namespaces can only access tenant-scoped secrets.
- **Secret engines:** Key-value (application secrets), PKI (certificates, ADR-015), Transit (KMS for key encryption), SSH (host access).
- **Authentication:** OIDC from CI issuer (GitHub/Gitea, ADR-115) + Kubernetes ServiceAccount auth (runtime workloads).
- **Rotation:** Automated via cave-secrets crate (ADR-083). Database credentials rotated on TTL or on-demand. Certificates auto-renewed 30 days before expiry.

## Implementation Reference

**Implementation Status:** Production

- **cave-secrets** crate: OpenBao deployment, secret rotation automation, Kubernetes auth setup
- **Storage:** PostgreSQL (CNPG cluster, ADR-105) with encrypted backups
- **K8s integration:** ServiceAccount token auth for workload pods. RBAC prevents cross-tenant secret access.
- **Audit:** All secret access logged to Loki (ADR-029) + Sovereign Ledger (immutable audit trail)

## Rejected Options

### Vault (HashiCorp) — BSL Licensing Concern

**Reasons:**
1. **Business Source License (BSL):** As of Vault v1.15+, HashiCorp moved to BSL for new features. OpenBao is the community fork created after BSL adoption. CAVE cannot depend on proprietary BSL software for critical infrastructure.
2. **Licensing complexity:** Community edition vs. Enterprise vs. SaaS models. BSL requires careful tracking of which features are under which license. Risk of accidental license violation.
3. **OpenBao alternative:** OpenBao is a direct fork of Vault 1.14 (pre-BSL). Community-maintained, Apache 2.0. All Vault skills transfer directly.

### External Secrets Operator (K8s-native) — Not Sufficient as Primary

**Reasons:**
1. **Integration-focused, not generation-focused:** ESO syncs secrets FROM external sources into K8s. Excellent for reading Vault/AWS Secrets. Not designed as primary secret backend.
2. **No built-in generation:** ESO cannot generate SSH keys, PKI certificates, or rotating credentials. Requires Vault/AWS backend.
3. **Used complementarily:** CAVE uses ESO to sync OpenBao secrets into K8s Secret resources (ADR-053). ESO is secondary to OpenBao primary.

### AWS Secrets Manager — Not Portable

**Reasons:**
1. **AWS-only:** CAVE runs on both Hetzner and Azure. AWS Secrets Manager cannot support Hetzner profiles.
2. **Vendor lock-in:** OpenBao is cloud-agnostic. Secrets management must be.

## Consequences

### Positive

- **Self-hosted:** Full control over secret storage, encryption, audit logs. No external dependency.
- **Multi-tenancy:** Namespace-scoped secret isolation prevents cross-tenant access. ServiceAccount auth binds secrets to workload identity.
- **Compliance:** Encryption at rest (etcd KMS), audit logging (Loki + Sovereign Ledger), certificate auto-renewal all supported out of box.
- **Apache 2.0:** No licensing restrictions. Community-maintained fork ensures continued development.
- **Vault-compatible:** All Vault documentation applies. Team skills directly transferable.

### Negative

- **Operational complexity:** Self-managed PostgreSQL backend, backup/restore procedures, HSM setup (optional).
- **High availability:** Single OpenBao instance is bottleneck. HA setup requires additional Raft consensus layer + state replication.
- **Learning curve:** Vault/OpenBao concepts (auth methods, secret engines, policies) require training.
- **Audit log volume:** Every secret read/write logged. Large tenants generate high audit volume.

### Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| OpenBao instance becomes unavailable | Low | High | Deploy in HA cluster (Raft). Automated failover. Backup + restore test monthly. |
| Secret rotation job fails (DB cred rotation) | Medium | Medium | Monitoring alert on failed rotation. Manual override runbook. Failed rotation doesn't block workloads (old cred still valid until TTL). |
| Audit log fills storage | Low | Low | Audit log rotation to S3/Azure (cold storage). Retention policy: 90d hot, 1y cold. |
| Namespace isolation bypassed by misconfigured policy | Low | High | Policy-as-code review. Staging validates policies before prod. Regular RBAC audit. |

## License

**OpenBao:** Apache 2.0 (https://github.com/openbao/openbao/blob/main/LICENSE)

## Compliance Mapping

**SOC2 CC6.7:** Credential lifecycle — OpenBao manages secret creation, storage, rotation, and destruction.
**SOC2 CC7.1:** Monitoring — audit logging of all secret access.
**ISO/IEC 27001 A.8.2:** Access control — RBAC prevents unauthorized secret access.
**ISO/IEC 27001 A.8.13:** Cryptography — encryption at rest for all secrets.
**NIS2 Directive Article 21:** Access control and incident response — credential management + audit trail.
