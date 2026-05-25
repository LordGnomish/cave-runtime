# cave-llm-tracker — PARITY_REPORT

> Charter v2 close-out — multi-source aggregator
> snapshot pin: **2026-05-21**
> last_audit:   **2026-05-21**
> fill_ratio:   **1.0000** (workspace formula = (m + p + s) / total = (11 + 0 + 6) / 17)
> honest_ratio: **0.6471** (mapped / total = 11 / 17)

## §0 — TL;DR

cave-llm-tracker is the daily always-latest tracker for the local-LLM
seat. Burak runs `qwen3.6:35b-a3b-coding-mxfp8` today; this crate polls
HuggingFace / Ollama library / LMSys leaderboard / vLLM+llama.cpp+MLX-LM
releases, runs five cave-specific eval prompts against each viable
candidate, and writes a `daily-<date>.{md,json}` report. **Phase 0
ships report-only — no automatic baseline swap.**

## §1 — Upstream pin (multi-source)

| source                  | pin                                                          |
|-------------------------|--------------------------------------------------------------|
| HuggingFace `/api/models` | `2026-05-21` snapshot                                      |
| Ollama library index    | `2026-05-21` snapshot                                        |
| LMSys leaderboard CSV   | `2026-05-21:elo_results.csv`                                 |
| vLLM                    | `vllm-project/vllm@2026-05-21`                               |
| llama.cpp               | `ggml-org/llama.cpp@2026-05-21`                              |
| MLX-LM                  | `ml-explore/mlx-lm@2026-05-21`                               |

`parity.manifest.toml::[upstream] source_sha` records the same set as
an inline TOML table so the workspace parity-index can read it without
extra plumbing.

## §2 — Coverage by subsystem

11 mapped, 0 partial, 6 skipped, 0 unmapped, total 17.

### Mapped (11)

| subsystem                       | location                                                      |
|---------------------------------|---------------------------------------------------------------|
| HuggingFace registry            | `src/registry.rs::LiveFetcher::fetch_huggingface`             |
| Ollama library registry         | `src/registry.rs::LiveFetcher::fetch_ollama_library`          |
| LMSys leaderboard registry      | `src/registry.rs::LiveFetcher::fetch_lmsys + parse_lmsys_csv` |
| GitHub backend registry         | `src/registry.rs::LiveFetcher::fetch_github_backend`          |
| Seed catalog (in-binary floor)  | `src/registry.rs::seed_catalog` (>= 5 entries always)         |
| Aggregate poller + dedupe       | `src/poll.rs::poll_all + dedupe`                              |
| Cave-specific bench harness     | `src/bench.rs::run_bench + cave_prompts + score_response`     |
| Selection / Verdict logic       | `src/selection.rs::evaluate + Verdict + SelectionStatus`      |
| Daily report emitter (md+json)  | `src/report.rs::DailyReport`                                  |
| Tracker config + validate       | `src/config.rs::TrackerConfig`                                |
| CLI binary (clap, 4 modes)      | `src/bin/cave-llm-tracker.rs`                                 |

### Skipped (6) — formal scope cuts

| subsystem                           | defer_to                                  |
|-------------------------------------|-------------------------------------------|
| auto_swap orchestration             | Phase 1 — cave-llm-tracker-phase-1        |
| portal admin page                   | cave-portal phase-2 llm-tracker tile      |
| observability dashboard             | cave-metrics + cave-dashboard phase-2     |
| multi-tier bench (CPU+GPU split)    | Phase 1 — bench matrix                    |
| VRAM/disk auto-probing              | Phase 1 — cave-llm-tracker-probes         |
| multi-day signal trending           | Phase 1 — cave-llm-tracker-trend          |

## §3 — Cave-specific bench suite

Five prompts; order is stable so historic reports diff line-for-line.

1. **Charter v2 close paperwork** — 8 gates by name in a markdown checklist.
2. **Parity manifest TOML** — `[upstream]` + `[parity]` blocks for a hypothetical port.
3. **Rust refactor** — replace `.unwrap()` with `?`-propagation.
4. **TR + EN dual** — two short paragraphs explaining the close-out audit.
5. **Conventional commit** — `feat(cave-llm-tracker):` for a new registry module.

Scoring is **deterministic** — keyword hits × length plateau. Quality is a
fraction in `[0.0, 1.0]`; reports are reproducible offline.

## §4 — Selection guards + uplift floors

- License must be in `Apache-2.0 / MIT / AGPL-3.0(-or-later)`.
- VRAM ≤ 64 GiB, disk ≤ 96 GiB (configurable).
- `speed_uplift_floor = 0.10` (10% throughput uplift) **OR**
  `eval_uplift_floor = 0.05` (5 quality-points uplift) flags an
  `UpgradeCandidate` — but **never** an auto-swap in Phase 0.

`TrackerConfig::validate()` hard-rejects `selection.auto_swap = true`.

## §5 — Scheduled run

LaunchAgent `~/Library/LaunchAgents/com.cave.llm-tracker-daily.plist`
fires at **03:00 Europe/Berlin** every day, invoking
`~/.local/bin/cave-llm-tracker --mode report --output
~/Library/Application\ Support/cave-runtime/llm-tracker/daily-$date.json`.

## §6 — Charter v2 8-gate stamp

| # | gate                                                                | pass |
|---|---------------------------------------------------------------------|------|
| 1 | TDD red → green                                                     | ✅ tests authored before bin smoke; `cargo test` green |
| 2 | SPDX AGPL header on every .rs file                                  | ✅ enforced by `tests/parity_self_audit.rs::gate_8` |
| 3 | multi-source `source_sha` inline-table                              | ✅ enforced by `tests/parity_self_audit.rs::gate_2` |
| 4 | no `unimplemented!()` / `todo!()` in `src/`                         | ✅ enforced by `tests/parity_self_audit.rs::gate_7` |
| 5 | no backcompat shims / `#[allow(deprecated)]` hatch                  | ✅ visual: zero deprecated paths; new crate, no prior surface |
| 6 | always-latest mandate (multi-source pin snapshot date stays current)| ✅ `last_audit = 2026-05-21` matches `version` matches lib constant |
| 7 | 4-track delivery (Backend + cavectl; Portal/Obs deferred per §7)    | ✅ backend lib + `cave-llm-tracker` bin + `cavectl` shell-out follow-up |
| 8 | honest fill (every subsystem classified mapped/partial/skipped)     | ✅ 0 unmapped; `fill_ratio = 1.0000` honest under workspace formula |

## §7 — Honest scope cuts (Portal + Obs)

Portal admin page and observability dashboard are not in this drop.
The JSON wire layout (`schema_version: 1`, stable field names) is
designed so the Phase-2 portal tile + Grafana ingest can land without
needing this crate to move. Both are tracked under the `[[skipped]]`
entries above.

## §8 — Phase 0 → Phase 1 promotion criteria

Promotion to Phase 1 (auto-swap unlocked, gated by `cavectl llm-tracker
apply`) needs:

1. ≥ 14 consecutive daily reports without panics or non-`Unknown`
   verdict regressions.
2. One concrete candidate that clears both uplift floors on three
   consecutive days, with the bench results manually spot-checked.
3. An ADR-152 amendment recording the apply path + audit-log location.

Until then, this crate **never** mutates `ollama` config, `~/.ollama`,
or any other on-disk model state.
