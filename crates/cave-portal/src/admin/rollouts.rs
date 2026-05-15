//! `/admin/rollouts` — Argo Rollouts UI parity. Canary progression
//! with per-state summary + traffic-percentage badge.
//!
//! Upstream UI: <https://argo-rollouts.readthedocs.io/en/stable/dashboard/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, RolloutStatus};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RolloutsViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<RolloutStatus>, RolloutsViewError> {
    ctx.authorise(Permission::RolloutsRead)?;
    let mut rows: Vec<RolloutStatus> = scope(&state.rollout_statuses.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect();
    rows.sort_by(|a, b| b.traffic_pct.cmp(&a.traffic_pct).then(a.name.cmp(&b.name)));
    Ok(rows)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RolloutSummary {
    pub total: u32,
    pub healthy: u32,
    pub progressing: u32,
    pub degraded: u32,
    pub avg_traffic_pct: u32,
}

pub fn rollout_summary(rows: &[RolloutStatus]) -> RolloutSummary {
    if rows.is_empty() { return RolloutSummary::default(); }
    let total = rows.len() as u32;
    let mut healthy = 0;
    let mut progressing = 0;
    let mut degraded = 0;
    let mut traffic_total = 0u32;
    for r in rows {
        match r.state {
            "Healthy" => healthy += 1,
            "Progressing" => progressing += 1,
            "Degraded" => degraded += 1,
            _ => {}
        }
        traffic_total += r.traffic_pct;
    }
    RolloutSummary { total, healthy, progressing, degraded, avg_traffic_pct: traffic_total / total }
}

pub fn by_strategy<'a>(rows: &'a [RolloutStatus], strategy: &str) -> Vec<&'a RolloutStatus> {
    rows.iter().filter(|r| r.strategy == strategy).collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, RolloutsViewError> {
    let rows = list_records(state, ctx)?;
    let summary = rollout_summary(&rows);
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![
        escape(&r.name), r.strategy.into(), format!("{}%", r.traffic_pct), r.state.into(),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">Argo Rollouts (cave-rollouts). Upstream: <a class="text-blue-700 underline" href="https://argo-rollouts.readthedocs.io/en/stable/dashboard/">argo-rollouts.readthedocs.io</a>.</p>
  <div class="mb-4 grid grid-cols-4 gap-2 text-center text-sm">
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">TOTAL</div><div class="text-2xl font-bold">{total}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">HEALTHY</div><div class="text-2xl font-bold text-green-700">{healthy}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">PROGRESSING</div><div class="text-2xl font-bold text-blue-700">{progressing}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">DEGRADED</div><div class="text-2xl font-bold text-red-700">{degraded}</div></div>
  </div>
  <h2 class="text-lg font-semibold mb-2">Rollouts ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        total = summary.total,
        healthy = summary.healthy,
        progressing = summary.progressing,
        degraded = summary.degraded,
        tbl = table(&["name", "strategy", "traffic", "state"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/rollouts", &format!("rollouts · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/rollouts/src/components/StatusList.tsx", "StatusList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner_and_sorts_by_traffic_desc() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::RolloutsRead])).unwrap();
        assert_eq!(r.len(), 2);
        for w in r.windows(2) { assert!(w[0].traffic_pct >= w[1].traffic_pct); }
    }

    #[test]
    fn list_refuses_without_perm() {
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn rollout_summary_counts_states_and_average_traffic() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::RolloutsRead])).unwrap();
        let s = rollout_summary(&r);
        assert_eq!(s.total, r.len() as u32);
        let expected_avg = if r.is_empty() { 0 } else { r.iter().map(|x| x.traffic_pct).sum::<u32>() / r.len() as u32 };
        assert_eq!(s.avg_traffic_pct, expected_avg);
    }

    #[test]
    fn rollout_summary_handles_empty() {
        let s = rollout_summary(&[]);
        assert_eq!(s.total, 0);
        assert_eq!(s.avg_traffic_pct, 0);
    }

    #[test]
    fn by_strategy_filters() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::RolloutsRead])).unwrap();
        if let Some(f) = r.first() {
            let strat = f.strategy;
            assert!(by_strategy(&r, strat).iter().all(|x| x.strategy == strat));
        }
        assert!(by_strategy(&r, "no-such").is_empty());
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::RolloutsRead])).unwrap();
        assert!(html.contains("web-canary"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::RolloutsRead])).unwrap();
        assert!(!html.contains("evil-rollout"));
    }

    #[test]
    fn render_includes_summary_cards_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::RolloutsRead])).unwrap();
        assert!(html.contains("HEALTHY"));
        assert!(html.contains("argo-rollouts"));
    }
}
