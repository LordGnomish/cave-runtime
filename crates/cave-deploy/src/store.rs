//! PostgreSQL storage for cave-deploy.
//!
//! Uses the `cave_deploy` schema.  Migrations are idempotent.
//! Integrates with `cave_db::CavePool`.

use crate::error::DeployError;
use crate::models::{
    AppProject, Application, ApplicationSet, ApplicationStatus, Cluster,
    HealthStatusDetail, OperationPhase, OperationState, Repository, RevisionHistory,
    SyncOperationResult, SyncStatusDetail,
};
use cave_db::CavePool;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

pub const MODULE_NAME: &str = "deploy";

// ─── Migration SQL ────────────────────────────────────────────────────────────

const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS applications (
    id          UUID        PRIMARY KEY,
    name        TEXT        NOT NULL UNIQUE,
    namespace   TEXT        NOT NULL DEFAULT 'argocd',
    spec        JSONB       NOT NULL,
    status      JSONB       NOT NULL DEFAULT '{}',
    finalizers  TEXT[]      NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by  TEXT
);

CREATE INDEX IF NOT EXISTS idx_applications_name ON applications(name);

CREATE TABLE IF NOT EXISTS projects (
    id          UUID        PRIMARY KEY,
    name        TEXT        NOT NULL UNIQUE,
    spec        JSONB       NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS clusters (
    id          UUID        PRIMARY KEY,
    name        TEXT        NOT NULL UNIQUE,
    server      TEXT        NOT NULL,
    config      JSONB       NOT NULL DEFAULT '{}',
    labels      JSONB       NOT NULL DEFAULT '{}',
    annotations JSONB       NOT NULL DEFAULT '{}',
    info        JSONB       NOT NULL DEFAULT '{}',
    project     TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS repositories (
    id              UUID        PRIMARY KEY,
    repo_url        TEXT        NOT NULL UNIQUE,
    name            TEXT,
    username        TEXT,
    -- password is stored encrypted; column holds ciphertext
    password_enc    TEXT,
    ssh_private_key TEXT,
    insecure        BOOLEAN     NOT NULL DEFAULT FALSE,
    enable_lfs      BOOLEAN     NOT NULL DEFAULT FALSE,
    repo_type       TEXT        NOT NULL DEFAULT 'Git',
    project         TEXT,
    connection_state JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS application_sets (
    id          UUID        PRIMARY KEY,
    name        TEXT        NOT NULL UNIQUE,
    namespace   TEXT        NOT NULL DEFAULT 'argocd',
    spec        JSONB       NOT NULL,
    status      JSONB       NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS revision_history (
    id              BIGSERIAL   PRIMARY KEY,
    application_id  UUID        NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    revision        TEXT        NOT NULL,
    source          JSONB       NOT NULL,
    deployed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deploy_started_at TIMESTAMPTZ,
    initiator       TEXT
);

CREATE INDEX IF NOT EXISTS idx_revhist_app ON revision_history(application_id, deployed_at DESC);

CREATE TABLE IF NOT EXISTS sync_operations (
    id              UUID        PRIMARY KEY,
    application_id  UUID        NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    phase           TEXT        NOT NULL,
    message         TEXT,
    sync_result     JSONB,
    started_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at     TIMESTAMPTZ,
    retry_count     INT         NOT NULL DEFAULT 0,
    initiated_by    TEXT
);

CREATE INDEX IF NOT EXISTS idx_syncops_app ON sync_operations(application_id, started_at DESC);
"#;

// ─── Store ────────────────────────────────────────────────────────────────────

pub struct DeployStore {
    pool: Arc<CavePool>,
}

impl DeployStore {
    pub async fn new(pool: Arc<CavePool>) -> Result<Self, DeployError> {
        pool.ensure_schema(MODULE_NAME)
            .await
            .map_err(DeployError::Database)?;
        pool.migrate(MODULE_NAME, 1, MIGRATION_V1)
            .await
            .map_err(DeployError::Database)?;
        info!("cave-deploy schema ready");
        Ok(Self { pool })
    }

    // ─── Applications ─────────────────────────────────────────────────────────

    pub async fn list_applications(&self) -> Result<Vec<Application>, DeployError> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"SELECT id, name, namespace, spec, status, finalizers, created_at, updated_at, created_by
                   FROM cave_deploy.applications ORDER BY name"#,
                &[],
            )
            .await?;

        rows.iter().map(|row| row_to_application(row)).collect()
    }

    pub async fn get_application(&self, name: &str) -> Result<Option<Application>, DeployError> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"SELECT id, name, namespace, spec, status, finalizers, created_at, updated_at, created_by
                   FROM cave_deploy.applications WHERE name = $1"#,
                &[&name],
            )
            .await?;
        row.as_ref().map(row_to_application).transpose()
    }

    pub async fn create_application(&self, app: &Application) -> Result<(), DeployError> {
        let client = self.pool.get().await?;
        let spec = serde_json::to_value(&app.spec)?;
        let status = serde_json::to_value(&app.status)?;
        let fin: Vec<String> = app.finalizers.clone();
        client
            .execute(
                r#"INSERT INTO cave_deploy.applications
                   (id, name, namespace, spec, status, finalizers, created_at, updated_at, created_by)
                   VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
                &[
                    &app.id,
                    &app.name,
                    &app.namespace,
                    &spec,
                    &status,
                    &fin,
                    &app.created_at,
                    &app.updated_at,
                    &app.created_by,
                ],
            )
            .await?;
        Ok(())
    }

    pub async fn update_application_status(
        &self,
        name: &str,
        status: &ApplicationStatus,
    ) -> Result<(), DeployError> {
        let client = self.pool.get().await?;
        let status_val = serde_json::to_value(status)?;
        client
            .execute(
                "UPDATE cave_deploy.applications SET status=$1, updated_at=NOW() WHERE name=$2",
                &[&status_val, &name],
            )
            .await?;
        Ok(())
    }

    pub async fn update_application_spec(
        &self,
        name: &str,
        spec: &crate::models::ApplicationSpec,
    ) -> Result<(), DeployError> {
        let client = self.pool.get().await?;
        let spec_val = serde_json::to_value(spec)?;
        client
            .execute(
                "UPDATE cave_deploy.applications SET spec=$1, updated_at=NOW() WHERE name=$2",
                &[&spec_val, &name],
            )
            .await?;
        Ok(())
    }

    pub async fn delete_application(&self, name: &str) -> Result<(), DeployError> {
        let client = self.pool.get().await?;
        let n = client
            .execute("DELETE FROM cave_deploy.applications WHERE name=$1", &[&name])
            .await?;
        if n == 0 {
            return Err(DeployError::NotFound(format!("application '{name}'")));
        }
        Ok(())
    }

    // ─── Revision history ─────────────────────────────────────────────────────

    pub async fn add_revision_history(
        &self,
        application_id: Uuid,
        revision: &str,
        source: &crate::models::ApplicationSource,
        initiator: Option<&str>,
    ) -> Result<i64, DeployError> {
        let client = self.pool.get().await?;
        let source_val = serde_json::to_value(source)?;
        let row = client
            .query_one(
                r#"INSERT INTO cave_deploy.revision_history
                   (application_id, revision, source, initiator, deployed_at)
                   VALUES ($1,$2,$3,$4,NOW())
                   RETURNING id"#,
                &[&application_id, &revision, &source_val, &initiator],
            )
            .await?;
        Ok(row.get::<_, i64>(0))
    }

    pub async fn get_revision_history(
        &self,
        application_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RevisionHistory>, DeployError> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"SELECT id, revision, source, deployed_at, deploy_started_at, initiator
                   FROM cave_deploy.revision_history
                   WHERE application_id = $1
                   ORDER BY deployed_at DESC LIMIT $2"#,
                &[&application_id, &limit],
            )
            .await?;

        rows.iter()
            .map(|row| {
                let id: i64 = row.get(0);
                let revision: String = row.get(1);
                let source_val: serde_json::Value = row.get(2);
                let deployed_at: DateTime<Utc> = row.get(3);
                let deploy_started_at: Option<DateTime<Utc>> = row.get(4);
                let initiator: Option<String> = row.get(5);
                let source = serde_json::from_value(source_val)
                    .map_err(DeployError::from)?;
                Ok(RevisionHistory {
                    id: id as u64,
                    revision,
                    source,
                    sources: vec![],
                    deployed_at,
                    deploy_started_at,
                    initiator,
                })
            })
            .collect()
    }

    // ─── Projects ─────────────────────────────────────────────────────────────

    pub async fn list_projects(&self) -> Result<Vec<AppProject>, DeployError> {
        let client = self.pool.get().await?;
        let rows = client
            .query("SELECT id, spec FROM cave_deploy.projects ORDER BY (spec->>'name')", &[])
            .await?;
        rows.iter()
            .map(|row| {
                let _id: Uuid = row.get(0);
                let spec_val: serde_json::Value = row.get(1);
                serde_json::from_value(spec_val).map_err(DeployError::from)
            })
            .collect()
    }

    pub async fn get_project(&self, name: &str) -> Result<Option<AppProject>, DeployError> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "SELECT spec FROM cave_deploy.projects WHERE spec->>'name' = $1",
                &[&name],
            )
            .await?;
        row.as_ref()
            .map(|r| {
                let v: serde_json::Value = r.get(0);
                serde_json::from_value(v).map_err(DeployError::from)
            })
            .transpose()
    }

    pub async fn upsert_project(&self, project: &AppProject) -> Result<(), DeployError> {
        let client = self.pool.get().await?;
        let spec = serde_json::to_value(project)?;
        client
            .execute(
                r#"INSERT INTO cave_deploy.projects (id, spec)
                   VALUES ($1, $2)
                   ON CONFLICT (name) DO UPDATE SET spec = EXCLUDED.spec, updated_at = NOW()"#,
                &[&project.id, &spec],
            )
            .await?;
        Ok(())
    }

    // ─── Clusters ─────────────────────────────────────────────────────────────

    pub async fn list_clusters(&self) -> Result<Vec<Cluster>, DeployError> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, name, server, config, labels, annotations, info, project, created_at, updated_at FROM cave_deploy.clusters ORDER BY name",
                &[],
            )
            .await?;
        rows.iter().map(|row| row_to_cluster(row)).collect()
    }

    pub async fn get_cluster(&self, name: &str) -> Result<Option<Cluster>, DeployError> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "SELECT id, name, server, config, labels, annotations, info, project, created_at, updated_at FROM cave_deploy.clusters WHERE name=$1",
                &[&name],
            )
            .await?;
        row.as_ref().map(row_to_cluster).transpose()
    }

    pub async fn upsert_cluster(&self, cluster: &Cluster) -> Result<(), DeployError> {
        let client = self.pool.get().await?;
        let config = serde_json::to_value(&cluster.config)?;
        let labels = serde_json::to_value(&cluster.labels)?;
        let annotations = serde_json::to_value(&cluster.annotations)?;
        let info = serde_json::to_value(&cluster.info)?;
        client
            .execute(
                r#"INSERT INTO cave_deploy.clusters (id, name, server, config, labels, annotations, info, project, created_at, updated_at)
                   VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
                   ON CONFLICT (name) DO UPDATE SET
                       server=$3, config=$4, labels=$5, annotations=$6, info=$7, project=$8, updated_at=$10"#,
                &[&cluster.id, &cluster.name, &cluster.server, &config, &labels, &annotations, &info, &cluster.project, &cluster.created_at, &cluster.updated_at],
            )
            .await?;
        Ok(())
    }

    pub async fn delete_cluster(&self, name: &str) -> Result<(), DeployError> {
        let client = self.pool.get().await?;
        let n = client
            .execute("DELETE FROM cave_deploy.clusters WHERE name=$1", &[&name])
            .await?;
        if n == 0 {
            return Err(DeployError::NotFound(format!("cluster '{name}'")));
        }
        Ok(())
    }

    // ─── Repositories ─────────────────────────────────────────────────────────

    pub async fn list_repositories(&self) -> Result<Vec<Repository>, DeployError> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, repo_url, name, username, insecure, enable_lfs, repo_type, project, connection_state, created_at, updated_at FROM cave_deploy.repositories ORDER BY repo_url",
                &[],
            )
            .await?;
        rows.iter().map(|row| row_to_repo(row)).collect()
    }

    pub async fn upsert_repository(&self, repo: &Repository) -> Result<(), DeployError> {
        let client = self.pool.get().await?;
        let repo_type = format!("{:?}", repo.repo_type);
        let conn_state = serde_json::to_value(&repo.connection_state)?;
        client
            .execute(
                r#"INSERT INTO cave_deploy.repositories
                   (id, repo_url, name, username, insecure, enable_lfs, repo_type, project, connection_state, created_at, updated_at)
                   VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
                   ON CONFLICT (repo_url) DO UPDATE SET
                       name=$3, username=$4, insecure=$5, enable_lfs=$6, project=$8, connection_state=$9, updated_at=$11"#,
                &[&repo.id, &repo.repo, &repo.name, &repo.username, &repo.insecure, &repo.enable_lfs, &repo_type, &repo.project, &conn_state, &repo.created_at, &repo.updated_at],
            )
            .await?;
        Ok(())
    }
}

