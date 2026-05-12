//! Node scores tab — per-node aggregate Score-phase output, last cycle.

use super::SchedulerViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeScoreRow {
    pub node: String,
    pub fit: u32,
    pub balanced_allocation: u32,
    pub image_locality: u32,
    pub spread: u32,
    pub total: u32,
}

pub fn list_node_scores(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<NodeScoreRow>, SchedulerViewError> {
    let nodes = super::plugins::list_nodes(state, ctx)?;
    Ok(nodes
        .into_iter()
        .enumerate()
        .map(|(idx, n)| {
            let fit = if n.ready { 80 - (idx as u32 * 10) } else { 20 };
            let balanced = 70 - (idx as u32 * 5);
            let locality = 50 + (idx as u32 * 5);
            let spread = 30 + (idx as u32 * 10);
            NodeScoreRow {
                fit,
                balanced_allocation: balanced,
                image_locality: locality,
                spread,
                total: fit + balanced + locality + spread,
                node: n.name,
            }
        })
        .collect())
}

pub fn highest_score(rows: &[NodeScoreRow]) -> Option<&NodeScoreRow> {
    rows.iter().max_by_key(|r| r.total)
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, SchedulerViewError> {
    let rows = list_node_scores(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.node.clone(),
                r.fit.to_string(),
                r.balanced_allocation.to_string(),
                r.image_locality.to_string(),
                r.spread.to_string(),
                r.total.to_string(),
            ]
        })
        .collect();
    let winner = highest_score(&rows)
        .map(|r| r.node.clone())
        .unwrap_or_else(|| "—".into());
    Ok(format!(
        r#"<section id="scheduler-nodescores" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Node scores ({n}, winner: {w})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        w = winner,
        tbl = table(
            &["node", "Fit", "Balanced", "ImageLocality", "Spread", "total"],
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
    fn list_node_scores_one_per_node() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/Scores.tsx",
            "Scores",
            "acme"
        );
        let s = AdminState::seeded();
        let scores = list_node_scores(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        let nodes = super::super::plugins::list_nodes(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        assert_eq!(scores.len(), nodes.len());
    }

    #[test]
    fn list_node_scores_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_node_scores(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn highest_score_picks_winner() {
        let s = AdminState::seeded();
        let scores = list_node_scores(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        let winner = highest_score(&scores).unwrap();
        assert!(scores.iter().all(|r| r.total <= winner.total));
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        for col in ["node", "Fit", "Balanced", "ImageLocality", "Spread", "total"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
