# cave-vault

HashiCorp Vault-compatible secrets engine — Rust port for cave-runtime

## Status

This crate is in a pre-OSS-launch state, with API parity actively tracked against `openbao/openbao` v2.5.3. It ports the full Vault HTTP API surface and policy semantics line-by-line. Per the `cave-runtime` charter, no backward compatibility shims are implemented; the implementation is strict and sovereign.

## Upstream

- [openbao/openbao v2.5.3](https://github.com/openbao/openbao) — a fork of HashiCorp Vault, licensed under Apache-2.0 since v2.

## Surface ported

- KV v2 secrets engine: Versioned secrets storage at `/v1/secret/data/...` with read/write/delete/versioning support.
- Policy engine: Path-glob ACLs with capability precedence, where Deny explicitly overrides Allow.
- Authentication methods: Token auth, AppRole auth, and Userpass auth methods fully implemented.
- Lease lifecycle: Complete management of TTLs, lease renewal, and lease revocation.
- PKI engine: Certificate issuance and revocation capabilities.
- Transit engine: Encryption-as-a-service primitives for data-at-rest and in-transit protection.
- Shamir Secret Sharing: Initial root unseal flow using Shamir's algorithm.
- Multi-tenancy: Mount table, Auth table, and Namespace isolation support.

## Public API

- `cave_vault::VaultState` — The top-level engine state, designed to be shared via `Arc` across the runtime.
- `cave_vault::router(state)` — Returns an `axum::Router` instance that mounts the full Vault HTTP API surface.
- Re-exports from sub-modules including `cave_vault::policy`, `cave_vault::token`, `cave_vault::lease`, and others.
- See `crates/cave-vault/parity.manifest.toml` for the detailed file-by-file upstream-to-local mapping.

## Tests

- Nine integration test files covering audit logging, AppRole, Token, Userpass, identity, and deep KV v2 scenarios.
- Five Mode B-prime behavior tests for the policy engine located in `tests/qwen_drafted.rs`.

## License

Apache-2.0 (matches upstream OpenBao/HashiCorp Vault licensing).

## See also

- [../cave-pki](../cave-pki) — Standalone PKI implementation (subset of cave-vault PKI engine).
- [../cave-permission](../cave-permission) — Casbin-based authorization system (orthogonal to vault policy ACLs).
