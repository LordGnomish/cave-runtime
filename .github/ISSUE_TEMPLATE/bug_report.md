---
name: Bug report
about: Report a defect in Cave Runtime
title: "bug(<crate>): <one-line summary>"
labels: ["bug", "needs-triage"]
assignees: []
---

<!--
Before opening:
  * Search existing issues to avoid duplicates.
  * If this is a security issue, STOP and use the private advisory channel
    in SECURITY.md instead.
  * If this is a parity gap (an upstream feature that is unmapped), use
    the `parity_gap` template instead — it gives us better context.
-->

## Summary

<!-- One sentence describing what's wrong. -->

## Affected crate / module

- **Crate:** `cave-<name>`
- **Upstream project being ported:** <e.g. kubernetes/kubernetes>
- **Upstream version pinned in manifest:** <e.g. v1.31.0>
- **Cave commit SHA under test:** <git rev-parse HEAD>

## Reproduction

```bash
# Minimal shell snippet, single test name, or single curl command.
# We need to be able to reproduce on a clean checkout in < 5 minutes.
```

## Expected behaviour

<!-- What you expected — ideally referencing the upstream test or doc that
     codifies the expected behaviour. -->

## Actual behaviour

<!-- What happens instead. Include exact error messages, stack traces,
     and any relevant log lines (with RUST_LOG=debug if useful). -->

## Environment

| Field | Value |
|-------|-------|
| OS | <e.g. Ubuntu 24.04 / macOS 15.x / Linux 7.1+> |
| Architecture | <e.g. x86_64, arm64> |
| Rust version | `rustc --version` |
| Cave Runtime version / commit | `git rev-parse HEAD` |
| Single-node / multi-node | <single / 3-node / N-node> |
| Tenant count | <1 / N> |

## Charter-gate impact

- [ ] Reproduces under `cargo test -p cave-<name> --test parity_self_audit`
- [ ] Multi-tenant boundary affected (cross-tenant leak — escalate to security)
- [ ] PQC regression (classical-only path discovered — escalate to security)
- [ ] 4-track inconsistency (backend ok, portal/cavectl/observability not)
- [ ] None of the above (regular functional bug)

## Logs / artifacts

<!-- Attach or paste:
       * `RUST_LOG=debug` portion of relevant logs
       * Coredump / backtrace (`RUST_BACKTRACE=1`)
       * `cavectl <command> --output=json` output if applicable
-->

## Additional context

<!-- Anything else we should know. Workarounds, suspected root cause,
     related issues, links to upstream bug reports. -->
