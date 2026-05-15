//! `/admin/keda/triggerauthentications` — TriggerAuthentication CRUD.
//!
//! KEDA's `TriggerAuthentication` / `ClusterTriggerAuthentication` carries
//! the secrets/env-vars/pod-identity bindings that a scaler trigger
//! resolves via `spec.triggers[].authenticationRef`. The list view shows
//! the bindings at a glance; the detail view shows every binding source.

use crate::admin::keda::types::{
    KedaAzureKvBinding, KedaEnvRef, KedaSecretRef, KedaTriggerAuthentication, KedaVaultBinding,
};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("trigger authentication {namespace}/{name} not found")]
    NotFound { namespace: String, name: String },
    #[error("invalid field: {0}")]
    Invalid(String),
}

pub fn list(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<KedaTriggerAuthentication>, Error> {
    ctx.authorise(Permission::KedaTriggerAuthRead)?;
    Ok(scope(&state.keda_trigger_authentications.read().unwrap(), &ctx.tenant, |r| {
        &r.tenant
    })
    .into_iter()
    .cloned()
    .collect())
}

pub fn get(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<KedaTriggerAuthentication, Error> {
    ctx.authorise(Permission::KedaTriggerAuthRead)?;
    state
        .keda_trigger_authentications
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

fn validate(t: &KedaTriggerAuthentication) -> Result<(), Error> {
    if t.namespace.is_empty() || t.name.is_empty() {
        return Err(Error::Invalid("namespace + name required".into()));
    }
    let has_source = !t.secret_refs.is_empty()
        || !t.env_refs.is_empty()
        || t.pod_identity_provider != "none"
        || t.hashicorp_vault.is_some()
        || t.azure_key_vault.is_some();
    if !has_source {
        return Err(Error::Invalid(
            "must specify at least one of secretTargetRef, env, podIdentity, hashicorpVault, azureKeyVault"
                .into(),
        ));
    }
    Ok(())
}

pub fn create(
    state: &AdminState,
    ctx: &RequestCtx,
    auth: KedaTriggerAuthentication,
) -> Result<(), Error> {
    ctx.authorise(Permission::KedaTriggerAuthWrite)?;
    validate(&auth)?;
    let mut rows = state.keda_trigger_authentications.write().unwrap();
    if rows
        .iter()
        .any(|r| r.tenant == ctx.tenant && r.namespace == auth.namespace && r.name == auth.name)
    {
        return Err(Error::Invalid(format!(
            "{}/{} already exists",
            auth.namespace, auth.name
        )));
    }
    let mut row = auth;
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
    ctx.authorise(Permission::KedaTriggerAuthWrite)?;
    let mut rows = state.keda_trigger_authentications.write().unwrap();
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
    let auths = list(state, ctx)?;
    let rows: Vec<Vec<String>> = auths
        .iter()
        .map(|a| {
            vec![
                a.namespace.clone(),
                a.name.clone(),
                if a.cluster_scoped { "cluster" } else { "namespace" }.into(),
                format!("{}", a.secret_refs.len()),
                format!("{}", a.env_refs.len()),
                a.pod_identity_provider.clone(),
                if a.hashicorp_vault.is_some() { "yes" } else { "no" }.into(),
                if a.azure_key_vault.is_some() { "yes" } else { "no" }.into(),
            ]
        })
        .collect();
    let body = format!(
        "<h2 class=\"text-lg font-semibold mb-2\">TriggerAuthentications ({n})</h2>{tbl}",
        n = rows.len(),
        tbl = table(
            &[
                "namespace",
                "name",
                "scope",
                "secretRefs",
                "envRefs",
                "podIdentity",
                "hashicorpVault",
                "azureKeyVault",
            ],
            &rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/keda/triggerauthentications",
        &format!("keda · triggerauthentications · {}", ctx.tenant.as_str()),
        &body,
    ))
}

pub fn render_detail(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<String, Error> {
    let a = get(state, ctx, namespace, name)?;
    let body = format!(
        r#"<a class="text-blue-700 underline" href="/admin/keda/triggerauthentications?tenant_id={tenant}">← all</a>
<h2 class="text-xl font-semibold mt-2">{ns}/{name}</h2>
<p class="text-sm text-gray-500">scope: <strong>{scope}</strong></p>
<section class="mt-6"><h3 class="text-md font-semibold mb-1">secretTargetRef</h3>{secrets}</section>
<section class="mt-6"><h3 class="text-md font-semibold mb-1">env</h3>{envs}</section>
<section class="mt-6"><h3 class="text-md font-semibold mb-1">podIdentity</h3><p class="text-sm">{pid}</p></section>
<section class="mt-6"><h3 class="text-md font-semibold mb-1">hashicorpVault</h3>{vault}</section>
<section class="mt-6"><h3 class="text-md font-semibold mb-1">azureKeyVault</h3>{azure}</section>"#,
        tenant = escape(ctx.tenant.as_str()),
        ns = escape(&a.namespace),
        name = escape(&a.name),
        scope = if a.cluster_scoped { "ClusterTriggerAuthentication" } else { "TriggerAuthentication (namespaced)" },
        secrets = render_secret_refs(&a.secret_refs),
        envs = render_env_refs(&a.env_refs),
        pid = escape(&a.pod_identity_provider),
        vault = a.hashicorp_vault.as_ref().map(render_vault).unwrap_or_else(|| "<p class=\"text-sm text-gray-500\">—</p>".into()),
        azure = a.azure_key_vault.as_ref().map(render_azure_kv).unwrap_or_else(|| "<p class=\"text-sm text-gray-500\">—</p>".into()),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/keda/triggerauthentications",
        &format!("keda · triggerauth {}/{}", a.namespace, a.name),
        &body,
    ))
}

fn render_secret_refs(refs: &[KedaSecretRef]) -> String {
    if refs.is_empty() {
        return "<p class=\"text-sm text-gray-500\">—</p>".into();
    }
    let rows: Vec<Vec<String>> = refs
        .iter()
        .map(|r| {
            vec![
                r.parameter.clone(),
                r.secret_name.clone(),
                r.key.clone(),
            ]
        })
        .collect();
    table(&["parameter", "secretName", "key"], &rows)
}

fn render_env_refs(refs: &[KedaEnvRef]) -> String {
    if refs.is_empty() {
        return "<p class=\"text-sm text-gray-500\">—</p>".into();
    }
    let rows: Vec<Vec<String>> = refs
        .iter()
        .map(|r| {
            vec![
                r.parameter.clone(),
                r.name.clone(),
                r.container_name.clone(),
            ]
        })
        .collect();
    table(&["parameter", "envName", "containerName"], &rows)
}

fn render_vault(v: &KedaVaultBinding) -> String {
    format!(
        r#"<dl class="grid grid-cols-[12rem_1fr] gap-x-4 gap-y-1 text-sm">
  <dt class="text-gray-500">address</dt><dd><code>{addr}</code></dd>
  <dt class="text-gray-500">authentication</dt><dd>{auth}</dd>
  <dt class="text-gray-500">mount</dt><dd>{mount}</dd>
  <dt class="text-gray-500">role</dt><dd>{role}</dd>
  <dt class="text-gray-500">credentialSecret</dt><dd>{cred}</dd>
  <dt class="text-gray-500">paths</dt><dd><code>{paths}</code></dd>
</dl>"#,
        addr = escape(&v.address),
        auth = escape(&v.authentication),
        mount = escape(&v.mount),
        role = escape(&v.role),
        cred = escape(&v.credential_secret_name),
        paths = escape(&v.paths.join(", ")),
    )
}

fn render_azure_kv(a: &KedaAzureKvBinding) -> String {
    format!(
        r#"<dl class="grid grid-cols-[12rem_1fr] gap-x-4 gap-y-1 text-sm">
  <dt class="text-gray-500">vaultUri</dt><dd><code>{u}</code></dd>
  <dt class="text-gray-500">tenantId</dt><dd>{tid}</dd>
  <dt class="text-gray-500">clientId</dt><dd>{cid}</dd>
  <dt class="text-gray-500">secrets</dt><dd>{s}</dd>
</dl>"#,
        u = escape(&a.vault_uri),
        tid = escape(&a.tenant_id),
        cid = escape(&a.client_id),
        s = escape(&a.secrets.join(", ")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::keda::types::KedaTriggerAuthentication;
    use crate::admin::types::TenantId;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    fn auth(name: &str) -> KedaTriggerAuthentication {
        KedaTriggerAuthentication {
            tenant: TenantId::new("acme").unwrap(),
            namespace: "ns".into(),
            name: name.into(),
            cluster_scoped: false,
            secret_refs: vec![KedaSecretRef {
                parameter: "p".into(),
                secret_name: "s".into(),
                key: "k".into(),
            }],
            env_refs: vec![],
            pod_identity_provider: "none".into(),
            hashicorp_vault: None,
            azure_key_vault: None,
        }
    }

    #[test]
    fn list_filters_by_tenant() {
        let state = AdminState::seeded();
        let html = render_list(&state, &ctx(&[Permission::KedaTriggerAuthRead])).unwrap();
        assert!(html.contains("kafka-sasl"));
        assert!(html.contains("aws-irsa"));
        assert!(html.contains("vault-bound"));
        assert!(!html.contains("evil-azure")); // tenant scope
    }

    #[test]
    fn detail_renders_vault_block_for_vault_bound() {
        let state = AdminState::seeded();
        let html = render_detail(
            &state,
            &ctx(&[Permission::KedaTriggerAuthRead]),
            "reports",
            "vault-bound",
        )
        .unwrap();
        assert!(html.contains("hashicorpVault"));
        assert!(html.contains("vault.acme.svc"));
        assert!(html.contains("keda-reader"));
    }

    #[test]
    fn validate_rejects_empty_binding() {
        let mut a = auth("empty");
        a.secret_refs.clear();
        assert!(matches!(validate(&a), Err(Error::Invalid(_))));
    }

    #[test]
    fn create_then_delete_roundtrips() {
        let state = AdminState::empty();
        let c = ctx(&[Permission::KedaTriggerAuthRead, Permission::KedaTriggerAuthWrite]);
        create(&state, &c, auth("hello")).unwrap();
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
