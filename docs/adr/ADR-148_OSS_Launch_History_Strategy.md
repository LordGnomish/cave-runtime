# ADR-148 — OSS Launch History Strategy

**Status:** Accepted
**Scope:** Cave Runtime (repo-wide)
**Category:** Release / Hygiene
**Date:** 2026-05-02
**Decided:** Burak Tartan, 2026-05-02
**OSS launch target:** 2026-05-21

## Context

Cave Runtime is a 6-day-old prototype repo carrying a working tree of substantial value alongside a commit history that was never built for public consumption.

**Current shape (`main` at 2026-05-02):**

| Metric | Count | Notes |
|---|---:|---|
| Total commits | 1356 | from `ac517587` initial commit on 2026-04-26 to today |
| Qwen-amele commits | 454 | local-LLM-generated test scaffolds (`#[ignore]`'d, pump cycle artefacts) |
| Claude `Co-Authored-By` | 297 | session pair-programming trailers |
| Sonnet mentions | 4 | early background-agent commits |
| Implicit Burak commits | ~601 | committer where no AI co-author is named |
| Repo size on disk | ~hundreds of MB | including `.git/` packs, blob churn from refactors, deleted-then-readded files |
| `Co-Authored-By` email forms | 1 + many internal | `noreply@anthropic.com` plus stray developer-email leakage |

The history contains real engineering value — the sweep-001/002 refactor narrative, Wave 1/3a manifest fills, CRI/etcd/apiserver parity sprints — but it also contains shapes the project must not ship publicly:

1. **Path leakage.** Many commit messages, scripts, CI snippets, and inline comments embed `/Users/gnomish/...` paths. Useful local; trivial fingerprint when public.
2. **Email leakage.** `btartan@gmail.com` appears in committer fields throughout, plus other developer emails embedded in comments / test fixtures.
3. **Co-Authored-By noise.** Claude/Qwen/Sonnet trailers are not "co-authors" in the SPDX legal sense; they are pairing markers. Public viewers will infer either too much (Anthropic owns the code) or too little (no human responsibility).
4. **Private-repo URLs and secrets-shaped strings.** Even after `gitleaks` cleanup, private-repo refs may persist in commit messages or sample config snippets.
5. **Reordering churn.** 454 qwen-amele scaffold merges + sweep retrofits create a history that reads as "AI thrash" rather than "human-led system design with AI assistance," misrepresenting the actual contribution distribution.

The 1000+ commit prototype-shaped history is **not the artifact we want to publish on day one**. The working tree at HEAD is.

## Decision

**Adopt orphan-commit + force-push as the OSS launch history strategy.** On 2026-05-21 we publish a *new initial commit* on `main`:

```sh
# At launch time, on a separate working copy:
git checkout --orphan public-launch
git add -A
git commit -m "v0.1.0: initial public release of Cave Runtime"
git branch -M public-launch main
git push --force origin main
git tag -a v0.1.0 -m "Initial public release"
git push --force --tags origin v0.1.0
```

The 1356-commit history is **not preserved in the public remote**. It survives in a local archive (see Faz 3) and in human memory; it is not part of the OSS contract.

## Alternatives considered

| Option | Why rejected |
|---|---|
| **Keep full history** | Path/email/Co-Authored-By leak surface too high. 6 days of cleanup before launch is unrealistic given a 1356-commit log; even with `git filter-repo` we'd be picking through every blob. The launch ships the *artifact*, not the *journal*. |
| **`git filter-repo` rewrite (path/email scrubbing in place)** | Slow on 1356 commits + ~hundreds of MB packs (~30-60 min per pass). Each new leak class found means another pass + force-push. Co-Authored-By trailers are textual in commit bodies; scrubbing them either leaves stub trailers or rewrites authorship metadata, both messy. Tried-and-true tool, but the *output* — a force-pushed rewritten history — is identical in trust to an orphan commit, and gets there with much more risk of partial leak slipping through. |
| **Squash all commits into one** | Same metadata leak surface as filter-repo: the squashed commit's message is concatenated from prior messages by default; opting out means a manually-written long message that misrepresents authorship. And the squash still shows a single committer with a single date, which is closer to orphan than to "preserved history" anyway — orphan is the cleaner version of the same idea. |
| **Public mirror with sanitized branch, private with full** | Operational burden: two branches, two trust surfaces, drift inevitable. Easier to declare the public artifact the source of truth and keep the prototype log local. |

The orphan strategy gets the **same trust outcome** as any cleanup option (clean history at the public remote), with the **least risk of partial leak** and the **least implementation cost** (one shell snippet, ~5 minutes on launch day).

## Consequences

### Positive

- **One canonical artifact.** The public history starts at `v0.1.0` and grows from there. Future contributors see the codebase, not the messy prototype journey.
- **Zero residual path/email leak.** No `git filter-repo` corner-case worries — the orphan commit has no parent whose objects could survive.
- **Honest launch framing.** "v0.1.0: initial public release" is exactly true. The 6-day prototype is not erased — it is local engineering provenance, separate from the public contract.
- **Authorship clarified via `CREDITS.md`** (see Faz 2, 3) instead of muddied via 297 Co-Authored-By trailers.

### Negative / accepted

- **Public history loses the prototype narrative.** The sweep-001/002 refactor reasoning, Wave 3a Tier-A close-out arc, the CRI deep-port sprint — all visible only in local archive. Mitigation: a `docs/HISTORY.md` summary at v0.1.0 lists the milestone arcs in prose, with internal ADR refs.
- **No "git blame" continuity past v0.1.0 for any line.** Acceptable: blame on a 6-day-old prototype isn't load-bearing for OSS contributors.
- **External contributors who want to see "why was it written this way?" must read ADRs**, not `git log -p`. ADR-001 through ADR-148 stay; that's the explicit decision record.
- **Any OSS launch follower who clones before vs after force-push will diverge.** Mitigation: force-push happens during a single 5-minute window on launch day, before public announcement. Pre-announcement clones are zero.

### Out of scope

- This ADR does not change the local `.git/` of any machine. Burak's local prototype repo retains its full history.
- This ADR does not address ongoing post-launch attribution. Once `main` is the public artifact, future commits use real `Co-Authored-By` trailers (or none) per project policy, and `CREDITS.md` is appended.
- This ADR does not waive Faz 1 hygiene: working-tree contents at launch must still be free of secrets / private paths / personal emails. Orphan commits don't sanitize file content, only history.

## Implementation plan

### Faz 1 — Hygiene sprint (2026-05-14 → 2026-05-19, **5 days**)

Working-tree-level cleanup that benefits whether we orphan or filter:

- **Secret scan.** `trufflehog` + `gitleaks` against working tree (not history). Triage hits, scrub or move to `.env.example` placeholders.
- **Path scrubbing.** `rg -l '/Users/gnomish'` across `crates/`, `docs/`, `scripts/`, configs. Replace with `$HOME` / `~/` / `<repo>/` placeholders or env-var lookups.
- **Email scrubbing.** Personal addresses in test fixtures, comments, `.gitmessage` templates, CI configs → replace with `noreply@example.com` / `<contributor>` placeholders.
- **`.DS_Store` purge.** `find . -name '.DS_Store' -delete`.
- **Build artefact purge.** Confirm `target/`, `node_modules/`, `dist/`, `.cache/` listed in `.gitignore` and not committed; `git rm --cached` if any slipped.
- **Compliance sweep.** Add `LICENSE` (Apache-2.0), `NOTICE`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`. Confirm every crate's `Cargo.toml` has `license = "Apache-2.0"` and `repository = "https://github.com/cave-runtime/..."` (placeholder until org name finalized).
- **Output:** `docs/oss-prep/hijyen-report-2026-05-19.md` — list of leaks scrubbed, files touched, residual risk acknowledged.

The dry-run script (this ADR's deliverable, see `scripts/oss-hijyen-prep.sh`) inventories the scope today; the sprint executes the scrubs in increments with `cargo check --workspace` green at each step.

### Faz 2 — Orphan launch (2026-05-21, **launch day**)

Single canonical sequence, run from a clean working copy on a verified branch:

```sh
# 1. Final hygiene sanity check (idempotent).
bash scripts/oss-hijyen-prep.sh --strict --output docs/oss-prep/hijyen-report-launch.md
# Expectation: zero leaks remaining; otherwise abort and re-run Faz 1.

# 2. Confirm working tree is exactly what we want public.
cargo check --workspace
cargo test --workspace --lib --no-fail-fast --exclude cave-upstream
# (cave-upstream excluded because of pre-existing unsafe-block edition-2024 lint;
#  fix as part of Faz 1 sprint.)

# 3. Create the orphan branch from the current tree.
git checkout --orphan public-launch
git add -A
git commit -m "$(cat <<'EOF'
v0.1.0: initial public release of Cave Runtime

Cave Runtime is a sovereign Cloud OS reimplementation: a single
Rust workspace that mirrors the OSS stack a Hetzner deployment
needs (Kubernetes core, networking, observability, data
persistence, identity, supply chain) into one cave-native runtime.

This is the v0.1.0 artifact. See:

  - README.md       — what's inside
  - docs/adr/       — every architectural decision (ADR-001..148)
  - docs/HISTORY.md — six-day prototype timeline (summary; full log
                     in private archive per ADR-148)
  - CREDITS.md      — humans + AI co-authors of this artifact
  - LICENSE         — Apache-2.0

Co-Authored-By: Cave Runtime contributors <noreply@cave-runtime.org>
EOF
)"

