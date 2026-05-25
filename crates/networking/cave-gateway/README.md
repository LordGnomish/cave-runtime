# cave-gateway

API Gateway — Kong + Gravitee parity. Reverse proxy, plugin chain, Admin API, Gravitee API/plan/application/subscription surface.

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.9667** (honest: 0.7333). Tier **C**.

## Upstream

- [Kong/kong](https://github.com/Kong/kong) (License: Apache-2.0), tracked at version `3.9.1`.


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
