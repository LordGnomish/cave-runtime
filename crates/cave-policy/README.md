# cave-policy

Policy triumvirate — OPA v1.16.2 (Rego + REST) + Gatekeeper v3.22.2 (ConstraintTemplate/Constraint) + Kyverno v1.18.1 (validate/mutate/generate/verifyImages)

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **0.9615** (honest: 0.5769). Tier **C**.

## Upstream

- [open-policy-agent/opa](https://github.com/open-policy-agent/opa) — `Open Policy Agent` (License: Apache-2.0), tracked at version `v1.16.2`.


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
