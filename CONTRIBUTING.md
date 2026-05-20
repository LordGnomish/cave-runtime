# Contributing to Cave Runtime

Thank you for considering a contribution. Cave Runtime is a platform-grade,
line-by-line upstream parity project. The bar is higher than most repos —
but the rules are simple, machine-checkable, and the same for everyone.

## TL;DR

1. Read three ADRs: [CHARTER-001](docs/adr/ADR-CHARTER-001.md),
   [GOLDEN-001](docs/adr/ADR-GOLDEN-001-upstream-parity.md),
   [GOLDEN-003](docs/adr/ADR-GOLDEN-003-no-backcompat-pqc.md).
2. Pick an upstream feature. Find the upstream test for it. Port that
   test as `[RED]`. See it fail.
3. Implement until it passes. Commit as `[GREEN]`.
4. Update `parity.manifest.toml` and ship all 4 tracks (backend + portal
   + cavectl + observability) in the same PR.
5. The PR template walks you through the 8-gate Charter checklist.

## Setup

```bash
# Toolchain (pinned in rust-toolchain.toml)
rustup toolchain install 1.85 --component clippy rustfmt
rustup default 1.85

# Optional system tooling
brew install protobuf            # protoc for code generation
brew install postgresql@16       # for cave-rdbms integration tests

# Git hooks (SPDX + commit-msg + parity gate)
./scripts/install-git-hooks.sh

# First build (cold ~6 min; warm ~25 s)
cargo check --workspace --all-targets
cargo test  --workspace --lib
```

A local portal can be brought up with `CAVE_JWT_SECRET=dev cargo run -p
cave-runtime`; navigate to <http://localhost:8080>. See
[docs/quickstart.md](docs/quickstart.md) for the 10-line recipe and
[docs/runbook/](docs/runbook/) for operator-grade runbooks.

## The non-negotiables (Charter v2, 8 gates)

Every PR that touches a crate must keep that crate green on these eight
gates. The first six are enforced by `cargo test -p <crate>
--test parity_self_audit`. The last two are enforced in code review.

| # | Gate | Enforced by |
|---|------|-------------|
| 1 | **TDD-strict** — RED commit precedes GREEN commit | reviewer + commit log |
| 2 | **SPDX 100%** — every `.rs` carries `AGPL-3.0-or-later` | `parity_self_audit::spdx_coverage_100pct` + `scripts/check-spdx.sh` |
| 3 | **Source pinned** — manifest has `source_sha` + `upstream_version` + `last_audit` | `parity_self_audit::source_sha_pinned` |
| 4 | **No stubs** — no `todo!()` / `unimplemented!()` / silent `Ok(())` | reviewer + grep |
| 5 | **No backwards-compat shims** — Linux 7.1+ kernel paths only, PQC primitives only | reviewer |
| 6 | **Always-latest upstream** — re-pin per audit cycle | `parity_self_audit::last_audit_recent` |
| 7 | **4-track ship** — backend + portal + cavectl + observability in same PR | reviewer |
| 8 | **Honest measured `fill_ratio`** — manifest-sourced, not self-graded | `parity_self_audit::parity_ratio_from_manifest` |

### TDD workflow (RED → GREEN → REFACTOR)

```bash
# 1. RED: port the upstream test, see it fail
git checkout -b feat/cave-<area>-<short-desc>
# write tests/parity/<feature>.rs with the upstream's test logic
cargo test -p cave-<crate> --test <feature>     # fails
git commit -am "[RED] cave-<crate>: port upstream test for <feature> (upstream sha: <sha>)"

# 2. GREEN: minimum code to pass
# write src/<feature>.rs
cargo test -p cave-<crate>                      # passes
git commit -am "[GREEN] cave-<crate>: implement <feature>"

# 3. REFACTOR (optional): tighten, dedupe against cave-kernel primitives
cargo clippy -p cave-<crate> -- -D warnings
cargo fmt --all
git commit -am "refactor(cave-<crate>): use cave-kernel::watch for <feature>"
```

Reference upstream commit SHA in the test's module-level doc-comment:

```rust
//! Port of upstream `pkg/scheduler/algorithm/predicates.go::PodFitsResources`
//! from kubernetes/kubernetes@v1.31.0 (commit 8c1c4d5e).
```

### Adding a new crate

A new crate is an architectural change. It requires an ADR before any
code lands. Use the [docs/adr/TEMPLATE.md](docs/adr/TEMPLATE.md). Once
the ADR is merged:

1. `cargo new --lib crates/cave-<name>` and add to `[workspace.members]`.
2. Write `crates/cave-<name>/parity.manifest.toml` with the literal upstream
   subsystem inventory, `source_sha`, `parity_ratio_source = "manifest"`,
   and `[[scope_cuts]]` for anything you defer.
3. Write `tests/parity_self_audit.rs` (9 assertions — copy from any
   existing crate as starter).
