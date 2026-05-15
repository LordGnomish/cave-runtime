# Portal admin → cave-apiserver wiring

**Date:** 2026-05-12
**Status:** **Landed** (partial — see "Not wired" below).
**Owner:** runtime

## Why

`AdminState::seeded()` fixtures were the only data source backing the
`/admin/*` views. Now that the real cave-runtime is up (TLS apiserver on
6443, real bootstrap join, WAL crash recovery), Portal needs a path to
display live cluster data instead of canned rows.

## What landed

### `cave_portal::admin::runtime_client`

New module, two implementations of a `RuntimeClient` trait:

```rust
#[async_trait]
pub trait RuntimeClient: Send + Sync + std::fmt::Debug {
    async fn list_kubelet_pods(&self, tenant: &TenantId)         -> Result<Vec<KubeletPod>, RuntimeError>;
    async fn list_scheduler_nodes(&self, tenant: &TenantId)      -> Result<Vec<SchedulerNode>, RuntimeError>;
    async fn list_net_endpoints(&self, tenant: &TenantId)        -> Result<Vec<NetEndpoint>, RuntimeError>;
    async fn list_keda_scaled_objects(&self, tenant: &TenantId)  -> Result<Vec<KedaScaledObject>, RuntimeError>;
    async fn list_vault_secrets(&self, tenant: &TenantId)        -> Result<Vec<VaultSecretMeta>, RuntimeError>;
}
```

* **`MockClient`** — empty-vec implementation. When `AdminState` has no
  runtime installed, the materialise methods are no-ops; the views read
  from the existing seeded fixtures. The MockClient exists so axum
  handler code is the same shape in both modes.
* **`ApiserverClient`** — reqwest 0.12 client with a CA pinned from
  `kubeconfig.clusters[0].cluster.certificate-authority-data`, mTLS
  identity from the user block, optional bearer token. Endpoints called:
  - `GET /api/v1/pods` → `KubeletPod`
  - `GET /api/v1/nodes` → `SchedulerNode` (cpu/memory quantities parsed
    via `parse_cpu_quantity` / `parse_mem_quantity_mib`)
  - `GET /api/v1/endpoints` → `NetEndpoint` (subsets flattened, ready vs
    notReadyAddresses preserved)
  - `GET /apis/keda.sh/v1alpha1/scaledobjects` → `KedaScaledObject`
    (triggers + currentReplicas extracted from CRD status)

### Kubeconfig parser

`ApiserverConfig::from_kubeconfig(path)` reads a kubeconfig YAML (the
same shape `cluster::render_kubeconfig` writes), picks the
`current-context`, and decodes the base64 CA + client cert + key into
the structured config a reqwest client needs. Bearer-token path
supported for bootstrap-time joins.

### AdminState seam

```rust
pub struct AdminState {
    pub runtime_client: OnceLock<SharedRuntime>,
    // ... existing RwLock<Vec<T>> fixture collections ...
}

impl AdminState {
    pub fn set_runtime_client(&self, client: SharedRuntime);
    pub fn runtime(&self) -> Option<&SharedRuntime>;

    pub async fn materialise_kubelet_pods(&self, tenant: &TenantId)         -> Result<(), RuntimeError>;
    pub async fn materialise_scheduler_nodes(&self, tenant: &TenantId)      -> Result<(), RuntimeError>;
    pub async fn materialise_net_endpoints(&self, tenant: &TenantId)        -> Result<(), RuntimeError>;
    pub async fn materialise_keda_scaled_objects(&self, tenant: &TenantId)  -> Result<(), RuntimeError>;
}
```

`materialise_*` is per-tenant: rows for OTHER tenants are preserved.
This means an apiserver refresh for `acme` doesn't stomp on the `evil`
fixture row used by the cross-tenant filter tests.

### Handler refactor

Four `/admin/*` handlers in `cave_portal::admin::mod::*_handler` now
call the corresponding `materialise_*` before invoking the existing
sync `render(&state, &ctx)`:

