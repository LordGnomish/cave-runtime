# TDD coverage audit — cave-changelog vs towncrier 25.8.0

- Cave crate: `crates/ops/cave-changelog` (theme: ops)
- Upstream: https://github.com/twisted/towncrier @ 25.8.0
- Upstream test symbols inventoried: 177 (12 test files)
- Cave test fns: 17 (engine.rs 8, models.rs 5, tests/proptest_smoke.rs 4) — `store.rs` is **not** wired into `lib.rs` (no `pub mod store`), so it is dead code with no public surface.

## Architectural framing (honesty note)

towncrier is a **CLI news-fragment assembler**: it scans a `newsfragments/` directory, parses fragment
filenames into `(issue, category, counter)`, groups them by section/type, renders them through Jinja
templates, and writes the result into a `NEWS`/`CHANGELOG` file — with heavy git/hg/novcs VCS integration,
TOML config loading, and project-version discovery.

cave-changelog is a **tiny custom backend** with an entirely different model: it parses *conventional git
commit* strings into a `ChangeType`, filters/counts/sorts those changes, defines serde models, and exposes a
single `/api/changelog/health` route. It does **not** implement fragment-file parsing, templating, VCS
drivers, TOML config, version discovery, or a CLI. Per the crate header it is "git + SBOM diff based release
notes", a deliberately narrow subset.

Consequently the vast majority of upstream tests are **scope-cut** (CLI/VCS/config/templating/web-plumbing).
Only the category-classification + section-grouping/ordering behaviors have a portable analogue in cave, and
those are largely already tested. The honest finding is a **small** number of real gaps — untested branches
of already-implemented logic.

## Classification table

| Upstream test file | Behavioral unit | Classification | Cave mapping |
|---|---|---|---|
| test_builder.py `TestParseNewsfragmentBasename` (16) | parse filename → (issue, category, counter) | scope-cut (no fragment-file model in cave) — *but* the category-classification idea maps to `parse_commit` | partial analogue → `engine::parse_commit` |
| test_builder.py `TestNewsFragmentsOrdering::test_ordering` | deterministic ordering of fragments | portable analogue | `engine::sort_by_version` (covered) |
| test_build.py `test_section_and_type_sorting` | group/sort changes by section & type | portable analogue | `engine::filter_by_type` + `engine::count_by_type` (covered) |
| test_build.py (44 others: CLI runner, drafts, confirmation, keep-fragments, start-string, markdown, title-format, orphans, uncommitted/ignored files, toml arrays) | CLI `build` command behavior | scope-cut (no CLI / no fragment dir / no templating) | — |
| test_check.py (20) | `check` command, git staged/diff detection | scope-cut (VCS + CLI) | — |
| test_create.py (30) | `create` command, fragment file creation/editing | scope-cut (CLI + editor + filesystem) | — |
| test_format.py (8: split, markdown, issue_format, line_wrapping, trailing_block) | Jinja template rendering / line wrap | scope-cut (no templating engine in cave) | — |
| test_git.py / test_hg.py / test_novcs.py / test_vcs.py (17) | VCS driver: default branch, remove, changed files | scope-cut (no VCS abstraction in cave) | — |
| test_project.py (15) | project version fetching (str/tuple/incremental/metadata) | scope-cut (no version discovery in cave) | — |
| test_settings.py (17) | TOML config loading, template extension, custom types | scope-cut (no config layer in cave) | — |
| test_write.py (6) | write/append changelog into file, dup-version handling | scope-cut (no file writer in cave) | — |

### Counts
- portable-coverage gaps (cave IMPLEMENTS, no/partial test — PRIORITY): **2**
- missing-impl: 0 (cave deliberately does not implement the rest)
- scope-cut: remainder (CLI / VCS / config / templating / file-writer / version-discovery / web plumbing)

## Recommended TDD fills (portable-coverage first)

Both fills target **already-implemented** branches of `engine::parse_commit` that currently have **no test**.
This mirrors towncrier's category-classification coverage (`test_builder` basename→category) applied to
cave's conventional-commit classifier.

1. **`cave_changelog::engine::parse_commit`** — untested `Deprecated` + `Removed` branches and their prefix
   aliases. Implemented at engine.rs:22–25 (`deprecated`/`deprecate` → `ChangeType::Deprecated`,
   `remove`/`revert` → `ChangeType::Removed`) but only feat/fix/chore/security/unknown are currently
   exercised. Add:
   - `parse_commit("deprecated: drop old api")` → `Some((ChangeType::Deprecated, "drop old api"))`
   - `parse_commit("deprecate(core): ...")` → `ChangeType::Deprecated` (alias branch)
   - `parse_commit("remove: legacy module")` → `Some((ChangeType::Removed, "legacy module"))`
   - `parse_commit("revert: bad merge")` → `ChangeType::Removed` (alias branch)
   - `parse_commit("refactor: tidy")` / `parse_commit("style: fmt")` → `ChangeType::Changed` (untested
     `chore`-group aliases at engine.rs:26–29)

2. **`cave_changelog::engine::parse_commit`** — description extraction edge cases. The impl splits on the
   first `:` (engine.rs:11–15) and falls back to empty string when absent. Currently untested:
   - `parse_commit("feat: a: b")` → desc `"a: b"` (only first colon splits — `splitn(2, ':')`)
   - `parse_commit("feat")` (no colon) → `Some((ChangeType::Added, ""))` (empty-desc fallback)

No new fill is warranted for `filter_by_type`, `count_by_type`, `sort_by_version`, or the serde model
roundtrips — those public fns/types are already covered by existing tests in engine.rs and models.rs.
