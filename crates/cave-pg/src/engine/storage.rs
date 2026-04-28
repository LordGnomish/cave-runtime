//! Per-table row storage with explicit row-id (= primary key surrogate).

use std::collections::BTreeMap;

use super::value::ScalarValue;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum StorageError {
    #[error("table not in storage: {0}")]
    TableNotFound(String),
    #[error("row not found: {0}")]
    RowNotFound(u64),
    #[error("duplicate row id: {0}")]
    DuplicateRowId(u64),
    #[error("column count mismatch: expected {expected}, got {got}")]
    ColumnCountMismatch { expected: usize, got: usize },
}

pub type StorageResult<T> = Result<T, StorageError>;

#[derive(Debug, Clone)]
pub struct Row {
    pub values: Vec<ScalarValue>,
    /// Transaction id that inserted this row.
    pub xmin: u64,
    /// Transaction id that deleted this row (0 = visible).
    pub xmax: u64,
}

#[derive(Debug, Default)]
pub struct TableData {
    rows: BTreeMap<u64, Row>,
    next_id: u64,
    column_count: usize,
}

#[derive(Debug, Default)]
pub struct Storage {
    tables: BTreeMap<String, TableData>,
}

impl Storage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_table(&mut self, qualified_name: &str, column_count: usize) {
        self.tables.insert(
            qualified_name.into(),
            TableData {
                rows: BTreeMap::new(),
                next_id: 1,
                column_count,
            },
        );
    }

    pub fn drop_table(&mut self, qualified_name: &str) -> StorageResult<()> {
        self.tables
            .remove(qualified_name)
            .ok_or_else(|| StorageError::TableNotFound(qualified_name.into()))?;
        Ok(())
    }

    pub fn insert(
        &mut self,
        qualified_name: &str,
        values: Vec<ScalarValue>,
        xmin: u64,
    ) -> StorageResult<u64> {
        let table = self
            .tables
            .get_mut(qualified_name)
            .ok_or_else(|| StorageError::TableNotFound(qualified_name.into()))?;
        if values.len() != table.column_count {
            return Err(StorageError::ColumnCountMismatch {
                expected: table.column_count,
                got: values.len(),
            });
        }
        let id = table.next_id;
        table.next_id += 1;
        table.rows.insert(
            id,
            Row {
                values,
                xmin,
                xmax: 0,
            },
        );
        Ok(id)
    }

    pub fn delete(
        &mut self,
        qualified_name: &str,
        row_id: u64,
        xmax: u64,
    ) -> StorageResult<()> {
        let table = self
            .tables
            .get_mut(qualified_name)
            .ok_or_else(|| StorageError::TableNotFound(qualified_name.into()))?;
        let row = table
            .rows
            .get_mut(&row_id)
            .ok_or(StorageError::RowNotFound(row_id))?;
        row.xmax = xmax;
        Ok(())
    }

    pub fn purge_deleted(&mut self, qualified_name: &str) -> StorageResult<usize> {
        let table = self
            .tables
            .get_mut(qualified_name)
            .ok_or_else(|| StorageError::TableNotFound(qualified_name.into()))?;
        let before = table.rows.len();
        table.rows.retain(|_, r| r.xmax == 0);
        Ok(before - table.rows.len())
    }

    pub fn update(
        &mut self,
        qualified_name: &str,
        row_id: u64,
        new_values: Vec<ScalarValue>,
    ) -> StorageResult<()> {
        let table = self
            .tables
            .get_mut(qualified_name)
            .ok_or_else(|| StorageError::TableNotFound(qualified_name.into()))?;
        let row = table
            .rows
            .get_mut(&row_id)
            .ok_or(StorageError::RowNotFound(row_id))?;
        if new_values.len() != table.column_count {
            return Err(StorageError::ColumnCountMismatch {
                expected: table.column_count,
                got: new_values.len(),
            });
        }
        row.values = new_values;
        Ok(())
    }

    pub fn scan(&self, qualified_name: &str) -> StorageResult<Vec<(u64, Row)>> {
        let table = self
            .tables
            .get(qualified_name)
            .ok_or_else(|| StorageError::TableNotFound(qualified_name.into()))?;
        Ok(table
            .rows
            .iter()
            .filter(|(_, r)| r.xmax == 0)
            .map(|(id, r)| (*id, r.clone()))
            .collect())
    }

    pub fn row_count(&self, qualified_name: &str) -> StorageResult<usize> {
        let table = self
            .tables
            .get(qualified_name)
            .ok_or_else(|| StorageError::TableNotFound(qualified_name.into()))?;
        Ok(table.rows.values().filter(|r| r.xmax == 0).count())
    }

    pub fn get_row(&self, qualified_name: &str, row_id: u64) -> StorageResult<Row> {
        let table = self
            .tables
            .get(qualified_name)
            .ok_or_else(|| StorageError::TableNotFound(qualified_name.into()))?;
        let row = table
            .rows
            .get(&row_id)
            .ok_or(StorageError::RowNotFound(row_id))?;
        if row.xmax != 0 {
            return Err(StorageError::RowNotFound(row_id));
        }
        Ok(row.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_insert() {
        let mut s = Storage::new();
        s.create_table("t", 2);
        let id = s
            .insert("t", vec![ScalarValue::Int4(1), ScalarValue::Text("a".into())], 1)
            .unwrap();
        assert_eq!(id, 1);
        assert_eq!(s.row_count("t").unwrap(), 1);
    }

    #[test]
    fn insert_with_wrong_column_count_errors() {
        let mut s = Storage::new();
        s.create_table("t", 2);
        let err = s
            .insert("t", vec![ScalarValue::Int4(1)], 1)
            .unwrap_err();
        assert_eq!(
            err,
            StorageError::ColumnCountMismatch {
                expected: 2,
                got: 1
            }
        );
    }

    #[test]
    fn insert_unknown_table_errors() {
        let mut s = Storage::new();
        let err = s.insert("nope", vec![ScalarValue::Null], 1).unwrap_err();
        assert!(matches!(err, StorageError::TableNotFound(_)));
    }

    #[test]
    fn delete_marks_xmax() {
        let mut s = Storage::new();
        s.create_table("t", 1);
        let id = s.insert("t", vec![ScalarValue::Int4(1)], 1).unwrap();
        s.delete("t", id, 2).unwrap();
        assert_eq!(s.row_count("t").unwrap(), 0);
    }

    #[test]
    fn delete_unknown_row_errors() {
        let mut s = Storage::new();
        s.create_table("t", 1);
        let err = s.delete("t", 99, 2).unwrap_err();
        assert_eq!(err, StorageError::RowNotFound(99));
    }

    #[test]
    fn update_replaces_values() {
        let mut s = Storage::new();
        s.create_table("t", 2);
        let id = s
            .insert("t", vec![ScalarValue::Int4(1), ScalarValue::Text("a".into())], 1)
            .unwrap();
        s.update(
            "t",
            id,
            vec![ScalarValue::Int4(99), ScalarValue::Text("z".into())],
        )
        .unwrap();
        let row = s.get_row("t", id).unwrap();
        assert_eq!(row.values[0], ScalarValue::Int4(99));
    }

    #[test]
    fn scan_excludes_deleted() {
        let mut s = Storage::new();
        s.create_table("t", 1);
        let id1 = s.insert("t", vec![ScalarValue::Int4(1)], 1).unwrap();
        let _id2 = s.insert("t", vec![ScalarValue::Int4(2)], 1).unwrap();
        s.delete("t", id1, 2).unwrap();
        let rows = s.scan("t").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1.values[0], ScalarValue::Int4(2));
    }

    #[test]
    fn purge_deleted_returns_count() {
        let mut s = Storage::new();
        s.create_table("t", 1);
        let id = s.insert("t", vec![ScalarValue::Int4(1)], 1).unwrap();
        s.delete("t", id, 2).unwrap();
        let n = s.purge_deleted("t").unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn drop_table_removes_storage() {
        let mut s = Storage::new();
        s.create_table("t", 1);
        s.drop_table("t").unwrap();
        let err = s.scan("t").unwrap_err();
        assert!(matches!(err, StorageError::TableNotFound(_)));
    }

    #[test]
    fn ids_are_monotonic_per_table() {
        let mut s = Storage::new();
        s.create_table("t", 1);
        let id1 = s.insert("t", vec![ScalarValue::Int4(1)], 1).unwrap();
        let id2 = s.insert("t", vec![ScalarValue::Int4(2)], 1).unwrap();
        assert!(id2 > id1);
    }
}
