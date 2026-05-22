# Qwen-Pump Capability Matrix (2026-05-04)

Per-mode classification of what `qwen3.6:35b-a3b-coding-mxfp8` (via Ollama 0.23.0,
MLX backend, MXFP8 quant, `think:false`) can reliably produce in a one-shot pump
cycle. Honest categorization based on this session's measurements + structural
reasoning about the codebase.

## Output-format axis (the most important variable)

The single largest factor in pump-cycle success is the **output format Qwen is
asked to produce**:

| Format | Reliability | Why |
|---|---|---|
| **Full file replace** (whole `tests/qwen_drafted.rs` body) | ✅ HIGH | No line-number arithmetic; Qwen writes the file as if from scratch. Validated 15/15 in Mode B-prime pilots. |
| **Unified diff** (`--- a/x +++ b/x @@ -1,3 +1,4 @@`) | ❌ LOW | Qwen invents hunk line numbers, wraps in markdown fences, mis-references self-imports. **0/1** in Mode B real-impl pilot. `git apply --check` rejects. |
| **Search/replace blocks** (aider-style) | ⚠️ UNTESTED | Plausible middle ground. Not validated this session. |

**Workflow rule**: every new mode MUST emit full files OR small additive snippets
(append-only) — never unified diffs.

## Modes (proven / likely / shelved)

### ✅ PROVEN — full file pattern, validated in pilots

| Mode | Input | Output | Gate sequence | Success rate | Leverage value |
|---|---|---|---|---|---|
| **B-prime: Test gen against existing impl** | `<crate>` `<pub API surface>` `<5 specific behavior cases>` (manually-specified) | Replaces `tests/qwen_drafted.rs` whole file | check + clippy(qwen_drafted-only) + fmt + **test** | **15/15 = 100%** (cave-status, cave-trace, cave-vault, all 5 tests/each pass) | High — every cycle adds 5 real test rows + 5 manifest `[[tests]]` entries; parity calculator advances honestly |

### ⚠️ HIGHLY LIKELY — same full-file pattern, untested but structurally similar

| Mode | Input | Output | Gate sequence | Forecast success rate | Leverage value |
|---|---|---|---|---|---|
| **E: Rustdoc gen** | `<crate>` `<file path with undocumented pub items>` `<existing item signatures>` | Full file with `///` doc comments added before each pub item; preserves all impl bodies verbatim | `cargo check + cargo doc --no-deps -p <X>` (zero warnings) | 80-95% — rustdoc is well-bounded, models trained on Rust have seen tons of doc examples | Medium-high — improves docs.rs landing page + IDE hover; OSS launch credibility |
| **F: Error message polish** | `<crate>` `<existing thiserror::Error / Display impl>` | Full file with improved Display strings (kept variant names) | `cargo check + cargo test -p <X>` | 70-85% — bounded surface, low risk of breaking anything | Medium — UX/ergonomics for downstream consumers |
| **G: Module docstring** | `<crate>` `<file path, usually src/lib.rs>` `<list of pub items + brief crate summary>` | Full file with `//!` module-level doc block prepended | `cargo doc --no-deps -p <X>` zero warnings | 85-95% — single header, low complexity | Medium — docs.rs landing page |
| **H: Per-crate README** | `<crate>` `<upstream ref>` `<pub API list>` `<charter purpose>` | New file `crates/<X>/README.md` (markdown) | `markdownlint` (optional) + commit | 90%+ — pure markdown, no compile gate | High — OSS-launch shop window; each crate has its own short page |
| **I: Cargo.toml metadata** | `<crate>` `<existing Cargo.toml>` `<charter / upstream-name>` | Full file with `description`/`keywords`/`categories`/`license`/`repository` fields filled | `cargo check -p <X>` + `cargo metadata --no-deps` succeeds | 90%+ — TOML is structured, narrow surface | Medium-high — required for crates.io publish + docs.rs |

### ❌ SHELVED — measured failure or structural impossibility

| Mode | Why shelved | Concrete signal |
|---|---|---|
| **B real-impl (diff-mode)** | Qwen 35B-A3B cannot produce correct unified diffs. Multi-file edits via `--- a/+++ b/@@` headers fail at `git apply --check` 0/1. | Hunk line numbers invented; `\`\`\`diff` markdown fences wrap output; self-imports reference parent module. |
| **B real-impl (full-file replace, untested)** | Plausible alternative path but not budgeted this session. Risk: src/ corruption if test/impl coherence missing. Defer to Sonnet/Opus + human pair. | n/a |
| **D: Cross-cutting refactor** | Workspace-wide pattern detection requires multi-file context that exceeds Qwen's effective coverage in 32k ctx. | n/a |

## Pump cycle dispatch rule (proposed)

Each crate's `parity.manifest.toml` carries a `pump_mode` field. Round-robin via:

| Crate state | Recommended mode |
|---|---|
| `tests/qwen_drafted.rs` empty / placeholder bodies | Mode B-prime (real test gen) |
| Has tests, < 30% pub items have `///` rustdoc | Mode E (rustdoc) |
| Has tests + rustdoc, but no `crates/<X>/README.md` | Mode H (README) |
| Has README, but `Cargo.toml` description/keywords/categories empty | Mode I (Cargo metadata) |
| Has all of the above | Mode F (error polish) or Mode G (lib.rs head) |

**Selection heuristic in `run-cycle.sh`**: at cycle start, `discover_mode <crate>`
inspects state and picks the highest-leverage mode that still has work to do.
Manifest `pump_mode` field overrides if explicitly set.

## OSS launch projection (17 days, 80 crates)

Realistic throughput per Burak's vision: each cycle ~25-30s Qwen + 30-60s gates
= 1 minute average. With ThrottleInterval=300s, **~12 cycles/hour**. At 18h/day
× 17 days = **3672 cycles available**.

If we keep the priority queue + multi-mode dispatch:

| Mode | Cycles needed (80 crates × N) | Wall-clock |
|---|---:|---:|
| Mode B-prime | 80 × 1 = 80 | 7h |
| Mode E rustdoc | 80 × 1 = 80 | 7h |
| Mode H README | 80 × 1 = 80 | 7h |
| Mode I Cargo.toml | 80 × 1 = 80 | 7h |
| **Subtotal** | **320 cycles** | **~28h** |

Easily fits in the 3672-cycle budget. Plenty of headroom for retries + Mode F/G
fillers.

**Net OSS-launch deliverable**: ~400 real tests + ~640 doc comments + 80 README
files + 80 metadata-complete Cargo.toml files. Plus parity manifest `[[tests]]`
incremented.

## What this matrix does NOT cover

- **Real impl gaps** (TLS handshake, JWKS+RS256, etcd client, subresource
  modelling): Qwen-shelved. Sonnet/Opus + human pair sprints.
- **Architectural decisions**: ADR drafting / charter changes — never delegated.
- **Cross-crate refactors**: Mode D shelved; Sonnet/Opus or hand.
- **Security-sensitive crypto code**: defense in depth — even if Qwen produced
  it, requires human security review before merge.

## Revision policy

This matrix is a snapshot. Each pump-mode validation run that produces concrete
success/fail data should update the relevant row's "success rate" column with
the new sample size. Update via PR / commit, not in-session edits.
