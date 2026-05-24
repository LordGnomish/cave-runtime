# cave-cri

Container Runtime Interface — Linux container lifecycle, namespaces, cgroups v2, OCI images

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **1.0000** (honest: 0.9118). Tier **100**.

## Upstream

This crate has no single upstream — see `parity.manifest.toml` for details
(or, for infra-only crates, the in-tree design notes).


## Public surface

See `src/lib.rs` for the public surface. The crate manifest
(`Cargo.toml`) and the parity manifest (`parity.manifest.toml`) are
the authoritative descriptions of what is in scope.

## License

AGPL-3.0-or-later (matches workspace policy).

## See also

- `parity.manifest.toml` — file-by-file upstream mapping
- `docs/PARITY_INDEX.md` — workspace-wide fill / honest ratios
- `docs/architecture/workspace-topology.md` — where this crate sits
