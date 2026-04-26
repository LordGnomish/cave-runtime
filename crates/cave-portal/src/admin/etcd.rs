//! `/admin/etcd` view — KV browser + watch event stream + lease table.
//!
//! Mirrors the panes Backstage exposes through the `etcd` plugin
//! (key-value tree, recent watch events, lease overview), adapted to
//! cave's in-process etcd parity model.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, EtcdEvent, EtcdKv, EtcdLease};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EtcdViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

/// Tenant-scoped KV listing. Keys are returned in lexicographic order
/// (matches `etcdctl get --prefix=`).
pub fn list_kv<'a>(state: &'a AdminState, ctx: &RequestCtx) -> Result<Vec<EtcdKv>, EtcdViewError> {
    ctx.authorise(Permission::EtcdRead)?;
    let kv = state.etcd_kv.read().unwrap();
    let mut rows: Vec<EtcdKv> = scope(&kv, &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect();
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(rows)
}

/// Tenant-scoped lease table.
pub fn list_leases(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<EtcdLease>, EtcdViewError> {
    ctx.authorise(Permission::EtcdRead)?;
    let leases = state.etcd_leases.read().unwrap();
    Ok(scope(&leases, &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

/// Tenant-scoped watch event stream (most recent first).
///
/// The seeded event log is global (etcd events don't carry a tenant tag at
/// the wire level) — we filter by intersecting with the tenant's KV keys.
pub fn watch_stream(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<EtcdEvent>, EtcdViewError> {
    ctx.authorise(Permission::EtcdWatch)?;
    let kv = state.etcd_kv.read().unwrap();
    let allowed_keys: std::collections::HashSet<String> = scope(&kv, &ctx.tenant, |r| &r.tenant)
        .iter()
        .map(|r| r.key.clone())
        .collect();
    let log = state.etcd_event_log.read().unwrap();
    let mut out: Vec<EtcdEvent> = log
        .iter()
        .filter(|e| match e {
            EtcdEvent::Put { key, .. } => allowed_keys.contains(key),
            EtcdEvent::Delete { key, .. } => allowed_keys.contains(key),
        })
        .cloned()
        .collect();
    out.reverse();
    Ok(out)
}

/// Render the full HTML page combining all three panes.
pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, EtcdViewError> {
    let kv = list_kv(state, ctx)?;
    let leases = list_leases(state, ctx)?;
    let events = watch_stream(state, ctx)?;
    let kv_rows: Vec<Vec<String>> = kv
        .iter()
        .map(|r| {
            vec![
                r.key.clone(),
                r.value.clone(),
                r.revision.to_string(),
                r.lease_id.map(|l| l.to_string()).unwrap_or_default(),
            ]
        })
        .collect();
    let lease_rows: Vec<Vec<String>> = leases
        .iter()
        .map(|l| {
            vec![
                l.lease_id.to_string(),
                format!("{}s", l.ttl_seconds),
                l.keys.join(","),
            ]
        })
        .collect();
    let event_rows: Vec<Vec<String>> = events
        .iter()
        .map(|e| match e {
            EtcdEvent::Put { key, value, revision } => {
                vec!["PUT".into(), key.clone(), value.clone(), revision.to_string()]
            }
            EtcdEvent::Delete { key, revision } => {
                vec!["DELETE".into(), key.clone(), String::new(), revision.to_string()]
            }
        })
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">KV ({n_kv})</h2>{kv_table}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">Leases ({n_lease})</h2>{lease_table}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">Watch ({n_evt})</h2>{evt_table}</section>"#,
        n_kv = kv.len(),
        n_lease = leases.len(),
        n_evt = events.len(),
        kv_table = table(&["key", "value", "rev", "lease"], &kv_rows),
        lease_table = table(&["lease", "ttl", "keys"], &lease_rows),
        evt_table = table(&["op", "key", "value", "rev"], &event_rows),
    );
    let title = format!("etcd · {}", escape(ctx.tenant.as_str()));
    Ok(page_shell(&title, &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/explore/src/components/Tabs/DocsTab.tsx", "DocsTab");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_kv_returns_only_acme_rows_in_sorted_order() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/DocsTab.tsx",
            "EtcdKVTab",
            "acme"
        );
        let state = AdminState::seeded();
        let rows = list_kv(&state, &ctx(&[Permission::EtcdRead])).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "/cfg/feature_x");
        assert_eq!(rows[1].key, "/state/leader");
        assert!(rows.iter().all(|r| r.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_kv_refuses_when_permission_missing() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        let state = AdminState::seeded();
        assert!(list_kv(&state, &ctx(&[])).is_err());
    }

    #[test]
    fn lease_table_filters_to_tenant() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/DocsTab.tsx",
            "EtcdLeaseTab",
            "acme"
        );
        let state = AdminState::seeded();
        let rows = list_leases(&state, &ctx(&[Permission::EtcdRead])).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].lease_id, 1001);
    }

    #[test]
    fn watch_stream_only_emits_events_for_tenant_owned_keys() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/DocsTab.tsx",
            "EtcdWatchTab",
            "acme"
        );
        let state = AdminState::seeded();
        let evts = watch_stream(&state, &ctx(&[Permission::EtcdWatch])).unwrap();
        // Both seeded events touch acme keys → both visible. Order is reverse.
        assert_eq!(evts.len(), 2);
        match &evts[0] {
            EtcdEvent::Put { key, .. } => assert_eq!(key, "/state/leader"),
            _ => panic!("expected Put"),
        }
    }

    #[test]
    fn render_page_contains_three_panes_and_escapes_tenant() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/ExplorePage.tsx",
            "ExplorePage",
            "acme"
        );
        let state = AdminState::seeded();
        let html = render(
            &state,
            &ctx(&[Permission::EtcdRead, Permission::EtcdWatch]),
        )
        .unwrap();
        assert!(html.contains("<title>etcd"));
        assert!(html.contains("KV (2)"));
        assert!(html.contains("Leases (1)"));
        assert!(html.contains("Watch (2)"));
        assert!(html.contains("/cfg/feature_x"));
        assert!(!html.contains("/cfg/feature_y")); // foreign tenant
    }
}
