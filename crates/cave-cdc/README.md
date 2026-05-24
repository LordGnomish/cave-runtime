# cave-cdc

CDC pipeline (Debezium reimpl) — Postgres logical, MySQL binlog, MongoDB oplog, transactional outbox, cave-streams sink

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.0000** (honest: 0.0000). Tier **D2**.

## Upstream

- [debezium/debezium-server](https://github.com/debezium/debezium-server) — `Debezium Server` (License: Apache-2.0), tracked at version `v3.5.0.Final`.


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
