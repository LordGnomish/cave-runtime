# cave-ha

CAVE HA/DR — production-grade Raft consensus, automatic failover, cross-region DR

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.0000** (honest: 0.0000). Tier **C**.

## Upstream

- [etcd-io/etcd](https://github.com/etcd-io/etcd) (License: Apache-2.0), tracked at version `v3.5.13`.


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
