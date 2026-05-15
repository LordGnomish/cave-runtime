// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-pg.

use crate::manager;
use crate::models::*;
use crate::PgState;
use axum::{
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::Utc;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<PgState>) -> Router {
    Router::new()
        // Health
        .route("/api/pg/health", get(health))
        // Database instances
        .route(
            "/api/pg/databases",
            get(list_databases).post(register_database),
        )
        .route(
            "/api/pg/databases/{id}",
            get(get_database).delete(delete_database),
        )
        .route("/api/pg/databases/{id}/healthcheck", post(healthcheck_database))
        // Schema migrations
        .route(
            "/api/pg/migrations",
            get(list_migrations).post(record_migration),
        )
        .route("/api/pg/migrations/{id}", get(get_migration))
        // Connection pools
        .route("/api/pg/pools", get(list_pools).post(create_pool))
        .route("/api/pg/pools/{id}", get(get_pool).put(update_pool))
        // Query analytics — literal /slow before /{id} to ensure priority
        .route("/api/pg/queries/slow", get(slow_queries))
        .route("/api/pg/queries", get(list_queries).post(record_query))
        // Backup / restore
        .route("/api/pg/backups", get(list_backups).post(create_backup))
        .route("/api/pg/backups/{id}", get(get_backup))
        .route("/api/pg/backups/{id}/restore", post(restore_backup))
        // Replication monitoring
        .route(
            "/api/pg/replication",
            get(list_replication).post(register_replication),
        )
        .route("/api/pg/replication/{id}", put(update_replication))
        // Table / index statistics
        .route("/api/pg/tables/bloat", get(bloated_tables))
        .route(
            "/api/pg/tables",
            get(list_tables).post(record_table_stat),
        )
        // User / role management
        .route("/api/pg/users", get(list_users).post(create_user))
        .route("/api/pg/users/{id}", delete(delete_user))
        // Size monitoring & alerts
        .route("/api/pg/sizes", get(list_sizes).post(record_size))
        .route("/api/pg/alerts", get(size_alerts))
        .with_state(state)
}

// ── Health ────────────────────────────────────────────────────────────────────

async fn health() -> Json<Value> {
    Json(json!({
        "module": "cave-pg",
        "status": "ok",
        "upstream": ["pgAdmin", "CloudNativePG"]
    }))
}

// ── Databases ─────────────────────────────────────────────────────────────────

async fn list_databases(AxumState(state): AxumState<Arc<PgState>>) -> Json<Value> {
    let dbs = state.databases.lock().unwrap();
    let list: Vec<DatabaseInstance> = dbs.values().cloned().collect();
    Json(json!({ "databases": list, "count": list.len() }))
}

async fn register_database(
    AxumState(state): AxumState<Arc<PgState>>,
    Json(req): Json<RegisterDatabaseRequest>,
) -> (StatusCode, Json<Value>) {
    let instance = DatabaseInstance {
        id: Uuid::new_v4(),
        name: req.name,
        host: req.host,
        port: req.port,
        database: req.database,
        status: DbStatus::Unknown,
        version: None,
        registered_at: Utc::now(),
        last_checked: None,
        tags: req.tags.unwrap_or_default(),
    };
    let id = instance.id;
    state.databases.lock().unwrap().insert(id, instance.clone());
    tracing::info!(id = %id, "registered database");
    (StatusCode::CREATED, Json(json!({ "database": instance })))
}

async fn get_database(
    AxumState(state): AxumState<Arc<PgState>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Value>) {
    let dbs = state.databases.lock().unwrap();
    match dbs.get(&id) {
        Some(db) => (StatusCode::OK, Json(json!({ "database": db }))),
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "database not found" }))),
    }
}

async fn delete_database(
    AxumState(state): AxumState<Arc<PgState>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Value>) {
    let removed = state.databases.lock().unwrap().remove(&id).is_some();
    if removed {
        tracing::info!(id = %id, "deregistered database");
        (StatusCode::OK, Json(json!({ "deleted": id })))
    } else {
        (StatusCode::NOT_FOUND, Json(json!({ "error": "database not found" })))
    }
}

async fn healthcheck_database(
    AxumState(state): AxumState<Arc<PgState>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Value>) {
    let mut dbs = state.databases.lock().unwrap();
    match dbs.get_mut(&id) {
        Some(db) => {
            // Simulate connectivity check; real impl would open a pg connection.
            db.status = DbStatus::Online;
            db.last_checked = Some(Utc::now());
            let checked_at = db.last_checked;
            (
                StatusCode::OK,
                Json(json!({
                    "id": id,
                    "status": "online",
                    "checked_at": checked_at
                })),
            )
        }
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "database not found" }))),
    }
}

// ── Migrations ────────────────────────────────────────────────────────────────

