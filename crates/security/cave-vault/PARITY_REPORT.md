# cave-vault — Charter v2 close-out report (2026-05-23 wave-4)

**Date:** 2026-05-23  
**Branch:** `claude/cave-vault-eso-sealed-2026-05-23`  
**Status:** 1.0000 fill ratio, 8/8 Charter v2 gates GREEN.

## 2026-05-31 cont2 honest uplift (honest_ratio 0.5625 → 0.5833)

- Promoted **`openbao:plugins/`** skipped → mapped: ported the plugin-catalog
  registry/data layer (`vault/plugin_catalog.go` + `sdk/helper/consts/plugin_types.go`)
  into `src/plugins/mod.rs` via 3 strict-TDD cycles — `PluginType`
  (iota-faithful `auth`/`database`/`secret`/`unknown` + `ParsePluginType`),
  `PluginRunner`, `SetPluginInput`, and `PluginCatalog` (`set`/`get`/`delete`/
  `list`/`list_versions`). `set` guards `..` parent-refs and validates sha256
  hex; storage keys are type-namespaced `<type>/<name>[/<version>]`; `get`
  falls back to the builtin registry (external unversioned shadows builtin);
  `list`/`list_versions` merge+dedup+semver-sort.
- **scope_cut remainder:** the external-process runner (os/exec of a plugin
  binary from a plugin dir) + go-plugin gRPC multiplexing stay out of crate —
  cave-runtime does not exec arbitrary binaries. The catalog is what mount
  resolution consults, so the registry itself is in-crate.
- Wired `/v1/sys/plugins/catalog/{type}[/{name}]` (GET/POST/DELETE) +
  `cavectl vault plugin {list,info,register,deregister}`.
- Counts: mapped 27 → 28, skipped 21 → 20, total 48 (unchanged).
- Maintenance: brought the stale `tests/parity_self_audit.rs` back in line with
  the live manifest (the wave-4 v2.5.3→v2.5.4 bump + placeholder→real-SHA swap
  had left gates 1/2/5/9 literal-matching old strings; gate_7 false-positived on
  the scanner's own macro-name literals). 9/9 gates GREEN.

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
