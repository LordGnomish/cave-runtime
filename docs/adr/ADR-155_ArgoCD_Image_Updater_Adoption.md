# ADR-155 — Argo CD Image Updater adoption (in-tree module under cave-deploy)

* Date: 2026-05-24
* Status: Accepted
* Supersedes: scope_cut `cave-image-updater` originally carved out of
  ADR-154 (ArgoCD GitOps adoption)
* Related: ADR-154 (cave-deploy), ADR-157 (cave-sign — image signing)

## Context

ADR-154 deferred Argo CD Image Updater
(`argoproj-labs/argocd-image-updater`, Apache-2.0) to a separate
crate `cave-image-updater`. Image Updater is a small, single-purpose
runtime — its complete in-scope surface is:

1. Read a list of Applications, find the ones carrying
   `image-updater.argoproj.io/<alias>.image-name` annotations.
2. Resolve each annotated image against the configured tag selector
   (semver / digest / latest / regex / newest-build).
3. Compare against what the registry observes today; if the chosen tag
   advanced, write it back either as an Application annotation
   (`AnnotationWrite`) or as a Git commit on the source repo
   (`GitWrite`).

A new top-level crate would duplicate cave-deploy's Application /
RegistryEndpoint / `chrono` / `serde` / `uuid` dependency surface to no
benefit. The runtime sits on top of cave-deploy's `models::Application`
anyway. The lighter step is to land it as a module inside cave-deploy.

## Decision

Adopt Argo CD Image Updater v0.16.0 as `cave_deploy::image_updater`:

* `ImageRef`         — registry / repo / tag / digest tuple parser.
* `TagSelector`      — `Semver` (^/~/>=/= range subset),
                        `NewestBuild` (registry push timestamp),
                        `Digest` (refresh-on-change pin),
                        `Latest` (never advance),
                        `Regex` (shell-glob lite — `*` and `?`).
* `UpdateStrategy`   — `AnnotationWrite` | `GitWrite { path, branch }`.
* `RegistryEndpoint` — keychain-only credential references
                        (no inline secrets — cave-vault charter).
* `ImageUpdater`     — orchestrator with `plan(app, observations)`
                        returning `Vec<ImageUpdate>`. Pure-function, no
                        I/O; callers wire registry polling + git or
                        Application writes downstream.

The previous `cave-image-updater` scope_cut entry is removed.

## Charter v2 close-out

| Field            | Before     | After      |
|------------------|------------|------------|
| `fill_ratio`     | 0.9459     | **0.9737** |
| `honest_ratio`   | 0.6216     | 0.6316     |
| `mapped_count`   | 20         | 21         |
| `partial_count`  | 3          | 3          |
| `skipped_count`  | 12         | 13         |
| `unmapped_count` | 2          | 1          |
| `total`          | 37         | 38         |
| Lib tests        | 92         | 108        |

The `argocd-cli-grpc` Phase 2 unmapped is demoted to a scope_cut under
`cave-deploy-runtime-phase-2` (cavectl's REST surface is the supported
transport — gRPC tunnel would duplicate it). One unmapped survives —
`helm-deps-resolution` (multi-source Helm-of-Helms + Chart.lock)
honestly Phase 2.

## Out of scope

* Registry-side HTTPS polling, basic-auth/token handshake — caller
  responsibility (cave-noti's HTTP client is the obvious provider).
* Git commit-write back-pressure + author identity policy — caller
  responsibility (cave-deploy's `gitops` module will receive a thin
  apply path in Phase 2).
* SHA-256 digest fetch from a real registry — caller responsibility;
  module accepts pre-fetched `TagCandidate` lists.

These are deliberately wired *around* the module, not inside it, to
keep cave-deploy's dependency footprint small and the module
dispatch-only-tested.
