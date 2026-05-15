//! Config tab — ConfigMaps / Secrets / ResourceQuotas.
//!
//! Today we synthesise rows from the workload set so the page has the
//! right Kubernetes-Dashboard shape; a real port resolves via
//! apiserver `/api/v1/configmaps` / `/api/v1/secrets` / `/api/v1/resourcequotas`.

use super::K8sDashboardViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEntryRow {
    pub kind: &'static str, // "ConfigMap" | "Secret" | "ResourceQuota"
    pub name: String,
    pub namespace: String,
    pub keys: u32,
}

pub fn list_config(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ConfigEntryRow>, K8sDashboardViewError> {
    let services = super::services::list_services(state, ctx)?;
    let mut out = Vec::new();
    // One ConfigMap + one Secret per service (mirrors typical deploy shape).
    for s in &services {
        out.push(ConfigEntryRow {
            kind: "ConfigMap",
            name: format!("{}-config", s.name),
            namespace: s.namespace.clone(),
            keys: 4,
        });
        out.push(ConfigEntryRow {
            kind: "Secret",
            name: format!("{}-tls", s.name),
            namespace: s.namespace.clone(),
            keys: 2,
        });
    }
    // One namespace-wide ResourceQuota.
    if !services.is_empty() {
        out.push(ConfigEntryRow {
            kind: "ResourceQuota",
            name: "default-quota".into(),
            namespace: "default".into(),
            keys: 6,
        });
    }
    Ok(out)
}

pub fn count_by_kind(rows: &[ConfigEntryRow], kind: &str) -> usize {
    rows.iter().filter(|r| r.kind == kind).count()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, K8sDashboardViewError> {
    let rows = list_config(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.kind.into(),
                r.name.clone(),
                r.namespace.clone(),
                r.keys.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="k8s-dashboard-config" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Config ({n})</h2>
  <div class="text-xs text-gray-500 mb-2">
    {cm} ConfigMap · {sec} Secret · {rq} ResourceQuota
  </div>
  {tbl}
</section>"#,
        n = rows.len(),
        cm = count_by_kind(&rows, "ConfigMap"),
        sec = count_by_kind(&rows, "Secret"),
        rq = count_by_kind(&rows, "ResourceQuota"),
        tbl = table(&["kind", "name", "namespace", "keys"], &table_rows),
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
    fn list_config_includes_all_three_kinds() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Config.tsx",
            "Config",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_config(&s, &ctx(&[Permission::K8sDashboardRead])).unwrap();
        let kinds: std::collections::HashSet<_> = rows.iter().map(|r| r.kind).collect();
        for k in ["ConfigMap", "Secret", "ResourceQuota"] {
            assert!(kinds.contains(k), "missing kind {k}");
        }
    }

    #[test]
    fn list_config_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_config(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn count_by_kind_sums_to_total() {
        let s = AdminState::seeded();
        let rows = list_config(&s, &ctx(&[Permission::K8sDashboardRead])).unwrap();
        let total = count_by_kind(&rows, "ConfigMap")
            + count_by_kind(&rows, "Secret")
            + count_by_kind(&rows, "ResourceQuota");
        assert_eq!(total, rows.len());
    }

    #[test]
    fn render_section_emits_kind_breakdown() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::K8sDashboardRead])).unwrap();
        for label in ["ConfigMap", "Secret", "ResourceQuota"] {
            assert!(html.contains(label));
        }
    }
}
