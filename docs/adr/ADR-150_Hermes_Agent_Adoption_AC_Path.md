# ADR-150: Hermes Agent Adoption (A+C Path)

**Status:** Accepted

**Date:** 2026-05-19

**Owner:** Burak (btartan@gmail.com)

**Scope:** cave-runtime host (Burak's macOS workstation), worktree-pump pipeline

**Category:** AI/LLM, Runtime Orchestration

**Related ADRs:** 013 (LiteLLM Gateway), 147 (Data Persistence + Naming)

---

## Context

The cave-runtime pump pipeline today executes via three independent macOS
launchd units acting in concert:

- `com.btartan.cave-upstream-watchd` — polls upstream feeds, surfaces
  `GAP_OPENED` events every 5 minutes.
- `com.caveruntime.qwen-pump` + `qwen-pump-refiller` — dispatches gap tasks
  into worktrees, where a local Qwen3 model performs refactor / smoke /
  small-edit work.
- `com.caveruntime.local-llm-daemon` + `homebrew.mxcl.ollama` — serve the
  local model (`qwen3.6:35b-a3b-coding-mxfp8`, 35B MoE, MXFP8 quant) over
  the OpenAI-compatible Ollama HTTP endpoint at `localhost:11434/v1`.

Two known gaps are blocking further scale:

1. **No plan + recovery layer.** When a worktree stalls, watchd-poller
   eventually notices, but there is no agent that can read pump state,
   reason about the failure, route a recovery task to a stronger model,
   and persist what it learned.
2. **No tiered model routing.** Every dispatch goes to the same local
   model. Architectural / cross-cutting work (cross-crate refactors,
   data-model migrations) gets the same treatment as a one-line rename,
   producing low-quality results on the hard cases and wasted local GPU
   time on the trivial ones.

A parallel `cave-hermes` Rust port effort (separate ray
`local_2d6e6009`) is underway in cave-runtime; this ADR covers only the
**host-side** Hermes Agent installation and pre-wiring, not the crate
work.

## Candidates Considered

| Criterion | Hermes Agent (Nous) | Continue.dev | Aider as daemon | Roll-our-own |
|---|---|---|---|---|
| MIT license | ✅ | Apache-2.0 | Apache-2.0 | n/a |
| Persistent memory + skills layer | ✅ built-in (memories/, skills/, SOUL.md) | ⚠️ chat-only | ⚠️ session-only | ❌ |
| Multi-provider tier routing | ✅ (custom + anthropic + ollama-cloud + nous etc.) | ⚠️ chat-only | ⚠️ | n/a |
| Headless daemon mode | ✅ (`-z` one-shot, gateway, cron, webhook) | ❌ IDE-bound | ⚠️ CLI loop | n/a |
| Local OpenAI-compatible backend | ✅ `provider: custom` + `base_url` | ✅ | ✅ | n/a |
| Zero telemetry by default | ✅ stated, no analytics keys in config | ⚠️ | ✅ | ✅ |
| Self-hosted | ✅ `~/.hermes/` only, no cloud lockin | ✅ | ✅ | ✅ |
| Maintenance burden for us | low (curl-bash install, pinned tag) | low | medium | high |

Continue.dev is IDE-bound and does not provide a headless orchestrator.
Aider is excellent as a per-task CLI but does not have persistent memory
or a plan/recovery layer. Rolling our own would duplicate work that
Hermes already does well.

## Decision

Adopt **Hermes Agent v0.14.0 (2026.5.16)** on the macOS host as the
**A+C path**:

- **A** — host-side orchestrator, sitting **above** the existing pump.
  Reads pump state, can route tasks to the right tier, persists
  dispatch history in `~/.hermes/memories/`.
- **C** — the parallel `cave-hermes` Rust crate port (separate ray)
  remains independent; this ADR does not commit to that direction.

### Install profile

- Pinned to release tag `v2026.5.16` (latest stable at 2026-05-19).
- Source: official `NousResearch/hermes-agent` repo, MIT, downloaded via
  `scripts/install.sh` (SHA-256 `ade99101...` verified locally before
  invocation).
- Headless install flags: `--skip-setup --skip-browser` — bypasses the
  interactive wizard (which would prompt for API keys, an explicit red
  line for the operator), and skips Playwright/Chromium (browser
  automation is not part of the cave-runtime use case).
- Binary: `~/.local/bin/hermes` (shim → venv inside
  `~/.hermes/hermes-agent/`).
- Config: `~/.hermes/config.yaml`; secrets: `~/.hermes/.env` (chmod 600).

### Tier routing (configured in `~/.hermes/config.yaml`)

| Tier | Provider | Model | Use case | Status |
|---|---|---|---|---|
| 1 | `custom` (Ollama OpenAI-compat, `http://localhost:11434/v1`) | `qwen3.6:35b-a3b-coding-mxfp8` | refactor, small-edit, smoke, format | **ACTIVE** |
| 2 | `anthropic` | `claude-sonnet-4.6` | feature, multi-file, new-module | **PRE-WIRED, DISABLED** — flips when `ANTHROPIC_API_KEY` is present in `.env` or keychain |
| 2 | `anthropic` | `claude-opus-4.7` | architectural, cross-cutting, design | **PRE-WIRED, DISABLED** — same |
| 3 | OpenRouter / others | — | — | **NOT WIRED** — explicit operator decision to skip |

Provider timeouts captured per tier (Opus gets 900s headroom for
extended-thinking turns; Qwen local gets 600s to absorb cold-start cost
on 35B MoE warmup).

### Telemetry

Explicitly disabled in `config.yaml` (`telemetry.enabled: false`,
`analytics.enabled: false`). Hermes does not currently honour these
keys — they are no-ops — but they are present as defensive future-proofs
in case a future release adds opt-in analytics surfaces.

### Pump integration

A read-only bridge script ships in this commit at
`scripts/hermes-pump-bridge.sh` (chmod 644, NOT executable on disk).
The bridge:

- Enumerates pump unit liveness via log-file mtime (no `launchctl`
  writes).
- Identifies units idle beyond `STUCK_THRESHOLD_SECONDS` (default
  900s).
- Optionally routes recovery tasks to the right tier via `hermes -z`.
- Persists dispatch records into `~/.hermes/memories/pump-dispatch/`.
- **`DRY_RUN=1` by default** — all write paths are gated. Burak flips
  this manually after he has verified the bridge against his live
  pump state.

A companion launchd plist ships at
`~/Library/LaunchAgents/com.cave.hermes-orchestrator.plist` with
`Disabled=true` and `RunAtLoad=false`. The plist is NOT loaded at
install time; Burak `launchctl bootstrap`s it manually once the bridge
is validated.

### Coexistence guarantee

The orchestrator sits **alongside**, never replaces, the existing pump
units. Adoption of this ADR explicitly does **not** retire:

- `com.btartan.cave-upstream-watchd`
- `com.cave.upstream-watchd-poller`
- `com.caveruntime.local-llm-daemon`
- `com.caveruntime.qwen-pump` + `qwen-pump-refiller`
- `com.cave.ollama-safe-upgrade`
- `homebrew.mxcl.ollama`

A future ADR will decide whether to deprecate any of these once the
orchestrator has shadowed the pump for ≥1 week without regression.

## Consequences

### Positive

- A real plan + recovery + memory layer exists on the host for the
  first time. Stuck-state detection moves from "Burak notices on next
  laptop wake" to "Hermes notices within `STUCK_THRESHOLD_SECONDS`".
- Tiered routing becomes a config edit, not a fork. The moment Burak
  drops `ANTHROPIC_API_KEY` into `.env`, hard tasks start reaching
  Sonnet / Opus with no further plumbing.
- The agent's persistent memory builds a per-task ↔ worktree ↔ commit
  audit trail under `~/.hermes/memories/`, which is also useful for
  RLHF / fine-tuning data export down the line (Hermes's own use case).
- Self-hosted, MIT, zero-telemetry — matches CAVE's stance on tooling
  sovereignty (`ADR-001`).

### Negative

- Another moving piece on the host. The plist is disabled at install
  time precisely so Burak can introduce it deliberately rather than
  inheriting an extra always-on daemon.
- Hermes pins us to its release cadence (v0.14.0 release is already
  539 commits behind `main` at install time — fast moving). Upgrade
  policy: re-run `scripts/install.sh --branch <tag>` per release; do
  not auto-update. Mirror or fork if we ever need to freeze.
- Tier-2 latency / cost is on the operator. Until Burak adds budget
  controls (currently out of scope) Hermes will happily route every
  `architectural` task to Opus.

### Neutral

- Existing pump units are not modified by this ADR. If we later
  decide to deprecate `qwen-pump-refiller`, that decision lives in a
  follow-up ADR.

## Rollback

1. `launchctl bootout gui/$UID/com.cave.hermes-orchestrator` (no-op if
   never bootstrapped).
2. `rm ~/Library/LaunchAgents/com.cave.hermes-orchestrator.plist`.
3. `~/.local/bin/hermes uninstall` OR `rm -rf ~/.hermes ~/.local/bin/hermes`.
4. Revert this commit. Pump continues running as before — nothing in
   the existing pipeline depends on Hermes.

## Operator follow-ups (Burak, when back at the keyboard)

1. Put `ANTHROPIC_API_KEY` into the macOS keychain (preferred) or
   `~/.hermes/.env` (chmod 600 already). Do **not** export it in
   `~/.zshrc` — Hermes reads `.env` directly.
2. Flip `model.provider` from `custom` to `anthropic` (and `model.default`
   from `qwen3.6:...` to `claude-opus-4.6`) only when you want Tier-2
   to be the *default* route. Otherwise leave Tier-1 as default and let
   the bridge script's `route_task` make per-task decisions.
3. `chmod +x scripts/hermes-pump-bridge.sh` once you've validated the
   `status` and `stuck` subcommands against live pump state.
4. `launchctl enable gui/$UID/com.cave.hermes-orchestrator` then
   `launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.cave.hermes-orchestrator.plist`.
5. Watch `~/Library/Application Support/cave-runtime/hermes-orchestrator/orch.log`
   for one week. If nothing regresses, open the follow-up ADR to
   discuss deprecating any redundant pump unit.

## References

- Hermes Agent landing: <https://hermes-agent.org/>
- Source: <https://github.com/NousResearch/hermes-agent>
- Release pinned: `v2026.5.16` (Hermes Agent v0.14.0)
- Install script SHA-256: `ade99101ec9bde981919a38b4c486123dcc341b5f33fc2e75e22e4e306835299`
