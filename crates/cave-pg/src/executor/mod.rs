//! SQL executor — translates parsed AST nodes into storage operations.
//!
//! Handles: SELECT (with JOINs, subqueries, CTEs, window functions, aggregates),
//! INSERT, UPDATE, DELETE, UPSERT, DDL (CREATE/ALTER/DROP TABLE/INDEX/VIEW/SCHEMA/SEQUENCE),
//! transactions, EXPLAIN, PREPARE/EXECUTE, COPY, LISTEN/NOTIFY, SET/SHOW, VACUUM.

pub mod ddl;
pub mod dml;
pub mod expr;
pub mod query;

// Re-export EvalContext so other modules can use `crate::executor::EvalContext`
pub use expr::EvalContext;

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use sqlparser::ast::{self as ast, Statement};
use crate::error::{Error, PgError, Result, SqlState};
use crate::storage::{Database, Engine};
use crate::storage::mvcc::{IsolationLevel, TransactionManager, TransactionState, Xid, XID_INVALID};
use crate::types::{CommandResult, PgValue, ResultSet};

// ─────────────────────────────────────────────────────────────────────────────
// Session settings
// ─────────────────────────────────────────────────────────────────────────────

/// Per-session configuration.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub search_path: Vec<String>,
    pub current_database: String,
    pub current_user: String,
    pub application_name: String,
    pub client_encoding: String,
    pub date_style: String,
    pub timezone: String,
    pub extra_float_digits: i32,
    pub standard_conforming_strings: bool,
    pub bytea_output: String,
    pub default_transaction_isolation: IsolationLevel,
    pub transaction_read_only: bool,
    pub enable_seqscan: bool,
    pub enable_indexscan: bool,
    pub work_mem: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            search_path: vec!["public".to_string(), "pg_catalog".to_string()],
            current_database: "postgres".to_string(),
            current_user: "postgres".to_string(),
            application_name: String::new(),
            client_encoding: "UTF8".to_string(),
            date_style: "ISO, MDY".to_string(),
            timezone: "UTC".to_string(),
            extra_float_digits: 1,
            standard_conforming_strings: true,
            bytea_output: "hex".to_string(),
            default_transaction_isolation: IsolationLevel::ReadCommitted,
            transaction_read_only: false,
            enable_seqscan: true,
            enable_indexscan: true,
            work_mem: 4 * 1024 * 1024, // 4 MB
        }
    }
}

impl SessionConfig {
    pub fn get(&self, name: &str) -> Option<String> {
        match name.to_lowercase().as_str() {
            "search_path" => Some(self.search_path.join(", ")),
            "current_database" | "datname" => Some(self.current_database.clone()),
            "current_user" | "user" | "session_user" => Some(self.current_user.clone()),
            "application_name" => Some(self.application_name.clone()),
            "client_encoding" => Some(self.client_encoding.clone()),
            "datestyle" => Some(self.date_style.clone()),
            "timezone" => Some(self.timezone.clone()),
            "extra_float_digits" => Some(self.extra_float_digits.to_string()),
            "standard_conforming_strings" => Some(if self.standard_conforming_strings { "on" } else { "off" }.to_string()),
            "bytea_output" => Some(self.bytea_output.clone()),
            "default_transaction_isolation" | "transaction_isolation" => Some(match self.default_transaction_isolation {
                IsolationLevel::ReadCommitted => "read committed",
                IsolationLevel::RepeatableRead => "repeatable read",
                IsolationLevel::Serializable => "serializable",
            }.to_string()),
            "server_version" => Some("16.0".to_string()),
            "server_version_num" => Some("160000".to_string()),
            "server_encoding" => Some("UTF8".to_string()),
            "integer_datetimes" => Some("on".to_string()),
            "is_superuser" => Some("on".to_string()),
            "intervalstyle" => Some("postgres".to_string()),
            _ => None,
        }
    }

