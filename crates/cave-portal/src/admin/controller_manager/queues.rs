// SPDX-License-Identifier: AGPL-3.0-or-later
//! Queues tab — per-controller work-queue depth + processing rate.
//! Mirrors `workqueue_depth` + `workqueue_adds_total` Prometheus
//! series that k-c-m exports.

use super::ControllerManagerViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRow {
    pub controller: String,
    pub depth: u32,
    pub adds_per_sec: u32,
    pub retries_per_sec: u32,
}

pub fn list_queues(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<QueueRow>, ControllerManagerViewError> {
    let leases = super::leader_election::list_leases(state, ctx)?;
    Ok(leases
        .into_iter()
        .map(|l| QueueRow {
            // Synthetic: more renewals → busier queue.
            depth: ((l.renewals % 10) as u32) + 1,
            adds_per_sec: ((l.renewals % 100) as u32) / 5 + 1,
            retries_per_sec: ((l.renewals % 10) as u32) / 3,
            controller: l.controller,
        })
        .collect())
}

pub fn total_depth(rows: &[QueueRow]) -> u32 {
    rows.iter().map(|r| r.depth).sum()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, ControllerManagerViewError> {
    let rows = list_queues(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|q| {
            vec![
                q.controller.clone(),
                q.depth.to_string(),
                q.adds_per_sec.to_string(),
                q.retries_per_sec.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="cm-queues" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Work queues ({n}, depth {d})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        d = total_depth(&rows),
        tbl = table(
            &["controller", "depth", "adds/s", "retries/s"],
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
    fn list_queues_one_per_lease() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Resources/Queues.tsx",
            "Queues",
            "acme"
        );
        let s = AdminState::seeded();
        let queues = list_queues(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        let leases = super::super::leader_election::list_leases(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        assert_eq!(queues.len(), leases.len());
    }

    #[test]
    fn list_queues_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_queues(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn depth_is_always_positive() {
        let s = AdminState::seeded();
        let queues = list_queues(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        for q in &queues {
            assert!(q.depth >= 1);
            assert!(q.adds_per_sec >= 1);
        }
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        for col in ["controller", "depth", "adds/s", "retries/s"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
