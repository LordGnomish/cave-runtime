# cave-net

eBPF-based pod networking — CNI, service discovery, network policy, load balancing

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.9851** (honest: 0.9851). Tier **C**.

## Upstream

- [cilium/cilium](https://github.com/cilium/cilium) (License: Apache-2.0), tracked at version `v1.19.3`.


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
