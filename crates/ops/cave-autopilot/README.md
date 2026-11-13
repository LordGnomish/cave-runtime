<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-autopilot

A 7/24 autonomous code-generation orchestrator for the Cave Runtime monorepo —
the loop Burak asked for: *"lokal llm ile bir workflow çıkar güzel 7/24 hiç
durmadan çalışın."*

It reads tracker state (`docs/parity/parity-index.json`), ranks under-complete
subsystems into a priority queue, and dispatches each as a Charter-compliant TDD
port job through a tiered LLM escalation ladder. Charter compliance (strict TDD,
no stubs, honest LOC) is enforced before any local merge. **Push is never
performed by the daemon** — shipping to a remote is a human gate.

## Pipeline

```
  parity-index.json (tracker state)
        │  tracker::TrackerState::load → ranked Vec<Subsystem>
        ▼
  queue::TaskQueue  ──►  one under-complete crate
        │
        ▼  escalation::decide
  L1 router (Mellum2) → L2 coder (Qwen3-Coder-Next) → L3 Claude API → L4 human
        │
        ▼  worktree::WorktreeJob (+ codegen::FileSet)
  git worktree add → cargo build → cargo test → charter::audit → commit → merge --no-ff
        │
        ▼
  metrics (:9101/9102) + daily report (docs/audit/autopilot-daily-*.md)
```

## Tiered escalation

| Tier | Model | Role |
|------|-------|------|
| L1 | `mellum2:12b-moe` | route/analyse: pick the surface, size the context |
| L2 | `qwen3-coder-next:80b-moe` | local code-gen, test-first |
| L3 | Claude API (`claude-opus-4-7`) | escalation when local retries are spent |
| L4 | human (Burak) | architectural/strategic calls only |

Named models that aren't pulled fall back to the resident coding model
(`qwen3.6:35b-a3b-coding-mxfp8`) — a tier is never silently skipped.

## Stop conditions

* **disk < 5 GiB** → `Halt` (notify human)
* **all subsystems ≥ idle threshold** → `Idle` (monitor only)
* **Claude daily token budget spent** → `LocalOnly` (no L3)
* otherwise → `Active`

## Usage

```bash
cargo build --release -p cave-autopilot
cp target/release/cave-autopilot ~/.local/bin/

# print + edit config, then install the LaunchAgent (RunAtLoad + KeepAlive)
cave-autopilot init-config --instance cave-runtime > ~/.config/cave-autopilot/cave-runtime.toml
cave-autopilot install --instance cave-runtime --binary ~/.local/bin/cave-autopilot
cave-autopilot install --instance cave-home    --binary ~/.local/bin/cave-autopilot

# verify
launchctl list | grep cave-autopilot
curl localhost:9101/metrics
curl localhost:9101/healthz

# pull the tiered models (large; review first)
cave-autopilot setup-script --instance cave-runtime > /tmp/ollama-setup.sh && bash /tmp/ollama-setup.sh

# one-shot diagnostics
cave-autopilot once   --instance cave-runtime    # read tracker, print ranked queue
cave-autopilot mock   --instance cave-runtime    # end-to-end: scaffold→test→commit→merge
cave-autopilot report --instance cave-runtime    # write today's daily report
cave-autopilot uninstall --instance cave-runtime # unload + remove the LaunchAgent
```

Two instances run in parallel: cave-runtime on :9101, cave-home on :9102, each
pointed at its own repo + tracker output.

## Scope (this ray = foundation)

Implemented and tested end-to-end: tracker reader, priority queue, escalation
policy, Ollama + Claude clients (pure request/response), Charter gate, worktree
build/test/commit/merge, metrics + health, daily report, LaunchAgent
install/uninstall, the deterministic mock task, and the daemon loop with stop
conditions.

**Deferred (multi-week):** the live LLM-driven dispatch inside the tick loop —
turning a real `parity.manifest.toml` gap into an applied, test-passing
`FileSet`. The clients, prompts, FileSet contract, and the worktree/charter/merge
pipeline it will drive are all in place and exercised by the mock task; wiring
them into the tick is the production hardening phase.
