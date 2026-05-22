# Cave Runtime — OSS Release Plan

**Target date:** 21 May 2026 (Burak's 49th birthday)
**Archive snapshot:** 2026-04-22 (this document)
**Strategy:** Orphan-branch squash + archive (Option C)

---

## Release Strategy: Orphan-Branch Squash

All local platform-specific history stays in `legacy/pre-oss-archive-*` branches (never pushed).
The public OSS repo receives a single squashed initial commit on an orphan branch, containing
only the sovereign reimplementation code — no internal ADRs 001–143, no personal commit messages,
no tenant/infra identifiers.

### Why Option C

- Full git history contains ~178K lines of platform-specific context that has no value to OSS contributors.
- Squash gives a clean `git log` from day one on the public repo.
- Archive branches preserved locally as `legacy/pre-oss-archive-2026-04-22` (SHA: `0a64e4a`) and
  `legacy/pre-oss-archive-all-feature-branches` — recoverable forever, never pushed.

---

## Timeline

| Date | Event |
|------|-------|
| 2026-04-22 | Archive snapshot created; Qwen daemon live; queue seeded (129 items) |
| 2026-04-22 – 2026-05-19 | Qwen amele mode: 24/7 draft generation on `qwen/auto-*` branches |
| 2026-05-19 (Monday) | ADR audit + refactor complete; final Sonnet sweep |
| **2026-05-20 (Tuesday night)** | **Git history rewrite: squash → orphan → OSS prep** |
| **2026-05-21 (Wednesday)** | **Public OSS repo goes live — Burak's 49th birthday** |

### May 20 Night — History Rewrite Steps

```bash
# 1. Final archive snapshot
git branch legacy/pre-oss-final-2026-05-20 main

# 2. Create orphan branch with squashed code
git checkout --orphan oss/main
git add -A
git commit -m "feat: Cave Runtime — sovereign cloud OS in Rust (initial OSS release)"

# 3. Strip platform docs from OSS branch
# Remove: docs/adr/ADR-001 through ADR-143, docs/chain/, internal runbooks
# Keep: README, docs/adr/CHARTER-001, GOLDEN-001..004, ADR-144..166, LOCAL-LLM-001

# 4. Force-rename to main on empty OSS repo (never force-push to existing remote)
```

---

## ADR Disposition

### ADRs to INCLUDE in OSS release (~28)

| Group | Range | Content |
|-------|-------|---------|
| Charter | CHARTER-001 | OSS governance, contribution model |
| Golden Rules | GOLDEN-001, GOLDEN-002, GOLDEN-003, GOLDEN-004 | Core architectural invariants |
| Platform ADRs | ADR-144 through ADR-166 | Recent sovereign reimplementation decisions |
| Local LLM | LOCAL-LLM-001 | Tiered self-improving LLM workflow |

### ADRs to EXCLUDE (platform-specific, internal)

| Range | Reason |
|-------|--------|
| ADR-001 through ADR-143 | Platform-specific implementation details, tenant config, infra choices |

---

## Pre-Release Checklist

- [ ] All `todo!()` / `unimplemented!()` stubs resolved or wrapped in feature flags
- [ ] No hardcoded tenant identifiers, internal hostnames, or credentials
- [ ] `docs/adr/` pruned to OSS-safe set (28 ADRs)
- [ ] `README.md` updated for public audience
- [ ] `CONTRIBUTING.md` written
- [ ] `LICENSE` file present (Apache 2.0 recommended)
- [ ] CI workflow (GitHub Actions) added for `cargo test --workspace`
- [ ] `Cargo.toml` workspace metadata clean (no internal registry paths)
- [ ] `deploy/` directory reviewed — remove infra-specific manifests
- [ ] cave-local-llm Qwen queue drained or archived (not shipped in initial release)

---

## Notes

This document will be updated after the ADR audit sprint (targeted completion: 2026-04-23).
The Sonnet refactor dispatch loop (scheduled task) is separate from the Qwen amele daemon and
runs on a weekly cadence reviewing `qwen/auto-*` branch output.

> **Archive branches are LOCAL ONLY — never push `legacy/*` to any remote.**