# 4. Replace main with the orphan; tag and force-push.
git branch -M public-launch main
git tag -a v0.1.0 -m "Initial public release of Cave Runtime"
git push --force origin main
git push --force origin v0.1.0
```

The launch announcement (blog / hn / etc.) goes out only **after** the push completes and a fresh clone has been verified. Any pre-clone is by definition pre-public.

### Faz 3 — Local archive (2026-05-21, **immediately after launch push**)

The 1356-commit prototype is engineering provenance worth keeping privately:

```sh
# Bundle the full prototype history before any reset on the local repo.
git bundle create \
  ~/Documents/cave-runtime-prototype-2026-04-26-to-2026-05-21.bundle \
  --all

# Verify the bundle is readable.
git bundle verify \
  ~/Documents/cave-runtime-prototype-2026-04-26-to-2026-05-21.bundle

# Hash it for archival integrity.
shasum -a 256 \
  ~/Documents/cave-runtime-prototype-2026-04-26-to-2026-05-21.bundle \
  > ~/Documents/cave-runtime-prototype-2026-04-26-to-2026-05-21.bundle.sha256
```

The bundle goes to encrypted long-term storage. The local working repo can then `git fetch --all --prune` against the now-public `origin` and re-anchor at v0.1.0, preserving working tree but discarding the local prototype reflog.

`docs/HISTORY.md` (written in Faz 1, lands in v0.1.0) carries the prose summary of what the prototype-era arcs were, so post-launch contributors have context without the bundle:

```
v0.1.0 — 2026-05-21 — initial public release.

