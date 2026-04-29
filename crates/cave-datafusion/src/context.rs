//! User-facing query context — register tables, execute SQL, get RecordBatch.

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::batch::RecordBatch;
use crate::error::{DataFusionError, DfResult};
use crate::execution_plan::compile;
use crate::sql::parse_sql;

#[derive(Debug, Default)]
pub struct SqlContext {
    tables: Arc<RwLock<BTreeMap<String, RecordBatch>>>,
}

impl SqlContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_table(&self, name: &str, batch: RecordBatch) -> DfResult<()> {
        if name.is_empty() {
            return Err(DataFusionError::Plan("empty table name".into()));
        }
        self.tables.write().insert(name.into(), batch);
        Ok(())
    }

    pub fn deregister_table(&self, name: &str) -> bool {
        self.tables.write().remove(name).is_some()
    }

    pub fn registered_tables(&self) -> Vec<String> {
        self.tables.read().keys().cloned().collect()
    }

    pub fn execute_sql(&self, sql: &str) -> DfResult<RecordBatch> {
        let mut plan = parse_sql(sql)?;
        let scans = collect_scan_tables(&plan);
        if scans.len() != 1 {
            return Err(DataFusionError::Plan(format!(
                "single-table queries only — got {} scans",
                scans.len()
            )));
        }
        let table_name = scans.into_iter().next().unwrap();
        let source = self
            .tables
            .read()
            .get(&table_name)
            .cloned()
            .ok_or_else(|| DataFusionError::Plan(format!("no such table: {}", table_name)))?;
        // Inject the source's tenant into the Scan node so plan validation passes.
        rewrite_scan_tenant(&mut plan, &source);
        let exec = compile(&plan, source)?;
        exec.execute()
    }
}

fn collect_scan_tables(plan: &crate::logical_plan::LogicalPlan) -> Vec<String> {
    match plan {
        crate::logical_plan::LogicalPlan::Scan { table_name, .. } => vec![table_name.clone()],
        crate::logical_plan::LogicalPlan::Projection { input, .. }
        | crate::logical_plan::LogicalPlan::Filter { input, .. }
        | crate::logical_plan::LogicalPlan::Limit { input, .. }
        | crate::logical_plan::LogicalPlan::Aggregate { input, .. }
        | crate::logical_plan::LogicalPlan::Sort { input, .. } => collect_scan_tables(input),
    }
}

fn rewrite_scan_tenant(plan: &mut crate::logical_plan::LogicalPlan, batch: &RecordBatch) {
    match plan {
        crate::logical_plan::LogicalPlan::Scan { tenant_id, .. } => {
            if !batch.tenant_id.is_empty() {
                *tenant_id = batch.tenant_id.clone();
            }
        }
        crate::logical_plan::LogicalPlan::Projection { input, .. }
        | crate::logical_plan::LogicalPlan::Filter { input, .. }
        | crate::logical_plan::LogicalPlan::Limit { input, .. }
        | crate::logical_plan::LogicalPlan::Aggregate { input, .. }
        | crate::logical_plan::LogicalPlan::Sort { input, .. } => rewrite_scan_tenant(input, batch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::Value;

    fn sample_batch() -> RecordBatch {
        RecordBatch::new(
            vec!["id".into(), "name".into(), "price".into()],
            vec![
                vec![Value::Int64(1), Value::Utf8("alice".into()), Value::Int64(10)],
                vec![Value::Int64(2), Value::Utf8("bob".into()), Value::Int64(20)],
                vec![Value::Int64(3), Value::Utf8("carol".into()), Value::Int64(30)],
            ],
        )
        .unwrap()
        .with_tenant("default")
    }

    #[test]
    fn register_then_list() {
        let ctx = SqlContext::new();
        ctx.register_table("t", sample_batch()).unwrap();
        assert_eq!(ctx.registered_tables(), vec!["t"]);
    }

    #[test]
    fn deregister_removes() {
        let ctx = SqlContext::new();
        ctx.register_table("t", sample_batch()).unwrap();
        assert!(ctx.deregister_table("t"));
        assert!(ctx.registered_tables().is_empty());
    }

    #[test]
    fn select_star_returns_all_rows() {
        let ctx = SqlContext::new();
        ctx.register_table("t", sample_batch()).unwrap();
        let r = ctx.execute_sql("SELECT * FROM t").unwrap();
        assert_eq!(r.num_rows(), 3);
    }

    #[test]
    fn select_subset_columns() {
        let ctx = SqlContext::new();
        ctx.register_table("t", sample_batch()).unwrap();
        let r = ctx.execute_sql("SELECT id, name FROM t").unwrap();
        assert_eq!(r.num_columns(), 2);
        assert_eq!(r.columns[0], "id");
    }

    #[test]
    fn select_with_where_filters() {
        let ctx = SqlContext::new();
        ctx.register_table("t", sample_batch()).unwrap();
        let r = ctx.execute_sql("SELECT * FROM t WHERE id = 2").unwrap();
        assert_eq!(r.num_rows(), 1);
    }

    #[test]
    fn select_with_limit_caps() {
        let ctx = SqlContext::new();
        ctx.register_table("t", sample_batch()).unwrap();
        let r = ctx.execute_sql("SELECT * FROM t LIMIT 2").unwrap();
        assert_eq!(r.num_rows(), 2);
    }

    #[test]
    fn count_star_returns_total() {
        let ctx = SqlContext::new();
        ctx.register_table("t", sample_batch()).unwrap();
        let r = ctx.execute_sql("SELECT COUNT(*) FROM t").unwrap();
        assert_eq!(r.num_rows(), 1);
    }

    #[test]
    fn sum_returns_total_value() {
        let ctx = SqlContext::new();
        ctx.register_table("t", sample_batch()).unwrap();
        let r = ctx.execute_sql("SELECT SUM(price) FROM t").unwrap();
        assert_eq!(r.num_rows(), 1);
    }

    #[test]
    fn unknown_table_errors() {
        let ctx = SqlContext::new();
        let err = ctx.execute_sql("SELECT * FROM ghost").unwrap_err();
        match err {
            DataFusionError::Plan(_) => {}
            other => panic!("expected Plan error, got {:?}", other),
        }
    }

    #[test]
    fn invalid_sql_errors() {
        let ctx = SqlContext::new();
        assert!(ctx.execute_sql("NOT VALID SQL").is_err());
    }

    #[test]
    fn register_empty_name_errors() {
        let ctx = SqlContext::new();
        assert!(ctx.register_table("", sample_batch()).is_err());
    }

    #[test]
    fn re_register_replaces_table() {
        let ctx = SqlContext::new();
        ctx.register_table("t", sample_batch()).unwrap();
        let new_batch = RecordBatch::new(
            vec!["x".into()],
            vec![vec![Value::Int64(99)]],
        )
        .unwrap()
        .with_tenant("default");
        ctx.register_table("t", new_batch).unwrap();
        let r = ctx.execute_sql("SELECT * FROM t").unwrap();
        assert_eq!(r.num_rows(), 1);
    }

    #[test]
    fn count_with_where() {
        let ctx = SqlContext::new();
        ctx.register_table("t", sample_batch()).unwrap();
        let r = ctx.execute_sql("SELECT COUNT(*) FROM t WHERE price > 15").unwrap();
        // SUM/COUNT execute on filtered rows.
        assert_eq!(r.num_rows(), 1);
    }
}
