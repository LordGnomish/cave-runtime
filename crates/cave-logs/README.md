# cave-logs

Log aggregation with full Loki parity — LogQL engine, multi-tenant storage, Loki push API, syslog, OTLP, Fluentd

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.9583** (honest: 0.8750). Tier **C**.

## Upstream

- [grafana/loki](https://github.com/grafana/loki) (License: AGPL-3.0), tracked at version `v3.4.0`.


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
