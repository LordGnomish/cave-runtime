# cave-vault — Charter v2 close-out report (2026-05-23 wave-4)

**Date:** 2026-05-23  
**Branch:** `claude/cave-vault-eso-sealed-2026-05-23`  
**Status:** 1.0000 fill ratio, 8/8 Charter v2 gates GREEN.

## Triumvirate upstreams

| Role | Upstream | Version | License | source_sha |
|---|---|---|---|---|
| Primary — KV / PKI / transit / auth | `openbao/openbao` (Vault fork) | v2.5.4 | MPL-2.0 | `4f6d47246a053375271a5fd8af85c3b75695aa46` |
| External Secrets reconciler | `external-secrets/external-secrets` | v2.5.0 | Apache-2.0 | `0755b0af7de7f05a104b0df29ba84f43513fee8b` |
| GitOps-safe sealed manifests | `bitnami-labs/sealed-secrets` | v0.37.0 | Apache-2.0 | `8e4ed463552a6a6462648a9ff090a1f42abbda30` |

## What changed in this close

- Bumped OpenBao pin **v2.5.3 → v2.5.4** + replaced placeholder `source_sha = "v2.5.3"` with the actual commit SHA `4f6d47246a05…`.
- Added **External Secrets Operator** (ESO) as a sub-module under `src/external_secrets/`:
  - `mod.rs` — `SecretStore` / `ClusterSecretStore` / `ExternalSecret` / `PushSecret` CRD types + `ProviderConfig` enum (Vault / AWS-SM / GCP-SM / Azure-KV / Kubernetes / Fake).
  - `providers.rs` — `Provider` async trait + `FakeProvider` reference implementation + `build_provider()` dispatch (cloud-SDK adapters scope-cut to Phase 2 cloud-provider crate).
  - `reconciler.rs` — synchronous `reconcile_once(store, es)` reconciliation; continuous reconciler scope-cut to `cave-policy-controller (Phase 2)`.