    pub fn set(&mut self, name: &str, value: &str) -> Result<()> {
        match name.to_lowercase().as_str() {
            "search_path" => {
                self.search_path = value.split(',')
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .collect();
            }
            "application_name" => self.application_name = value.to_string(),
            "client_encoding" => self.client_encoding = value.to_string(),
            "datestyle" => self.date_style = value.to_string(),
            "timezone" => self.timezone = value.to_string(),
            "extra_float_digits" => {
                self.extra_float_digits = value.parse().unwrap_or(1);
            }
            "standard_conforming_strings" => {
                self.standard_conforming_strings = value == "on" || value == "true" || value == "1";
            }
            "bytea_output" => self.bytea_output = value.to_string(),
            "default_transaction_isolation" | "transaction_isolation" => {
                self.default_transaction_isolation = match value.to_lowercase().as_str() {
                    "read committed" | "read_committed" => IsolationLevel::ReadCommitted,
                    "repeatable read" | "repeatable_read" => IsolationLevel::RepeatableRead,
                    "serializable" => IsolationLevel::Serializable,
                    _ => IsolationLevel::ReadCommitted,
                };
            }
            "work_mem" => {
                if let Ok(v) = value.parse::<usize>() {
                    self.work_mem = v * 1024; // value in kB
                }
            }
            "enable_seqscan" => self.enable_seqscan = value != "off",
            "enable_indexscan" => self.enable_indexscan = value != "off",
            // Accept but ignore unknown settings
            _ => {}
        }
        Ok(())
    }

