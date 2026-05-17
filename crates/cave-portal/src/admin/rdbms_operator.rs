// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/rdbms-operator` view — Postgres-flavour cluster operator.
//!
//! Mirrors the CloudNativePG dashboard: per-tenant Cluster CRDs with
//! their replication state, the elected primary, lag, and the in-flight
//! plus completed backups. Mutators expose two operator-level actions:
//! `trigger_failover` (manual primary switchover) and `trigger_backup`
//! (out-of-schedule physical backup).

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, RdbmsOperatorBackup, RdbmsOperatorCluster};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RdbmsOperatorViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("cluster {0} not found in this tenant")]
    ClusterNotFound(String),
    #[error("cluster {0} only has 1 instance — failover requires ≥2")]
    SinglePrimaryNoFailover(String),
    #[error("backup_id must be non-empty")]
    EmptyBackupId,
    #[error("a backup is already running on cluster {0}")]
    BackupAlreadyRunning(String),
}

pub fn list_clusters(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<RdbmsOperatorCluster>, RdbmsOperatorViewError> {
    ctx.authorise(Permission::RdbmsOperatorRead)?;
    Ok(scope(&state.rdbms_operator_clusters.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn list_backups(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<RdbmsOperatorBackup>, RdbmsOperatorViewError> {
    ctx.authorise(Permission::RdbmsOperatorRead)?;
    Ok(scope(&state.rdbms_operator_backups.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn inspect_cluster(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<RdbmsOperatorCluster, RdbmsOperatorViewError> {
    list_clusters(state, ctx)?
        .into_iter()
        .find(|c| c.name == name)
        .ok_or_else(|| RdbmsOperatorViewError::ClusterNotFound(name.into()))
}

/// Trigger a manual failover. Picks an arbitrary new primary by
/// appending `-2` to the name (mirrors CNPG's "promote next-in-line"
/// behaviour for the seeded fixture). Refuses on single-instance
/// clusters since there is nowhere to fail over to.
pub fn trigger_failover(
    state: &AdminState,
    ctx: &RequestCtx,
    cluster: &str,
) -> Result<String, RdbmsOperatorViewError> {
    ctx.authorise(Permission::RdbmsOperatorFailover)?;
    let mut clusters = state.rdbms_operator_clusters.write().unwrap();
    let target = clusters
        .iter_mut()
        .find(|c| c.tenant == ctx.tenant && c.name == cluster)
        .ok_or_else(|| RdbmsOperatorViewError::ClusterNotFound(cluster.into()))?;
    if target.instances < 2 {
        return Err(RdbmsOperatorViewError::SinglePrimaryNoFailover(cluster.into()));
    }
    let new_primary = format!("{}-2", target.name);
    target.primary_pod = new_primary.clone();
    target.replication_state = "Catchup";
    Ok(new_primary)
}

/// Trigger an out-of-schedule backup. Refuses if a backup with state
/// `Running` already exists for the cluster.
pub fn trigger_backup(
    state: &AdminState,
    ctx: &RequestCtx,
    cluster: &str,
    backup_id: &str,
    started_unix: i64,
) -> Result<(), RdbmsOperatorViewError> {
    ctx.authorise(Permission::RdbmsOperatorBackup)?;
    if backup_id.trim().is_empty() {
        return Err(RdbmsOperatorViewError::EmptyBackupId);
    }
    let mut backups = state.rdbms_operator_backups.write().unwrap();
    if backups
        .iter()
        .any(|b| b.tenant == ctx.tenant && b.cluster == cluster && b.state == "Running")
    {
        return Err(RdbmsOperatorViewError::BackupAlreadyRunning(cluster.into()));
    }
    backups.push(RdbmsOperatorBackup {
        tenant: ctx.tenant.clone(),
        cluster: cluster.into(),
        backup_id: backup_id.into(),
        started_unix,
        finished_unix: None,
        size_mib: 0,
        state: "Running",
    });
    Ok(())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, RdbmsOperatorViewError> {
    let clusters = list_clusters(state, ctx)?;
    let backups = list_backups(state, ctx)?;
    let c_rows: Vec<Vec<String>> = clusters
        .iter()
        .map(|c| {
            vec![
                c.name.clone(),
                c.upstream_kind.into(),
                c.version.clone(),
                c.instances.to_string(),
                c.primary_pod.clone(),
                c.replication_state.into(),
                format!("{} B", c.replication_lag_bytes),
            ]
        })
        .collect();
    let b_rows: Vec<Vec<String>> = backups
        .iter()
        .map(|b| {
            vec![
                b.cluster.clone(),
                b.backup_id.clone(),
                b.state.into(),
                b.started_unix.to_string(),
                b.finished_unix.map(|f| f.to_string()).unwrap_or_else(|| "—".into()),
                format!("{} MiB", b.size_mib),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Clusters ({n_c})</h2>{c_tbl}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">Backups ({n_b})</h2>{b_tbl}</section>"#,
        n_c = clusters.len(),
        n_b = backups.len(),
        c_tbl = table(
            &["name", "upstream", "version", "instances", "primary", "state", "lag"],
            &c_rows,
        ),
        b_tbl = table(
            &["cluster", "id", "state", "started", "finished", "size"],
            &b_rows,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/rdbms-operator",
        &format!("rdbms-operator · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/database/src/components/PostgresClusters/ClustersPage.tsx",
    "ClustersPage",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_clusters_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/database/src/components/PostgresClusters/ClustersList.tsx",
            "ClustersList",
            "acme"
        );
        let s = AdminState::seeded();
        let c = list_clusters(&s, &ctx(&[Permission::RdbmsOperatorRead])).unwrap();
        assert_eq!(c.len(), 2);
        assert!(c.iter().all(|x| x.tenant.as_str() == "acme"));
        assert!(c.iter().any(|x| x.name == "primary-prod"));
    }

    #[test]
    fn list_backups_excludes_evil_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/database/src/components/Backups/BackupsTab.tsx",
            "BackupsTab",
            "acme"
        );
        let s = AdminState::seeded();
        let b = list_backups(&s, &ctx(&[Permission::RdbmsOperatorRead])).unwrap();
        assert!(!b.iter().any(|x| x.cluster == "evil-cluster"));
    }

    #[test]
    fn trigger_failover_promotes_secondary_and_marks_catchup() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/database/src/components/Cluster/FailoverDialog.tsx",
            "FailoverDialog",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::RdbmsOperatorRead, Permission::RdbmsOperatorFailover]);
        let new_primary = trigger_failover(&s, &c, "primary-prod").unwrap();
        assert_eq!(new_primary, "primary-prod-2");
        let cluster = inspect_cluster(&s, &c, "primary-prod").unwrap();
        assert_eq!(cluster.primary_pod, "primary-prod-2");
        assert_eq!(cluster.replication_state, "Catchup");
    }

    #[test]
    fn trigger_failover_refuses_single_instance() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/database/src/components/Cluster/FailoverDialog.tsx",
            "instanceGuard",
            "acme"
        );
        let s = AdminState::seeded();
        // Force "analytics" cluster (2 instances) down to 1 for this test.
        s.rdbms_operator_clusters
            .write()
            .unwrap()
            .iter_mut()
            .find(|c| c.name == "analytics")
            .unwrap()
            .instances = 1;
        let err = trigger_failover(
            &s,
            &ctx(&[Permission::RdbmsOperatorRead, Permission::RdbmsOperatorFailover]),
            "analytics",
        )
        .unwrap_err();
        assert!(matches!(err, RdbmsOperatorViewError::SinglePrimaryNoFailover(_)));
    }

    #[test]
    fn trigger_backup_appends_running_record_and_rejects_concurrent() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/database/src/components/Backups/BackupActions.tsx",
            "TriggerBackup",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::RdbmsOperatorRead, Permission::RdbmsOperatorBackup]);
        // primary-prod already has a Running backup in seed → should reject.
        let err = trigger_backup(&s, &c, "primary-prod", "bk-new", 1_002_500).unwrap_err();
        assert!(matches!(err, RdbmsOperatorViewError::BackupAlreadyRunning(_)));
        // analytics has no Running backup → should accept.
        trigger_backup(&s, &c, "analytics", "bk-an-1", 1_002_600).unwrap();
        let backups = list_backups(&s, &c).unwrap();
        assert!(backups
            .iter()
            .any(|b| b.cluster == "analytics" && b.state == "Running"));
    }

    #[test]
    fn trigger_backup_rejects_empty_id_and_lacks_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/database/src/components/Backups/BackupActions.tsx",
            "validateBackupRequest",
            "acme"
        );
        let s = AdminState::seeded();
        let c_full = ctx(&[Permission::RdbmsOperatorRead, Permission::RdbmsOperatorBackup]);
        assert!(matches!(
            trigger_backup(&s, &c_full, "analytics", "  ", 0).unwrap_err(),
            RdbmsOperatorViewError::EmptyBackupId
        ));
        let c_read_only = ctx(&[Permission::RdbmsOperatorRead]);
        assert!(trigger_backup(&s, &c_read_only, "analytics", "x", 0).is_err());
    }

    #[test]
    fn render_excludes_evil_cluster_and_evil_backup() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/database/src/components/PostgresClusters/ClustersPage.tsx",
            "ClustersPage",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::RdbmsOperatorRead])).unwrap();
        assert!(html.contains("Clusters (2)"));
        assert!(html.contains("primary-prod"));
        assert!(!html.contains("evil-cluster"));
        assert!(!html.contains("evil-bk-1"));
    }
}
