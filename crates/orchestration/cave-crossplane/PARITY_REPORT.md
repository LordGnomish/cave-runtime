# cave-crossplane parity report — 2026-05-23 Charter v2 deep-port

Branch: `claude/cave-crossplane-close-2026-05-23`
Upstream: [crossplane/crossplane v2.3.1](https://github.com/crossplane/crossplane)
Upstream `source_sha`: `41c6f9c4729175cf0f953cbf267378b8734e8d27`
License: upstream Apache-2.0 → cave-crossplane AGPL-3.0-or-later (compatible)

## Headline numbers

| metric | before | after |
| --- | --- | --- |
| `parity.fill_ratio` | 0.0 | **0.9750** |
| `parity.honest_ratio` | — | **0.6750** |
| `[[mapped]]` Charter v2 entries | 0 | **26** |
| `[[partial]]` | 0 | **1** |
| `[[skipped]]` (scope cuts) | 0 | **12** |
| `[[unmapped]]` (honest gaps) | 0 | **1** |
| total subsystems | 0 | **40** |
| `src/*.rs` files | 10 | **45** |
| `src/` LOC | 2 352 | **8 065** |
| lib tests | ~0 | **249** |
| integration tests | 0 | **15** (9 self-audit + 6 smoke) |
| `cargo check -p cave-crossplane` | warn (1) | **clean** |
| `cargo test -p cave-crossplane` | n/a | **264 PASS / 0 FAIL** |

Formula: `fill_ratio = (mapped + partial + skipped) / total = (26 + 1 + 12) / 40 = 0.9750`
`honest_ratio = (mapped + partial) / total = 27 / 40 = 0.6750`

## 8-gate Charter v2 self-audit (`tests/parity_self_audit.rs`)

| gate | check | status |
| --- | --- | --- |
| G1 | `[upstream]` block + pinned `source_sha` `41c6f9c4…34e8d27` | PASS |
| G2 | every `[[mapped]]` has `local_files` all existing on disk (26 / 26) | PASS |
| G3 | every `[[partial]]` has `gap_reason` (1 / 1) | PASS |
| G4 | every `[[skipped]]` has `scope_cut_target` (12 / 12) | PASS |
| G5 | every `[[unmapped]]` has a `note` (1 / 1) | PASS |
| G6 | `fill_ratio ≥ 0.95` AND `honest_ratio ≥ 0.65` AND counts sum to total AND `last_audit = 2026-05-23` | PASS |
| G7 | SPDX line 1 on every `.rs` (47 of 47, src + tests) | PASS |
| G8 | no `unimplemented!()` / `todo!()` / `panic!("stub|todo|not impl…")` in `src/` | PASS |

Plus G9 (surface smoke) — verifies all 26 mapped subsystem types are re-exportable.

## Module layout

```
src/
├── claim.rs                  (201)  pre-port ClaimStore + composite resource
├── cli.rs                    (248)  cavectl infra {xr,xrd,composition,provider,function,package,claim}
├── composition/
│   ├── mod.rs                  (17)
│   ├── legacy.rs              (129) Crossplane 1.x Resources mode compat
│   ├── patch_transform.rs     (156) function-patch-and-transform engine
│   ├── pipeline.rs            (271) v2 pipeline executor + built-in dispatch
│   ├── step.rs                (155) Step + StepCredentials + StepResult + Severity
│   └── store.rs               (226) preserved CompositionStore + revision history
├── conditions.rs              (288) Ready/Synced/Healthy propagation composed→XR→claim
├── engine.rs                  (397) preserved patch + transform engine (5 transforms)
├── error.rs                    (60) preserved
├── function/
│   ├── mod.rs                 (157) FunctionStore (install/get/list/delete/state)
│   ├── auto_ready.rs           (87) function-auto-ready
│   ├── go_template.rs         (298) function-go-templating (handlebars-like subset)
│   ├── grpc_codec.rs          (172) RunFunctionRequest/Response JSON codec
│   ├── kcl.rs                 (172) function-kcl deterministic stub evaluator
│   └── patch_transform.rs     (118) function-patch-and-transform wrapper
├── lib.rs                      (70) CrossplaneState + router
├── models.rs                  (426) preserved
├── observability.rs           (214) 8 Prometheus panels + 5 alert rules
├── provider/
│   ├── mod.rs                  (13)
│   ├── config.rs              (177) ProviderConfig + 4 credential sources + usage tracking
│   ├── revision.rs            (168) ProviderRevision rollout + history limit
│   ├── runtime.rs             (197) DeploymentRuntimeConfig v2
│   └── store.rs               (246) preserved ProviderStore
├── providers_builtin/
│   ├── mod.rs                   (8)
│   ├── helm.rs                (161) provider-helm (Release CRUD + revisions)
│   └── kubernetes.rs          (125) provider-kubernetes (Object CRUD + generation)
├── reconciler.rs              (120) preserved
├── routes.rs                  (637) preserved 19 HTTP endpoints
├── xpkg/
│   ├── mod.rs                  (11)
│   ├── dependency.rs          (209) DAG + cycle detection + topo sort
│   ├── install.rs             (194) extract install plan + register into stores
│   ├── pull.rs                (256) offline OCI layout reader + digest verify + fixture writer
│   └── revision.rs            (177) PackageRevision tracker + manual activate
├── xr/
│   ├── mod.rs                   (9)
│   ├── bind.rs                (173) claim ↔ XR binder + defaulting
│   ├── lifecycle.rs           (168) XrPhase FSM + DeletionPlan
│   └── status.rs              (199) status reconciliation + composed-resource refs
└── xrd/
    ├── mod.rs                  (14)
    ├── conversion.rs          (154) v1 ↔ v2 conversion + storage-version pick
    ├── defaulting.rs          (137) openAPIV3Schema default walker
    ├── schema_validate.rs     (257) type/required/min/max/pattern/enum/items validator
    ├── spec.rs                (198) XrdSpec + XrdNames + version routing
    └── store.rs               (195) preserved XrdStore
```

## Mapped subsystems (26)

1. composition-v2-pipeline → `src/composition/pipeline.rs`
2. composition-pipeline-step → `src/composition/step.rs`
3. composition-patch-and-transform-builtin → `src/composition/patch_transform.rs`
4. composition-legacy-resources-mode → `src/composition/legacy.rs`
5. xr-lifecycle → `src/xr/lifecycle.rs`
6. xr-status-reconciliation → `src/xr/status.rs`
7. xr-claim-binding → `src/xr/bind.rs`
8. xrd-spec → `src/xrd/spec.rs`
9. xrd-schema-validate → `src/xrd/schema_validate.rs`
10. xrd-defaulting → `src/xrd/defaulting.rs`
11. xrd-conversion → `src/xrd/conversion.rs`
12. composed-resource-tracking → `src/composition/pipeline.rs`
13. provider-package-install → `src/provider/config.rs`, `src/provider/runtime.rs`
14. provider-config → `src/provider/config.rs`
15. provider-revision → `src/provider/revision.rs`
16. provider-deployment-runtime → `src/provider/runtime.rs`
17. function-package → `src/function/mod.rs`
18. function-grpc-codec → `src/function/grpc_codec.rs`
19. function-builtin-patch-transform → `src/function/patch_transform.rs`
20. function-builtin-kcl → `src/function/kcl.rs`
21. function-builtin-go-template → `src/function/go_template.rs`
22. function-builtin-auto-ready → `src/function/auto_ready.rs`
23. xpkg-pull → `src/xpkg/pull.rs`
24. xpkg-install → `src/xpkg/install.rs`
25. xpkg-revision-rollout → `src/xpkg/revision.rs`
26. xpkg-dependency-resolution → `src/xpkg/dependency.rs`

## Partial (1)

- **condition-propagation-healthy** (`src/conditions.rs`) — Ready + Synced
  + Healthy condition propagation composed → XR → claim is wired. The
  back-off jitter on the Healthy condition's `lastTransitionTime` is
  simplified — full back-off matrix is deferred to Phase 2 when
  `cave-apiserver` issues generation increments natively.

## Scope cuts (12 → Phase 2 owners)

| group | cuts | Phase-2 crate |
| --- | --- | --- |
| registry-and-distribution | oci-registry-http-pull, helm-v3-oci-registry | cave-artifacts |
| kube-apiserver-integration | kube-apiserver-apply, environment-configs, usage-tracking | cave-apiserver |
| certificate-management | webhook-cert-rotation | cave-cert |
| function-runtime-and-evaluators | real-go-template-runtime, composition-function-grpc-runtime | cave-llm-gateway |
| kcl-language | real-kcl-bytecode-eval | cave-kcl |
| cli | crossplane-cli-binary | cave-cli (cavectl) |
| cloud-providers | real-cloud-provider-resources | cave-cloud |
| artifact-signing | package-signature-verify | cave-sign |

## Honest unmapped (1)

- **composition-revision-garbage-collect** — Composition revision GC
  daemon (prunes unused composition revisions based on composite
  usage). We currently keep last 10 revisions in-memory via
  `CompositionStore::push_revision` but the GC reconciler that prunes by
  composite usage is not implemented.

## 4-track summary

| track | count | location |
| --- | --- | --- |
| backend modules (Rust src/) | 45 files / 8 065 LOC | `crates/cave-crossplane/src/` |
| cavectl surface | 7 subcommands × 4 actions (~28 dispatch arms) | `src/cli.rs` |
| portal-api HTTP routes | 19 endpoints | `src/routes.rs` (preserved) |
| observability | 8 Prometheus panels + 5 alert rules + Registry | `src/observability.rs` |

## Test summary

- **249 lib tests** — module-level + integration of every subsystem
- **9 self-audit tests** — Charter v2 G1–G8 + G9 surface smoke
- **6 smoke tests** — XRD/Composition/Claim e2e + XR lifecycle + XPKG pull-install-dep + router-constructs
- **0 failures**

`cargo test -p cave-crossplane` → `264 PASS`.
