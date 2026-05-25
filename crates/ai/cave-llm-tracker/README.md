# cave-llm-tracker

Daily local-LLM tracker — polls HuggingFace / Ollama library / LMSys leaderboard / backend (vLLM, llama.cpp, MLX-LM) releases, benches candidates against cave-specific prompts, emits md+JSON report (Phase 0: report only, no auto-swap)

## Status

Tracked by `parity.manifest.toml`. Current fill ratio: **1.0000** (honest: 0.6471). Tier **C**.

## Upstream

- [cave-runtime/cave-llm-tracker](https://github.com/cave-runtime/cave-runtime/tree/main/crates/cave-llm-tracker) — `cave-llm-tracker (multi-source aggregator)` (License: AGPL-3.0-or-later), tracked at version `2026-05-21`.


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
