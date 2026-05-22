# ADR-CONTRIB-ATTRIBUTION-001: commit author + trailer attribution

**Status:** Accepted
**Date:** 2026-04-23
**Author:** Burak Tartan (raised model-split question), Sonnet/Opus (scribe)
**Scope:** Universal (OSS-hygiene; per-commit attribution)

## Context

Cave Runtime is assembled by a heterogeneous set of contributors: the human project lead, multiple Claude models (Opus, Sonnet, Haiku) across sessions, a local Qwen3-Coder-Next "amele" daemon, and — post-OSS — community contributors. Because the contribution mix informs both credit and quality tracking, every commit needs machine-readable provenance.

## Decision

Every commit MUST carry enough metadata for the portal's `/api/v1/attribution` endpoint to classify it correctly.

Rules:

1. **Human contributors** commit with their real name and email as the git author (configured once via `git config user.name` / `user.email`).
2. **Qwen3-amele daemon** commits with author `cave-local-llm <cave-local-llm@localhost>` AND subject line prefixed `[qwen-amele] <scope>: …`. Both the author and the prefix are classification signals; either alone suffices but both are defence-in-depth.
3. **Claude-assisted commits** carry a `Co-Authored-By:` trailer naming the exact model and version, e.g.:
   - `Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>`
   - `Co-Authored-By: Claude Sonnet 4.7 <noreply@anthropic.com>`
   - `Co-Authored-By: Claude Haiku 4.5 <noreply@anthropic.com>`
   The model name is case-sensitive enough for the classifier; the version string is informational.
4. **Mixed-authorship commits** (human + model or model-A + model-B) list every contributor in order of material contribution. The first `Co-Authored-By` is treated as the primary.
5. **Merge commits** are housekeeping; the classifier buckets them under `other` regardless of author.

## Rationale

- OSS attribution is a first-order honesty concern. "Who wrote what" must be recoverable from the history.
- Model split helps quality forensics: if a class of bug correlates with a specific model, we learn.
- Trailer-based attribution is git-native, renders in GitHub UI, survives rebases, and is picked up by every code-forensics tool.

## Consequences

- CONTRIBUTING.md requires the trailer for LLM-assisted PRs.
- The classifier in `cave-portal/src/routes.rs::attribution_api` splits buckets accordingly.
- Retroactive backfill on historical commits is impractical; pre-2026-04-23 Claude commits remain in `claude_legacy`.

## References

- [CONTRIBUTING.md](../../CONTRIBUTING.md) — commit-message requirements
- `crates/cave-portal/src/routes.rs::attribution_api` — classifier implementation
- 2026-04-23 user question: *"Sonnet ve Opus ayrı ayrı yok mu?"* — yes, now we can split.
