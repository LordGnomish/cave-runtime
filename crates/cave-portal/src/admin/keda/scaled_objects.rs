// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/keda/scaledobjects` — ScaledObject CRUD UI.
//!
//! Mirrors upstream's `oc get scaledobject -o yaml` + Backstage `KedaCard`
//! shape. The list view is the entry point; the detail view renders the
//! full CRD schema; create/edit forms walk the operator through every
//! field the upstream CRD accepts; delete is a confirm-then-go flow.

use crate::admin::keda::scalers;
use crate::admin::keda::types::{
    KedaAdvanced, KedaAuthRef, KedaFallback, KedaHealth, KedaScaleTargetRef,
    KedaScaledObjectDetail, KedaScaledObjectStatus, KedaTrigger,
};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, scope};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("scaled object {namespace}/{name} not found")]
    NotFound { namespace: String, name: String },
    #[error("scaled object {namespace}/{name} already exists")]
    AlreadyExists { namespace: String, name: String },
    #[error("invalid field: {0}")]
    Invalid(String),
}

// ── Accessors ──────────────────────────────────────────────────────────────

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<KedaScaledObjectDetail>, Error> {
    ctx.authorise(Permission::KedaScaledObjectRead)?;
    Ok(scope(
        &state.keda_scaled_object_details.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect())
}

pub fn get(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<KedaScaledObjectDetail, Error> {
    ctx.authorise(Permission::KedaScaledObjectRead)?;
    state
        .keda_scaled_object_details
        .read()
        .unwrap()
        .iter()
        .find(|r| r.tenant == ctx.tenant && r.namespace == namespace && r.name == name)
        .cloned()
        .ok_or_else(|| Error::NotFound {
            namespace: namespace.into(),
            name: name.into(),
        })
}

// ── Mutators ───────────────────────────────────────────────────────────────

pub fn create(
    state: &AdminState,
    ctx: &RequestCtx,
    so: KedaScaledObjectDetail,
) -> Result<(), Error> {
    ctx.authorise(Permission::KedaScaledObjectWrite)?;
    validate(&so)?;
    let mut sos = state.keda_scaled_object_details.write().unwrap();
    if sos
        .iter()
        .any(|r| r.tenant == ctx.tenant && r.namespace == so.namespace && r.name == so.name)
    {
        return Err(Error::AlreadyExists {
            namespace: so.namespace,
            name: so.name,
        });
    }
    // Force the row to land under the caller's tenant — never trust the
    // tenant field in the payload.
    let mut row = so;
    row.tenant = ctx.tenant.clone();
    sos.push(row);
    Ok(())
}

pub fn update(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
    edit: impl FnOnce(&mut KedaScaledObjectDetail),
) -> Result<(), Error> {
    ctx.authorise(Permission::KedaScaledObjectWrite)?;
    let mut sos = state.keda_scaled_object_details.write().unwrap();
    let target = sos
        .iter_mut()
        .find(|r| r.tenant == ctx.tenant && r.namespace == namespace && r.name == name)
        .ok_or_else(|| Error::NotFound {
            namespace: namespace.into(),
            name: name.into(),
        })?;
    edit(target);
    validate(target)?;
    Ok(())
}

pub fn delete(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<(), Error> {
    ctx.authorise(Permission::KedaScaledObjectWrite)?;
    let mut sos = state.keda_scaled_object_details.write().unwrap();
    let before = sos.len();
    sos.retain(|r| !(r.tenant == ctx.tenant && r.namespace == namespace && r.name == name));
    if sos.len() == before {
        return Err(Error::NotFound {
            namespace: namespace.into(),
            name: name.into(),
        });
    }
    Ok(())
}

fn validate(so: &KedaScaledObjectDetail) -> Result<(), Error> {
    if so.name.is_empty() {
        return Err(Error::Invalid("name cannot be empty".into()));
    }
    if so.namespace.is_empty() {
        return Err(Error::Invalid("namespace cannot be empty".into()));
    }
    if so.scale_target_ref.kind.is_empty() || so.scale_target_ref.name.is_empty() {
        return Err(Error::Invalid(
            "scaleTargetRef.kind + .name required".into(),
        ));
    }
    if so.min_replica_count > so.max_replica_count {
        return Err(Error::Invalid(format!(
            "minReplicaCount ({}) must be <= maxReplicaCount ({})",
            so.min_replica_count, so.max_replica_count
        )));
    }
    if let Some(idle) = so.idle_replica_count {
        if idle >= so.min_replica_count {
            return Err(Error::Invalid(format!(
                "idleReplicaCount ({}) must be < minReplicaCount ({})",
                idle, so.min_replica_count
            )));
        }
    }
    if so.triggers.is_empty() {
        return Err(Error::Invalid("at least one trigger is required".into()));
    }
    for t in &so.triggers {
        if scalers::lookup(&t.kind).is_none() {
            return Err(Error::Invalid(format!(
                "trigger type `{}` is not in the registered scaler catalog",
                t.kind
            )));
        }
    }
    Ok(())
}

// ── Render ─────────────────────────────────────────────────────────────────

pub fn render_list(state: &AdminState, ctx: &RequestCtx) -> Result<String, Error> {
    let sos = list(state, ctx)?;
    let rows: Vec<Vec<String>> = sos
        .iter()
        .map(|s| {
            vec![
                s.namespace.clone(),
                s.name.clone(),
                format!("{}/{}", s.scale_target_ref.kind, s.scale_target_ref.name),
                format!("{}/{}", s.min_replica_count, s.max_replica_count),
                s.idle_replica_count
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "—".into()),
                s.status
                    .last_active_time
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "—".into()),
                s.triggers.len().to_string(),
                s.status.health.overall.clone(),
                if is_paused(s) { "yes" } else { "no" }.into(),
            ]
        })
        .collect();
    let body = format!(
        r#"<div class="flex justify-between items-center mb-3">
  <h2 class="text-lg font-semibold">ScaledObjects ({n})</h2>
  <a class="px-3 py-1 rounded bg-blue-600 text-white" href="/admin/keda/scaledobjects/new?tenant_id={tenant}">+ New</a>
</div>
{tbl}
<p class="mt-3 text-xs text-gray-500">Click a row to drill into the full CRD detail.</p>"#,
        n = sos.len(),
        tenant = escape(ctx.tenant.as_str()),
        tbl = table(
            &[
                "namespace",
                "name",
                "scaleTargetRef",
                "min/max",
                "idle",
                "lastActiveTime",
                "triggers",
                "health",
                "paused",
            ],
            &rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/keda/scaledobjects",
        &format!("keda · scaledobjects · {}", ctx.tenant.as_str()),
        &body,
    ))
}

pub fn render_detail(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<String, Error> {
    let so = get(state, ctx, namespace, name)?;

    let triggers_tbl = render_triggers(&so.triggers);
    let target = render_scale_target(&so.scale_target_ref);
    let annotations = render_annotations(&so.annotations);
    let advanced = so
        .advanced
        .as_ref()
        .map(render_advanced)
        .unwrap_or_else(|| "<p class=\"text-sm text-gray-500\">—</p>".into());
    let fallback = so
        .fallback
        .as_ref()
        .map(render_fallback)
        .unwrap_or_else(|| "<p class=\"text-sm text-gray-500\">—</p>".into());
    let status_block = render_status(&so.status);

    let body = format!(
        r#"<a class="text-blue-700 underline" href="/admin/keda/scaledobjects?tenant_id={tenant}">← all scaledobjects</a>
<h2 class="text-xl font-semibold mt-2">{ns}/{name}</h2>
<dl class="mt-2 grid grid-cols-[12rem_1fr] gap-x-4 gap-y-1 text-sm">
  <dt class="text-gray-500">minReplicaCount</dt><dd>{min}</dd>
  <dt class="text-gray-500">maxReplicaCount</dt><dd>{max}</dd>
  <dt class="text-gray-500">idleReplicaCount</dt><dd>{idle}</dd>
  <dt class="text-gray-500">pollingInterval</dt><dd>{poll}s</dd>
  <dt class="text-gray-500">cooldownPeriod</dt><dd>{cool}s</dd>
  <dt class="text-gray-500">initialCooldownPeriod</dt><dd>{icool}s</dd>
</dl>
<section class="mt-6"><h3 class="text-md font-semibold mb-1">scaleTargetRef</h3>{target}</section>
<section class="mt-6"><h3 class="text-md font-semibold mb-1">annotations</h3>{annotations}</section>
<section class="mt-6"><h3 class="text-md font-semibold mb-1">triggers ({trig_count})</h3>{triggers}</section>
<section class="mt-6"><h3 class="text-md font-semibold mb-1">advanced</h3>{advanced}</section>
<section class="mt-6"><h3 class="text-md font-semibold mb-1">fallback</h3>{fallback}</section>
<section class="mt-6"><h3 class="text-md font-semibold mb-1">status</h3>{status_block}</section>
<section class="mt-6 flex gap-3">
  <a class="px-3 py-1 rounded bg-yellow-100 border border-yellow-300" href="/admin/keda/scaledobjects/{ns}/{name}/edit?tenant_id={tenant}">Edit</a>
  <a class="px-3 py-1 rounded bg-red-100 border border-red-300" href="/admin/keda/scaledobjects/{ns}/{name}/delete?tenant_id={tenant}">Delete…</a>
</section>"#,
        tenant = escape(ctx.tenant.as_str()),
        ns = escape(&so.namespace),
        name = escape(&so.name),
        min = so.min_replica_count,
        max = so.max_replica_count,
        idle = so
            .idle_replica_count
            .map(|i| i.to_string())
            .unwrap_or_else(|| "—".into()),
        poll = so.polling_interval_secs,
        cool = so.cooldown_period_secs,
        icool = so.initial_cooldown_period_secs,
        target = target,
        annotations = annotations,
        trig_count = so.triggers.len(),
        triggers = triggers_tbl,
        advanced = advanced,
        fallback = fallback,
        status_block = status_block,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/keda/scaledobjects",
        &format!("keda · {}/{}", so.namespace, so.name),
        &body,
    ))
}

pub fn render_new_form(_state: &AdminState, ctx: &RequestCtx) -> Result<String, Error> {
    ctx.authorise(Permission::KedaScaledObjectWrite)?;
    let scaler_options = scalers::all()
        .iter()
        .map(|e| {
            format!(
                r#"<option value="{kind}">{kind} — {summary}</option>"#,
                kind = escape(e.kind),
                summary = escape(e.summary),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let body = format!(
        r#"<form method="post" action="/admin/keda/scaledobjects?tenant_id={tenant}" class="grid grid-cols-2 gap-3 max-w-3xl">
  <label class="col-span-1">namespace<input class="w-full border rounded px-2 py-1" name="namespace" required></label>
  <label class="col-span-1">name<input class="w-full border rounded px-2 py-1" name="name" required></label>
  <label class="col-span-1">scaleTargetRef.kind<input class="w-full border rounded px-2 py-1" name="target_kind" value="Deployment" required></label>
  <label class="col-span-1">scaleTargetRef.name<input class="w-full border rounded px-2 py-1" name="target_name" required></label>
  <label class="col-span-1">minReplicaCount<input class="w-full border rounded px-2 py-1" name="min_replica_count" type="number" value="0"></label>
  <label class="col-span-1">maxReplicaCount<input class="w-full border rounded px-2 py-1" name="max_replica_count" type="number" value="100"></label>
  <label class="col-span-1">pollingInterval (s)<input class="w-full border rounded px-2 py-1" name="polling_interval_secs" type="number" value="30"></label>
  <label class="col-span-1">cooldownPeriod (s)<input class="w-full border rounded px-2 py-1" name="cooldown_period_secs" type="number" value="300"></label>
  <label class="col-span-2">first trigger type
    <select class="w-full border rounded px-2 py-1" name="trigger_kind">
      {scaler_options}
    </select>
  </label>
  <button class="col-span-2 px-3 py-2 rounded bg-blue-600 text-white" type="submit">Create</button>
</form>
<p class="mt-3 text-xs text-gray-500">The trigger dropdown is sourced from <a class="underline" href="/admin/keda/scalers?tenant_id={tenant}">the live scaler catalog</a> ({n_scalers} entries). Edit-after-create lets you append more triggers with full metadata.</p>"#,
        tenant = escape(ctx.tenant.as_str()),
        scaler_options = scaler_options,
        n_scalers = scalers::all().len(),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/keda/scaledobjects",
        "keda · new scaledobject",
        &body,
    ))
}

pub fn render_edit_yaml(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<String, Error> {
    let so = get(state, ctx, namespace, name)?;
    let yaml = to_yaml(&so);
    let body = format!(
        r#"<a class="text-blue-700 underline" href="/admin/keda/scaledobjects/{ns}/{n}?tenant_id={tenant}">← back</a>
<h2 class="text-lg font-semibold mt-2">edit {ns}/{n}</h2>
<form method="post" action="/admin/keda/scaledobjects/{ns}/{n}?tenant_id={tenant}">
  <textarea class="w-full h-96 font-mono text-sm border rounded p-2" name="yaml">{yaml}</textarea>
  <div class="mt-3 flex gap-3">
    <button class="px-3 py-1 rounded bg-blue-600 text-white" type="submit">Apply</button>
    <a class="px-3 py-1 rounded bg-gray-100 border" href="/admin/keda/scaledobjects/{ns}/{n}?tenant_id={tenant}">Cancel</a>
  </div>
</form>"#,
        tenant = escape(ctx.tenant.as_str()),
        ns = escape(&so.namespace),
        n = escape(&so.name),
        yaml = escape(&yaml),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/keda/scaledobjects",
        &format!("keda · edit {}/{}", so.namespace, so.name),
        &body,
    ))
}

pub fn render_delete_confirm(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<String, Error> {
    let so = get(state, ctx, namespace, name)?;
    let body = format!(
        r#"<div class="rounded border border-red-300 bg-red-50 px-4 py-3 max-w-2xl">
  <h2 class="text-lg font-semibold mb-2">Delete ScaledObject {ns}/{n}?</h2>
  <p class="text-sm mb-3">This unsets KEDA's HPA on <code class="bg-white px-1 rounded">{tgt}</code>. If <code class="bg-white px-1 rounded">spec.advanced.restoreToOriginalReplicaCount</code> is set, KEDA restores the workload to its pre-attach replica count ({orig}).</p>
  <form method="post" action="/admin/keda/scaledobjects/{ns}/{n}/delete?tenant_id={tenant}" class="flex gap-3">
    <button class="px-3 py-1 rounded bg-red-600 text-white" type="submit">Delete</button>
    <a class="px-3 py-1 rounded bg-white border" href="/admin/keda/scaledobjects/{ns}/{n}?tenant_id={tenant}">Cancel</a>
  </form>
</div>"#,
        tenant = escape(ctx.tenant.as_str()),
        ns = escape(&so.namespace),
        n = escape(&so.name),
        tgt = format!("{}/{}", so.scale_target_ref.kind, so.scale_target_ref.name),
        orig = so.status.original_replica_count,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/keda/scaledobjects",
        &format!("keda · delete {}/{}", so.namespace, so.name),
        &body,
    ))
}

// ── Render helpers ─────────────────────────────────────────────────────────

fn render_scale_target(t: &KedaScaleTargetRef) -> String {
    table(
        &["apiVersion", "kind", "name", "envSourceContainerName"],
        &[vec![
            t.api_version.clone(),
            t.kind.clone(),
            t.name.clone(),
            t.env_source_container_name.clone().unwrap_or_default(),
        ]],
    )
}

fn render_annotations(items: &[(String, String)]) -> String {
    if items.is_empty() {
        return "<p class=\"text-sm text-gray-500\">—</p>".into();
    }
    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|(k, v)| vec![k.clone(), v.clone()])
        .collect();
    table(&["key", "value"], &rows)
}

fn render_triggers(triggers: &[KedaTrigger]) -> String {
    let mut out = String::new();
    for (i, t) in triggers.iter().enumerate() {
        let auth = t
            .auth_ref
            .as_ref()
            .map(render_auth_ref)
            .unwrap_or_else(|| "—".into());
        let metadata_rows: Vec<Vec<String>> = t
            .metadata
            .iter()
            .map(|(k, v)| vec![k.clone(), v.clone()])
            .collect();
        let metadata_html = if metadata_rows.is_empty() {
            "<p class=\"text-sm text-gray-500\">no metadata</p>".into()
        } else {
            table(&["key", "value"], &metadata_rows)
        };
        let catalog = scalers::lookup(&t.kind)
            .map(|e| format!(
                r#"<a class="text-blue-700 underline" href="{u}" target="_blank" rel="noopener">docs</a>"#,
                u = escape(e.docs_url)
            ))
            .unwrap_or_else(|| "<span class=\"text-red-600\">unknown scaler</span>".into());
        out.push_str(&format!(
            r#"<details class="border rounded mb-2"{open}>
  <summary class="px-3 py-2 cursor-pointer bg-gray-100"><strong>#{i}</strong> {kind} {name} · {metric_type} · {cache} · authRef: {auth} · {catalog}</summary>
  <div class="px-3 py-2">{metadata_html}</div>
</details>"#,
            open = if i == 0 { " open" } else { "" },
            i = i + 1,
            kind = escape(&t.kind),
            name = t.name.as_deref().map(|s| format!("({})", escape(s))).unwrap_or_default(),
            metric_type = escape(&t.metric_type),
            cache = if t.use_cached_metrics { "cached" } else { "live" },
            auth = auth,
            catalog = catalog,
            metadata_html = metadata_html,
        ));
    }
    out
}

fn render_auth_ref(a: &KedaAuthRef) -> String {
    format!(
        r#"<a class="underline" href="/admin/keda/triggerauthentications">{kind}/{name}</a>"#,
        kind = escape(&a.kind),
        name = escape(&a.name),
    )
}

fn render_advanced(a: &KedaAdvanced) -> String {
    format!(
        r#"<dl class="grid grid-cols-[16rem_1fr] gap-x-4 gap-y-1 text-sm">
  <dt class="text-gray-500">restoreToOriginalReplicaCount</dt><dd>{restore}</dd>
  <dt class="text-gray-500">hpaName</dt><dd>{hpa}</dd>
  <dt class="text-gray-500">hpaBehavior</dt>
  <dd><pre class="bg-gray-50 rounded p-2 text-xs overflow-x-auto">{behavior}</pre></dd>
</dl>"#,
        restore = a.restore_to_original_replica_count,
        hpa = escape(a.hpa_name.as_deref().unwrap_or("—")),
        behavior = escape(a.hpa_behavior_yaml.as_deref().unwrap_or("—")),
    )
}

fn render_fallback(f: &KedaFallback) -> String {
    format!(
        r#"<p class="text-sm">on <code>{n}</code> consecutive failures, hold at <code>{r}</code> replicas</p>"#,
        n = f.failure_threshold,
        r = f.replicas,
    )
}

fn render_status(s: &KedaScaledObjectStatus) -> String {
    let active = if s.active_triggers.is_empty() {
        "—".into()
    } else {
        s.active_triggers.join(", ")
    };
    format!(
        r#"<dl class="grid grid-cols-[16rem_1fr] gap-x-4 gap-y-1 text-sm">
  <dt class="text-gray-500">lastActiveTime</dt><dd>{lat}</dd>
  <dt class="text-gray-500">originalReplicaCount</dt><dd>{orc}</dd>
  <dt class="text-gray-500">health</dt><dd>{h_status} — {h_msg}</dd>
  <dt class="text-gray-500">activeTriggers</dt><dd>{active}</dd>
  <dt class="text-gray-500">reason</dt><dd>{reason}</dd>
</dl>"#,
        lat = s
            .last_active_time
            .map(|t| t.to_string())
            .unwrap_or_else(|| "—".into()),
        orc = s.original_replica_count,
        h_status = render_health_pill(&s.health),
        h_msg = escape(&s.health.message),
        active = escape(&active),
        reason = escape(&s.reason),
    )
}

fn render_health_pill(h: &KedaHealth) -> String {
    let class = match h.overall.as_str() {
        "Healthy" => "bg-green-100 text-green-800",
        "Degraded" => "bg-yellow-100 text-yellow-800",
        "Unhealthy" => "bg-red-100 text-red-800",
        _ => "bg-gray-100 text-gray-800",
    };
    format!(
        r#"<span class="px-2 py-0.5 rounded text-xs {class}">{label}</span>"#,
        class = class,
        label = escape(&h.overall),
    )
}

/// Cheap YAML projection of a ScaledObject for the edit textarea. Not a
/// round-trippable serializer — operators using the YAML editor are
/// expected to re-submit using the form path. Documented in the UI text.
fn to_yaml(so: &KedaScaledObjectDetail) -> String {
    let mut out = String::new();
    out.push_str("apiVersion: keda.sh/v1alpha1\nkind: ScaledObject\nmetadata:\n");
    out.push_str(&format!(
        "  namespace: {}\n  name: {}\n",
        so.namespace, so.name
    ));
    if !so.annotations.is_empty() {
        out.push_str("  annotations:\n");
        for (k, v) in &so.annotations {
            out.push_str(&format!("    {}: \"{}\"\n", k, v));
        }
    }
    out.push_str("spec:\n  scaleTargetRef:\n");
    out.push_str(&format!(
        "    apiVersion: {}\n    kind: {}\n    name: {}\n",
        so.scale_target_ref.api_version, so.scale_target_ref.kind, so.scale_target_ref.name
    ));
    if let Some(env) = &so.scale_target_ref.env_source_container_name {
        out.push_str(&format!("    envSourceContainerName: {}\n", env));
    }
    out.push_str(&format!("  minReplicaCount: {}\n", so.min_replica_count));
    out.push_str(&format!("  maxReplicaCount: {}\n", so.max_replica_count));
    if let Some(idle) = so.idle_replica_count {
        out.push_str(&format!("  idleReplicaCount: {}\n", idle));
    }
    out.push_str(&format!(
        "  pollingInterval: {}\n",
        so.polling_interval_secs
    ));
    out.push_str(&format!("  cooldownPeriod: {}\n", so.cooldown_period_secs));
    if so.initial_cooldown_period_secs != 0 {
        out.push_str(&format!(
            "  initialCooldownPeriod: {}\n",
            so.initial_cooldown_period_secs
        ));
    }
    if let Some(fb) = &so.fallback {
        out.push_str(&format!(
            "  fallback:\n    failureThreshold: {}\n    replicas: {}\n",
            fb.failure_threshold, fb.replicas
        ));
    }
    out.push_str("  triggers:\n");
    for t in &so.triggers {
        out.push_str(&format!("  - type: {}\n", t.kind));
        if let Some(n) = &t.name {
            out.push_str(&format!("    name: {}\n", n));
        }
        if t.metric_type != "AverageValue" {
            out.push_str(&format!("    metricType: {}\n", t.metric_type));
        }
        if t.use_cached_metrics {
            out.push_str("    useCachedMetrics: true\n");
        }
        if !t.metadata.is_empty() {
            out.push_str("    metadata:\n");
            for (k, v) in &t.metadata {
                out.push_str(&format!("      {}: \"{}\"\n", k, v));
            }
        }
        if let Some(a) = &t.auth_ref {
            out.push_str(&format!(
                "    authenticationRef:\n      kind: {}\n      name: {}\n",
                a.kind, a.name
            ));
        }
    }
    out
}

fn is_paused(so: &KedaScaledObjectDetail) -> bool {
    so.annotations
        .iter()
        .any(|(k, v)| k == "autoscaling.keda.sh/paused" && v == "true")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::keda::types::{
        KedaScaleTargetRef, KedaScaledObjectDetail, KedaScaledObjectStatus, KedaTrigger,
    };
    use crate::admin::permission::Permission;
    use crate::admin::types::TenantId;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    fn minimal_so(name: &str) -> KedaScaledObjectDetail {
        KedaScaledObjectDetail {
            tenant: TenantId::new("acme").unwrap(),
            namespace: "default".into(),
            name: name.into(),
            annotations: vec![],
            scale_target_ref: KedaScaleTargetRef {
                api_version: "apps/v1".into(),
                kind: "Deployment".into(),
                name: name.into(),
                env_source_container_name: None,
            },
            min_replica_count: 0,
            max_replica_count: 10,
            idle_replica_count: None,
            polling_interval_secs: 30,
            cooldown_period_secs: 300,
            initial_cooldown_period_secs: 0,
            fallback: None,
            triggers: vec![KedaTrigger {
                kind: "cpu".into(),
                name: None,
                metadata: vec![("value".into(), "75".into())],
                auth_ref: None,
                metric_type: "Utilization".into(),
                use_cached_metrics: false,
            }],
            advanced: None,
            status: KedaScaledObjectStatus {
                last_active_time: None,
                original_replica_count: 0,
                health: KedaHealth {
                    overall: "Healthy".into(),
                    message: "".into(),
                },
                active_triggers: vec![],
                reason: "".into(),
            },
        }
    }

    #[test]
    fn list_filters_to_tenant_and_renders_expected_columns() {
        let state = AdminState::seeded();
        let html = render_list(&state, &ctx(&[Permission::KedaScaledObjectRead])).unwrap();
        // Backstage column headers — these must match the upstream
        // ScaledObject `oc get` short-form output one-for-one so an
        // operator can switch between tools without re-mapping fields.
        for expected in [
            "namespace",
            "name",
            "scaleTargetRef",
            "min/max",
            "idle",
            "lastActiveTime",
            "triggers",
            "health",
            "paused",
        ] {
            assert!(
                html.contains(&format!(">{}<", expected)),
                "missing column `{}` in list view",
                expected
            );
        }
        assert!(html.contains("ingest-worker"));
        assert!(html.contains("report-runner"));
        assert!(!html.contains("evil-worker")); // tenant scope
    }

    #[test]
    fn detail_view_renders_every_top_level_crd_section() {
        let state = AdminState::seeded();
        let html = render_detail(
            &state,
            &ctx(&[Permission::KedaScaledObjectRead]),
            "ingest",
            "ingest-worker",
        )
        .unwrap();
        for header in [
            "scaleTargetRef",
            "annotations",
            "triggers",
            "advanced",
            "fallback",
            "status",
        ] {
            assert!(html.contains(header), "missing detail section `{}`", header);
        }
        // Trigger expansion content (metadata keys) must appear too.
        assert!(html.contains("bootstrapServers"));
        assert!(html.contains("lagThreshold"));
        // Per-trigger docs link goes to keda.sh.
        assert!(html.contains("https://keda.sh/docs/2.14/scalers/apache-kafka/"));
    }

    #[test]
    fn list_without_permission_is_refused() {
        let state = AdminState::seeded();
        assert!(matches!(
            list(&state, &ctx(&[])).unwrap_err(),
            Error::Auth(_)
        ));
    }

    #[test]
    fn create_then_delete_roundtrips() {
        let state = AdminState::empty();
        let c = ctx(&[
            Permission::KedaScaledObjectRead,
            Permission::KedaScaledObjectWrite,
        ]);
        create(&state, &c, minimal_so("hello")).expect("create");
        assert_eq!(list(&state, &c).unwrap().len(), 1);
        delete(&state, &c, "default", "hello").expect("delete");
        assert!(list(&state, &c).unwrap().is_empty());
    }

    #[test]
    fn create_rejects_duplicate() {
        let state = AdminState::empty();
        let c = ctx(&[
            Permission::KedaScaledObjectWrite,
            Permission::KedaScaledObjectRead,
        ]);
        create(&state, &c, minimal_so("dupe")).unwrap();
        let err = create(&state, &c, minimal_so("dupe")).unwrap_err();
        assert!(matches!(err, Error::AlreadyExists { .. }));
    }

    #[test]
    fn validate_catches_min_above_max() {
        let mut so = minimal_so("bad");
        so.min_replica_count = 20;
        so.max_replica_count = 5;
        let err = validate(&so).unwrap_err();
        assert!(matches!(err, Error::Invalid(s) if s.contains("minReplicaCount")));
    }

    #[test]
    fn validate_rejects_idle_above_min() {
        let mut so = minimal_so("idle-bad");
        so.min_replica_count = 3;
        so.idle_replica_count = Some(5);
        let err = validate(&so).unwrap_err();
        assert!(matches!(err, Error::Invalid(s) if s.contains("idleReplicaCount")));
    }

    #[test]
    fn validate_rejects_unknown_trigger() {
        let mut so = minimal_so("unknown");
        so.triggers[0].kind = "nonexistent-scaler".into();
        let err = validate(&so).unwrap_err();
        assert!(matches!(err, Error::Invalid(s) if s.contains("scaler catalog")));
    }

    #[test]
    fn yaml_export_includes_keda_apiversion_and_target_ref() {
        let state = AdminState::seeded();
        let c = ctx(&[Permission::KedaScaledObjectRead]);
        let so = get(&state, &c, "ingest", "ingest-worker").unwrap();
        let yaml = to_yaml(&so);
        assert!(yaml.contains("apiVersion: keda.sh/v1alpha1"));
        assert!(yaml.contains("kind: ScaledObject"));
        assert!(yaml.contains("kind: Deployment"));
        assert!(yaml.contains("type: kafka"));
        assert!(yaml.contains("authenticationRef:"));
    }

    #[test]
    fn new_form_lists_every_catalog_entry_in_dropdown() {
        let state = AdminState::seeded();
        let html = render_new_form(&state, &ctx(&[Permission::KedaScaledObjectWrite])).unwrap();
        // Spot-check a handful from each category.
        for kind in [
            "kafka",
            "prometheus",
            "azure-eventhub",
            "gcp-pubsub",
            "cron",
            "cpu",
        ] {
            assert!(
                html.contains(&format!(r#"value="{}""#, kind)),
                "missing option `{}`",
                kind
            );
        }
    }

    #[test]
    fn delete_confirm_warns_about_restore() {
        let state = AdminState::seeded();
        let html = render_delete_confirm(
            &state,
            &ctx(&[Permission::KedaScaledObjectRead]),
            "ingest",
            "ingest-worker",
        )
        .unwrap();
        assert!(html.contains("restoreToOriginalReplicaCount"));
        assert!(html.contains("Delete"));
    }
}