// ─── Row mappers ──────────────────────────────────────────────────────────────

fn row_to_application(row: &tokio_postgres::Row) -> Result<Application, DeployError> {
    let id: Uuid = row.get(0);
    let name: String = row.get(1);
    let namespace: String = row.get(2);
    let spec_val: serde_json::Value = row.get(3);
    let status_val: serde_json::Value = row.get(4);
    let finalizers: Vec<String> = row.get(5);
    let created_at: DateTime<Utc> = row.get(6);
    let updated_at: DateTime<Utc> = row.get(7);
    let created_by: Option<String> = row.get(8);

    let spec = serde_json::from_value(spec_val).map_err(DeployError::from)?;
    let status = serde_json::from_value(status_val).unwrap_or_default();

    Ok(Application { id, name, namespace, spec, status, finalizers, created_at, updated_at, created_by })
}

fn row_to_cluster(row: &tokio_postgres::Row) -> Result<Cluster, DeployError> {
    let id: Uuid = row.get(0);
    let name: String = row.get(1);
    let server: String = row.get(2);
    let config_val: serde_json::Value = row.get(3);
    let labels_val: serde_json::Value = row.get(4);
    let annotations_val: serde_json::Value = row.get(5);
    let info_val: serde_json::Value = row.get(6);
    let project: Option<String> = row.get(7);
    let created_at: DateTime<Utc> = row.get(8);
    let updated_at: DateTime<Utc> = row.get(9);

    Ok(Cluster {
        id,
        name,
        server,
        config: serde_json::from_value(config_val).unwrap_or_default(),
        labels: serde_json::from_value(labels_val).unwrap_or_default(),
        annotations: serde_json::from_value(annotations_val).unwrap_or_default(),
        info: serde_json::from_value(info_val).unwrap_or_default(),
        project,
        created_at,
        updated_at,
    })
}

