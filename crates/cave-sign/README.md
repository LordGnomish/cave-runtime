# cave-sign

Artifact signing & verification — Sigstore Cosign reimplementation (keypair + keyless + Fulcio + Rekor + SLSA attestation + policy)

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.9487** (honest: 0.5385). Tier **D1**.

## Upstream

- [sigstore/cosign](https://github.com/sigstore/cosign) — `Sigstore Cosign` (License: Apache-2.0), tracked at version `v3.0.6`.


## Public surface

See `src/lib.rs` for the public surface. The crate manifest
(`Cargo.toml`) and the parity manifest (`parity.manifest.toml`) are
the authoritative descriptions of what is in scope.

## License

Apache-2.0 (matches workspace policy).

## See also

- `parity.manifest.toml` — file-by-file upstream mapping
- `docs/PARITY_INDEX.md` — workspace-wide fill / honest ratios
- `docs/architecture/workspace-topology.md` — where this crate sits
