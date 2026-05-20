<!--
Thank you for the contribution. This template encodes the Charter v2 8-gate
checklist. Filling it out honestly is the fastest path to a green review —
reviewers will mirror this list, gate by gate.

Delete sections that genuinely don't apply, but DO NOT skip the 8-gate
checklist itself.
-->

## Summary

<!-- One paragraph: what changes and why. Link the issue(s) this closes. -->

Closes #

## Changes

<!-- Bullet list of the concrete deltas. Group by crate / area. -->

-

## Charter v2 — 8-gate checklist

| # | Gate | Status |
|---|------|--------|
| 1 | **TDD-strict** — `[RED]` commit precedes `[GREEN]` for every new behaviour | <!-- yes / n/a-docs --> |
| 2 | **SPDX 100%** — every new `.rs` carries `AGPL-3.0-or-later` | |
| 3 | **Source pinned** — `parity.manifest.toml` has `source_sha` + `upstream_version` + `last_audit` (today) | |
| 4 | **No stubs** — no `todo!()`, `unimplemented!()`, silent `Ok(())` | |
| 5 | **No backwards-compat shims** — Linux 7.1+, PQC primitives only | |
| 6 | **Always-latest upstream** — pinned upstream version is the latest stable | |
| 7 | **4-track ship** — backend + portal + cavectl + observability in this PR | |
| 8 | **Honest measured `fill_ratio`** — `parity_ratio_source = "manifest"`, no self-grading | |

## Test coverage delta

```
crate                            before   after   Δ
cave-<name>                      <count>  <count> <Δ>
```

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace --all-targets` passes
- [ ] `cargo test -p cave-<name> --test parity_self_audit` passes
      (every crate touched)
- [ ] No new `#[ignore]`-d tests introduced
- [ ] `PARITY_REPORT.md` reflects current state

## 4-track ship

For user-visible changes, all four tracks land in this PR:

- [ ] **Backend** — `crates/cave-<name>/src/...`
- [ ] **Portal** — `crates/cave-portal/src/admin/...`
- [ ] **cavectl** — `crates/cave-cli/src/cmd/...`
- [ ] **Observability** — `crates/cave-alerts/` and/or
      `crates/cave-dashboard/` (alerts, dashboards, SLO/SLI)

If `infra_only = true` for the affected crate, mark here:

- [ ] `infra_only = true` (documented in manifest)

## Memory / ADR reference

<!-- Link any ADR that this PR implements, conforms to, or modifies.
     Link any memory entry that captured prior context. -->

- ADR(s): <ADR-XXX-... / N/A>
- Memory: <topic-file.md / N/A>

## Upstream parity reference

<!-- Where in the upstream is the behaviour we are mirroring? Pin the
     commit SHA or tag we are porting from. -->

- Upstream project: <github.com/org/repo>
- Upstream version: <e.g. v1.31.0>
- Upstream test file(s) ported: <path/to/test.go>

## Sign-off

By submitting this pull request you certify that you have the right to
contribute the change under AGPL-3.0-or-later and have signed every
commit (`git commit -s`) per the [Developer Certificate of Origin](https://developercertificate.org/).

- [ ] All commits are sign-off-ed (`Signed-off-by:` trailer present)
- [ ] LLM-assisted contributions include a `Co-Authored-By:` trailer

## Additional notes

<!-- Anything reviewers should know that didn't fit above. Performance
     considerations, known follow-ups, deferred work, etc. -->
