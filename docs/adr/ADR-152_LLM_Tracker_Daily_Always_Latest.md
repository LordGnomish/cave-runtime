# ADR-152 — cave-llm-tracker: daily always-latest local-LLM tracker

- **Date:** 2026-05-21
- **Status:** Accepted
- **Owner:** Burak Tartan
- **Crate:** `cave-llm-tracker`
- **Branch:** `claude/cave-llm-tracker-2026-05-21`

## Context

Burak's coding seat is a single local LLM (today:
`qwen3.6:35b-a3b-coding-mxfp8`). The "always-latest" mandate that
governs cave-runtime upstreams (ADR-RUNTIME-UPSTREAM-WATCH-001) was
written for code dependencies; it does not cover the local model.

There is no equivalent of `cave-upstream-watchd` for *models*, so the
seat can stay frozen on a stale checkpoint for weeks while better
candidates land on HuggingFace, ship in the Ollama library, or top the
LMSys leaderboard. Today the only signal that an upgrade is overdue is
Burak noticing a slow turn-around or a hallucinated parity manifest.

## Decision

Add a first-party crate, `cave-llm-tracker`, that runs once per day at
**03:00 Europe/Berlin** and writes a `daily-<date>.{md,json}` report
covering:

1. Four sources — HuggingFace `/api/models`, Ollama library index,
   LMSys leaderboard CSV, and GitHub release tags for the three local
   runtimes that actually load the model on this box (vLLM,
   llama.cpp, MLX-LM).
2. A deterministic 5-prompt cave-specific bench against each viable
   candidate (Charter v2 close paperwork; parity-manifest TOML; Rust
   refactor; TR+EN dual; Conventional Commits message).
3. Selection guards (license allow-list; VRAM/disk ceilings) and
   uplift floors (+10% throughput **or** +5% quality flags an
   `UpgradeCandidate`).

**Phase 0 is report-only.** The crate must **not** mutate any local
model state. `TrackerConfig::validate()` hard-rejects
`selection.auto_swap = true`.

Phase 1 (separate ADR) will wire `cavectl llm-tracker apply` to swap
the baseline once a candidate clears both floors on three consecutive
days.

## Consequences

- The local-LLM seat now has the same kind of always-latest paper
  trail that the upstream watch system gives to code dependencies.
- The daily report is small (KiB-scale JSON) and idempotent;
  storage cost on the box is negligible.
- A new LaunchAgent plist
  `~/Library/LaunchAgents/com.cave.llm-tracker-daily.plist` is added.
  Reverting the tracker means `launchctl unload <plist> && rm <plist>`
  plus `cargo uninstall cave-llm-tracker`.
- The crate has no single upstream Git pin; the `parity.manifest.toml
  [upstream] source_sha` is an inline TOML table keyed by source.
  Workspace parity-index logic already handles inline tables (see the
  cave-streams precedent for `kafka` + `pulsar`).
- Phase 0 deliberately defers Portal + Obs surfaces; the
  `schema_version: 1` JSON layout is stable so those phases can land
  without coordination.

## Alternatives considered

- **Bash + jq daily cron.** Rejected — no test coverage, no
  type-checked selection logic, painful to evolve to Phase 1
  auto-swap.
- **Reuse cave-upstream-watchd.** Rejected — the watch daemon's
  signal model is "Git release atom feed → mark project stale"; the
  tracker's signal is "rank N candidates against a baseline using
  bench results", which is a different shape.
- **Auto-swap from day one.** Rejected — local model swaps cost
  GiBs of disk + warm-up time + benchmark drift. Burak wants a
  human-in-the-loop apply step for at least one sprint.

## Charter v2 stamp

- 8 gates checked in `crates/cave-llm-tracker/tests/parity_self_audit.rs`.
- `fill_ratio = 1.0000` honest under the workspace formula
  `(mapped + partial + skipped) / total = (11 + 0 + 6) / 17`.
- 4-track Backend + cavectl ships; Portal + Obs deferred per
  `PARITY_REPORT.md §7`.

## Promotion to Phase 1

See `crates/cave-llm-tracker/PARITY_REPORT.md §8` for the promotion
criteria. An amendment to this ADR will record the apply path and
audit-log location when Phase 1 lands.
