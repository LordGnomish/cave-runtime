# cave-sign — Charter v2 8-gate close-out

**Date:** 2026-05-22
**Branch:** `claude/cave-sign-2026-05-22`
**Upstream pin:** sigstore/cosign `v3.0.6` (`f1ad3ee952313be5d74a49d67ba0aa8d0d5e351f`) + sigstore/sigstore `v1.10.6` (`311895e7870187320e47337734a9c321c0a8819c`) — Apache-2.0
**Parity:** `fill_ratio = 0.9487` (37/39) · `honest_ratio = 0.5385` (21/39)

| # | Gate | Status | Evidence |
| - | --- | --- | --- |
| 1 | **Upstream pinned** (always-latest) | PASS | `parity.manifest.toml::[upstream].version = "v3.0.6"` (cosign latest 2026-04-06) + `[[upstreams]].version = "v1.10.6"` (sigstore latest 2026-05-14). `assertion_1_cosign_version_pinned`. |
| 2 | **source_sha pinned** | PASS | Cosign `f1ad3ee9…35e351f` + sigstore `311895e7…21c0a8819c`. `assertion_2_source_sha_matches_versions`. |
| 3 | **fill_ratio ≥ 0.65** | PASS | `0.9487` = (20 mapped + 1 partial + 16 skipped) / 39. `assertion_3_fill_ratio_meets_floor`. |
| 4 | **parity_ratio_source = "manifest"** | PASS | `[parity].parity_ratio_source = "manifest"`. `assertion_4_parity_ratio_source_is_manifest`. |
| 5 | **last_audit = 2026-05-22** | PASS | `[parity].last_audit = "2026-05-22"`. `assertion_5_last_audit_is_today`. |
| 6 | **counts sum to total + ≥ 15 mapped** | PASS | 20 + 1 + 16 + 2 = 39 total; 20 mapped ≥ 15 floor. `assertion_6_counts_sum_to_total`. |
| 7 | **AGPL SPDX header coverage 100%** | PASS | All 22 `.rs` files in `src/` + `tests/` carry `SPDX-License-Identifier: AGPL-3.0-or-later`. `assertion_7_agpl_spdx_header_coverage`. |
| 8 | **no stub macros in src/** | PASS | No `todo!()` / `unimplemented!()` / `panic!("stub")` / `panic!("todo")` in `src/**/*.rs`. `assertion_8_no_stub_macros_in_src`. |

Bonus gate 9 (Charter v2 surface integrity): full sign / verify / attest / policy / keyless / Fulcio / Rekor surface reachable through `cave_sign` crate-root re-exports. `assertion_9_cosign_surface_intact`.

## Subsystem counts

| Bucket | Count | Examples |
| --- | --- | --- |
| Mapped | 20 | keypair-generate-import, blob-sign/verify, oci-image-sign/verify, keyless-sign-orchestrator, fulcio-csr-client, rekor-hashedrekord-client, in-toto-statement, dsse-envelope, slsa-provenance-v1, openvex-predicate, cosign-bundle, tlog-binding-verify, policy-cert-{identity,issuer}, policy-require-rekor, trusted-root, signing-config, verify-orchestrator |
| Partial | 1 | sct-presence-check (signature verify against CT log keys → Phase 2 cave-ctlog) |
| Skipped | 16 | piv-key-yubikey, pkcs11-hsm, kms-{aws,azure,gcp,hashivault}, tuf-root-rotation, tsa-rfc3161, dockerfile-verify, manifest-verify, tree, triangulate, copy, clean, sigstore-protobuf-bundle, fuzz-harness |
| Unmapped (honest gaps) | 2 | ctlog-fetch-public-keys, fulcio-x509-cert-chain-validation |

## Test totals

| Suite | Pass | Fail | Skip |
| --- | ---: | ---: | ---: |
| Lib unit tests | 153 | 0 | 0 |
| `tests/parity_self_audit.rs` | 9 | 0 | 0 |
| `tests/smoke.rs` | 5 | 0 | 0 |
| **TOTAL** | **167** | **0** | **0** |

## Scope-cuts → Phase 2 owners

| Group | Phase 2 crate(s) | Items |
| --- | --- | --- |
| Hardware keys | `cave-hwsign` | piv-key-yubikey, pkcs11-hsm |
| Cloud KMS | `cave-cloud` | kms-aws, kms-azure, kms-gcp, kms-hashivault |
| TUF root rotation | `cave-tuf` | tuf-root-rotation |
| TSA timestamping | `cave-tsa` | tsa-rfc3161 |
| Admission policy | `cave-admission` | dockerfile-verify, manifest-verify |
| Artifacts side commands | `cave-artifacts`, `cave-portal-api` | tree, copy, clean, triangulate |
| Protobuf bundle + fuzz | `cave-sign` (next deep port) | sigstore-protobuf-bundle, fuzz-harness |

## Smoke evidence

| Scenario | Test | Result |
| --- | --- | --- |
| Keypair sign + verify roundtrip (P-256 + Ed25519) | `smoke_1_keypair_sign_verify_roundtrip` | PASS |
| Rekor log entry mock + Merkle inclusion proof (4-leaf tree) | `smoke_2_rekor_log_entry_mock_with_inclusion_proof` | PASS |
| SLSA Provenance v1 + DSSE envelope verify | `smoke_3_slsa_attestation_fixture` | PASS |
| Cosign bundle JSON parse + emit + triple | `smoke_4_bundle_format_parse_emit` | PASS |
| Keyless (Fulcio mock + Rekor + policy) end-to-end | `smoke_5_keyless_end_to_end_with_policy` | PASS |

## cavectl integration

`cavectl sign {sign,verify,attest,policy,fulcio,rekor}` wired in `crates/cave-cli/src/main.rs` against the `/api/sign/{sign,verify,attest,policy,fulcio,rekor}` routes.

## Workspace integration

- `cave-vault` keypair PEM is line-protocol compatible with `cave_sign::keypair::{encode,decode}_public_pem` — Vault is expected to encrypt at rest and hand cave-sign a plaintext private-key PEM for signing.
- `cave-artifacts` (Pulp + Harbor + Nexus) consumes the `SignatureLayer` from `cave_sign::oci::sign_image_keypair{,_with_rekor}` and pushes it under the `sha256-<hex>.sig` tag.
- `cave-sbom` provenance-attestation chain consumes DSSE envelopes from `cave_sign::attestation::sign_attestation` keyed by SLSA Provenance subject digests.
- `cave-vulns` correlates CVE IDs to OpenVEX predicates emitted by `cave_sign::attestation::build_vex_predicate`.

## ADR

- [ADR-157 — Sigstore Cosign Adoption](../../docs/adr/ADR-157_Sigstore_Cosign_Adoption.md)
