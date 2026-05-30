# Changelog — cave-local-llm

All notable changes to this crate are recorded here.
Format follows [Keep a Changelog](https://keepachangelog.com/) loosely;
versions track the workspace cadence rather than per-crate semver until
the runtime hits `v1.0`.

## [Unreleased]

- Tracked under `parity.manifest.toml` — see `docs/PARITY_INDEX.md` for the
  current fill / honest ratios.
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