```rust
async fn kubelet_handler(...) -> ... {
    let ctx = extract_ctx_from_query(q);
    if let Err(e) = state.materialise_kubelet_pods(&ctx.tenant).await {
        tracing::warn!(error = %e, "kubelet materialise failed; falling back to cached rows");
    }
    kubelet::render(&state, &ctx).map(Html).map_err(err_to_response)
}
```

The fallback is intentional: if the apiserver is unreachable mid-render,
the operator sees the last successful materialisation (or the seeded
fixture if this is the first call), not a 500.

### Startup hook

`cave-runtime serve`:

```rust
let admin_state = Arc::new(cave_portal::admin::state::AdminState::seeded());
match probe_data_dir_for_runtime(&admin_state, cli.data_dir.as_deref()) {
    WireOutcome::Wired              => info!("portal admin → ApiserverClient"),
    WireOutcome::NoDataDir          => info!("portal admin → seeded fixtures"),
    WireOutcome::KubeconfigBroken   => warn!("kubeconfig unparseable, falling back"),
}
```

The probe is idempotent (OnceLock-backed) and never panics on missing
or malformed kubeconfig — the dashboard always works.

## Not wired (deferred with reason)

* **Vault secrets** — vault has its own API surface (`/v1/secret/metadata/<path>`
  with `X-Vault-Token`), not served by the k8s-style apiserver. A proper
  adoption needs a separate `VaultClient` configured from the data-dir's
  vault.json. Out of scope here; `ApiserverClient::list_vault_secrets`
  returns `RuntimeError::NotWired { resource: "vault_secrets" }`.
* **Admin router mounting** — `cave_portal::admin::router(state)` exists
  but isn't currently mounted into `cave-runtime serve`. The startup hook
  builds the AdminState and probes for a runtime, but the HTTP routes
  themselves aren't yet on the public app. Follow-up.
* The 4 wired pages cover Backend + Workloads + Network + Autoscaling
  shapes. The 80-odd remaining admin pages stay on fixtures until each
  gets its own `materialise_*` method.

## Tests

20 new tests in `cave_portal::admin::runtime_client::tests`:

* MockClient: 1 no-op test for all 5 methods.
* ApiserverClient (httpmock-driven):
  - `list_kubelet_pods` phase + restart-count + tenant-label mapping.
  - `list_scheduler_nodes` Ready condition + cpu/memory quantity parse.
  - `list_net_endpoints` subset flattening + ready/notReady split.
  - `list_keda_scaled_objects` CRD spec + triggers + currentReplicas.
  - Error paths: 401, 503, garbage JSON.
  - `list_vault_secrets` returns `NotWired`.
* CPU + memory quantity helpers (units `m`/`Mi`/`Gi`/`Ti`).
* Kubeconfig: roundtrip from `cluster::render_kubeconfig` format +
  missing-context error.
* AdminState seam: materialise against mock apiserver, no-op when no
  runtime set, preserves other-tenant rows, `set_runtime_client` is
  idempotent.
* Probe: no-data-dir branch, missing-kubeconfig branch, valid-kubeconfig
  branch.

`cargo test -p cave-portal --lib` → **981 + 20 = 1001 pass, 1 ignored**
(the existing `live_snapshot_dual_grade_prints` diagnostic).
`cargo check --workspace` clean.

## Files changed

```
crates/cave-portal/Cargo.toml                     +6  / -1   (reqwest, base64, serde_yaml, httpmock)
crates/cave-portal/src/admin/mod.rs                +18 / -4   (4 handler refactors + module decl)
crates/cave-portal/src/admin/runtime_client.rs    +853       (new — RuntimeClient + MockClient + ApiserverClient + probe + 20 tests)
crates/cave-portal/src/admin/state.rs              +75 / -2   (runtime_client field + 4 materialise + replace_tenant_rows)
crates/cave-runtime/src/main.rs                    +18        (AdminState construction + probe + log)
docs/synergy/portal-runtime-wiring-2026-05-12.md   +135       (this doc)
```