4. Write `PARITY_REPORT.md` summarising the 8 Charter gates.
5. RED → GREEN as above.
6. Regenerate `docs/parity/parity-index.json` via
   `python3 scripts/build-parity-index.py`.
7. Update [NOTICE](NOTICE) with the new upstream attribution if needed.

## Pull-Request checklist

The PR template will surface this automatically. Manual version:

- [ ] All commits follow [Conventional Commits](https://www.conventionalcommits.org/):
      `feat(scope): …`, `fix(scope): …`, `test(scope): …`,
      `docs(scope): …`, `refactor(scope): …`, `chore(scope): …`.
- [ ] RED commit precedes GREEN commit for every new test.
- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] `cargo test --workspace --all-targets` passes; no new `#[ignore]`.
- [ ] `cargo test -p <crate> --test parity_self_audit` passes for every
      crate touched.
- [ ] `parity.manifest.toml` updated where surface changed: `source_sha`,
      `last_audit` (today), `parity_ratio_source = "manifest"`, fill_ratio
      recomputed.
- [ ] `PARITY_REPORT.md` updated where Charter gate state changed.
- [ ] 4-track delta: backend + portal + cavectl + observability, all four
      in the same PR (or `infra_only = true` documented in the manifest).
- [ ] DCO sign-off on every commit (`git commit -s`).
- [ ] If the PR is LLM-assisted, include a `Co-Authored-By: <model>`
      trailer.

## Commit-message format

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(cave-scheduler): port PodFitsResources predicate

Adds the upstream PodFitsResources logic to cave-scheduler. The upstream
unit test is ported in `tests/predicates/pod_fits_resources.rs` and
asserts cpu/memory/storage gating against the same fixture vectors.

Source: kubernetes/kubernetes@v1.31.0 commit 8c1c4d5e
Closes: #1234
Signed-off-by: Your Name <you@example.com>
```

Scopes are crate names without the `cave-` prefix (`apiserver`, `etcd`,
`scheduler`, `portal`, …). For workspace-wide work use the parent area:
`docs`, `chore`, `release`.

`[RED]` and `[GREEN]` prefixes are encouraged in the commit subject for
TDD pairs — they make the parity gate's commit-log audit trivial:

```
[RED]   cave-keda: port AzureServiceBus scaler upstream test
[GREEN] cave-keda: implement AzureServiceBus scaler
```

## DCO sign-off

Cave Runtime uses the [Developer Certificate of Origin](https://developercertificate.org/)
in lieu of a CLA. Sign every commit with `git commit -s`. The
`Signed-off-by:` trailer asserts that you have the right to contribute
the change under AGPL-3.0-or-later. We do not collect CLAs.

## Code of conduct

We hold ourselves to the [Contributor Covenant v2.1](CODE_OF_CONDUCT.md).
Be excellent to each other. Disagreements are resolved via ADRs, not by
volume.

## Reporting issues

- **Bug:** [`.github/ISSUE_TEMPLATE/bug_report.md`](.github/ISSUE_TEMPLATE/bug_report.md).
  Include the upstream commit SHA the module is porting, the Cave commit
  SHA under test, the failing upstream test name, and a minimal
  reproducer.
- **Feature:** [`.github/ISSUE_TEMPLATE/feature_request.md`](.github/ISSUE_TEMPLATE/feature_request.md).
- **Parity gap:** [`.github/ISSUE_TEMPLATE/parity_gap.md`](.github/ISSUE_TEMPLATE/parity_gap.md).
  Best entry-point for new contributors — pick one upstream subsystem and
  ship it.
- **Performance:** include `perf` or `cargo-flamegraph` output; compare to
  upstream benchmarks where applicable.
- **Security:** **do not** open a public issue. See
  [SECURITY.md](SECURITY.md).

## Architectural changes

Any change that:

- introduces a new crate,
- touches an ADR-bound invariant (charter, golden rules, multi-tenancy,
  PQC, self-improvement),
- adds a third-party dependency, or
- modifies the public API surface of a tier-1 module (apiserver, etcd,
  kubelet, scheduler, cri, net, kernel, auth),

requires an ADR before code merges. Template:
[docs/adr/TEMPLATE.md](docs/adr/TEMPLATE.md). Merge the ADR first, then
the implementation PR.

## Triage labels you might see on your PR

| Label | Meaning |
|-------|---------|
| `parity-gap` | A specific upstream subsystem is unmapped; PR closes a gap |
| `good-first-issue` | Scoped + isolated; new-contributor friendly |
| `charter-gate-fail` | One of the 8 gates regressed; reviewer-blocking |
| `4-track-incomplete` | Backend done but portal / cavectl / observability missing |
| `needs-adr` | Architectural change needs an ADR PR before this one merges |
| `needs-rebase` | Conflict against `main`; rebase needed |