async fn list_migrations(AxumState(state): AxumState<Arc<PgState>>) -> Json<Value> {
    let migs = state.migrations.lock().unwrap();
    let list: Vec<MigrationRecord> = migs.values().cloned().collect();
    Json(json!({ "migrations": list, "count": list.len() }))
}

async fn record_migration(
    AxumState(state): AxumState<Arc<PgState>>,
    Json(req): Json<RecordMigrationRequest>,
) -> (StatusCode, Json<Value>) {
    let record = MigrationRecord {
        id: Uuid::new_v4(),
        database_id: req.database_id,
        version: req.version,
        name: req.name,
        checksum: req.checksum,
        status: MigrationStatus::Pending,
        applied_at: None,
        execution_ms: None,
        error: None,
    };
    let id = record.id;
    state.migrations.lock().unwrap().insert(id, record.clone());
    (StatusCode::CREATED, Json(json!({ "migration": record })))
}

async fn get_migration(
    AxumState(state): AxumState<Arc<PgState>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Value>) {
    let migs = state.migrations.lock().unwrap();
    match migs.get(&id) {
        Some(m) => (StatusCode::OK, Json(json!({ "migration": m }))),
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "migration not found" }))),
    }
}

// ── Connection Pools ──────────────────────────────────────────────────────────

async fn list_pools(AxumState(state): AxumState<Arc<PgState>>) -> Json<Value> {
    let pools = state.pools.lock().unwrap();
    let list: Vec<ConnectionPool> = pools.values().cloned().collect();
    Json(json!({ "pools": list, "count": list.len() }))
}

async fn create_pool(
    AxumState(state): AxumState<Arc<PgState>>,
    Json(req): Json<CreatePoolRequest>,
) -> (StatusCode, Json<Value>) {
    let pool = ConnectionPool {
        id: Uuid::new_v4(),
        database_id: req.database_id,
        name: req.name,
        min_size: req.min_size,
        max_size: req.max_size,
        current_size: 0,
        idle_connections: 0,
        active_connections: 0,
        waiting_clients: 0,
        total_checkout_count: 0,
        avg_checkout_ms: 0.0,
        updated_at: Utc::now(),
    };
    let id = pool.id;
    state.pools.lock().unwrap().insert(id, pool.clone());
    (StatusCode::CREATED, Json(json!({ "pool": pool })))
}

async fn get_pool(
    AxumState(state): AxumState<Arc<PgState>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Value>) {
    let pools = state.pools.lock().unwrap();
    match pools.get(&id) {
        Some(p) => {
            let utilisation = manager::pool_utilisation_pct(p.current_size, p.max_size);
            (StatusCode::OK, Json(json!({ "pool": p, "utilisation_pct": utilisation })))
        }
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "pool not found" }))),
    }
}

async fn update_pool(
    AxumState(state): AxumState<Arc<PgState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdatePoolStatsRequest>,
) -> (StatusCode, Json<Value>) {
    let mut pools = state.pools.lock().unwrap();
    match pools.get_mut(&id) {
        Some(p) => {
            p.current_size = req.current_size;
            p.idle_connections = req.idle_connections;
            p.active_connections = req.active_connections;
            p.waiting_clients = req.waiting_clients;
            p.total_checkout_count = req.total_checkout_count;
            p.avg_checkout_ms = req.avg_checkout_ms;
            p.updated_at = Utc::now();
            let updated = p.clone();
            (StatusCode::OK, Json(json!({ "pool": updated })))
        }
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "pool not found" }))),
    }
}

// ── Query Analytics ───────────────────────────────────────────────────────────

async fn list_queries(AxumState(state): AxumState<Arc<PgState>>) -> Json<Value> {
    let stats = state.query_stats.lock().unwrap();
    let list: Vec<QueryStat> = stats.clone();
    Json(json!({ "queries": list, "count": list.len() }))
}

async fn record_query(
    AxumState(state): AxumState<Arc<PgState>>,
    Json(req): Json<RecordQueryStatRequest>,
) -> (StatusCode, Json<Value>) {
    let stat = QueryStat {
        id: Uuid::new_v4(),
        database_id: req.database_id,
        query_hash: req.query_hash,
        query_text: req.query_text,
        calls: req.calls,
        total_time_ms: req.total_time_ms,
        mean_time_ms: req.mean_time_ms,
        stddev_time_ms: req.stddev_time_ms,
        min_time_ms: req.min_time_ms,
        max_time_ms: req.max_time_ms,
        rows: req.rows,
        plan: req.plan,
        recorded_at: Utc::now(),
    };
    let id = stat.id;
    state.query_stats.lock().unwrap().push(stat.clone());
    (StatusCode::CREATED, Json(json!({ "query_stat": stat, "id": id })))
}

