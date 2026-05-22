// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Keyspace tab — KV browser + watch event stream.
//! Mirrors `etcdctl get --prefix=` for keys + `etcdctl watch` for events.

use super::EtcdViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::{AdminState, EtcdEvent, EtcdKv, scope};

pub fn list_kv(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<EtcdKv>, EtcdViewError> {
    ctx.authorise(Permission::EtcdRead)?;
    let kv = state.etcd_kv.read().unwrap();
    let mut rows: Vec<EtcdKv> = scope(&kv, &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect();
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(rows)
}

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

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, EtcdViewError> {
    let kv = list_kv(state, ctx)?;
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
    let event_rows: Vec<Vec<String>> = events
        .iter()
        .map(|e| match e {
            EtcdEvent::Put {
                key,
                value,
                revision,
            } => {
                vec![
                    "PUT".into(),
                    key.clone(),
                    value.clone(),
                    revision.to_string(),
                ]
            }
            EtcdEvent::Delete { key, revision } => {
                vec![
                    "DELETE".into(),
                    key.clone(),
                    String::new(),
                    revision.to_string(),
                ]
            }
        })
        .collect();
    Ok(format!(
        r#"<section id="etcd-keyspace" class="mt-2">
  <h2 class="text-lg font-semibold mb-2">KV ({n_kv})</h2>
  {kv_tbl}
  <h3 class="text-md font-semibold mt-3 mb-1">Watch stream ({n_evt})</h3>
  {evt_tbl}
</section>"#,
        n_kv = kv.len(),
        n_evt = events.len(),
        kv_tbl = table(&["key", "value", "rev", "lease"], &kv_rows),
        evt_tbl = table(&["op", "key", "value", "rev"], &event_rows),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
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
    }

    #[test]
    fn list_kv_refuses_when_permission_missing() {
        let state = AdminState::seeded();
        assert!(list_kv(&state, &ctx(&[])).is_err());
    }

    #[test]
    fn watch_stream_only_emits_events_for_tenant_owned_keys() {
        let state = AdminState::seeded();
        let evts = watch_stream(&state, &ctx(&[Permission::EtcdWatch])).unwrap();
        assert_eq!(evts.len(), 2);
    }

    #[test]
    fn render_section_emits_both_subsections() {
        let s = AdminState::seeded();
        let html =
            render_section(&s, &ctx(&[Permission::EtcdRead, Permission::EtcdWatch])).unwrap();
        assert!(html.contains("KV ("));
        assert!(html.contains("Watch stream"));
    }
}
