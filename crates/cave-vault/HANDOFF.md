# cave-vault — Handoff (2026-06-07, branch `feature/vault-real-impl`)

HashiCorp Vault / OpenBao **v2.5.4** Rust port. This session added the charter
**PQC-ready seal-wrap** (ML-KEM-768), the **PQC auto-seal lifecycle**, the
**Raft AppendEntries** consensus primitive + integration test, and reconciled
the stale self-audit gate set. Worktree: `../cave-vault-impl`. **Not pushed**
(per instructions); merged locally `--no-ff` into the working branch.

## State

- `cargo test -p cave-vault` — **GREEN, 362 tests, 0 failures** across all targets.
- `cargo test -p cavectl compat::vault` — GREEN (incl. new `seal_backends`).
- Self-audit `tests/parity_self_audit.rs` — **9/9 gates pass** (was 5 RED on
  this branch: stale v2.5.3 / 2026-05-19 pins + a gate_2 sha bug + gate_7
  scanner false-positive).
- OpenBao parity (manifest-authored): **27 mapped / 0 partial / 21 skipped / 48
  total / fill_ratio 1.0000 / honest_ratio 0.5625** — unchanged. This session
  is **count-neutral depth** + a charter extension, deliberately *not* an
  honest-ratio inflation.

## What landed (6 commits, strict TDD — each RED verified before GREEN)

| Commit | What |
|---|---|
| `3dd24ee3` | reconcile stale self-audit gates → committed v2.5.4 reality |
| `43eb6e50` | PQC ML-KEM-768 seal-wrap (`src/core/pqc_seal.rs`) |
| `77ce9f5d` | PQC auto-seal lifecycle (`AutoSealType::MlKem768` + `PqcSeal`) |
| `54975d5c` | Raft AppendEntries replication + `tests/raft_consensus.rs` |
| `e3b4fcf0` | `GET /v1/sys/seal-backends` + `cavectl vault seal-backends` |
| `d9f72c30` | manifest + PARITY_REPORT depth notes |

## PQC seal-wrap (the headline)

`src/core/pqc_seal.rs` — KEM-DEM hybrid envelope, **real** crypto:

```
(kem_ct, ss) = ek.encapsulate()            ML-KEM-768 (FIPS 203, NIST cat 3)
wrap_key     = HKDF-SHA256(ss, "cave-vault/pqc-seal/v1")
nonce‖sealed = AES-256-GCM(wrap_key).seal(master_key)
```

- Lattice math comes from the **vetted RustCrypto `ml-kem` 0.3.2** crate (no
  hand-rolled PQC). Envelope glue, KDF (ring hkdf) and DEM (ring AES-256-GCM)
  are ours.
- `PqcSealKeypair`: `generate` / `from_seed_bytes` (64-byte FIPS-203 seed) /
  `seed_bytes` / `public_key_bytes` (1184B) / `seal_wrap` / `seal_wrap_to_public`
  (seal with only the public key — separation of duties) / `seal_unwrap`.
- `PqcSeal` (auto-seal): `initialize(recovery_shares, recovery_threshold)` →
  mints + wraps master key, Shamir-splits it into recovery shares;
  `auto_unseal()` unwraps via the held dk; `recover_master_key(shares)` via
  recovery quorum; `from_persisted(seed, wrapped_json)`.
- `AutoSealType::MlKem768` (barrier_type `"mlkem768"`, local recovery-key seal;
  `AutoSealConfig::validate` skips the endpoint requirement for it).

Tamper on either ciphertext fails the GCM tag (ML-KEM uses implicit rejection,
so a bad KEM ct just yields a wrong shared secret → wrong AES key → tag fail);
`seal_unwrap` never returns a wrong plaintext.

## Raft consensus

`src/storage/raft.rs` gained the replication RPC missing for multi-node:
`append_entries` (§5.3 consistency check, conflict truncation, idempotent
retransmit, commit advance), `log_entries_from`, `last_log_term`.
`tests/raft_consensus.rs` drives a 3-node cluster entirely via the public API:
elect → propose → replicate → quorum-commit → converge, plus gap-reject,
conflict-overwrite, idempotency, and minority-not-committed safety.

## Acceptance criteria → tests

| Criterion | Where |
|---|---|
| cargo test PASS | 362 green |
| KV v2 secret round-trip | `tests/kv2_engine.rs`, `tests/kv2_deep.rs` |
| Transit encrypt/decrypt | `engines::transit::tests::test_aes256_gcm_round_trip` / `test_chacha20_round_trip` |
| PKI cert issue | `engines::pki` issuance tests |
| Raft consensus integration | `tests/raft_consensus.rs` (5) |
| PQC seal-wrap | `tests/pqc_seal.rs` (8) + `tests/pqc_autoseal.rs` (6) + in-src (8) |
| LOC | src 17 456 → 18 088 (+632); session +1122 / −26 over 14 files |
| TDD git log | 6 commits above, RED→GREEN |

## License check

OpenBao is **MPL-2.0** (Vault fork from before the BSL relicense). MPL-2.0 is a
file-level weak-copyleft that is **explicitly GPL/AGPL-compatible** (MPL 2.0 §3.3
/ Exhibit B, not marked "Incompatible With Secondary Licenses"), so it composes
cleanly with the cave-runtime **AGPL-3.0-or-later** target — and this is a
clean-room Rust reimplementation, not a copy of OpenBao Go source. `ml-kem` is
Apache-2.0/MIT (AGPL-compatible). Conclusion: **port OK**.

## Follow-ups (not done this session)

- Persist the PQC `PqcSeal` into `VaultState`/storage and bridge it into the
  real unseal lifecycle (currently `core::pqc_seal` is library + read-only
  `/v1/sys/seal-backends` reporting; it is not yet the live barrier behind
  `/v1/sys/unseal`). The wrapper, lifecycle and recovery paths are fully built
  and tested — only the VaultState wiring + storage entry remain.
- ML-DSA signature path for the charter (only ML-KEM/KEM landed; `ml-dsa` is in
  the cargo cache if wanted for a Transit PQC signing key type).
- The 3 `[[upstream_test]]` entries still marked `missing` (token renew,
  userpass login, identity alias) — kept honest, not falsely promoted.
- Portal `/admin/vault` panel for the seal-backends view (route + cavectl exist).
- A future honest-ratio bump would require a genuine OpenBao skip→mapped
  (e.g. `openbao:plugins/` in-process catalog), not PQC (which is a charter
  extension, correctly count-neutral here).
