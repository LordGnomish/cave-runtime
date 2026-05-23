# Refactor sweep — Phase 2.6 dead code + warning purge (2026-05-23)

Status: partial — most automated fixes reverted as unsafe.

## cargo fix --all --allow-dirty --allow-staged

Attempted. Modified 169 files. Result: **8 crates failed to compile**
because cargo fix removes imports it judges unused without checking
`#[cfg(test)]` blocks in the same module. Same footgun documented in
the 2026-05-19 OSS launch Wave 2 memory.

Crates broken by the fix (all reverted): `cave-portal` (141 errors),
`cave-infra` (2), `cave-apiserver` (11), `cave-cri` (1), `cave-rollouts`
(1), `cave-alerts` (1), `cave-net` (3), `cave-oncall` (10), plus a
`cave-gateway` over-removal where `cargo check` exit was 0 but the
test target failed (also reverted).

**Decision:** revert all cargo fix changes from this branch.
Per-crate `cargo fix --lib -p <crate>` with manual review of test
imports is the safe path; out of scope for a sweep.

## cargo clippy -D warnings

Skipped. Same footgun risk as cargo fix (the autofixes flow through
the same machinery for `unused_imports`). Workspace baseline:
**821 warnings** in `cargo check --workspace --all-targets`. No
automated reduction safely available without per-crate review.

## cargo +nightly udeps

Skipped — `cargo-udeps` not installed and the project does not yet
pin a nightly toolchain.

## cargo deny check

Ran. **14 advisories** (6 unmaintained, 5 vulnerability, 3 unsound):
RUSTSEC-2023-0071, 2024-0384, 2024-0436, 2025-0012, 2025-0052,
2025-0057, 2025-0134, 2026-0002, 2026-0008, 2026-0097, 2026-0098,
2026-0099, 2026-0104, 2026-0119. License + bans + sources passes.

Per-advisory dependency upgrades are out of scope for a sweep — each
needs targeted PRs (e.g. swap `async-std` callers off, replace
`backoff` with a maintained alternative, upgrade transitive ring
to a Marvin-Attack-patched release). Full report in
`.metrics/deny-check.log` (not committed — large).

## Net effect this phase

- Files changed: **0** (all autofixes reverted)
- Warnings before: 821, after: 821 (unchanged)
- Errors before: 0, after: 0
- Advisories: 14 logged for follow-up
