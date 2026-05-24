# cave-container-scan

OCI image + IaC + FS + secret + YARA + namespace-confusion scanner — compatible with Trivy

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.9615** (honest: 0.7115). Tier **C**.

## Upstream

- [aquasecurity/trivy](https://github.com/aquasecurity/trivy) (License: Apache-2.0), tracked at version `v0.70.0`.


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
