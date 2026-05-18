// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/keda/scaledjobs` — ScaledJob CRUD UI.
//!
//! Mirrors upstream's `ScaledJob` CRD. ScaledJob differs from ScaledObject
//! in that it creates one Kubernetes Job per scaling event (rather than
//! scaling an existing Deployment). The schema, validation rules, and
//! list view are intentionally similar so operators don't have to learn
//! two different consoles.

use crate::admin::keda::scalers;
use crate::admin::keda::types::{KedaScaledJob, KedaTrigger};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("scaled job {namespace}/{name} not found")]
    NotFound { namespace: String, name: String },
    #[error("invalid field: {0}")]
    Invalid(String),
}

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<KedaScaledJob>, Error> {
    ctx.authorise(Permission::KedaScaledJobRead)?;
    Ok(scope(&state.keda_scaled_jobs.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn get(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<KedaScaledJob, Error> {
    ctx.authorise(Permission::KedaScaledJobRead)?;
    state
        .keda_scaled_jobs
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

fn validate(j: &KedaScaledJob) -> Result<(), Error> {
    if j.name.is_empty() || j.namespace.is_empty() {
        return Err(Error::Invalid("namespace + name required".into()));
    }
    if !matches!(j.scaling_strategy.as_str(), "default" | "custom" | "accurate") {
        return Err(Error::Invalid(format!(
            "scalingStrategy `{}` not in {{default, custom, accurate}}",
            j.scaling_strategy
        )));
    }
    if j.triggers.is_empty() {
        return Err(Error::Invalid("at least one trigger required".into()));
    }
    for t in &j.triggers {
        if scalers::lookup(&t.kind).is_none() {
            return Err(Error::Invalid(format!(
                "trigger type `{}` is not in the registered scaler catalog",
                t.kind
            )));
        }
    }
    Ok(())
}

pub fn create(state: &AdminState, ctx: &RequestCtx, sj: KedaScaledJob) -> Result<(), Error> {
    ctx.authorise(Permission::KedaScaledJobWrite)?;
    validate(&sj)?;
    let mut rows = state.keda_scaled_jobs.write().unwrap();
    if rows
        .iter()
        .any(|r| r.tenant == ctx.tenant && r.namespace == sj.namespace && r.name == sj.name)
    {
        return Err(Error::Invalid(format!(
            "{}/{} already exists",
            sj.namespace, sj.name
        )));
    }
    let mut row = sj;
    row.tenant = ctx.tenant.clone();
    rows.push(row);
    Ok(())
}

pub fn delete(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<(), Error> {
    ctx.authorise(Permission::KedaScaledJobWrite)?;
    let mut rows = state.keda_scaled_jobs.write().unwrap();
    let before = rows.len();
    rows.retain(|r| !(r.tenant == ctx.tenant && r.namespace == namespace && r.name == name));
    if rows.len() == before {
        return Err(Error::NotFound {
            namespace: namespace.into(),
            name: name.into(),
        });
    }
    Ok(())
}

pub fn render_list(state: &AdminState, ctx: &RequestCtx) -> Result<String, Error> {
    let rows: Vec<Vec<String>> = list(state, ctx)?
        .iter()
        .map(|j| {
            vec![
                j.namespace.clone(),
                j.name.clone(),
                j.scaling_strategy.clone(),
                format!("{}", j.max_replica_count),
                format!("{}", j.running_jobs_label()),
                format!("{}", j.pending_jobs_label()),
                format!("{}/{}", j.status.succeeded_jobs_24h, j.status.failed_jobs_24h),
                j.triggers
                    .iter()
                    .map(|t| t.kind.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
            ]
        })
        .collect();
    let body = format!(
        "<h2 class=\"text-lg font-semibold mb-2\">ScaledJobs ({n})</h2>{tbl}",
        n = rows.len(),
        tbl = table(
            &[
                "namespace",
                "name",
                "scalingStrategy",
                "maxReplicaCount",
                "running",
                "pending",
                "ok/fail 24h",
                "triggers",
            ],
            &rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/keda/scaledjobs",
        &format!("keda · scaledjobs · {}", ctx.tenant.as_str()),
        &body,
    ))
}

pub fn render_detail(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<String, Error> {
    let j = get(state, ctx, namespace, name)?;
    let trig_html = render_triggers(&j.triggers);
    let body = format!(
        r#"<a class="text-blue-700 underline" href="/admin/keda/scaledjobs?tenant_id={tenant}">← all scaledjobs</a>
<h2 class="text-xl font-semibold mt-2">{ns}/{name}</h2>
<dl class="mt-2 grid grid-cols-[16rem_1fr] gap-x-4 gap-y-1 text-sm">
  <dt class="text-gray-500">scalingStrategy</dt><dd>{ss}</dd>
  <dt class="text-gray-500">maxReplicaCount</dt><dd>{max}</dd>
  <dt class="text-gray-500">pollingInterval</dt><dd>{p}s</dd>
  <dt class="text-gray-500">successfulJobsHistoryLimit</dt><dd>{sh}</dd>
  <dt class="text-gray-500">failedJobsHistoryLimit</dt><dd>{fh}</dd>
  <dt class="text-gray-500">running / pending</dt><dd>{run} / {pend}</dd>
  <dt class="text-gray-500">24h ok / fail</dt><dd>{ok} / {fail}</dd>
  <dt class="text-gray-500">lastActiveTime</dt><dd>{lat}</dd>
</dl>
<section class="mt-6"><h3 class="text-md font-semibold mb-1">triggers ({tcount})</h3>{triggers}</section>
<section class="mt-6">
  <h3 class="text-md font-semibold mb-1">jobTargetRef.template</h3>
  <pre class="bg-gray-50 rounded p-2 text-xs overflow-x-auto">{tmpl}</pre>
</section>"#,
        tenant = escape(ctx.tenant.as_str()),
        ns = escape(&j.namespace),
        name = escape(&j.name),
        ss = escape(&j.scaling_strategy),
        max = j.max_replica_count,
        p = j.polling_interval_secs,
        sh = j.successful_jobs_history_limit,
        fh = j.failed_jobs_history_limit,
        run = j.status.running_jobs,
        pend = j.status.pending_jobs,
        ok = j.status.succeeded_jobs_24h,
        fail = j.status.failed_jobs_24h,
        lat = j
            .status
            .last_active_time
            .map(|t| t.to_string())
            .unwrap_or_else(|| "—".into()),
        tcount = j.triggers.len(),
        triggers = trig_html,
        tmpl = escape(&j.job_template_yaml),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/keda/scaledjobs",
        &format!("keda · scaledjob {}/{}", j.namespace, j.name),
        &body,
    ))
}

fn render_triggers(triggers: &[KedaTrigger]) -> String {
    let mut out = String::new();
    for t in triggers {
        let auth = t
            .auth_ref
            .as_ref()
            .map(|a| format!("{}/{}", a.kind, a.name))
            .unwrap_or_else(|| "—".into());
        let rows: Vec<Vec<String>> = t
            .metadata
            .iter()
            .map(|(k, v)| vec![k.clone(), v.clone()])
            .collect();
        let docs = scalers::lookup(&t.kind)
            .map(|e| format!(
                r#"<a class="text-blue-700 underline" href="{}" target="_blank" rel="noopener">docs</a>"#,
                escape(e.docs_url)
            ))
            .unwrap_or_else(|| "<span class=\"text-red-600\">unknown</span>".into());
        out.push_str(&format!(
            r#"<details class="border rounded mb-2">
  <summary class="px-3 py-2 bg-gray-100 cursor-pointer"><strong>{kind}</strong> · authRef: {auth} · {docs}</summary>
  <div class="px-3 py-2">{md}</div>
</details>"#,
            kind = escape(&t.kind),
            auth = escape(&auth),
            docs = docs,
            md = table(&["key", "value"], &rows),
        ));
    }
    out
}

impl KedaScaledJob {
    fn running_jobs_label(&self) -> String {
        self.status.running_jobs.to_string()
    }
    fn pending_jobs_label(&self) -> String {
        self.status.pending_jobs.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::keda::types::{KedaAuthRef, KedaScaledJob, KedaScaledJobStatus, KedaTrigger};
    use crate::admin::types::TenantId;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    fn job(name: &str, strategy: &str) -> KedaScaledJob {
        KedaScaledJob {
            tenant: TenantId::new("acme").unwrap(),
            namespace: "ns".into(),
            name: name.into(),
            job_template_yaml: "spec: {}".into(),
            polling_interval_secs: 30,
            successful_jobs_history_limit: 1,
            failed_jobs_history_limit: 1,
            max_replica_count: 1,
            scaling_strategy: strategy.into(),
            triggers: vec![KedaTrigger {
                kind: "cron".into(),
                name: None,
                metadata: vec![],
                auth_ref: Some(KedaAuthRef {
                    kind: "TriggerAuthentication".into(),
                    name: "a".into(),
                }),
                metric_type: "AverageValue".into(),
                use_cached_metrics: false,
            }],
            status: KedaScaledJobStatus {
                last_active_time: None,
                running_jobs: 0,
                pending_jobs: 0,
                succeeded_jobs_24h: 0,
                failed_jobs_24h: 0,
            },
        }
    }

    #[test]
    fn list_filters_by_tenant_and_lists_strategy() {
        let state = AdminState::seeded();
        let html = render_list(&state, &ctx(&[Permission::KedaScaledJobRead])).unwrap();
        assert!(html.contains("backfill-runner"));
        assert!(!html.contains("evil-cron-jobs")); // tenant scoping
        // The scalingStrategy column must echo upstream.
        assert!(html.contains(">scalingStrategy<"));
        assert!(html.contains(">default<"));
    }

    #[test]
    fn detail_renders_jobtemplate_block() {
        let state = AdminState::seeded();
        let html = render_detail(
            &state,
            &ctx(&[Permission::KedaScaledJobRead]),
            "ingest",
            "backfill-runner",
        )
        .unwrap();
        assert!(html.contains("jobTargetRef.template"));
        assert!(html.contains("queueURL"));
    }

    #[test]
    fn validate_rejects_unknown_scaler() {
        let mut j = job("bad", "default");
        j.triggers[0].kind = "nope".into();
        assert!(matches!(validate(&j), Err(Error::Invalid(_))));
    }

    #[test]
    fn validate_rejects_bad_strategy() {
        let j = job("bad", "wishful");
        assert!(matches!(validate(&j), Err(Error::Invalid(s)) if s.contains("scalingStrategy")));
    }

    #[test]
    fn create_then_delete_roundtrips() {
        let state = AdminState::empty();
        let c = ctx(&[Permission::KedaScaledJobRead, Permission::KedaScaledJobWrite]);
        create(&state, &c, job("hello", "accurate")).unwrap();
        assert_eq!(list(&state, &c).unwrap().len(), 1);
        delete(&state, &c, "ns", "hello").unwrap();
        assert!(list(&state, &c).unwrap().is_empty());
    }

    #[test]
    fn list_without_permission_refused() {
        let state = AdminState::seeded();
        assert!(matches!(list(&state, &ctx(&[])).unwrap_err(), Error::Auth(_)));
    }
}
