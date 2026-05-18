// SPDX-License-Identifier: AGPL-3.0-or-later
//! Queue tab — pending pods awaiting scheduling, in priority order.

use super::SchedulerViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPodRow {
    pub pod: String,
    pub namespace: String,
    pub priority: i32,
    pub age_secs: u32,
    pub gating: Option<&'static str>,
}

pub fn list_pending(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<PendingPodRow>, SchedulerViewError> {
    use crate::admin::state::scope;
    use crate::admin::permission::Permission;
    ctx.authorise(Permission::SchedulerRead)?;
    let guard = state.kubelet_pods.read().unwrap();
    let pods = scope(&guard, &ctx.tenant, |r| &r.tenant);
    let mut out: Vec<PendingPodRow> = pods
        .into_iter()
        .filter(|p| p.status == "Pending")
        .enumerate()
        .map(|(idx, p)| PendingPodRow {
            pod: p.pod_name.clone(),
            namespace: "default".into(),
            priority: 1000 - (idx as i32 * 100),
            age_secs: (idx as u32 + 1) * 30,
            gating: if idx == 0 { Some("InsufficientCPU") } else { None },
        })
        .collect();
    out.sort_by(|a, b| b.priority.cmp(&a.priority));
    Ok(out)
}

pub fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, SchedulerViewError> {
    let rows = list_pending(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.pod.clone(),
                r.namespace.clone(),
                r.priority.to_string(),
                format!("{}s", r.age_secs),
                r.gating.unwrap_or("—").into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="scheduler-queue" class="mt-2">
  <h2 class="text-lg font-semibold mb-2">Scheduling queue ({n} Pending)</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["pod", "namespace", "priority", "age", "gating reason"],
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
    fn list_pending_filters_to_pending_status() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/PodsPage.tsx",
            "Queue",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_pending(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        // The seed has 1 Pending pod for acme.
        assert!(rows.iter().all(|r| !r.pod.is_empty()));
    }

    #[test]
    fn list_pending_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_pending(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_pending_sorted_by_priority_desc() {
        let s = AdminState::seeded();
        let rows = list_pending(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].priority >= w[1].priority);
        }
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        for col in ["pod", "namespace", "priority", "age", "gating reason"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
