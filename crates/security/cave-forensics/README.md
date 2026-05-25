# cave-forensics

Runtime forensics — Tetragon v1.7.0 deep-port (TracingPolicy + kernel hooks + process credentials + policy filter + enforcer + evidence chain-of-custody)

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.9583** (honest: 0.6818). Tier **D1**.

## Upstream

- [cilium/tetragon](https://github.com/cilium/tetragon) (License: Apache-2.0), tracked at version `v1.7.0`.


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
