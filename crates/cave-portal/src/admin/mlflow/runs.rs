// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Runs sub-page.

use super::types::{MlflowRun, MlflowViewError};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, scope};

pub fn list_all(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<MlflowRun>, MlflowViewError> {
    ctx.authorise(Permission::MlflowRead)?;
    let mut rows: Vec<MlflowRun> = scope(&state.mlflow_runs.read().unwrap(), &ctx.tenant, |r| {
        &r.tenant
    })
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| b.start_time_ms.cmp(&a.start_time_ms));
    Ok(rows)
}

pub fn list_for_experiment(
    state: &AdminState,
    ctx: &RequestCtx,
    experiment_id: &str,
) -> Result<Vec<MlflowRun>, MlflowViewError> {
    Ok(list_all(state, ctx)?
        .into_iter()
        .filter(|r| r.experiment_id == experiment_id)
        .collect())
}

pub fn get(
    state: &AdminState,
    ctx: &RequestCtx,
    run_id: &str,
) -> Result<MlflowRun, MlflowViewError> {
    list_all(state, ctx)?
        .into_iter()
        .find(|r| r.run_id == run_id)
        .ok_or_else(|| MlflowViewError::RunNotFound(run_id.into()))
}

pub fn status_histogram(rows: &[MlflowRun]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.status.clone()).or_insert(0) += 1;
    }
    acc.into_iter().collect()
}

pub fn failed_runs(rows: &[MlflowRun]) -> Vec<&MlflowRun> {
    rows.iter()
        .filter(|r| r.status == "FAILED" || r.status == "KILLED")
        .collect()
}

pub fn average_duration_ms(rows: &[MlflowRun]) -> u64 {
    let durations: Vec<u64> = rows
        .iter()
        .filter_map(|r| r.end_time_ms.map(|e| (e - r.start_time_ms).max(0) as u64))
        .collect();
    if durations.is_empty() {
        return 0;
    }
    durations.iter().sum::<u64>() / durations.len() as u64
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, MlflowViewError> {
    let rows = list_all(state, ctx)?;
    let hist = status_histogram(&rows);
    let avg = average_duration_ms(&rows);
    let chips: String = hist
        .iter()
        .map(|(s, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{s} <strong>×{n}</strong></span>"#,
                s = escape(s),
                n = n
            )
        })
        .collect();
    let rows_html: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.run_id),
                escape(&r.run_name),
                escape(&r.experiment_id),
                r.status.clone(),
                escape(&r.user),
                r.start_time_ms.to_string(),
                r.primary_metric
                    .as_ref()
                    .map(|(k, v)| format!("{k}={v:.4}"))
                    .unwrap_or_else(|| "—".into()),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><div class="mb-3 text-sm">avg duration: <strong>{avg}</strong> ms</div><div class="mb-3">{chips}</div>{tbl}</section>"#,
        avg = avg,
        chips = chips,
        tbl = table(
            &[
                "run_id",
                "name",
                "experiment",
                "status",
                "user",
                "start",
                "metric"
            ],
            &rows_html,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/mlflow/runs",
        &format!("mlflow/runs · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::types::TenantId;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    fn run(rid: &str, exp: &str, status: &str, start: i64, end: Option<i64>) -> MlflowRun {
        MlflowRun {
            tenant: TenantId::new("acme").expect("t"),
            run_id: rid.into(),
            experiment_id: exp.into(),
            user: "alice".into(),
            status: status.into(),
            start_time_ms: start,
            end_time_ms: end,
            artifact_uri: format!("s3://runs/{rid}"),
            primary_metric: Some(("auc".into(), 0.94)),
            run_name: format!("run-{rid}"),
        }
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        let mut g = s.mlflow_runs.write().unwrap();
        g.push(run("r1", "exp-1", "FINISHED", 1000, Some(2000)));
        g.push(run("r2", "exp-1", "FAILED", 1500, Some(1800)));
        g.push(run("r3", "exp-2", "RUNNING", 3000, None));
        drop(g);
        s
    }

    #[test]
    fn list_all_sorts_newest_first() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        assert_eq!(rows[0].run_id, "r3");
        assert_eq!(rows[2].run_id, "r1");
    }

    #[test]
    fn list_all_refuses_without_perm() {
        let s = seeded();
        assert!(list_all(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_for_experiment_filters() {
        let s = seeded();
        let rows = list_for_experiment(&s, &ctx(&[Permission::MlflowRead]), "exp-1").unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.experiment_id == "exp-1"));
    }

    #[test]
    fn get_returns_run_or_error() {
        let s = seeded();
        let c = ctx(&[Permission::MlflowRead]);
        assert_eq!(get(&s, &c, "r1").unwrap().status, "FINISHED");
        assert!(matches!(
            get(&s, &c, "nope").unwrap_err(),
            MlflowViewError::RunNotFound(_)
        ));
    }

    #[test]
    fn status_histogram_counts_statuses() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        let h = status_histogram(&rows);
        let fin = h
            .iter()
            .find(|(s, _)| s == "FINISHED")
            .map(|(_, n)| *n)
            .unwrap();
        assert_eq!(fin, 1);
    }

    #[test]
    fn failed_runs_returns_failed_and_killed() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        let failed = failed_runs(&rows);
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].run_id, "r2");
    }

    #[test]
    fn average_duration_handles_open_runs() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        // r1: 1000 ms; r2: 300 ms; r3 open (excluded) → avg = 650
        assert_eq!(average_duration_ms(&rows), 650);
    }

    #[test]
    fn render_includes_status_chips_and_columns() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::MlflowRead])).unwrap();
        for col in ["run_id", "name", "experiment", "status"] {
            assert!(html.contains(&format!(">{col}<")), "missing {col}");
        }
        assert!(html.contains("FINISHED"));
    }
}
