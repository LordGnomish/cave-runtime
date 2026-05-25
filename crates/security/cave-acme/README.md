# cave-acme

RFC 8555 ACMEv2 server reimpl — multi-tenant accounts + orders + challenges

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.0000** (honest: 0.0000). Infra-only crate — no upstream parity measured.

## Upstream

- [smallstep/certificates](https://github.com/smallstep/certificates) — `step-ca` (License: Apache-2.0), tracked at version `v0.30.2`.


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
