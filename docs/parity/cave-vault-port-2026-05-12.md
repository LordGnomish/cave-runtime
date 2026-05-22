# cave-vault parity — 2026-05-12 audit

**Upstream:** `openbao/openbao v2.5.3` (MPL-2.0; HashiCorp Vault fork).

## Methodology

Standard cave-etcd pattern. Inventory enumerates the top-level
OpenBao packages and classifies each. cave-vault's source tree
(folder per concern: `engines/`, `auth/`, `core/`, `api/`)
mirrors the OpenBao layout cleanly, so most packages are mapped.

## Counts

| Bucket   | Count |
|----------|------:|
| Mapped   | 18 |
| Skipped  | 11 |
| Unmapped | 8 |
| **Total** | **37** |
| **fill_ratio** | **0.7838** |

## What lands in the inventory

* **Mapped (18)** covers every secret engine (KV v1+v2, PKI,
  Transit, Database, SSH, TOTP, AWS, Cubbyhole, Identity), every
  primary auth method (userpass, AppRole, Kubernetes, OIDC, cert,
  LDAP, Token), the core (policy / audit / lease / Shamir unseal /
  response-wrapping), and the `/v1/sys/*` API surface.
* **Skipped (11)** covers Ember UI (cave-portal serves the UI via
  admin/vault/), CLI (cavectl), plugin SDK, Go-stdlib HTTP glue,
  Terraform module, vendor, etc.
* **Unmapped (8)** covers the honest gaps: persistent storage
  backends (file/raft/consul/etcd/s3 — the biggest one, in-memory
  today), DR + perf replication, additional cloud-auth methods
  (Azure/GCP/OCI), JWT auth, additional cloud-secret engines,
  namespaces + quotas, Vault Agent (client-side), activity
  tracking.

## What this PR does NOT claim

* `fill_ratio = 0.7838` does NOT mean "cave-vault is 78% of a
  production Vault". It means **78% of OpenBao's top-level
  packages** are either covered (49%, including every primary
  secret engine + auth method) or honestly out of scope (30%,
  CLI / UI / SDK / vendor).
* The 8 unmapped entries are the real production blockers —
  particularly `physical/` (persistence) and `vault/namespaces`
  (multi-tenancy). They are tracked, not implemented.
