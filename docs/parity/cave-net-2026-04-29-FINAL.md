---
date: 2026-04-29
author: Claude (autonomous run, second attempt)
status: MERGED to main
sprint_branch: feat/cave-net-real-100
sprint_tip: 0a0f618dd46081e0b4b7c6918aa089ef13ec10db
merge_strategy: per-commit cherry-pick onto main with surgical conflict resolution
---

# cave-net + cave-upstream merge to `main` — final record

The earlier halt note (`cave-net-2026-04-29-merge-halt-note.md`) documented
why the previous autonomous run refused to perform the merge: a naïve
`-X theirs` strategy would have silently overwritten main's M37–M52 cilium
production implementation with feat's parallel branch content. This run
retried with a different strategy that is safer and matches the user's
explicit instruction "feat'in 17 yeni pkg'i için feat'i tut, M37-M52
modüllerinde main'i tut".

## Strategy

`feat/cave-net-real-100` had 3 commits ahead of main but was branched from
the initial commit (parallel track, merge-base = `ac51758`). A direct rebase
or `-X theirs` merge was unsafe.

Instead: a fresh `cave-net-merge-staging` branch was cut from current main
and the three feat commits were cherry-picked onto it. The cilium parity
sprint commit (`31a7854`) was the only one with conflicts.

A pre-flight content audit on the 92 files that commit touches found:
- **66 cilium source files**: byte-identical between feat and main → git
  auto-resolved as zero-diff.
- **21 feat-only files**: clean addition (act, allocator, bgp_types,
  binary_cites, cec, controller, defaults, endpoint_mgr, envoy_bootstrap,
  idiom_map, ipmasq, kpr, metrics, net_types, node_mgr, nodediscovery,
  option, xds, ztunnel, plus PARITY_REPORT.md and tests/wire_faithful.rs).
- **2 real conflicts**: `src/cilium/mod.rs` (mod-decl list) and
  `parity.manifest.toml` (schema). Both resolved by taking feat's version
  because feat's mod.rs is a strict superset of main's (66 → 85 mod decls)
  and main's parity.manifest.toml has no test asserts attached, so it is
  safe to upgrade to feat's fill_ratio=1.0 schema without breaking parity
  gates.

A fourth fix-up commit (`fix(cave-upstream): add missing axum workspace
dep for routes module`) was added because the watch-daemon commit promoted
`routes`/`store`/`models` from orphan files to declared modules, exposing
a pre-existing missing dep that neither branch had caught.

## Verification

Re-run on the rebased staging tip:
* `cargo build --release -p cave-net` → clean, 0 warnings.
* `cargo test -p cave-net --release` → **1759 passed, 0 failed, 5 ignored**
  (1697 lib + 56 e2e + 6 wire-faithful + 5 qwen-scaffold ignored). Matches
  feat's pre-merge claim exactly.
* `cargo check -p cave-upstream --release` → clean.
* `cargo check --workspace --release` → 1 pre-existing error in
  `cave-runtime/src/main.rs:194` (imports `cave_portal_api` without
  declaring it as a dep). This error is in main and was NOT introduced by
  the merge — verified by inspecting `git show main:` versions of both
  files.
* `rg 'todo!\(|unimplemented!\('` on `crates/cave-net/src/` → 0 matches.

## What is preserved

* main's M37–M52 cilium implementation (66 byte-identical cilium source
  files plus the 56-test `cilium_parity_e2e.rs` integration suite).
* feat's 19 new cilium modules (act, allocator, bgp_types, binary_cites,
  cec, controller, defaults, endpoint_mgr, envoy_bootstrap, idiom_map,
  ipmasq, kpr, metrics, net_types, node_mgr, nodediscovery, option, xds,
  ztunnel) plus their unit tests (197 of the 1697 lib tests).
* feat's wire-faithful goldens (6 byte-comparison tests against upstream
  Cilium v1.19.3 wire artifacts).
* feat's parity.manifest.toml at fill_ratio=1.0 (117 mapped + 17 skipped =
  134 total).
* The cave-upstream watch-daemon (state/delta/pump/daemon plus the
  `cave-upstream-watchd` binary and `launchd` plist).
* All previously-merged main work since the parallel branch point: K8s
  parity additions (cave-apiserver, cave-cri, cave-etcd, cave-scheduler,
  cave-kubelet, cave-ccm), observability catalog (78 modules, 780 panels,
  624 alerts), and the qwen-pump scaffold merges that landed during this
  run.
