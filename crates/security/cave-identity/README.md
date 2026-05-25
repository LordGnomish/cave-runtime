# cave-identity

Workload identity — SPIRE v1.15.0 deep-port (server + agent + X.509-SVID + JWT-SVID + federation + OIDC discovery + k8s workload attestor)

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **1.0000** (honest: 0.7200). Tier **C**.

## Upstream

- [spiffe/spire](https://github.com/spiffe/spire) (License: Apache-2.0), tracked at version `v1.15.0`.


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
