# ADR-157 — Sigstore Cosign Adoption (cave-sign)

- **Status:** Accepted
- **Date:** 2026-05-22
- **Deciders:** Burak Tartan, cave-runtime maintainers
- **Branch:** `claude/cave-sign-2026-05-22`
- **Companion crates:** [`cave-fulcio`] (Phase 2), [`cave-rekor`] (Phase 2), [`cave-vault`], [`cave-artifacts`], [`cave-sbom`], [`cave-vulns`]

## Context

The cave-runtime supply chain needs an artifact-signing engine that:

1. Signs container images, blobs, and OCI artifacts in a way that Kubernetes admission policy can verify.
2. Carries SLSA provenance + OpenVEX attestations as first-class envelopes.
3. Supports **both** the Sigstore public good (`fulcio.sigstore.dev` + `rekor.sigstore.dev`) for portability and a fully sovereign deployment (`cave-fulcio` + `cave-rekor`) for air-gapped sites.
4. Hardens supply chains without forcing operators to learn a new tooling story — `cosign sign-blob`/`cosign verify` muscle memory carries over.

Pre-port `cave-sign` was a 4-file `~5 KB` scaffold with `parity.ratio = 0.0`. It needed a Charter v2 deep port to clear the OSS launch v0.1.0 readiness bar.

## Decision

Adopt **Sigstore Cosign v3.0.6** (Apache-2.0; commit `f1ad3ee952313be5d74a49d67ba0aa8d0d5e351f`) and **sigstore/sigstore v1.10.6** (Apache-2.0; commit `311895e7870187320e47337734a9c321c0a8819c`) as the upstream contract for `cave-sign`. Re-implement the cosign command surface (sign / verify / attest / verify-attestation / policy / fulcio / rekor) in pure Rust against the workspace crypto stack (`ring`, `ed25519-dalek`, `p256`, `sha2`, `base64`).

### Module layout (20 src/ modules)

- **Crypto primitives:** `signature.rs` (ECDSA P-256 + Ed25519 sign/verify, deterministic-from-seed), `keypair.rs` (cosign PEM encode/decode), `error.rs`.
- **Sigstore clients:** `fulcio.rs` (CSR + offline mock issuer + HTTP v2), `rekor.rs` (in-memory Merkle log + HTTP v1).
- **Signing flows:** `blob.rs`, `oci.rs`, `keyless.rs` (OIDC → Fulcio → Rekor orchestration).
- **Attestation:** `attestation.rs` (in-toto Statement v1 + DSSE envelope + SLSA Provenance v1 + OpenVEX 0.2.0).
- **Verification:** `verify.rs`, `tlog.rs`, `sct.rs`, `policy.rs` (cert-identity glob + cert-issuer exact + require-rekor + require-keyless).
- **Configuration:** `signing_config.rs` (public-good vs sovereign), `trustedroot.rs`.
- **Persistence + transport:** `store.rs`, `routes.rs`, `engine.rs`.
- **Models:** `models.rs`, `oidc.rs`.

### CLI surface

`cavectl sign {sign,verify,attest,policy,fulcio,rekor}` — wired in `crates/cave-cli/src/main.rs` against `/api/sign/{sign,verify,attest,verify-attest,policy,fulcio,rekor,list}` routes.

### Parity bookkeeping

`parity.manifest.toml` ships subsystem-count bookkeeping under Charter v2:

- `fill_ratio = 0.9487` (37/39) — (mapped + partial + skipped) / total
- `honest_ratio = 0.5385` (21/39) — (mapped + partial) / total
- 20 mapped, 1 partial, 16 skipped (formalised Phase 2 cuts), 2 unmapped (honest gaps)

## Alternatives considered

1. **Ship a thin wrapper around the cosign Go binary.** Rejected: brings a Go runtime into every cave deployment, breaks the "every cave service is a single Rust workspace" invariant, and rules out sovereign builds where `cosign` itself isn't permitted to phone home to `sigstore.dev` for trust root fetches.
2. **Use the `sigstore-rs` community crate directly.** Rejected for MVP: the crate's surface is narrower than cosign's CLI (no OpenVEX, partial DSSE) and its trust-root bootstrap is tightly coupled to public-good Sigstore. We may adopt sigstore-rs as a *backend* in Phase 2 once its sovereign-deployment story matures.
3. **Defer signing entirely to cave-vault.** Rejected: cave-vault owns *secret material*, not the signing protocol. Verification needs cosign bundle parsing + Rekor binding + cert-identity policy — none of which belong in a secrets engine.

## Consequences

### Positive

- Full keyless flow (Fulcio + OIDC + Rekor) supported in pure Rust with no `go` runtime.
- SLSA Provenance + OpenVEX attestations are first-class — cave-sbom and cave-vulns can build chains against them without an external attester.
- Sovereign deployment is a `SigningConfig::sovereign()` away — same code path, just different endpoints.
- 153 lib + 9 self-audit + 5 smoke tests catch the high-value invariants (digest-bound signatures, Rekor binding consistency, policy enforcement).

### Negative

- Hardware key signing (PIV / YubiKey / pkcs11) is **Phase 2** — cave-hwsign owns it.
- Cloud KMS (AWS / Azure / GCP / Vault transit) is **Phase 2** — cave-cloud owns it.
- Full TUF root rotation is **Phase 2** — cave-tuf owns it; cave-sign accepts a static `trusted_root.json` snapshot.
- CT log signature verification is **partial** — SCT presence is checked, but the SCT signature is verified by cave-ctlog in Phase 2.
- Real X.509 chain validation for Fulcio-issued certs is unmapped — we currently parse the cosign mock cert JSON; cave-pki will share the verifier in Phase 2.

### Risks

- **Sigstore protobuf bundle v0.3.** Upstream cosign emits both the JSON bundle (which we ship) and the protobuf bundle (which we skip). Verifiers that only consume the protobuf bundle will not interoperate with cave-sign-emitted signatures until the next deep port.
- **Always-latest gate.** Cosign v3.0.6 (2026-04-06) and sigstore v1.10.6 (2026-05-14) were both latest at audit time; cave-upstream-watchd will trip a renewal task when v3.1.0 or v1.11.0 lands.

## Verification

`cargo test -p cave-sign` → **167 PASS / 0 FAIL / 0 IGNORE** (153 lib + 9 self_audit + 5 smoke). Charter v2 8/8 gates GREEN.

## Follow-ups

- [ ] `cave-fulcio` Phase 2 — sovereign Fulcio CA with X.509 chain emission + cave-pki integration.
- [ ] `cave-rekor` Phase 2 — sovereign Rekor with persistent Merkle log + gossip witness.
- [ ] `cave-tuf` Phase 2 — TUF root rotation + sigstore TUF mirror.
- [ ] `cave-ctlog` Phase 2 — CT log client + SCT signature verification.
- [ ] `cave-hwsign` Phase 2 — PIV + YubiKey + PKCS#11 hardware signing.
- [ ] `cave-tsa` Phase 2 — RFC 3161 timestamp authority.
- [ ] `cave-sign` next deep port — Sigstore protobuf bundle v0.3 + fuzz harness.

[`cave-fulcio`]: ../../crates/
[`cave-rekor`]: ../../crates/
[`cave-vault`]: ../../crates/cave-vault
[`cave-artifacts`]: ../../crates/cave-artifacts
[`cave-sbom`]: ../../crates/cave-sbom
[`cave-vulns`]: ../../crates/cave-vulns
