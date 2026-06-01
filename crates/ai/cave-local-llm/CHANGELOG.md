# Changelog — cave-local-llm

All notable changes to this crate are recorded here.
Format follows [Keep a Changelog](https://keepachangelog.com/) loosely;
versions track the workspace cadence rather than per-crate semver until
the runtime hits `v1.0`.

## [Unreleased]

- Tracked under `parity.manifest.toml` — see `docs/PARITY_INDEX.md` for the
  current fill / honest ratios.
- **2026-06-01 — count-neutral depth (honest_ratio stays 1.0).** 5 strict
  RED→GREEN cycles deepening already-mapped modules, no manifest count change:
  GGUF tensor-info decode + `general.alignment` (`fs/ggml gguf.go`); Go
  text/template `{{ else }}` branches and `{{- -}}` whitespace-trim markers in
  the prompt engine (`docs/template.md`); the per-tensor ggml `TensorType`
  (`block_size`/`type_size`/`tensor_size` from `llm/ggml.go`) wired to
  `TensorInfo::size_bytes()`; and a `cave-local-llm gguf` inspect subcommand.
  +15 lib tests (159 → 174).
- **2026-05-30 — graceful shutdown (honest_ratio 0.9677 → 1.0).** Ported
  ollama/ollama `server.Serve` shutdown semantics (`server/routes.go`) into an
  in-crate `ShutdownController` state machine (`src/shutdown.rs`): the first
  `SIGINT`/`SIGTERM`/stop-file request drains the in-flight scheduler item to
  completion; a second signal forces an immediate abort. `daemon.rs::run` now
  also handles `SIGINT` (was `SIGTERM`-only). Closes the last partial. 3 strict
  RED→GREEN cycles, +16 lib tests (143 → 159).

## [0.1.0] — 2026-05-22

Initial OSS launch (see ADR-148 — OSS launch history strategy).
The pre-launch history was squashed; see the workspace `CHANGELOG`
or `docs/oss-launch-final-audit-2026-05-19.md` for the launch audit.
