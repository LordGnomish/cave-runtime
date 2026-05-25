# cave-etcd

Distributed key-value store — etcd reimplementation built on cave-ha Raft consensus

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.9577** (honest: 0.9296). Tier **100**.

## Upstream

- [etcd-io/etcd](https://github.com/etcd-io/etcd) (License: Apache-2.0), tracked at version `v3.6.10`.


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