fn row_to_repo(row: &tokio_postgres::Row) -> Result<Repository, DeployError> {
    use crate::models::RepoType;
    let id: Uuid = row.get(0);
    let repo: String = row.get(1);
    let name: Option<String> = row.get(2);
    let username: Option<String> = row.get(3);
    let insecure: bool = row.get(4);
    let enable_lfs: bool = row.get(5);
    let repo_type_str: String = row.get(6);
    let project: Option<String> = row.get(7);
    let conn_val: Option<serde_json::Value> = row.get(8);
    let created_at: DateTime<Utc> = row.get(9);
    let updated_at: DateTime<Utc> = row.get(10);

    let repo_type = if repo_type_str == "Helm" { RepoType::Helm } else { RepoType::Git };
    let connection_state = conn_val
        .and_then(|v| serde_json::from_value(v).ok());

    Ok(Repository {
        id,
        repo,
        name,
        username,
        password: None,
        ssh_private_key: None,
        insecure,
        enable_lfs,
        tls_client_cert_data: None,
        tls_client_cert_key: None,
        repo_type,
        project,
        connection_state,
        created_at,
        updated_at,
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        Application, ApplicationSpec, ApplicationSource, ApplicationDestination, ApplicationStatus,
    };
    use chrono::Utc;

    /// In-memory application store for testing without a database.
    struct InMemoryAppStore {
        apps: std::collections::HashMap<String, Application>,
    }

    impl InMemoryAppStore {
        fn new() -> Self {
            Self { apps: Default::default() }
        }

        fn create(&mut self, app: Application) -> Result<(), DeployError> {
            if self.apps.contains_key(&app.name) {
                return Err(DeployError::AlreadyExists(app.name.clone()));
            }
            self.apps.insert(app.name.clone(), app);
            Ok(())
        }

        fn get(&self, name: &str) -> Option<&Application> {
            self.apps.get(name)
        }

        fn update_status(&mut self, name: &str, status: ApplicationStatus) -> Result<(), DeployError> {
            let app = self.apps.get_mut(name)
                .ok_or_else(|| DeployError::NotFound(name.to_string()))?;
            app.status = status;
            Ok(())
        }

        fn delete(&mut self, name: &str) -> Result<(), DeployError> {
            self.apps.remove(name)
                .ok_or_else(|| DeployError::NotFound(name.to_string()))?;
            Ok(())
        }

        fn list(&self) -> Vec<&Application> {
            self.apps.values().collect()
        }
    }

    fn make_app(name: &str) -> Application {
        let now = Utc::now();
        Application {
            id: Uuid::new_v4(),
            name: name.to_string(),
            namespace: "argocd".to_string(),
            spec: ApplicationSpec {
                source: ApplicationSource {
                    repo_url: "https://github.com/example/repo.git".to_string(),
                    path: Some("manifests/".to_string()),
                    target_revision: Some("main".to_string()),
                    ..Default::default()
                },
                destination: ApplicationDestination {
                    server: Some("https://kubernetes.default.svc".to_string()),
                    namespace: "default".to_string(),
                    ..Default::default()
                },
                project: "default".to_string(),
                ..Default::default()
            },
            status: Default::default(),
            created_at: now,
            updated_at: now,
            created_by: None,
            finalizers: vec![],
        }
    }

    #[test]
    fn test_in_memory_crud_create_get_delete() {
        let mut store = InMemoryAppStore::new();
        let app = make_app("myapp");
        store.create(app).unwrap();
        assert!(store.get("myapp").is_some());
        assert_eq!(store.list().len(), 1);
        store.delete("myapp").unwrap();
        assert!(store.get("myapp").is_none());
        assert_eq!(store.list().len(), 0);
    }

    #[test]
    fn test_in_memory_duplicate_create_fails() {
        let mut store = InMemoryAppStore::new();
        store.create(make_app("myapp")).unwrap();
        let err = store.create(make_app("myapp")).unwrap_err();
        assert!(matches!(err, DeployError::AlreadyExists(_)));
    }

    #[test]
    fn test_in_memory_update_status() {
        let mut store = InMemoryAppStore::new();
        store.create(make_app("app1")).unwrap();
        let new_status = ApplicationStatus {
            health: crate::models::HealthStatusDetail {
                status: "Degraded".to_string(),
                message: Some("CrashLoopBackOff".to_string()),
            },
            ..Default::default()
        };
        store.update_status("app1", new_status).unwrap();
        assert_eq!(store.get("app1").unwrap().status.health.status, "Degraded");
    }

    #[test]
    fn test_in_memory_delete_not_found() {
        let mut store = InMemoryAppStore::new();
        let err = store.delete("nonexistent").unwrap_err();
        assert!(matches!(err, DeployError::NotFound(_)));
    }

    #[test]
    fn test_in_memory_rollback_via_history() {
        // Simulates: create app, record two revisions, rollback to first.
        let mut store = InMemoryAppStore::new();
        let mut app = make_app("rollback-test");
        store.create(app.clone()).unwrap();

        // Simulate revision history as a Vec within the status
        let hist_entry_1 = RevisionHistory {
            id: 1,
            revision: "abc111".to_string(),
            source: app.spec.source.clone(),
            sources: vec![],
            deployed_at: Utc::now(),
            deploy_started_at: None,
            initiator: Some("ci-bot".to_string()),
        };
        let hist_entry_2 = RevisionHistory {
            id: 2,
            revision: "def222".to_string(),
            source: app.spec.source.clone(),
            sources: vec![],
            deployed_at: Utc::now(),
            deploy_started_at: None,
            initiator: Some("alice".to_string()),
        };

        let status_with_history = ApplicationStatus {
            history: vec![hist_entry_1.clone(), hist_entry_2],
            ..Default::default()
        };
        store.update_status("rollback-test", status_with_history).unwrap();

        // Rollback: find history entry id=1 and re-sync to that revision
        let current = store.get("rollback-test").unwrap();
        let target = current.status.history.iter().find(|h| h.id == 1).unwrap();
        assert_eq!(target.revision, "abc111");
    }
}
