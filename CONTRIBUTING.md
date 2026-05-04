# Contributing to Cave Runtime

Thank you for considering a contribution. Cave Runtime is a single-binary, line-by-line upstream-parity Cloud OS in Rust; the bar is higher than most repos, but the rules are simple and repeatable.

## Mission (one paragraph)

Cave Runtime is a sovereign, self-improving Cloud OS for Linux 7.1+ that re-implements the Kubernetes control plane and the surrounding cloud-native ecosystem (etcd, kube-apiserver, kubelet, scheduler, controller-manager, CRI, CNI, service mesh, registry, secrets, observability, ~60 more) under one Rust binary, multi-tenant by construction, post-quantum-crypto ready from the primitives layer up. It is not a thin wrapper, not a behavioural approximation: every module ports its named upstream's source and tests, one-to-one, with `tenant_id` as a first-class argument on every persisted-state API.

## The non-negotiables (golden rules)

1. **Line-by-line upstream parity.** Function, type, error, flag — ported one-to-one from a named upstream commit. Cite the upstream project, file, and SHA in a comment on the test that exercises the port.
2. **TDD: red → green → refactor.** A new function lands with at least one test that exercised it before the function existed (or that exercised the prior bug, for fixes). The most consistent generator of these test rows is **Mode B-prime** — see the section further down.
3. **No stubs.** `todo!()`, `unimplemented!()`, silently-`Ok(())`-returning placeholders, and "fill this in later" handlers are not acceptable in merged code. If a function cannot be completed inside the PR, leave it undefined and document the gap in `docs/synergy/` or in an ADR.
4. **No behavioural approximation.** "80 % of the hot path" is not done. Done is "every ported upstream test passes".
5. **No backward-compat port.** If upstream has `if kernel < 5.8 { old_path }`, skip the `if` — Cave is Linux 7.1+ only.
6. **Shared primitives.** If a concept (Raft, WAL, EventBus, rate-limit, retry, labels, SPIFFE, watch) exists in `cave-kernel`, use it. Do not re-implement per-module.
7. **Multi-tenant invariant.** Every public API that touches persisted state takes `tenant_id` as a first-class argument. Every persisted resource owns exactly one tenant. Default-deny across tenants. Tests cover the cross-tenant isolation path explicitly.
8. **Post-quantum first.** New crypto calls use PQC primitives (ML-KEM, ML-DSA, SLH-DSA where applicable). Classical-only code paths in new code are bugs.
9. **Function-based crate naming (ADR-147).** Crates are named by function, not by vendor: `cave-rdbms` is the relational engine, `cave-pg` is the Postgres operator surface, `cave-lakehouse` is the consolidated table-format + query engine. When you add a new crate, derive its name from the role it plays in the layered architecture, not from the upstream you happened to start from.

## How to file an issue

Use the GitHub issue tracker.

- **Bug.** Affected crate, the upstream commit SHA the module is porting (if applicable), the Cave Runtime commit SHA you reproduced on, the failing test name, and a minimal reproducer.
- **Performance regression.** `perf` or `cargo flamegraph` output, plus a comparison against the upstream baseline if one exists.
- **Feature request.** Cite the upstream feature being requested (project + version + flag/API surface) and the user-visible behaviour expected.
- **Security vulnerability.** Do **not** open a public issue. Follow the disclosure flow in [`SECURITY.md`](SECURITY.md).

## Building

```bash
# Prerequisites: Rust 1.85+ stable, a Linux 7.1+ kernel for dataplane crates,
# Docker (for tests that exercise upstream wire protocols).
git clone https://github.com/cave-runtime/cave-runtime
cd cave-runtime

# Build everything in the workspace
cargo build --workspace --all-targets

# Or just the unified runtime binary
cargo build --release -p cave-runtime
```

The workspace currently contains roughly 100 crates plus the `tools/night-pump/` standalone helper. There is no `justfile` today; everything is driven through `cargo`.

## Test requirements

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test  --workspace --no-fail-fast
```

All three commands must pass. New tests should reference the upstream they port from in a `// upstream: <project>@<sha> <path>` comment near the top of the test function.

## Pull-request checklist

Before requesting review:

- [ ] `cargo fmt --check` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo test --workspace` green; no new `#[ignore]`d tests added in your PR.
- [ ] At least one new test row covers the change (red-green pair).
- [ ] If a new crate is introduced, its name follows ADR-147 (function-based, not vendor-based).
- [ ] If the change touches an architectural invariant (charter, multi-tenancy, PQC, golden rules), an ADR PR ships first and is referenced.
- [ ] Commit message follows [Conventional Commits](https://www.conventionalcommits.org/): `feat(scope): …`, `fix(scope): …`, `test(scope): …`, `docs(scope): …`, `refactor(scope): …`, `chore(scope): …`.
- [ ] Commits are signed with a Developer Certificate of Origin trailer (`git commit -s` adds `Signed-off-by:`); PRs without sign-off cannot be merged.
- [ ] Sensitive data scrubbed: no personal paths, emails, hostnames, or tenant identifiers committed.

## Mode B-prime / `night-pump` / `qwen-pump` (for repeated test-row work)

Cave Runtime ships a small set of internal helpers for high-throughput, predictable contributions. You do not need them to land a PR — they are an option, not a requirement.

- **Mode B-prime** is a TDD pattern that delegates test authoring to a local LLM. The model receives a target crate, its public API surface, and a list of 5 specific behaviours to cover, and writes the *whole* `tests/qwen_drafted.rs` file from scratch (whole-file replace, not unified diffs — diffs do not survive the model's line-arithmetic). Pilot data: 15/15 = 100 % on `cave-status`, `cave-trace`, `cave-vault`. Capability matrix: [`docs/qwen-pump/capability-matrix-2026-05-04.md`](docs/qwen-pump/capability-matrix-2026-05-04.md).
- **`night-pump`** (`tools/night-pump/`) is a standalone Rust binary that coordinates worker batches against a YAML queue with priority + dependency wiring + concurrency cap. Workers (Qwen, Sonnet, Opus, or a human at the keyboard) poll `GET /api/next-batch?worker_id=X`, do the work, and `POST /api/batch-complete` with the outcome. Useful when you want to schedule many small contributions against the same crate without stepping on yourself. See `tools/night-pump/README.md`.
- **`qwen-pump`** is the dispatch glue around Mode B-prime / Mode E (rustdoc) / Mode H (per-crate README) / Mode I (Cargo.toml metadata). Multi-mode cycle dispatcher in `tools/night-pump/run-mode-cycle.sh`.

If you do use these helpers, your PR is a normal PR — the test row, the rustdoc, the README, the Cargo.toml fields all merge through the same review gate as any hand-written change.

## Code of Conduct

By participating in this project you agree to abide by the [Contributor Covenant 2.1](CODE_OF_CONDUCT.md). Disagreements over technical direction are resolved through ADRs, not through volume.

## License of contributions

By submitting a contribution, you agree that your contribution is licensed under both the [Apache License, Version 2.0](LICENSE-APACHE) and the [MIT License](LICENSE-MIT) — Cave Runtime is dual-licensed, and contributions are dual-licensed by default. You retain copyright over your contribution; no copyright assignment is required. The Developer Certificate of Origin (`Signed-off-by:` trailer on each commit) is the mechanism by which you assert that you have the right to submit the patch under these terms.
