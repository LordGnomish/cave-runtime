# cave-kamaji TDD coverage audit

- **Crate:** `crates/compute/cave-kamaji` (theme: compute)
- **Upstream:** [clastix/kamaji](https://github.com/clastix/kamaji) @ `v1.0.0`
- **Upstream test inventory:** 19 test files / **2** symbols extracted by the
  Go-pattern scanner. This number is misleadingly low: Kamaji's behavioral
  suite is **Ginkgo BDD** (`Describe`/`Context`/`It`) e2e specs that the
  `^func Test` grep cannot see, plus one real unit test (`TestArgsFromSliceToMap`)
  and the suite bootstrap (`TestAPIs`). The portable behavior therefore lives in
  the **source** (`internal/utilities/*`, `internal/webhook/handlers/*`), not in
  greppable test symbols — this audit reads the source directly.
- **Cave test functions:** 86 `#[test]` / `#[tokio::test]` across `src/*.rs`.
  Coverage is already substantial; this audit reports only the *genuinely
  uncovered, portable* behaviors.

## Classification of upstream behavioral units

| Upstream unit (source) | Behavior | Cave status | Class |
|---|---|---|---|
| `utilities.ArgsFromSliceToMap` / `ArgsFromMapToSlice` | parse `flag=value` slice ↔ map; idempotent sort; bare flag w/o `=` | not implemented in cave | missing-impl |
| `utilities.ArgsRemoveFlag` / `ArgsAddFlagValue` | upsert/delete returning found bit | not implemented | missing-impl |
| `utilities.CalculateMapChecksum` | key-sorted md5 of ConfigMap/Secret values | not implemented | missing-impl |
| `utilities.MergeMaps` | last-wins map merge | not implemented (no public fn) | missing-impl |
| `utilities.GetControlPlaneAddressAndPortFromHostname` | split `host:port`, default port fallback | cave `cluster_api::parse_control_plane_endpoint` (analog) — **covered** by 3 tests | portable-covered |
| `handlers.TenantControlPlaneVersion` (downgrade / non-linear minor / > supported blocked) | semver compare on update | cave models version as **immutable** in `webhook::validate_update` — different design (no semver ladder) | scope-cut (design divergence) |
| `handlers.TenantControlPlaneKubeletAddresses` | reject duplicate preferred address types | not implemented | missing-impl |
| `handlers.TenantControlPlaneServiceCIDR` | DNS Service IP must be inside Service CIDR | not implemented | missing-impl |
| `handlers.TenantControlPlaneName` (DNS1035) | RFC1035 label validation | cave checks non-empty only (no DNS1035) | scope-cut (partial) |
| `internal/datastore/*` etcd/pg/mysql kine wiring, snapshot pruner | controller/CRD plumbing | cave model + `DataStore::validate` (**covered**) | scope-cut / portable-covered |
| Reconcilers, finalizers, controller-runtime resources, e2e tcp_* | infra/CRD/controller plumbing | n/a | scope-cut |
| **Status lifecycle transitions** (provision→running→delete; readiness) | mutate TCP status phase/ready/message | cave `lifecycle::*` + `status::status_summary` **implemented, NOT tested** | **portable-coverage (PRIORITY)** |
| **Validating webhook reject paths** (empty field / bad replicas / unknown datastore / immutable version) | admission validation | cave `webhook::validate_create`/`validate_update` **implemented, error branches NOT tested** | **portable-coverage (PRIORITY)** |
| **Konnectivity HTTP-CONNECT mode + token arg** | mode/port 8133 default, token path arg, server-host override | cave `Konnectivity` **implemented, those branches NOT tested** | **portable-coverage (PRIORITY)** |

## Already-covered portable behavior (no action)

- `cluster_api::{parse_control_plane_endpoint, build_capi_status, is_capi_ready,
  ControlPlaneEndpoint::to_url}` — 8 tests.
- `datastore::{validate (match + mismatch), connection_string}` — 3 tests.
- `pod_mgmt::plan_apiserver_pod` (etcd / kine endpoint selection, env) — 3 tests.
- `kubeadm::render_kubeadm_init_config` (both docs, deterministic) — 2 tests.
- `status::set_condition` (in-place update preserves order) — 1 test.
- `konnectivity::agent_manifest_args` gRPC mode + agent-id branch — 2 tests.

## Recommended TDD fills (portable-coverage first)

Each row names the **exact public cave fn** the new test would exercise.

| # | Cave fn | Module | Uncovered behavior to assert |
|---|---|---|---|
| 1 | `lifecycle::provision` | `lifecycle.rs` | sets `phase = Provisioning` and a non-empty `message` |
| 2 | `lifecycle::mark_running` | `lifecycle.rs` | sets `phase = Running`, `api_server_endpoint = Some(ep)`, `ready = true`, clears `message` |
| 3 | `lifecycle::deprovision` | `lifecycle.rs` | sets `phase = Deleting`, `ready = false`, sets deprovision `message` |
| 4 | `lifecycle::health_check` | `lifecycle.rs` | true only when `Running && ready`; false for every other phase / `ready=false` |
| 5 | `lifecycle::generate_kubeconfig` | `lifecycle.rs` | `Some(json)` with cluster server == endpoint when endpoint set; `None` when `api_server_endpoint` is `None` |
| 6 | `status::status_summary` | `status.rs` | Running+ready → all sub-conditions `True` incl. aggregate `Ready=True`; non-running → `ControlPlaneHealthy=False` and `Ready=False`; returns 5 condition types |
| 7 | `webhook::validate_create` | `webhook.rs` | rejects empty `name` (`EmptyField{field:"name"}`), empty `namespace`, `replicas < 1` (`InvalidReplicas`), and unrecognised `data_store` (`UnknownDataStore`) |
| 8 | `webhook::validate_update` | `webhook.rs` | rejects mutation of `spec.kubernetes_version` (`ImmutableField{field:"spec.kubernetes_version"}`); accepts an otherwise-identical update |
| 9 | `konnectivity::Konnectivity::new` + `agent_manifest_args` | `konnectivity.rs` | HTTP-CONNECT mode yields `--mode=http-connect` and `--proxy-server-port=8133`; gRPC default port is 8132 |
| 10 | `konnectivity::Konnectivity::with_agent_token` (+ `with_server_host`) | `konnectivity.rs` | after `with_agent_token`, args include `--service-account-token-path=…`; `with_server_host` overrides `--proxy-server-host=` |

### Notes on non-fills

- **Version semver ladder** (downgrade/non-linear/over-supported) is a deliberate
  *design divergence*: cave makes `kubernetes_version` immutable rather than
  porting Kamaji's `blang/semver` upgrade ladder. Not a coverage gap — it is a
  different invariant, already exercisable via fill #8.
- **ArgsFromSliceToMap/ToSlice, CalculateMapChecksum, MergeMaps, ServiceCIDR
  containment, kubelet duplicate-address-type, DNS1035 name** are *missing-impl*
  in the cave port (no public fn exists), so they are not TDD coverage fills —
  they would require source implementation first and are out of scope for a
  no-src-modification audit.