- Added **Sealed Secrets** as a sub-module under `src/sealed_secrets/`:
  - `mod.rs` — `SealedSecret` CRD type + `Scope` enum (Strict / Namespace / Cluster) + `binding_label()` per upstream `pkg/crypto/crypto.go`.
  - `crypto.rs` — envelope split / assemble + HKDF binding-label hash (RSA-OAEP wrap itself delegated to `crate::engines::transit`'s ring-backed primitives — kept out of this module to avoid duplicating crypto dependencies).
  - `controller.rs` — `KeyStore` current-vs-deprecated rotation logic.
- Updated `parity.manifest.toml` with 6 new `[[mapped]]` + 4 new `[[skipped]]` entries → total 27 mapped / 0 partial / 21 skipped / 0 unmapped / 48 total / **fill_ratio 1.0000** / honest_ratio 0.5625.
- Added `src/parity_self_audit.rs` with G1–G8 + roll-up tests.
- Added `observability.toml` with 9 panels + 5 alerts (OpenBao + External Secrets + Sealed Secrets).
- Added `[package.metadata.upstream]` + `[[package.metadata.upstreams]]` to `Cargo.toml`.
- Added `async-trait` + `parking_lot` to direct dependencies.

## Architecture map (new modules only)

```
cave-vault/src/
├── external_secrets/
│   ├── mod.rs           ← SecretStore / ClusterSecretStore / ExternalSecret / PushSecret CRDs + ProviderConfig + VaultAuth
│   ├── providers.rs     ← Provider trait + FakeProvider + build_provider()
│   └── reconciler.rs    ← reconcile_once() — synchronous variant
├── sealed_secrets/
│   ├── mod.rs           ← SealedSecret CRD + Scope + binding_label()
│   ├── crypto.rs        ← envelope split/assemble + HKDF label hash
│   └── controller.rs    ← KeyStore (current + deprecated keys)
└── parity_self_audit.rs ← G1–G8 + roll-up
```

## Parity ratios

| Metric | Value |
|---|---|
| mapped | 27 (21 OpenBao + 4 External Secrets + 2 Sealed Secrets) |
| partial | 0 |
| skipped (scope_cut) | 21 (17 OpenBao + 2 External Secrets + 1 Sealed Secrets + 1 shared metrics) |
| unmapped (honest gap) | 0 |
| total | 48 |
| **fill_ratio** | **1.0000** |
| honest_ratio | 0.5625 (mapped / total) |

## Charter v2 gate verdict

| Gate | Verdict |
|---|---|
| G1 SPDX headers on new src/* | PASS |
| G2 no stub macros (outside `#[cfg(test)]`) | PASS |
| G3 fill_ratio ≥ 0.95 | PASS (1.0000) |
| G4 parity_self_audit.rs embedded | PASS |
| G5 PARITY_REPORT.md ≥ 1 KiB + OpenBao + External Secrets + Sealed Secrets covered | PASS |
| G6 observability.toml ≥ 8 panels + ≥ 5 alerts | PASS (9 / 5) |
| G7 source_sha pinned for all 3 upstreams in Cargo.toml + manifest | PASS |
| G8 ≥ 27 mapped surfaces (3-upstream umbrella floor) | PASS |

## Scope cuts (this close — 4 new)

| Destination | Surface |
|---|---|
| `cave-policy-controller (Phase 2)` | External Secrets continuous-reconciler informer + queue |
| `cave-deploy` | External Secrets helm-chart bootstrap |
| `cave-cli` | kubeseal CLI + sealed-secrets controller daemon |
| `cave-metrics` | Prometheus exporters of both External Secrets and Sealed Secrets |

## cavectl wiring (orchestrator follow-up)

```
cavectl secrets {store, external, push, generator, sealed, seal, unseal, rotate}
```

To be wired in `crates/cave-cli/src/main.rs` post-merge.

---

## 2026-06-07 — PQC seal-wrap + Raft consensus (branch `feature/vault-real-impl`)

Continuation pass against OpenBao **v2.5.4**. OpenBao parity counts unchanged
(27 mapped / 48 total / honest_ratio **0.5625**) — this session is **depth**,
plus a charter PQC extension, not a skip→mapped promotion. 8/8 Charter gates
stay GREEN; the `tests/parity_self_audit.rs` gate set was reconciled from the
stale v2.5.3 pins to v2.5.4 (9/9 gates pass).

Strict-TDD cycles (each RED verified before GREEN):

1. **PQC ML-KEM-768 seal-wrap** (`src/core/pqc_seal.rs`, charter PQC-ready
   baseline). KEM-DEM hybrid envelope — ML-KEM-768 (NIST FIPS 203, cat 3) via
   vetted RustCrypto `ml-kem` 0.3.2 → HKDF-SHA256 → AES-256-GCM. Round-trip,
   per-call randomisation, tamper rejection (KEM ct + DEM ct), wrong-key
   rejection, seed determinism, FIPS-203 deterministic-encapsulation KAT,
   serde round-trip.
2. **PQC auto-seal lifecycle** — `AutoSealType::MlKem768` (barrier "mlkem768",
   local recovery-key seal) + `PqcSeal` (initialize / auto_unseal /
   recover_master_key via Shamir recovery quorum / from_persisted).
3. **Raft AppendEntries replication** (`src/storage/raft.rs`) — `append_entries`
   (§5.3 consistency + conflict truncation + idempotent retransmit + commit
   advance), `log_entries_from`, `last_log_term`; drives a 3-node consensus
   integration test (`tests/raft_consensus.rs`).
4. **Surface wiring** — read-only `GET /v1/sys/seal-backends` (cave-vault) +
   `cavectl vault seal-backends`.

Acceptance tests (all GREEN):

| Criterion | Test |
|---|---|
| KV v2 secret round-trip | `tests/kv2_engine.rs`, `tests/kv2_deep.rs` |
| Transit encrypt/decrypt | `engines::transit::tests::test_aes256_gcm_round_trip` / `test_chacha20_round_trip` |
| PKI cert issue | `engines::pki` issuance tests |
| Raft consensus integration | `tests/raft_consensus.rs` (5 tests) |
| PQC seal-wrap | `tests/pqc_seal.rs` (8) + `tests/pqc_autoseal.rs` (6) + in-src |

New tests this session: 8 + 6 + 5 + 1 (sys endpoint) + 1 (cavectl) + in-src
(seal/pqc/raft) ≈ 35.

Licensing: OpenBao is MPL-2.0; MPL-2.0 is compatible with the cave-runtime
AGPL-3.0-or-later target (a clean-room Rust port, no Go source copied). The
`ml-kem` crate is Apache-2.0/MIT — AGPL-compatible.
