<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Qwen3-Coder-Next MoE Pilot — Mac 64 GB

- **Date:** 2026-06-01
- **Operator:** Burak Tartan (`btartan@gmail.com`)
- **Hardware:** Apple Silicon (arm64), 64 GiB unified memory (`hw.memsize = 68719476736`)
- **Goal:** Evaluate `qwen3-coder-next` (MoE) as a candidate to replace the current
  local coding baseline, head-to-head vs. Qwen 2.5 Coder 32B, on tokens/sec and a
  SWE-bench quality sample; wire it into `cave-llm-tracker` if it wins.
- **Verdict (TL;DR):** **PILOT BLOCKED — DO NOT SWITCH PRODUCTION.** The machine is
  out of disk (215 MB free) and out of swap (16 MB free). The candidate model is
  ~48 GB and cannot be pulled; the model already on disk can no longer even load
  (the runner is SIGKILL'd by memory pressure). No honest benchmark or SWE-bench
  run is possible until ~70+ GB of disk is freed.

---

## Environment reality vs. task premise

The task brief was written against an outdated snapshot. Verified state on
2026-06-01:

| Premise in brief | Verified reality | Note |
|---|---|---|
| "Ollama 0.19 MLX upgrade" | `ollama version 0.24.0` already installed via Homebrew (`brew 5.1.14`) | Already **past** 0.19; a downgrade would be regressive. |
| "MLX" needs enabling | Ollama 0.24 ships an **MLX runner** already in use | The resident `mxfp8` model loads via `mlx runner` (confirmed in the crash log line). No 0.19 / no manual MLX build needed. |
| Compare vs. "Qwen 2.5 Coder 32B" | **Not installed.** Only model present: `qwen3.6:35b-a3b-coding-mxfp8` (37 GB) | No 2.5-Coder-32B locally; pulling it (~20 GB) is also blocked by disk. |
| `git checkout main` / `worktree add main` | `main` is **already checked out** in sibling worktree `/Users/gnomish/Code/cave-runtime-main` | The setup script cannot run verbatim; a second checkout of `main` is refused by git. |

---

## Blocker: disk + swap exhaustion (root cause)

```
/dev/disk3s5   926Gi   900Gi   215Mi   100%   /System/Volumes/Data   <- 215 MB free, shrinking
vm.swapusage:  total = 5120.00M  used = 5104.00M  free = 16.00M       <- swap can't grow (disk full)
```

- The data volume is **100 % full** (~215 MB free, and it dropped from 375 MB to
  215 MB during this session — the system is actively thrashing).
- macOS dynamic swap lives on that volume; with no free space the swapfile **cannot
  grow** (16 MB free of a frozen 5 GB).
- Loading any large model into unified memory therefore triggers jetsam, which
  **SIGKILLs the runner**. This is not a model defect — it is the host being out of
  resources.

This single fact gates every download/run-dependent step below.

---

## Step-by-step results

### 1. Ollama 0.19 MLX upgrade — ✅ N/A (already exceeded)
- `ollama 0.24.0` present (Homebrew). MLX runner already active for `mxfp8` models.
- **No action taken.** Downgrading to 0.19 would be a regression; the MLX goal is
  already met by the installed stack. AGPL/SPDX/sudo constraints not engaged (no
  install performed).

### 2. `ollama pull qwen3-coder-next` — ❌ BLOCKED (disk)
- Registry confirms the model **exists**:
  `GET registry.ollama.ai/v2/library/qwen3-coder-next/manifests/latest` → `200`,
  real v2 manifest. Model layer digest `sha256:30e51a7c…`, size **51 741 599 936 B
  (≈ 48 GiB total)**.
- Pull requires ≈ 48 GB; **215 MB free**. Physically impossible.
- I did **not** delete any user data to make room (out of scope; would be
  destructive and was not authorized).
- HF GGUF + Modelfile fallback: same blocker — the GGUF weights are the same
  order of magnitude and would not fit either.

### 3. Benchmark (tokens/sec) — ❌ BLOCKED (model won't load)
- Target pair (2.5-Coder-32B vs qwen3-coder-next): **neither installed**, neither
  pullable. No head-to-head possible.
- Attempted a free fallback: benchmark the resident `qwen3.6:35b-a3b-coding-mxfp8`
  (itself a Qwen3-family MoE coding model, 35B total / 3B active) to at least get a
  real hardware datapoint. Result:
  ```
  Error: 500 Internal Server Error: mlx runner exited unexpectedly: signal: killed
  ```
  The runner was SIGKILL'd while loading the 37 GB model — see the disk/swap root
  cause above. **No tokens/sec number can be reported honestly.** No fabricated
  figures are recorded here.

### 4. SWE-bench 10-task quality sample — ❌ BLOCKED
- Requires (a) a working inference model — none currently loads, and (b) the
  SWE-bench harness + per-task Docker images (many GB). Both are gated by the disk
  blocker. Not attempted; no synthetic scores recorded.

### 5. `cave-llm-tracker` TRACKED_PROJECTS + daemon preference — ⏸️ DEFERRED (intentionally)
- Tracker baseline is already `qwen3.6:35b-a3b-coding-mxfp8`
  (`crates/cave-llm-tracker/src/config.rs:96`, `registry.rs:98`). The curated
  candidate list lives in `crates/cave-llm-tracker/src/registry.rs`.
- **Not applied.** Per the KIRMIZI rule (do not steer toward a production switch when
  the candidate is slow/bad — here it is *un-runnable*), promoting `qwen3-coder-next`
  to a tracked/preferred model on this host would be wrong: it cannot load, so a
  daemon that prefers it would fail every poll.
- **Ready-to-apply candidate entry (apply only after disk is freed and a benchmark
  exists):** add to the curated registry list in `registry.rs` alongside the
  existing `qwen3.6:*` entries:
  ```rust
  CuratedModel {
      model_id: "qwen3-coder-next".to_string(),
      family: "qwen3-coder".to_string(),
      // ~48 GiB MoE; pilot candidate, NOT a baseline/preference yet.
  },
  ```
  Leave `config.rs` baseline and any daemon preference **unchanged** until a real
  tokens/sec + SWE-bench comparison justifies it.

### 6. Speculative (spec) decoding flag — ⏸️ DOCUMENTED, untestable here
- Ollama 0.24's `serve --help` exposes no first-class draft/spec-decoding flag
  (only `OLLAMA_CONTEXT_LENGTH`). Spec decoding for the MLX runner, where available,
  is driven by a **draft model** paired with the target model, not a CLI toggle.
- For an MoE like `qwen3-coder-next` (3B-class active params), speculative decoding
  typically yields **little benefit** — the active path is already small, so a draft
  model adds overhead for marginal acceptance gains. Recommended posture: **leave
  spec decoding off for the MoE candidate**; only revisit for dense models (e.g. a
  32B dense coder paired with a 0.5–1.5B draft).
- Cannot be measured on this host (no model loads). No flag flipped.

### 7. This document — ✅ written
- `docs/llm-pilot/2026-06-qwen3-coder-next.md` (this file), AGPL SPDX header.

---

## Recommendation

1. **Do NOT switch production to `qwen3-coder-next`.** No evidence supports it — the
   pilot could not run a single inference on this host. (Independent of model
   quality, the current host cannot serve it.)
2. **Fix the host first.** Free **≥ 70 GB** on `/System/Volumes/Data` (target the
   model cache `~/.ollama/models`, build artifacts `target/`, and Docker/VM images).
   At 215 MB free the box is thrashing and even the *current* baseline model no
   longer loads — this is an operational incident regardless of the LLM pilot.
3. **Then re-run the pilot, in order:** confirm `qwen3.6:35b-a3b-coding-mxfp8` loads
   again → `ollama pull qwen3-coder-next` (~48 GB) → optionally pull
   `qwen2.5-coder:32b` (~20 GB) for the head-to-head → tokens/sec with
   `ollama run --verbose` (eval rate) → SWE-bench 10-task sample → only then touch
   `cave-llm-tracker` registry/baseline if the candidate wins on both speed and
   quality.

## Reproduction notes / evidence commands

```bash
ollama --version                  # 0.24.0
ollama list                       # qwen3.6:35b-a3b-coding-mxfp8  37 GB
df -h /System/Volumes/Data        # 215Mi avail, 100%
sysctl vm.swapusage               # used 5104M / free 16M
curl -s registry.ollama.ai/v2/library/qwen3-coder-next/manifests/latest | grep size
                                  # model layer 51741599936 B (~48 GiB)
printf "..." | ollama run qwen3.6:35b-a3b-coding-mxfp8 --verbose
                                  # Error 500: mlx runner exited unexpectedly: signal: killed
```

## What was deliberately NOT done (per constraints)
- No `AskUserQuestion` (KIRMIZI). Decisions made with sensible defaults and recorded here.
- No force push; no commits/pushes made by this session (none requested).
- No `sudo`; no install/uninstall performed.
- No user data deleted to free disk.
- No production switch recommended; no daemon preference changed; no fabricated benchmark numbers.