async fn slow_queries(
    AxumState(state): AxumState<Arc<PgState>>,
    Query(params): Query<SlowQueryParams>,
) -> Json<Value> {
    let threshold_ms = params.threshold_ms.unwrap_or(100.0);
    let stats = state.query_stats.lock().unwrap();
    let slow: Vec<QueryStat> = manager::slow_queries(&stats, threshold_ms)
        .into_iter()
        .cloned()
        .collect();
    let count = slow.len();
    Json(json!({
        "slow_queries": slow,
        "count": count,
        "threshold_ms": threshold_ms
    }))
}

// ── Backups ───────────────────────────────────────────────────────────────────

async fn list_backups(AxumState(state): AxumState<Arc<PgState>>) -> Json<Value> {
    let backups = state.backups.lock().unwrap();
    let list: Vec<BackupJob> = backups.values().cloned().collect();
    Json(json!({ "backups": list, "count": list.len() }))
}

async fn create_backup(
    AxumState(state): AxumState<Arc<PgState>>,
    Json(req): Json<CreateBackupRequest>,
) -> (StatusCode, Json<Value>) {
    let job = BackupJob {
        id: Uuid::new_v4(),
        database_id: req.database_id,
        backup_type: req.backup_type,
        status: BackupStatus::Pending,
        destination: req.destination,
        size_bytes: None,
        started_at: None,
        completed_at: None,
        error: None,
        created_at: Utc::now(),
    };
    let id = job.id;
    state.backups.lock().unwrap().insert(id, job.clone());
    tracing::info!(id = %id, "backup job created");
    (StatusCode::CREATED, Json(json!({ "backup": job })))
}

async fn get_backup(
    AxumState(state): AxumState<Arc<PgState>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Value>) {
    let backups = state.backups.lock().unwrap();
    match backups.get(&id) {
        Some(b) => (StatusCode::OK, Json(json!({ "backup": b }))),
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "backup not found" }))),
    }
}

async fn restore_backup(
    AxumState(state): AxumState<Arc<PgState>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Value>) {
    let backups = state.backups.lock().unwrap();
    match backups.get(&id) {
        Some(b) if matches!(b.status, BackupStatus::Completed) => (
            StatusCode::ACCEPTED,
            Json(json!({
                "restore_job": {
                    "backup_id": id,
                    "database_id": b.database_id,
                    "destination": b.destination,
                    "status": "queued",
                    "queued_at": Utc::now()
                }
            })),
        ),
        Some(_) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "backup is not in completed state" })),
        ),
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "backup not found" }))),
    }
}

// ── Replication ───────────────────────────────────────────────────────────────

async fn list_replication(AxumState(state): AxumState<Arc<PgState>>) -> Json<Value> {
    let rep = state.replication.lock().unwrap();
    let list: Vec<ReplicationStatus> = rep.values().cloned().collect();
    Json(json!({ "replication": list, "count": list.len() }))
}

async fn register_replication(
    AxumState(state): AxumState<Arc<PgState>>,
    Json(req): Json<RegisterReplicationRequest>,
) -> (StatusCode, Json<Value>) {
    let status = ReplicationStatus {
        id: Uuid::new_v4(),
        database_id: req.database_id,
        role: req.role,
        primary_host: req.primary_host,
        replication_lag_bytes: 0,
        replication_lag_seconds: 0.0,
        slots: vec![],
        is_in_recovery: false,
        updated_at: Utc::now(),
    };
    let id = status.id;
    state.replication.lock().unwrap().insert(id, status.clone());
    (StatusCode::CREATED, Json(json!({ "replication": status })))
}

async fn update_replication(
    AxumState(state): AxumState<Arc<PgState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateReplicationRequest>,
) -> (StatusCode, Json<Value>) {
    let mut rep = state.replication.lock().unwrap();
    match rep.get_mut(&id) {
        Some(s) => {
            s.replication_lag_bytes = req.replication_lag_bytes;
            s.replication_lag_seconds = req.replication_lag_seconds;
            s.slots = req.slots;
            s.is_in_recovery = req.is_in_recovery;
            s.updated_at = Utc::now();
            let healthy = manager::replication_healthy(s);
            let updated = s.clone();
            if !healthy {
                tracing::warn!(
                    id = %id,
                    lag_bytes = updated.replication_lag_bytes,
                    lag_secs = updated.replication_lag_seconds,
                    "replication lag threshold exceeded"
                );
            }
            (StatusCode::OK, Json(json!({ "replication": updated, "healthy": healthy })))
        }
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "replication record not found" }))),
    }
}

// ── Tables ────────────────────────────────────────────────────────────────────

