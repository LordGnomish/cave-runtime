---
name: Parity gap
about: An upstream subsystem is unmapped or partially mapped
title: "parity-gap(<crate>): <upstream subsystem name>"
labels: ["parity-gap", "good-first-issue", "needs-triage"]
assignees: []
---

<!--
This is the canonical entry point for new contributors. Pick one upstream
subsystem, scope it into one PR. The Charter v2 gates and the PR template
will keep you on the rails.
-->

## Target crate

- **Crate:** `cave-<name>`
- **Upstream project:** <github.com/org/repo>
- **Upstream version pinned in manifest:** <e.g. v1.31.0>
- **Current `fill_ratio`:** <from `parity.manifest.toml` / `docs/parity/parity-index.json`>

## Unmapped subsystem

<!-- Name the upstream subsystem as it appears in the upstream's own
     directory or feature taxonomy. Examples:
       * "Scheduler predicate: PodFitsResources"
       * "Etcd v3 lease keepalive"
       * "KEDA scaler: AWS-SQS"
       * "Knative source: container_source"
     One subsystem per issue — keeps PRs scoped.
-->

## Upstream reference

<!-- Link to the upstream's source path, tag/commit, and test file
     that exercises the subsystem:
       https://github.com/org/repo/blob/<sha>/path/to/subsystem.go
       https://github.com/org/repo/blob/<sha>/path/to/subsystem_test.go
-->

- Source: <link>
- Tests: <link>
- Docs (if any): <link>

## Manifest classification

How should this be classified in the crate's `parity.manifest.toml`?

- [ ] `mapped` — port the subsystem in full, line-by-line.
- [ ] `partial` — port a subset; rest deferred (justify below).
- [ ] `skipped` with `[[scope_cuts]]` — out of scope for this Cave
      generation; explain why (deprecation, platform-only, hyperscaler-only,
      replaced by Cave-native primitive, etc.).

## Suggested implementation outline

<!-- Optional. If you have ideas, sketch them. Otherwise leave blank;
     reviewers will help scope. -->

## Acceptance criteria

A PR closing this issue must:

- [ ] Add a `[RED]` commit porting the upstream test(s) for the subsystem.
- [ ] Add `[GREEN]` commit(s) implementing the subsystem until the
      ported test passes.
- [ ] Update `crates/cave-<name>/parity.manifest.toml`:
      add the new subsystem under `mapped` (or `partial` + scope_cut),
      bump `last_audit`, keep `parity_ratio_source = "manifest"`.
- [ ] Update `crates/cave-<name>/PARITY_REPORT.md` if Charter-gate state
      changes.
- [ ] Pass `cargo test -p cave-<name> --test parity_self_audit`.
- [ ] Ship 4-track if user-visible (backend + portal + cavectl + observability).
- [ ] Sign-off (`git commit -s`).

## Hint for new contributors

If this is your first PR, `cargo test -p cave-<name> --test parity_self_audit
-- --nocapture` is the fastest feedback loop — it tells you exactly what
the Charter expects. The `PARITY_REPORT.md` of any existing crate at
`fill_ratio >= 0.9` is a good template.
