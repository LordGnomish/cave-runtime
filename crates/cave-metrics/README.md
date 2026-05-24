# cave-metrics

Prometheus + VictoriaMetrics parity — TSDB, PromQL, remote_write, alerting

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.9667** (honest: 0.9000). Tier **C**.

## Upstream

- [prometheus/prometheus](https://github.com/prometheus/prometheus) (License: Apache-2.0), tracked at version `v3.3.0`.


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
