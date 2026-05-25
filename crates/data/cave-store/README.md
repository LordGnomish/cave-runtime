# cave-store

Unified storage engine — etcd v3 KV + MinIO/S3 object store with WAL and MVCC

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.0000** (honest: 0.0000). Tier **C**.

## Upstream

- [minio/minio](https://github.com/minio/minio) (License: AGPL-3.0), tracked at version `RELEASE.2024-01-01`.


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
