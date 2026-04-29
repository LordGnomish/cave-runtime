//! In-process SQL engine — storage + planner + executor + txn.
//!
//! Pluggable backend behind the wire dispatcher. Mirrors the layering used
//! by `pgwire`'s example backends and the postgres-rs reference test harness:
//!
//! ```text
//! TcpListener → Codec → Session → SqlEngine
//!                                  ├─ Catalog  (CREATE/DROP TABLE)
//!                                  ├─ Storage  (per-table BTreeMap rows)
//!                                  ├─ Planner  (sqlparser AST → Plan)
//!                                  ├─ Executor (Plan → ResultSet)
//!                                  └─ TxnMgr   (BEGIN/COMMIT/ROLLBACK)
//! ```

pub mod catalog;
pub mod executor;
pub mod planner;
pub mod storage;
pub mod txn;
pub mod value;

use std::sync::Arc;

use parking_lot::RwLock;

pub use catalog::{Catalog, ColumnDef, GlobalCatalog, PgAuthidRow, PgDatabaseRow, TableSchema};
pub use executor::{ExecutionError, ResultSet};
pub use planner::{Plan, PlanError};
pub use storage::Storage;
pub use txn::{IsolationLevel, TxnError, TxnId, TxnManager, TxnState};
pub use value::{ScalarType, ScalarValue};

/// Shared, thread-safe SQL engine state.
#[derive(Debug)]
pub struct SqlEngine {
    pub catalog: Arc<RwLock<Catalog>>,
    pub storage: Arc<RwLock<Storage>>,
    pub txn_mgr: Arc<RwLock<TxnManager>>,
}

impl SqlEngine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            catalog: Arc::new(RwLock::new(Catalog::new())),
            storage: Arc::new(RwLock::new(Storage::new())),
            txn_mgr: Arc::new(RwLock::new(TxnManager::new())),
        })
    }
}

impl Default for SqlEngine {
    fn default() -> Self {
        Self {
            catalog: Arc::new(RwLock::new(Catalog::new())),
            storage: Arc::new(RwLock::new(Storage::new())),
            txn_mgr: Arc::new(RwLock::new(TxnManager::new())),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EngineError {
    #[error("plan: {0}")]
    Plan(#[from] PlanError),
    #[error("execution: {0}")]
    Execution(#[from] ExecutionError),
    #[error("txn: {0}")]
    Txn(#[from] TxnError),
    #[error("parse: {0}")]
    Parse(String),
}

pub type EngineResult<T> = Result<T, EngineError>;

/// Single entry-point: parse, plan, execute. Returns the result for the first
/// statement only — multi-statement scripts call this in a loop.
pub fn execute_sql(engine: &SqlEngine, sql: &str, txn: Option<TxnId>) -> EngineResult<ResultSet> {
    let dialect = sqlparser::dialect::PostgreSqlDialect {};
    let stmts = sqlparser::parser::Parser::parse_sql(&dialect, sql)
        .map_err(|e| EngineError::Parse(e.to_string()))?;
    let stmt = stmts
        .into_iter()
        .next()
        .ok_or_else(|| EngineError::Parse("empty statement".into()))?;
    let plan = planner::plan_statement(stmt, &engine.catalog.read())?;
    let result = executor::execute(plan, engine, txn)?;
    Ok(result)
}
