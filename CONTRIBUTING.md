# Contributing to Cave Runtime

Thank you for considering a contribution. Cave Runtime is a platform-grade, line-by-line upstream parity project; contributions have a higher bar than most repos, but the bar is simple and repeatable.

## The non-negotiables

Read these three ADRs before opening a PR:

1. [ADR-CHARTER-001](docs/adr/ADR-CHARTER-001.md) — the mission.
2. [ADR-GOLDEN-001](docs/adr/ADR-GOLDEN-001-upstream-parity.md) — upstream line-by-line parity + TDD.
3. [ADR-GOLDEN-003](docs/adr/ADR-GOLDEN-003-no-backcompat-pqc.md) — no backward-compat port; PQC-ready crypto.

Golden rule summary:

- **Line-by-line parity.** Function, type, error, flag — ported one-to-one from the named upstream commit.
- **TDD.** Before implementing, port the upstream's own test for the feature. Failing test first, then implementation.
- **No stubs.** `todo!()`, `unimplemented!()`, silently-`Ok(())`-returning placeholders are not allowed. If you cannot complete a function in a PR, document the gap in `docs/synergy/` and leave the function undefined.
- **No behavioral approximation.** "80% of the hot path" is not done. Done is "every upstream test passes".
- **No backward-compat port.** If upstream has an `if kernel < 5.8 { old_path }`, skip the `if`; Cave is Linux 7.1+ only.
- **Shared primitives.** If a concept (Raft, WAL, EventBus, rate-limit, retry, labels, SPIFFE, watch) exists in `cave-kernel`, use it. Do not reimplement per-module.
- **Multi-tenant awareness.** Every new public API takes `tenant_id`. Every persisted resource owns exactly one tenant. Default-deny across tenants. See [ADR-MULTI-TENANT-001](docs/adr/ADR-MULTI-TENANT-001.md).
- **Post-quantum first.** New crypto calls use `cave-crypto` PQC primitives; classical-only crypto is a bug.

## Development workflow

1. Fork and create a feature branch from `main`. Branch naming: `feat/<area>-<short-desc>`, `fix/<area>-<issue>`, `test/<area>-<scope>`.
2. Write failing test first, ported from the named upstream's test suite. Link the upstream commit SHA in the test comment.
3. Implement until tests pass.
4. Run the full workspace suite: `cargo test --workspace`. Zero failures, zero `#[ignore]`d tests added in your PR.
5. Lint: `cargo fmt --all && cargo clippy --all-targets --all-features -- -D warnings`.
6. Update the module's `parity.manifest.toml` (file_parity, function_parity, test_parity, surface_parity metrics).
7. If your change touches a 4-track module (backend + portal + cavectl + observability per [ADR-GOLDEN-004](docs/adr/)), all four tracks ship in the same PR.
8. Commit message uses [Conventional Commits](https://www.conventionalcommits.org/): `feat(scope): …`, `fix(scope): …`, `test(scope): …`, `docs(scope): …`.
9. Include a `Co-Authored-By: <model/name>` trailer if the PR was LLM-assisted (required for attribution; see [ADR-CONTRIB-ATTRIBUTION-001](docs/adr/)).

## Architectural changes

Any change that:
- Introduces a new crate,
- Touches an ADR-bound invariant (charter, golden rules, multi-tenancy, PQC, self-improve),
- Adds a third-party dependency,
- Modifies the public API surface of a tier-1 module (apiserver, etcd, kubelet, scheduler, cri, net, kernel, auth),

requires an ADR PR first. Template in [docs/adr/TEMPLATE.md](docs/adr/TEMPLATE.md). Merge ADR before merging implementation.

## Reporting issues

- Bug: include the upstream commit SHA the module is porting, the Cave commit SHA under test, the failing upstream test name, and a minimal reproducer.
- Performance: include `perf` or `cargo-flamegraph` output; compare to upstream benchmarks.
- Security: do **not** open a public issue. See [SECURITY.md](SECURITY.md).

## Code of conduct

Be excellent to each other. Disagreements are resolved via ADRs, not by volume.
