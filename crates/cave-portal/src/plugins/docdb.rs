// SPDX-License-Identifier: AGPL-3.0-or-later
//! DocDB wrap — native MongoDB-compatible admin UI.
//!
//! Replaces MongoDB Compass / Atlas Data Explorer for the cave runtime.
//! Tenants browse their own collections, list indexes, run aggregation
//! pipelines (validated to be read-only), and watch change-stream lag.
//! **No** redirect to a vendor UI exists.
//!
//! Panels (per ADR-147 portal contract):
//!   * `dashboard`    — collection list, doc count, replication state
//!   * `collections`  — browser + per-collection index list
//!   * `aggregate`    — pipeline runner (read-only stages only)
//!   * `change_streams` — viewer for active change-stream lag

use super::ViewPersona;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Domain types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplicaState {
    Primary,
    Secondary,
    Recovering,
    Down,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplicaMember {
    pub host: String,
    pub state: ReplicaState,
    pub lag_seconds: f64,
    pub oplog_window_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionSummary {
    pub database: String,
    pub name: String,
    pub tenant: String,
    pub document_count: u64,
    pub size_bytes: u64,
    pub avg_object_size_bytes: u64,
    pub index_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexInfo {
    pub database: String,
    pub collection: String,
    pub name: String,
    pub keys: Vec<(String, IndexDirection)>,
    pub unique: bool,
    pub sparse: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexDirection {
    Asc,
    Desc,
    Hashed,
    Text,
    TwoDSphere,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeStream {
    pub id: String,
    pub database: String,
    pub collection: String,
    pub tenant: String,
    pub resume_token: String,
    pub lag_seconds: f64,
    pub started_at: DateTime<Utc>,
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DocdbError {
    #[error("collection {0:?}.{1:?} not found")]
    CollectionNotFound(String, String),
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
    #[error("invalid pipeline: {0}")]
    InvalidPipeline(String),
    #[error("cross-tenant access blocked")]
    CrossTenant,
    #[error("invalid identifier: {0:?}")]
    InvalidIdent(String),
}

// ── Plugin state ─────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct DocdbPlugin {
    members: Vec<ReplicaMember>,
    collections: Vec<CollectionSummary>,
    indexes: Vec<IndexInfo>,
    change_streams: Vec<ChangeStream>,
}

impl DocdbPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_replica_set(&mut self, members: Vec<ReplicaMember>) {
        self.members = members;
    }

    pub fn register_collection(&mut self, summary: CollectionSummary) -> Result<(), DocdbError> {
        validate_ident(&summary.database)?;
        validate_ident(&summary.name)?;
        self.collections.retain(|c| {
            !(c.database == summary.database && c.name == summary.name)
        });
        self.collections.push(summary);
        Ok(())
    }

    pub fn register_index(&mut self, idx: IndexInfo) -> Result<(), DocdbError> {
        validate_ident(&idx.database)?;
        validate_ident(&idx.collection)?;
        validate_ident(&idx.name)?;
        if !self
            .collections
            .iter()
            .any(|c| c.database == idx.database && c.name == idx.collection)
        {
            return Err(DocdbError::CollectionNotFound(idx.database, idx.collection));
        }
        self.indexes.retain(|i| {
            !(i.database == idx.database && i.collection == idx.collection && i.name == idx.name)
        });
        self.indexes.push(idx);
        Ok(())
    }

    pub fn register_change_stream(&mut self, stream: ChangeStream) {
        self.change_streams.push(stream);
    }

    pub fn dashboard(&self, persona: ViewPersona, tenant: &str) -> DashboardPanel {
        let visible: Vec<&CollectionSummary> = self
            .collections
            .iter()
            .filter(|c| persona == ViewPersona::Admin || c.tenant == tenant)
            .collect();
        let primary_count = self
            .members
            .iter()
            .filter(|m| m.state == ReplicaState::Primary)
            .count();
        let max_lag_seconds = self
            .members
            .iter()
            .filter(|m| m.state == ReplicaState::Secondary)
            .map(|m| m.lag_seconds)
            .fold(0.0_f64, f64::max);
        let min_oplog_window_seconds = self
            .members
            .iter()
            .map(|m| m.oplog_window_seconds)
            .min()
            .unwrap_or(0);
        let total_documents: u64 = visible.iter().map(|c| c.document_count).sum();
        let total_bytes: u64 = visible.iter().map(|c| c.size_bytes).sum();
        DashboardPanel {
            collection_count: visible.len(),
            total_documents,
            total_bytes,
            primary_count,
            secondary_count: self
                .members
                .iter()
                .filter(|m| m.state == ReplicaState::Secondary)
                .count(),
            max_replica_lag_seconds: max_lag_seconds,
            min_oplog_window_seconds,
            change_streams_active: self.change_streams.len(),
        }
    }

    pub fn list_collections(&self, persona: ViewPersona, tenant: &str) -> Vec<&CollectionSummary> {
        self.collections
            .iter()
            .filter(|c| persona == ViewPersona::Admin || c.tenant == tenant)
            .collect()
    }

    pub fn list_indexes(&self, database: &str, collection: &str) -> Vec<&IndexInfo> {
        self.indexes
            .iter()
            .filter(|i| i.database == database && i.collection == collection)
            .collect()
    }

    pub fn change_stream_view<'a>(
        &'a self,
        persona: ViewPersona,
        tenant: &str,
    ) -> Vec<&'a ChangeStream> {
        self.change_streams
            .iter()
            .filter(|s| persona == ViewPersona::Admin || s.tenant == tenant)
            .collect()
    }

    /// Validate an aggregation pipeline. Only `$match`, `$project`, `$group`,
    /// `$sort`, `$limit`, `$skip`, `$count`, `$lookup`, `$unwind` are allowed.
    /// `$out`, `$merge` are forbidden because they'd write data through the
    /// portal — that path is reserved for cave-portal-api with a different
    /// authn scope.
    pub fn validate_pipeline(
        &self,
        pipeline: &[String],
        _persona: ViewPersona,
        _tenant: &str,
    ) -> Result<PipelinePreview, DocdbError> {
        if pipeline.is_empty() {
            return Err(DocdbError::InvalidPipeline("empty pipeline".into()));
        }
        const ALLOWED: &[&str] = &[
            "$match",
            "$project",
            "$group",
            "$sort",
            "$limit",
            "$skip",
            "$count",
            "$lookup",
            "$unwind",
            "$facet",
            "$addFields",
        ];
        const FORBIDDEN: &[&str] = &["$out", "$merge", "$indexStats", "$collStats"];
        let mut stages = Vec::new();
        for stage in pipeline {
            let trimmed = stage.trim();
            let op = trimmed
                .split_once(':')
                .map(|(k, _)| k.trim().trim_matches(|c: char| c == '"' || c == '{'))
                .unwrap_or("");
            if op.is_empty() {
                return Err(DocdbError::InvalidPipeline(format!(
                    "could not parse stage operator from {trimmed:?}"
                )));
            }
            if FORBIDDEN.iter().any(|f| f == &op) {
                return Err(DocdbError::InvalidPipeline(format!(
                    "stage {op} is forbidden in portal"
                )));
            }
            if !ALLOWED.iter().any(|a| a == &op) {
                return Err(DocdbError::InvalidPipeline(format!(
                    "stage {op} is not in the allow-list"
                )));
            }
            stages.push(op.to_string());
        }
        Ok(PipelinePreview {
            stage_count: stages.len(),
            stages,
        })
    }

    /// Returns members with lag above `threshold_seconds`.
    pub fn lagging_members(&self, threshold_seconds: f64) -> Vec<&ReplicaMember> {
        self.members
            .iter()
            .filter(|m| m.state == ReplicaState::Secondary && m.lag_seconds > threshold_seconds)
            .collect()
    }
}

// ── View-model panels ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DashboardPanel {
    pub collection_count: usize,
    pub total_documents: u64,
    pub total_bytes: u64,
    pub primary_count: usize,
    pub secondary_count: usize,
    pub max_replica_lag_seconds: f64,
    pub min_oplog_window_seconds: u64,
    pub change_streams_active: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelinePreview {
    pub stage_count: usize,
    pub stages: Vec<String>,
}

// ── Validation ───────────────────────────────────────────────────────────────

fn validate_ident(name: &str) -> Result<(), DocdbError> {
    if name.is_empty() || name.len() > 64 {
        return Err(DocdbError::InvalidIdent(name.into()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(DocdbError::InvalidIdent(name.into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(host: &str, state: ReplicaState, lag: f64) -> ReplicaMember {
        ReplicaMember {
            host: host.into(),
            state,
            lag_seconds: lag,
            oplog_window_seconds: 86_400,
        }
    }

    fn coll(db: &str, name: &str, tenant: &str, docs: u64, bytes: u64) -> CollectionSummary {
        CollectionSummary {
            database: db.into(),
            name: name.into(),
            tenant: tenant.into(),
            document_count: docs,
            size_bytes: bytes,
            avg_object_size_bytes: if docs > 0 { bytes / docs } else { 0 },
            index_count: 1,
        }
    }

    #[test]
    fn register_collection_dedupes_by_db_name() {
        let mut p = DocdbPlugin::new();
        p.register_collection(coll("app", "users", "acme", 10, 1024)).unwrap();
        p.register_collection(coll("app", "users", "acme", 20, 2048)).unwrap();
        let listed = p.list_collections(ViewPersona::Admin, "acme");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].document_count, 20);
    }

    #[test]
    fn register_index_requires_existing_collection() {
        let mut p = DocdbPlugin::new();
        let err = p
            .register_index(IndexInfo {
                database: "app".into(),
                collection: "users".into(),
                name: "ix1".into(),
                keys: vec![("email".into(), IndexDirection::Asc)],
                unique: true,
                sparse: false,
            })
            .unwrap_err();
        assert!(matches!(err, DocdbError::CollectionNotFound(_, _)));
    }

    #[test]
    fn dashboard_uses_replica_state_and_collection_visibility() {
        let mut p = DocdbPlugin::new();
        p.set_replica_set(vec![
            member("a", ReplicaState::Primary, 0.0),
            member("b", ReplicaState::Secondary, 1.5),
            member("c", ReplicaState::Secondary, 7.0),
        ]);
        // MongoDB-style: each tenant has its own database. Same collection name
        // is fine across databases.
        p.register_collection(coll("acme_app", "users", "acme", 100, 10_000)).unwrap();
        p.register_collection(coll("globex_app", "users", "globex", 50, 5_000)).unwrap();
        // Tenant view sees only their collection.
        let panel = p.dashboard(ViewPersona::Tenant, "acme");
        assert_eq!(panel.collection_count, 1);
        assert_eq!(panel.total_documents, 100);
        assert_eq!(panel.primary_count, 1);
        assert_eq!(panel.secondary_count, 2);
        assert_eq!(panel.max_replica_lag_seconds, 7.0);
        // Admin view sees both.
        let panel = p.dashboard(ViewPersona::Admin, "acme");
        assert_eq!(panel.collection_count, 2);
        assert_eq!(panel.total_documents, 150);
    }

    #[test]
    fn list_indexes_filters_by_collection() {
        let mut p = DocdbPlugin::new();
        p.register_collection(coll("app", "users", "acme", 0, 0)).unwrap();
        p.register_collection(coll("app", "orders", "acme", 0, 0)).unwrap();
        for name in ["ix1", "ix2"] {
            p.register_index(IndexInfo {
                database: "app".into(),
                collection: "users".into(),
                name: name.into(),
                keys: vec![("k".into(), IndexDirection::Asc)],
                unique: false,
                sparse: false,
            })
            .unwrap();
        }
        assert_eq!(p.list_indexes("app", "users").len(), 2);
        assert_eq!(p.list_indexes("app", "orders").len(), 0);
    }

    #[test]
    fn pipeline_blocks_forbidden_stages() {
        let p = DocdbPlugin::new();
        for forbidden in ["$out", "$merge", "$indexStats"] {
            let err = p
                .validate_pipeline(
                    &[format!("{}: {{}}", forbidden)],
                    ViewPersona::Admin,
                    "acme",
                )
                .unwrap_err();
            assert!(matches!(err, DocdbError::InvalidPipeline(_)));
        }
    }

    #[test]
    fn pipeline_allows_known_stages() {
        let p = DocdbPlugin::new();
        let preview = p
            .validate_pipeline(
                &[
                    "$match: { active: true }".into(),
                    "$group: { _id: '$tenant' }".into(),
                    "$sort: { count: -1 }".into(),
                    "$limit: 10".into(),
                ],
                ViewPersona::Tenant,
                "acme",
            )
            .unwrap();
        assert_eq!(preview.stage_count, 4);
        assert_eq!(preview.stages, vec!["$match", "$group", "$sort", "$limit"]);
    }

    #[test]
    fn pipeline_rejects_empty_and_malformed() {
        let p = DocdbPlugin::new();
        assert!(matches!(
            p.validate_pipeline(&[], ViewPersona::Admin, "acme"),
            Err(DocdbError::InvalidPipeline(_))
        ));
        assert!(matches!(
            p.validate_pipeline(&["no-colon".into()], ViewPersona::Admin, "acme"),
            Err(DocdbError::InvalidPipeline(_))
        ));
    }

    #[test]
    fn change_stream_view_scopes_to_tenant() {
        let mut p = DocdbPlugin::new();
        p.register_change_stream(ChangeStream {
            id: "cs1".into(),
            database: "app".into(),
            collection: "users".into(),
            tenant: "acme".into(),
            resume_token: "tok1".into(),
            lag_seconds: 0.5,
            started_at: Utc::now(),
        });
        p.register_change_stream(ChangeStream {
            id: "cs2".into(),
            database: "app".into(),
            collection: "users".into(),
            tenant: "globex".into(),
            resume_token: "tok2".into(),
            lag_seconds: 0.5,
            started_at: Utc::now(),
        });
        assert_eq!(p.change_stream_view(ViewPersona::Tenant, "acme").len(), 1);
        assert_eq!(p.change_stream_view(ViewPersona::Admin, "acme").len(), 2);
    }

    #[test]
    fn lagging_members_filters_by_threshold() {
        let mut p = DocdbPlugin::new();
        p.set_replica_set(vec![
            member("a", ReplicaState::Primary, 0.0),
            member("b", ReplicaState::Secondary, 3.0),
            member("c", ReplicaState::Secondary, 12.0),
        ]);
        let bad = p.lagging_members(10.0);
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].host, "c");
    }
}
