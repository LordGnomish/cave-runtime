# cave-changelog

Release intelligence — git + SBOM diff based release notes

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.0000** (honest: 0.0000). Infra-only crate — no upstream parity measured.

## Upstream

This crate has no single upstream — see `parity.manifest.toml` for details
(or, for infra-only crates, the in-tree design notes).


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
