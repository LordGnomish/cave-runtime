# cave-deploy — PARITY_REPORT

| field                 | value                                               |
|-----------------------|-----------------------------------------------------|
| upstream              | argoproj/argo-cd                                    |
| upstream_version      | v3.4.2                                              |
| upstream_license      | Apache-2.0                                          |
| source_sha            | `0dc6b1b57dd5bb925d5b03c3d09419ab9fb4225e`          |
| parity_ratio_source   | manifest                                            |
| fill_ratio            | **0.9459** ((20 + 3 + 12) / 37)                     |
| honest_ratio          | **0.6216** ((20 + 3) / 37)                          |
| mapped / partial / skipped / unmapped / total | 20 / 3 / 12 / 2 / 37            |
| last_audit            | 2026-05-22                                          |
| crate version         | 0.1.0                                               |
| MVP floor             | fill_ratio ≥ 0.65 ✓ (overshoot +0.296)              |
| lib tests             | 100 PASS                                            |
| parity_self_audit     | 9 PASS                                              |
| ADR                   | docs/adr/ADR-154_ArgoCD_GitOps_Adoption.md          |

## Charter v2 8-gate close — 2026-05-22

| # | gate                                | result                                                  |
|---|-------------------------------------|---------------------------------------------------------|
| 1 | upstream version pinned             | **PASS** — v3.4.2 (latest stable, published 2026-05-12) |
| 2 | source_sha matches commit           | **PASS** — `0dc6b1b5…ab9fb4225e` resolved via GitHub API |
| 3 | fill_ratio ≥ 0.65                   | **PASS** — 0.9459                                       |
| 4 | parity_ratio_source = "manifest"    | **PASS**                                                |
| 5 | last_audit = today                  | **PASS** — 2026-05-22                                   |
| 6 | counts sum to total + ≥15 mapped    | **PASS** — 20+3+12+2 = 37, 20 mapped                    |
| 7 | AGPL SPDX header coverage 100%      | **PASS** — 14/14 .rs files (13 src + 1 self-audit)      |
| 8 | no stub macros in `src/`            | **PASS** — 0 offenders                                  |

Self-audit suite: `tests/parity_self_audit.rs` runs 9 assertions; the 9th
walks the full deploy surface (CRD enums + appset generators + sync waves
+ hook parsing + sync options + diff + health + RBAC + sync/drift/auto-sync
+ manifest render + YAML/JSON parsers + cluster URL builders + rollout
strategies + notification engine + store + rollback + error variants).

## 20 mapped subsystems

1. `application-crd` — `src/models.rs`
2. `appproject-crd` — `src/rbac.rs`
3. `applicationset-crd-and-generators` — `src/appset.rs`
4. `sync-engine` — `src/sync.rs`
5. `sync-options-parser` — `src/sync.rs::parse_sync_options`
6. `hook-lifecycle` — `src/sync.rs::{parse_hook_phases, parse_delete_on_success}`
7. `diff-engine` — `src/diff.rs`
8. `health-assessor` (13 kinds) — `src/health.rs`
9. `rbac-evaluator` — `src/rbac.rs`
10. `rollback-engine` — `src/sync.rs::initiate_rollback + src/store.rs::rollback_to_history_id`
11. `rollout-strategies` — `src/rollout.rs`
12. `cluster-registry` — `src/cluster.rs`
13. `resource-tracking-label` — `src/cluster.rs::TRACKING_LABEL + src/gitops.rs`
14. `manifest-renderer-shapes` — `src/gitops.rs::render_manifests`
15. `drift-detection` — `src/gitops.rs::{detect_drift, auto_sync}`
16. `in-memory-store` — `src/store.rs`
17. `http-api-surface` (17 endpoints) — `src/routes.rs`
18. `notification-engine-mvp` — `src/notifications.rs`
19. `git-webhook-receiver` — `src/routes.rs::handle_webhook`
20. `sso-config-model` — `src/models.rs::SSOConfig`

## 3 partial subsystems

| subsystem             | what's present                                   | deferred                         |
|-----------------------|--------------------------------------------------|----------------------------------|
| `helm-render-exec`    | `HelmSource` model + render shapes              | `helm template` subprocess       |
| `kustomize-render-exec` | `KustomizeSource` model + render shapes       | `kustomize build` subprocess     |
| `kube-apply-client`   | URL builders + tracking-label + wave ordering   | `kube::Client` SSA PATCH path    |

## 12 skipped subsystems (scope_cuts → 6 Phase 2 crates)

| target crate                  | subsystems                                                          |
|-------------------------------|---------------------------------------------------------------------|
| `cave-image-updater`          | image-updater                                                       |
| `cave-notify`                 | notifications-template-engine, retries-dedup, thirty-plus-destinations |
| `cave-workflow`               | workflow-hook-integration                                           |
| `cave-portal-ui`              | argocd-react-ui                                                     |
| `cave-auth`                   | argocd-dex-server-runtime                                           |
| `cave-deploy-runtime-phase-2` | multi-cluster-federation, sync-windows-cron, gpg-signature-verification, jsonnet-render, plugin-generator-runtime, pull-request-generator-runtime, scm-provider-generator-runtime |

## 2 unmapped (honest gaps)

| subsystem              | rationale                                                      |
|------------------------|----------------------------------------------------------------|
| `helm-deps-resolution` | Multi-source Helm-of-Helms + Chart.lock resolution             |
| `argocd-cli-grpc`      | gRPC-over-WS tunnel for `argocd app sync`/`logs` streaming     |

## Integration points

- **cave-cri** — sync engine apply path (Phase 2 swap-in)
- **cave-net** — cluster discovery for `ClusterGenerator`
- **cave-secrets** — repository credentials (keychain-only `credential_ref`)
- **cave-auth/keycloak** — SSO runtime (Dex-equivalent flows)
- **cave-cli (`cavectl`)** — 5 deploy subcommands: app / sync / rollback / health / project

## Smoke

`tests/test_gap_close_edges.rs` contains 16 integration tests covering an
ApplicationSet fixture, sync engine dry-run, RBAC scope evaluation and a
PreSync→Sync→PostSync hook lifecycle.

## Notes

* `parity-index.json` regen picks the manifest as the source of truth via
  `parity_ratio_source = "manifest"`; the hourly regenerator
  (`com.cave.parity-index-regen` launchd) will catch this.
* Charter v2 floors raised: `assertion_3` floors fill_ratio at **0.65**;
  `assertion_6` floors mapped count at **15**.