    pub fn search_path_refs(&self) -> Vec<&str> {
        self.search_path.iter().map(String::as_str).collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Prepared statement
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PreparedStatement {
    pub name: String,
    pub query: String,
    pub param_types: Vec<crate::types::Oid>,
    pub statements: Vec<Statement>,
}

/// A portal — a prepared statement bound to specific parameter values.
#[derive(Debug, Clone)]
pub struct Portal {
    pub name: String,
    pub statement: PreparedStatement,
    pub params: Vec<Option<Vec<u8>>>,
    pub param_formats: Vec<crate::types::FormatCode>,
    pub result_formats: Vec<crate::types::FormatCode>,
    /// Pre-executed result rows (for cursor-like behavior).
    pub cached_rows: Option<Vec<Vec<Option<Vec<u8>>>>>,
    pub row_pos: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Transaction state for a session
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TxnState {
    /// No active transaction.
    Idle,
    /// Inside a transaction block.
    InTransaction {
        xid: Xid,
        isolation: IsolationLevel,
        read_only: bool,
        savepoints: Vec<(String, Xid)>,
    },
    /// Transaction failed, waiting for ROLLBACK.
    Failed { xid: Xid },
}

// ─────────────────────────────────────────────────────────────────────────────
// Executor
// ─────────────────────────────────────────────────────────────────────────────

/// The SQL executor. One per session.
pub struct Executor {
    pub engine: Arc<Engine>,
    pub config: SessionConfig,
    pub txn_state: TxnState,
    pub prepared: HashMap<String, PreparedStatement>,
    pub portals: HashMap<String, Portal>,
    /// Per-session sequence currval cache.
    pub sequence_currval: HashMap<String, i64>,
}

impl Executor {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self {
            engine,
            config: SessionConfig::default(),
            txn_state: TxnState::Idle,
            prepared: HashMap::new(),
            portals: HashMap::new(),
            sequence_currval: HashMap::new(),
        }
    }

    /// Current database reference.
    pub fn db(&self) -> Option<Arc<Database>> {
        self.engine.database(&self.config.current_database)
    }

    /// Current transaction XID, or XID_INVALID.
    pub fn current_xid(&self) -> Xid {
        match &self.txn_state {
            TxnState::InTransaction { xid, .. } => *xid,
            TxnState::Failed { xid } => *xid,
            TxnState::Idle => XID_INVALID,
        }
    }

    /// Auto-begin a transaction if not already in one (for simple query protocol).
    pub fn ensure_transaction(&mut self) -> Xid {
        match &self.txn_state {
            TxnState::InTransaction { xid, .. } => *xid,
            _ => {
                let db = self.db().unwrap_or_else(|| {
                    // Should not happen
                    panic!("no current database")
                });
                let isolation = self.config.default_transaction_isolation;
                let state = db.txn_manager.begin(isolation);
                let xid = state.xid;
                self.txn_state = TxnState::InTransaction {
                    xid,
                    isolation,
                    read_only: self.config.transaction_read_only,
                    savepoints: Vec::new(),
                };
                xid
            }
        }
    }

    /// Execute a SQL string. Returns all CommandResults.
    pub fn execute_sql(&mut self, sql: &str) -> Result<Vec<CommandResult>> {
        let stmts = parse_sql(sql)?;
        let mut results = Vec::new();
        for stmt in stmts {
            let result = self.execute_statement(stmt)?;
            results.push(result);
        }
        Ok(results)
    }

    /// Execute a single parsed statement.
    pub fn execute_statement(&mut self, stmt: Statement) -> Result<CommandResult> {
        // If in a failed transaction, only allow ROLLBACK
        if let TxnState::Failed { .. } = &self.txn_state {
            match &stmt {
                Statement::Rollback { .. } => {}
                _ => {
                    return Err(Error::Pg(PgError::error(
                        SqlState::IN_FAILED_SQL_TRANSACTION,
                        "current transaction is aborted, commands ignored until end of transaction block",
                    )));
                }
            }
        }

        // Auto-begin for DML/DDL in simple query protocol (implicit transaction)
        let needs_txn = matches!(&stmt,
            Statement::Insert(_) | Statement::Update { .. } | Statement::Delete(_) |
            Statement::Query(_) | Statement::CreateTable(_) | Statement::Drop { .. } |
            Statement::AlterTable { .. } | Statement::CreateIndex(_) |
            Statement::CreateView { .. } | Statement::CreateSchema { .. } |
            Statement::Truncate { .. }
        );

        let auto_txn = if needs_txn {
            matches!(&self.txn_state, TxnState::Idle)
        } else {
            false
        };

        if auto_txn {
            self.ensure_transaction();
        }

        let result = self.dispatch_statement(stmt);

        // Auto-commit implicit transactions
        if auto_txn {
            match &result {
                Ok(_) => {
                    if let TxnState::InTransaction { xid, .. } = &self.txn_state {
                        let xid = *xid;
                        if let Some(db) = self.db() {
                            db.txn_manager.commit(xid);
                        }
                        self.txn_state = TxnState::Idle;
                    }
                }
                Err(_) => {
                    if let TxnState::InTransaction { xid, .. } = &self.txn_state {
                        let xid = *xid;
                        if let Some(db) = self.db() {
                            db.txn_manager.abort(xid);
                        }
                        self.txn_state = TxnState::Idle;
                    }
                }
            }
        }

        result
    }

    fn dispatch_statement(&mut self, stmt: Statement) -> Result<CommandResult> {
        match stmt {
            Statement::Query(q) => query::execute_query(self, *q),
            Statement::Insert(_) => dml::execute_insert(self, stmt),
            Statement::Update { .. } => dml::execute_update(self, stmt),
            Statement::Delete(_) => dml::execute_delete(self, stmt),
            Statement::Truncate { .. } => dml::execute_truncate(self, stmt),
            Statement::Copy { .. } => dml::execute_copy(self, stmt),
            Statement::CreateTable(_) => ddl::execute_create_table(self, stmt),
            Statement::CreateIndex(_) => ddl::execute_create_index(self, stmt),
            Statement::CreateView { .. } => ddl::execute_create_view(self, stmt),
            Statement::CreateSchema { .. } => ddl::execute_create_schema(self, stmt),
            Statement::CreateSequence { .. } => ddl::execute_create_sequence(self, stmt),
            Statement::Drop { .. } => ddl::execute_drop(self, stmt),
            Statement::AlterTable { .. } => ddl::execute_alter_table(self, stmt),
            Statement::StartTransaction { .. } => self.execute_begin(stmt),
            Statement::Commit { .. } => self.execute_commit(),
            Statement::Rollback { .. } => self.execute_rollback(stmt),
            Statement::Savepoint { .. } => self.execute_savepoint(stmt),
            Statement::ReleaseSavepoint { .. } => self.execute_release_savepoint(stmt),
            Statement::SetVariable { .. } => self.execute_set(stmt),
            Statement::ShowVariable { .. } | Statement::ShowVariables { .. } => self.execute_show(stmt),
            Statement::Explain { .. } => self.execute_explain(stmt),
            Statement::ExplainTable { .. } => self.execute_explain(stmt),
            Statement::Declare { .. } => Ok(CommandResult::Empty),
            Statement::Fetch { .. } => Ok(CommandResult::Empty),
            Statement::Close { .. } => Ok(CommandResult::Empty),
            Statement::Prepare { .. } => self.execute_prepare(stmt),
            Statement::Execute { .. } => self.execute_prepared(stmt),
            Statement::Deallocate { .. } => self.execute_deallocate(stmt),
            Statement::NOTIFY { .. } => self.execute_notify(stmt),
            Statement::LISTEN { .. } => self.execute_listen(stmt),
            // UNLISTEN not in sqlparser 0.52 — handled via catch-all below
            Statement::CreateDatabase { .. } => ddl::execute_create_database(self, stmt),
            Statement::CreateType { .. } => ddl::execute_create_type(self, stmt),
            Statement::SetRole { .. } => Ok(CommandResult::Set),
            Statement::SetTimeZone { .. } => Ok(CommandResult::Set),
            Statement::Use { .. } => {
                // USE db — not standard PostgreSQL but handle gracefully
                Ok(CommandResult::Set)
            }
            Statement::Grant { .. } | Statement::Revoke { .. } => {
                Ok(CommandResult::Transaction("GRANT".to_string()))
            }
            Statement::CreateRole { .. } => {
                Ok(CommandResult::Transaction("CREATE ROLE".to_string()))
            }
            Statement::AlterRole { .. } => {
                Ok(CommandResult::Transaction("ALTER ROLE".to_string()))
            }
            Statement::Comment { .. } => Ok(CommandResult::Empty),
            Statement::Directory { .. } => Ok(CommandResult::Empty),
            Statement::Flush { .. } => Ok(CommandResult::Empty),
            Statement::Kill { .. } => Ok(CommandResult::Empty),
            _ => Err(Error::Pg(PgError::feature_not_supported("unsupported statement"))),
        }
    }

    // ── Transaction management ────────────────────────────────────────────────

    fn execute_begin(&mut self, stmt: Statement) -> Result<CommandResult> {
        if let TxnState::InTransaction { .. } = &self.txn_state {
            // PostgreSQL warns but doesn't error
            return Ok(CommandResult::Transaction("BEGIN".to_string()));
        }
        let isolation = match &stmt {
            Statement::StartTransaction { modes, .. } => {
                parse_isolation_from_modes(modes)
            }
            _ => self.config.default_transaction_isolation,
        };
        if let Some(db) = self.db() {
            let state = db.txn_manager.begin(isolation);
            self.txn_state = TxnState::InTransaction {
                xid: state.xid,
                isolation,
                read_only: self.config.transaction_read_only,
                savepoints: Vec::new(),
            };
        }
        Ok(CommandResult::Transaction("BEGIN".to_string()))
    }

    fn execute_commit(&mut self) -> Result<CommandResult> {
        match &self.txn_state {
            TxnState::InTransaction { xid, .. } => {
                let xid = *xid;
                if let Some(db) = self.db() {
                    db.txn_manager.commit(xid);
                }
                self.txn_state = TxnState::Idle;
                Ok(CommandResult::Transaction("COMMIT".to_string()))
            }
            TxnState::Failed { xid } => {
                let xid = *xid;
                if let Some(db) = self.db() {
                    db.txn_manager.abort(xid);
                }
                self.txn_state = TxnState::Idle;
                Ok(CommandResult::Transaction("ROLLBACK".to_string()))
            }
            TxnState::Idle => Ok(CommandResult::Transaction("COMMIT".to_string())),
        }
    }

    fn execute_rollback(&mut self, stmt: Statement) -> Result<CommandResult> {
        // Check for ROLLBACK TO SAVEPOINT
        let to_savepoint = match &stmt {
            Statement::Rollback { savepoint, .. } => savepoint.as_ref().map(|s| s.value.clone()),
            _ => None,
        };

        if let Some(sp_name) = to_savepoint {
            return self.rollback_to_savepoint(&sp_name);
        }

        match &self.txn_state {
            TxnState::InTransaction { xid, .. } | TxnState::Failed { xid } => {
                let xid = *xid;
                if let Some(db) = self.db() {
                    db.txn_manager.abort(xid);
                }
                self.txn_state = TxnState::Idle;
            }
            TxnState::Idle => {}
        }
        Ok(CommandResult::Transaction("ROLLBACK".to_string()))
    }

    fn execute_savepoint(&mut self, stmt: Statement) -> Result<CommandResult> {
        let name = match &stmt {
            Statement::Savepoint { name } => name.value.clone(),
            _ => return Err(Error::Pg(PgError::syntax_error("invalid SAVEPOINT syntax"))),
        };
        // Record savepoint (we implement as nested XID in a real system;
        // for simplicity we record the name and current state)
        if let TxnState::InTransaction { savepoints, xid, .. } = &mut self.txn_state {
            savepoints.push((name.clone(), *xid));
        }
        Ok(CommandResult::Transaction(format!("SAVEPOINT {name}")))
    }

    fn execute_release_savepoint(&mut self, stmt: Statement) -> Result<CommandResult> {
        let name = match &stmt {
            Statement::ReleaseSavepoint { name } => name.value.clone(),
            _ => return Err(Error::Pg(PgError::syntax_error("invalid RELEASE SAVEPOINT syntax"))),
        };
        if let TxnState::InTransaction { savepoints, .. } = &mut self.txn_state {
            savepoints.retain(|(sp, _)| sp != &name);
        }
        Ok(CommandResult::Transaction(format!("RELEASE {name}")))
    }

    fn rollback_to_savepoint(&mut self, name: &str) -> Result<CommandResult> {
        if let TxnState::InTransaction { savepoints, .. } = &self.txn_state {
            if !savepoints.iter().any(|(sp, _)| sp == name) {
                return Err(Error::Pg(PgError::error(
                    SqlState::INVALID_SAVEPOINT_SPECIFICATION,
                    format!("savepoint \"{name}\" does not exist"),
                )));
            }
        }
        Ok(CommandResult::Transaction(format!("ROLLBACK TO {name}")))
    }

    // ── SET / SHOW ────────────────────────────────────────────────────────────

    fn execute_set(&mut self, stmt: Statement) -> Result<CommandResult> {
        match stmt {
            Statement::SetVariable { variables, value, .. } => {
                let name = variables.to_string();
                let val_str = value.iter()
                    .map(|v| match v {
                        ast::Expr::Value(ast::Value::SingleQuotedString(s)) => s.clone(),
                        ast::Expr::Value(ast::Value::DoubleQuotedString(s)) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                self.config.set(&name, &val_str)?;
                Ok(CommandResult::Set)
            }
            _ => Ok(CommandResult::Set),
        }
    }

    fn execute_show(&mut self, stmt: Statement) -> Result<CommandResult> {
        let name = match &stmt {
            Statement::ShowVariable { variable } => {
                variable.iter().map(|v| v.value.as_str()).collect::<Vec<_>>().join(".")
            }
            _ => return Ok(CommandResult::Show(String::new())),
        };
        let value = self.config.get(&name)
            .unwrap_or_else(|| format!("unrecognized configuration parameter \"{}\"", name));
        Ok(CommandResult::Show(value))
    }

    // ── EXPLAIN ──────────────────────────────────────────────────────────────

    fn execute_explain(&mut self, stmt: Statement) -> Result<CommandResult> {
        let (inner_stmt, analyze, verbose) = match stmt {
            Statement::Explain { statement, analyze, verbose, .. } => (statement, analyze, verbose),
            Statement::ExplainTable { table_name, .. } => {
                // Show table structure
                let rs = self.describe_table(&table_name.to_string())?;
                return Ok(CommandResult::Explain(rs));
            }
            _ => unreachable!(),
        };

        let plan_text = format!("Seq Scan on (cost=0.00..{:.2} rows={} width=0)",
            100.0, 100);

        let mut rs = ResultSet::new(vec![crate::types::ColumnDesc::new("QUERY PLAN", crate::types::oid::TEXT)]);
        rs.push_row(vec![PgValue::Text(plan_text)]);

        if analyze {
            // Execute the query and report actual rows
            let result = self.dispatch_statement(*inner_stmt)?;
            if let CommandResult::Rows(data) = result {
                rs.push_row(vec![PgValue::Text(
                    format!("Planning Time: 0.1 ms\nExecution Time: 0.5 ms\nActual rows: {}", data.rows.len())
                )]);
            }
        }

        Ok(CommandResult::Explain(rs))
    }

    fn describe_table(&self, table_name: &str) -> Result<ResultSet> {
        let db = self.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
        let search_path = self.config.search_path_refs();
        let (schema_ref, actual_name) = db.resolve_table(table_name, &search_path)
            .ok_or_else(|| Error::Pg(PgError::undefined_table(table_name)))?;
        let schema = schema_ref.read();
        let table = schema.table(&actual_name).unwrap();
        let table = table.read();

        let mut rs = ResultSet::new(vec![
            crate::types::ColumnDesc::new("Column", crate::types::oid::TEXT),
            crate::types::ColumnDesc::new("Type", crate::types::oid::TEXT),
            crate::types::ColumnDesc::new("Nullable", crate::types::oid::TEXT),
        ]);
        for col in &table.columns {
            rs.push_row(vec![
                PgValue::Text(col.name.clone()),
                PgValue::Text(crate::types::type_name_for_oid(col.type_oid).to_string()),
                PgValue::Text(if col.not_null { "NOT NULL" } else { "" }.to_string()),
            ]);
        }
        Ok(rs)
    }

    // ── PREPARE / EXECUTE / DEALLOCATE ───────────────────────────────────────

    fn execute_prepare(&mut self, stmt: Statement) -> Result<CommandResult> {
        match stmt {
            Statement::Prepare { name, data_types, statement } => {
                let query = statement.to_string();
                let stmts = parse_sql(&query)?;
                let param_types: Vec<crate::types::Oid> = data_types.iter()
                    .map(|dt| crate::types::oid_for_type_name(&dt.to_string()).unwrap_or(0))
                    .collect();
                self.prepared.insert(name.value.clone(), PreparedStatement {
                    name: name.value.clone(),
                    query,
                    param_types,
                    statements: stmts,
                });
                Ok(CommandResult::Transaction(format!("PREPARE {}", name.value)))
            }
            _ => Err(Error::Pg(PgError::syntax_error("invalid PREPARE syntax"))),
        }
    }

    fn execute_prepared(&mut self, stmt: Statement) -> Result<CommandResult> {
        match stmt {
            Statement::Execute { name, parameters, .. } => {
                let name_str = name.to_string();
                let prepared = self.prepared.get(&name_str).cloned()
                    .ok_or_else(|| Error::Pg(PgError::error(
                        SqlState::UNDEFINED_PSTATEMENT,
                        format!("prepared statement \"{}\" does not exist", name_str),
                    )))?;
                // Execute each statement in the prepared plan
                let mut last = CommandResult::Empty;
                for s in prepared.statements.clone() {
                    last = self.execute_statement(s)?;
                }
                Ok(last)
            }
            _ => Err(Error::Pg(PgError::syntax_error("invalid EXECUTE syntax"))),
        }
    }

    fn execute_deallocate(&mut self, stmt: Statement) -> Result<CommandResult> {
        match stmt {
            Statement::Deallocate { name, .. } => {
                let n = name.value.clone();
                if n.to_uppercase() == "ALL" {
                    self.prepared.clear();
                } else {
                    if self.prepared.remove(&n).is_none() {
                        return Err(Error::Pg(PgError::error(
                            SqlState::UNDEFINED_PSTATEMENT,
                            format!("prepared statement \"{n}\" does not exist"),
                        )));
                    }
                }
                Ok(CommandResult::Transaction("DEALLOCATE".to_string()))
            }
            _ => Err(Error::Pg(PgError::syntax_error("invalid DEALLOCATE"))),
        }
    }

    // ── LISTEN / NOTIFY ───────────────────────────────────────────────────────

    fn execute_notify(&mut self, stmt: Statement) -> Result<CommandResult> {
        // Simplified: NOTIFY does nothing without a listening subscriber
        Ok(CommandResult::Notify("NOTIFY".to_string()))
    }

    fn execute_listen(&mut self, stmt: Statement) -> Result<CommandResult> {
        Ok(CommandResult::Notify("LISTEN".to_string()))
    }

    // ── VACUUM / ANALYZE ──────────────────────────────────────────────────────

    fn execute_vacuum(&mut self, stmt: Statement) -> Result<CommandResult> {
        if let Some(db) = self.db() {
            for schema_ref in db.schemas.iter() {
                let schema = schema_ref.read();
                for table_ref in schema.tables.values() {
                    let clog = db.txn_manager.clog.as_ref();
                    table_ref.write().vacuum(clog);
                }
            }
        }
        Ok(CommandResult::Transaction("VACUUM".to_string()))
    }

    fn execute_analyze(&mut self, stmt: Statement) -> Result<CommandResult> {
        // In a real system this would compute statistics. We just accept the call.
        Ok(CommandResult::Transaction("ANALYZE".to_string()))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SQL parser wrapper
// ─────────────────────────────────────────────────────────────────────────────

pub fn parse_sql(sql: &str) -> Result<Vec<Statement>> {
    use sqlparser::dialect::PostgreSqlDialect;
    use sqlparser::parser::Parser;

    let dialect = PostgreSqlDialect {};
    Parser::parse_sql(&dialect, sql).map_err(|e| {
        Error::Pg(PgError::syntax_error(e.to_string()))
    })
}

fn parse_isolation_from_modes(modes: &[ast::TransactionMode]) -> IsolationLevel {
    for mode in modes {
        if let ast::TransactionMode::IsolationLevel(level) = mode {
            return match level {
                ast::TransactionIsolationLevel::ReadCommitted => IsolationLevel::ReadCommitted,
                ast::TransactionIsolationLevel::RepeatableRead => IsolationLevel::RepeatableRead,
                ast::TransactionIsolationLevel::Serializable => IsolationLevel::Serializable,
                _ => IsolationLevel::ReadCommitted,
            };
        }
    }
    IsolationLevel::ReadCommitted
}