Six days of pre-public engineering produced this artifact:
- 26-29 Apr: bootstrap + initial 60-crate workspace shape
- 29 Apr - 1 May: 17 modules to calculator-100% parity
- 1-2 May: sweep-002 primitive extraction (consensus / eventbus /
  reconcile / identity / ns) into cave-kernel + adoption across
  6+ crates
- 2 May: 4-track audit + Tier-A close-out for Kubernetes core,
  data persistence, auth.

Bundle: cave-runtime-prototype-2026-04-26-to-2026-05-21.bundle
(sha256 in private archive; not part of the OSS contract.)
```

## Verification gates

A launch is **not** valid unless:

1. `bash scripts/oss-hijyen-prep.sh --strict` exits 0 (no path/email/secret leaks).
2. `cargo check --workspace` is green at the orphan-commit tree.
3. `cargo test --workspace --lib --no-fail-fast --exclude cave-upstream` failed-count is 0.
4. `LICENSE`, `NOTICE`, `CREDITS.md`, `README.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, `docs/HISTORY.md` all exist.
5. The orphan commit's message references this ADR (ADR-148).
6. The bundle (Faz 3) is created **and** verified **before** the local repo is reset.

If any gate fails, the launch slips and the gate is fixed first. No partial-orphan publication.

## References

- `scripts/oss-hijyen-prep.sh` — dry-run hygiene scanner produced alongside this ADR.
- `CREDITS.md` — authorship attribution; companion to this ADR.
- `docs/HISTORY.md` — to be written in Faz 1; lands in v0.1.0.
- `docs/oss-prep/hijyen-report-2026-05-02.md` — first dry-run output (this ADR session).
