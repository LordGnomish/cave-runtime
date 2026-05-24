# cave-trace

Distributed tracing engine — full Jaeger/Tempo parity

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.9474** (honest: 0.6053). Tier **C**.

## Upstream

- [jaegertracing/jaeger](https://github.com/jaegertracing/jaeger) (License: Apache-2.0), tracked at version `v1.52.0`.


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
