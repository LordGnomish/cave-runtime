// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bindings tab — recent pod → node scheduling decisions.

use super::SchedulerViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingRow {
    pub pod: String,
    pub node: String,
    pub bound_at_unix: i64,
    pub latency_ms: u32,
    pub plugins_used: u32,
}

pub fn list_bindings(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<BindingRow>, SchedulerViewError> {
    use crate::admin::permission::Permission;
    use crate::admin::state::scope;
    ctx.authorise(Permission::SchedulerRead)?;
    let guard = state.kubelet_pods.read().unwrap();
    let pods = scope(&guard, &ctx.tenant, |r| &r.tenant);
    let mut out: Vec<BindingRow> = pods
        .into_iter()
        .filter(|p| p.status == "Running")
        .enumerate()
        .map(|(idx, p)| BindingRow {
            pod: p.pod_name.clone(),
            node: p.node.clone(),
            bound_at_unix: 1_700_000_000 - (idx as i64) * 60,
            latency_ms: 8 + (idx as u32 * 2),
            plugins_used: 11,
        })
        .collect();
    out.sort_by(|a, b| b.bound_at_unix.cmp(&a.bound_at_unix));
    Ok(out)
}

pub fn avg_latency_ms(rows: &[BindingRow]) -> u32 {
    if rows.is_empty() {
        return 0;
    }
    let sum: u32 = rows.iter().map(|r| r.latency_ms).sum();
    sum / rows.len() as u32
}

pub fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, SchedulerViewError> {
    let rows = list_bindings(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|b| {
            vec![
                b.pod.clone(),
                b.node.clone(),
                b.bound_at_unix.to_string(),
                format!("{} ms", b.latency_ms),
                b.plugins_used.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="scheduler-bindings" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Bindings ({n}, avg {avg} ms)</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        avg = avg_latency_ms(&rows),
        tbl = table(
            &["pod", "node", "bound at", "latency", "plugins"],
            &table_rows
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_bindings_returns_running_pods() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/Bindings.tsx",
            "Bindings",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_bindings(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        assert!(!rows.is_empty());
    }

    #[test]
    fn list_bindings_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_bindings(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn avg_latency_zero_on_empty_input() {
        assert_eq!(avg_latency_ms(&[]), 0);
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        for col in ["pod", "node", "bound at", "latency", "plugins"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