async fn list_tables(AxumState(state): AxumState<Arc<PgState>>) -> Json<Value> {
    let tables = state.table_stats.lock().unwrap();
    let list: Vec<TableStat> = tables.clone();
    Json(json!({ "tables": list, "count": list.len() }))
}

async fn record_table_stat(
    AxumState(state): AxumState<Arc<PgState>>,
    Json(req): Json<RecordTableStatRequest>,
) -> (StatusCode, Json<Value>) {
    let stat = TableStat {
        id: Uuid::new_v4(),
        database_id: req.database_id,
        schema_name: req.schema_name,
        table_name: req.table_name,
        live_tuples: req.live_tuples,
        dead_tuples: req.dead_tuples,
        bloat_ratio: req.bloat_ratio,
        table_size_bytes: req.table_size_bytes,
        index_size_bytes: req.index_size_bytes,
        last_vacuum: req.last_vacuum,
        last_analyze: req.last_analyze,
        recorded_at: Utc::now(),
    };
    let id = stat.id;
    state.table_stats.lock().unwrap().push(stat.clone());
    (StatusCode::CREATED, Json(json!({ "table_stat": stat, "id": id })))
}

async fn bloated_tables(
    AxumState(state): AxumState<Arc<PgState>>,
    Query(params): Query<BloatParams>,
) -> Json<Value> {
    let min_ratio = params.min_ratio.unwrap_or(0.2);
    let tables = state.table_stats.lock().unwrap();
    let bloated: Vec<TableStat> = manager::bloated_tables(&tables, min_ratio)
        .into_iter()
        .cloned()
        .collect();
    let count = bloated.len();
    Json(json!({
        "bloated_tables": bloated,
        "count": count,
        "min_ratio": min_ratio
    }))
}

// ── Users ─────────────────────────────────────────────────────────────────────

async fn list_users(AxumState(state): AxumState<Arc<PgState>>) -> Json<Value> {
    let users = state.users.lock().unwrap();
    let list: Vec<DbUser> = users.values().cloned().collect();
    Json(json!({ "users": list, "count": list.len() }))
}

async fn create_user(
    AxumState(state): AxumState<Arc<PgState>>,
    Json(req): Json<CreateUserRequest>,
) -> (StatusCode, Json<Value>) {
    let user = DbUser {
        id: Uuid::new_v4(),
        database_id: req.database_id,
        username: req.username,
        roles: req.roles,
        can_login: req.can_login,
        is_superuser: req.is_superuser,
        connection_limit: req.connection_limit,
        valid_until: req.valid_until,
        created_at: Utc::now(),
    };
    let id = user.id;
    state.users.lock().unwrap().insert(id, user.clone());
    tracing::info!(id = %id, username = %user.username, "created database user");
    (StatusCode::CREATED, Json(json!({ "user": user })))
}

async fn delete_user(
    AxumState(state): AxumState<Arc<PgState>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Value>) {
    let removed = state.users.lock().unwrap().remove(&id).is_some();
    if removed {
        tracing::info!(id = %id, "deleted database user");
        (StatusCode::OK, Json(json!({ "deleted": id })))
    } else {
        (StatusCode::NOT_FOUND, Json(json!({ "error": "user not found" })))
    }
}

// ── Sizes & Alerts ────────────────────────────────────────────────────────────

async fn list_sizes(AxumState(state): AxumState<Arc<PgState>>) -> Json<Value> {
    let sizes = state.sizes.lock().unwrap();
    let list: Vec<DbSizeRecord> = sizes.clone();
    Json(json!({ "sizes": list, "count": list.len() }))
}

async fn record_size(
    AxumState(state): AxumState<Arc<PgState>>,
    Json(req): Json<RecordSizeRequest>,
) -> (StatusCode, Json<Value>) {
    let record = DbSizeRecord {
        id: Uuid::new_v4(),
        database_id: req.database_id,
        size_bytes: req.size_bytes,
        table_count: req.table_count,
        index_count: req.index_count,
        recorded_at: Utc::now(),
    };
    let id = record.id;
    state.sizes.lock().unwrap().push(record.clone());
    (StatusCode::CREATED, Json(json!({ "size_record": record, "id": id })))
}

async fn size_alerts(
    AxumState(state): AxumState<Arc<PgState>>,
    Query(params): Query<SizeAlertParams>,
) -> Json<Value> {
    const GIB: u64 = 1024 * 1024 * 1024;
    let threshold_bytes = params.threshold_bytes.unwrap_or(10 * GIB);
    let sizes = state.sizes.lock().unwrap();
    let alerts: Vec<DbSizeRecord> = manager::size_alert_records(&sizes, threshold_bytes)
        .into_iter()
        .cloned()
        .collect();
    let count = alerts.len();
    Json(json!({
        "alerts": alerts,
        "count": count,
        "threshold_bytes": threshold_bytes
    }))
}
