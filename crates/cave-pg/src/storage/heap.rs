//! Heap table storage — tuples with MVCC visibility info, TOAST for large values.
//!
//! Each table is stored as a vector of `Tuple`s. Each tuple has:
//!   - ctid: physical tuple ID (monotonically increasing)
//!   - xmin: creating transaction
//!   - xmax: deleting transaction (0 = live)
//!   - infomask: flags
//!   - data: column values (None = NULL)

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use crate::error::{Error, PgError, Result, SqlState};
use crate::storage::mvcc::{CommitLog, IsolationLevel, Snapshot, Xid, XID_INVALID};
use crate::types::{ColumnDesc, Oid, PgValue};

// ─────────────────────────────────────────────────────────────────────────────
// Column definition
// ─────────────────────────────────────────────────────────────────────────────

/// A column definition in a table.
#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub name: String,
    pub type_oid: Oid,
    pub type_modifier: i32,  // e.g., varchar(255) → 255+4
    pub not_null: bool,
    pub default_expr: Option<String>,
    pub attr_num: i16,
}

impl ColumnDef {
    pub fn new(name: impl Into<String>, type_oid: Oid) -> Self {
        Self {
            name: name.into(),
            type_oid,
            type_modifier: -1,
            not_null: false,
            default_expr: None,
            attr_num: 0,
        }
    }

    pub fn not_null(mut self) -> Self {
        self.not_null = true;
        self
    }

    pub fn with_modifier(mut self, modifier: i32) -> Self {
        self.type_modifier = modifier;
        self
    }

    pub fn with_default(mut self, expr: impl Into<String>) -> Self {
        self.default_expr = Some(expr.into());
        self
    }

    pub fn to_column_desc(&self, table_oid: Oid) -> ColumnDesc {
        let mut cd = ColumnDesc::new(self.name.clone(), self.type_oid);
        cd.table_oid = table_oid;
        cd.col_attr_num = self.attr_num;
        cd.type_modifier = self.type_modifier;
        cd
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Constraint definitions
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Constraint {
    PrimaryKey {
        name: String,
        columns: Vec<String>,
    },
    Unique {
        name: String,
        columns: Vec<String>,
    },
    NotNull {
        name: String,
        column: String,
    },
    Check {
        name: String,
        expr: String,
    },
    ForeignKey {
        name: String,
        columns: Vec<String>,
        ref_table: String,
        ref_columns: Vec<String>,
        on_delete: FkAction,
        on_update: FkAction,
    },
    Exclude {
        name: String,
        expr: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FkAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl Constraint {
    pub fn name(&self) -> &str {
        match self {
            Constraint::PrimaryKey { name, .. } => name,
            Constraint::Unique { name, .. } => name,
            Constraint::NotNull { name, .. } => name,
            Constraint::Check { name, .. } => name,
            Constraint::ForeignKey { name, .. } => name,
            Constraint::Exclude { name, .. } => name,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tuple (row) storage
// ─────────────────────────────────────────────────────────────────────────────

/// Tuple infomask flags.
bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TupleFlags: u16 {
        const XMIN_COMMITTED   = 0x0001;  // xmin was committed
        const XMIN_INVALID     = 0x0002;  // xmin was aborted (this tuple is dead)
        const XMAX_COMMITTED   = 0x0004;  // xmax was committed (tuple was deleted)
        const XMAX_INVALID     = 0x0008;  // xmax was aborted (deletion rolled back)
        const HAS_EXTERNAL     = 0x0010;  // has TOAST attributes
        const IS_UPDATED       = 0x0020;  // this is an updated version of a tuple
        const HEAP_HOT_UPDATED = 0x0040;  // heap-only tuple update (HOT)
        const IS_HOT           = 0x0080;  // this is a HOT tuple
    }
}

/// A heap tuple — one version of a row.
#[derive(Debug, Clone)]
pub struct Tuple {
    /// Physical tuple ID (ctid).
    pub ctid: u64,
    /// Transaction that created this version.
    pub xmin: Xid,
    /// Transaction that deleted this version (0 = live).
    pub xmax: Xid,
    /// Hint bits and flags.
    pub flags: TupleFlags,
    /// Column values, indexed by column position. None = NULL.
    pub data: Vec<Option<PgValue>>,
}

impl Tuple {
    pub fn new(ctid: u64, xmin: Xid, data: Vec<Option<PgValue>>) -> Self {
        Self {
            ctid,
            xmin,
            xmax: XID_INVALID,
            flags: TupleFlags::empty(),
            data,
        }
    }

    pub fn is_live(&self) -> bool {
        self.xmax == XID_INVALID
    }

    /// Is this tuple visible under the given snapshot?
    pub fn is_visible(&self, snapshot: &Snapshot, clog: &CommitLog) -> bool {
        snapshot.tuple_visible(self.xmin, self.xmax, clog)
    }

    /// Mark this tuple as deleted by a transaction.
    pub fn delete(&mut self, xmax: Xid) {
        self.xmax = xmax;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Table
// ─────────────────────────────────────────────────────────────────────────────

/// A heap table.
#[derive(Debug)]
pub struct Table {
    pub name: String,
    pub schema: String,
    pub oid: Oid,
    pub columns: Vec<ColumnDef>,
    pub constraints: Vec<Constraint>,
    pub tuples: Vec<Tuple>,
    pub next_ctid: AtomicU64,
    /// Row-level write locks: ctid → locking xid.
    pub row_locks: HashMap<u64, Xid>,
    /// Estimated statistics for the planner.
    pub reltuples: f64,
    pub relpages: u32,
    /// Whether this is a system table.
    pub is_system: bool,
    /// If this is a materialized view, the defining query.
    pub matview_query: Option<String>,
}

impl Table {
    pub fn new(
        name: impl Into<String>,
        schema: impl Into<String>,
        oid: Oid,
        columns: Vec<ColumnDef>,
    ) -> Self {
        // Set attr_num on columns (1-based)
        let columns = columns
            .into_iter()
            .enumerate()
            .map(|(i, mut col)| {
                col.attr_num = (i + 1) as i16;
                col
            })
            .collect();

        Self {
            name: name.into(),
            schema: schema.into(),
            oid,
            columns,
            constraints: Vec::new(),
            tuples: Vec::new(),
            next_ctid: AtomicU64::new(1),
            row_locks: HashMap::new(),
            reltuples: 0.0,
            relpages: 1,
            is_system: false,
            matview_query: None,
        }
    }

    pub fn column_idx(&self, name: &str) -> Option<usize> {
        let name_lower = name.to_lowercase();
        self.columns.iter().position(|c| c.name.to_lowercase() == name_lower)
    }

    pub fn column_by_name(&self, name: &str) -> Option<&ColumnDef> {
        let name_lower = name.to_lowercase();
        self.columns.iter().find(|c| c.name.to_lowercase() == name_lower)
    }

    /// Qualified name: schema.table
    pub fn qualified_name(&self) -> String {
        format!("{}.{}", self.schema, self.name)
    }

    /// Column descriptors for RowDescription messages.
    pub fn column_descs(&self) -> Vec<ColumnDesc> {
        self.columns.iter().map(|c| c.to_column_desc(self.oid)).collect()
    }

    /// Insert a new tuple. Returns the ctid.
    pub fn insert(&mut self, xmin: Xid, values: Vec<Option<PgValue>>) -> Result<u64> {
        let ctid = self.next_ctid.fetch_add(1, Ordering::SeqCst);
        self.tuples.push(Tuple::new(ctid, xmin, values));
        self.reltuples += 1.0;
        Ok(ctid)
    }

    /// Scan all visible tuples.
    pub fn scan<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        clog: &'a CommitLog,
    ) -> impl Iterator<Item = &'a Tuple> {
        self.scan_raw()
            .filter(move |t| t.is_visible(snapshot, clog))
    }

    /// Scan all tuples (including dead ones).
    pub fn scan_raw(&self) -> impl Iterator<Item = &Tuple> {
        self.tuples.iter()
    }

    /// Get a tuple by ctid (visible check not applied).
    pub fn get_by_ctid(&self, ctid: u64) -> Option<&Tuple> {
        self.tuples.iter().find(|t| t.ctid == ctid)
    }

    /// Get a mutable tuple by ctid.
    pub fn get_by_ctid_mut(&mut self, ctid: u64) -> Option<&mut Tuple> {
        self.tuples.iter_mut().find(|t| t.ctid == ctid)
    }

    /// Logical delete — mark tuple as deleted.
    pub fn delete_tuple(&mut self, ctid: u64, xid: Xid) -> Result<()> {
        let tuple = self.tuples.iter_mut().find(|t| t.ctid == ctid)
            .ok_or_else(|| Error::Protocol(format!("ctid {ctid} not found")))?;
        if tuple.xmax != XID_INVALID {
            return Err(Error::Pg(PgError::serialization_failure()));
        }
        tuple.delete(xid);
        self.reltuples -= 1.0;
        Ok(())
    }

    /// Update = delete old tuple + insert new one. Returns new ctid.
    pub fn update_tuple(
        &mut self,
        old_ctid: u64,
        xid: Xid,
        new_values: Vec<Option<PgValue>>,
    ) -> Result<u64> {
        self.delete_tuple(old_ctid, xid)?;
        let new_ctid = self.insert(xid, new_values)?;
        Ok(new_ctid)
    }

    /// Truncate — delete all tuples (as a DDL operation, no MVCC).
    pub fn truncate(&mut self) {
        self.tuples.clear();
        self.reltuples = 0.0;
    }

    /// VACUUM — physically remove dead tuples.
    pub fn vacuum(&mut self, clog: &CommitLog) {
        self.tuples.retain(|t| {
            // Keep if: not yet deleted, or delete was aborted
            t.xmax == XID_INVALID || clog.is_aborted(t.xmax)
        });
        self.reltuples = self.tuples.len() as f64;
    }

    /// Validate NOT NULL constraints for a row.
    pub fn check_not_null(&self, values: &[Option<PgValue>]) -> Result<()> {
        for (i, col) in self.columns.iter().enumerate() {
            if col.not_null {
                let val = values.get(i).and_then(|v| v.as_ref());
                if val.is_none() || val == Some(&PgValue::Null) {
                    return Err(Error::Pg(PgError::not_null_violation(&self.name, &col.name)));
                }
            }
        }
        // Also check explicit NOT NULL constraints
        for constraint in &self.constraints {
            if let Constraint::NotNull { column, .. } = constraint {
                if let Some(idx) = self.column_idx(column) {
                    let val = values.get(idx).and_then(|v| v.as_ref());
                    if val.is_none() || val == Some(&PgValue::Null) {
                        return Err(Error::Pg(PgError::not_null_violation(&self.name, column)));
                    }
                }
            }
        }
        Ok(())
    }

    /// Add a constraint to the table.
    pub fn add_constraint(&mut self, constraint: Constraint) {
        self.constraints.push(constraint);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Sequence
// ─────────────────────────────────────────────────────────────────────────────

/// A PostgreSQL sequence.
#[derive(Debug)]
pub struct Sequence {
    pub name: String,
    pub schema: String,
    pub oid: Oid,
    pub current: std::sync::atomic::AtomicI64,
    pub start: i64,
    pub increment: i64,
    pub min_value: i64,
    pub max_value: i64,
    pub cycle: bool,
    pub cache: i64,
}

impl Sequence {
    pub fn new(name: impl Into<String>, schema: impl Into<String>, oid: Oid) -> Self {
        Self {
            name: name.into(),
            schema: schema.into(),
            oid,
            current: std::sync::atomic::AtomicI64::new(0),
            start: 1,
            increment: 1,
            min_value: 1,
            max_value: i64::MAX,
            cycle: false,
            cache: 1,
        }
    }

    pub fn with_range(mut self, start: i64, min: i64, max: i64) -> Self {
        self.start = start;
        self.min_value = min;
        self.max_value = max;
        self.current.store(start - 1, Ordering::SeqCst);
        self
    }

    pub fn with_increment(mut self, increment: i64) -> Self {
        self.increment = increment;
        self
    }

    pub fn nextval(&self) -> Result<i64> {
        let next = self.current.fetch_add(self.increment, Ordering::SeqCst) + self.increment;
        if next > self.max_value {
            if self.cycle {
                self.current.store(self.min_value, Ordering::SeqCst);
                Ok(self.min_value)
            } else {
                Err(Error::Pg(PgError::error(
                    SqlState::SEQUENCE_GENERATOR_LIMIT_EXCEEDED,
                    format!(
                        "nextval: reached maximum value of sequence \"{}\" ({})",
                        self.name, self.max_value
                    ),
                )))
            }
        } else {
            Ok(next)
        }
    }

    pub fn currval(&self) -> i64 {
        self.current.load(Ordering::SeqCst)
    }

    pub fn setval(&self, val: i64) -> i64 {
        self.current.store(val, Ordering::SeqCst);
        val
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Index
// ─────────────────────────────────────────────────────────────────────────────

/// Index access method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexMethod {
    BTree,
    Hash,
    Gin,
    Gist,
    Brin,
    SpGist,
}

/// An index on a table.
#[derive(Debug)]
pub struct Index {
    pub name: String,
    pub table_schema: String,
    pub table_name: String,
    pub oid: Oid,
    pub method: IndexMethod,
    pub key_columns: Vec<String>,
    pub key_exprs: Vec<Option<String>>,  // for expression indexes
    pub is_unique: bool,
    pub is_primary: bool,
    /// Partial index predicate (WHERE clause), if any.
    pub predicate: Option<String>,
    /// BTree data: key_bytes → Vec<ctid>
    pub btree: std::collections::BTreeMap<IndexKey, Vec<u64>>,
}

/// A serializable index key for BTree ordering.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IndexKey(Vec<IndexKeyPart>);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum IndexKeyPart {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
    Bytes(Vec<u8>),
}

impl IndexKey {
    pub fn from_values(values: &[&PgValue]) -> Self {
        let parts = values.iter().map(|v| match v {
            PgValue::Null => IndexKeyPart::Null,
            PgValue::Bool(b) => IndexKeyPart::Bool(*b),
            PgValue::Int2(v) => IndexKeyPart::Int(*v as i64),
            PgValue::Int4(v) => IndexKeyPart::Int(*v as i64),
            PgValue::Int8(v) => IndexKeyPart::Int(*v),
            PgValue::Float4(v) => IndexKeyPart::Int((*v as f64 * 1e9) as i64),
            PgValue::Float8(v) => IndexKeyPart::Int((*v * 1e9) as i64),
            PgValue::Text(s) | PgValue::Varchar(s) | PgValue::Char(s) => IndexKeyPart::Text(s.clone()),
            PgValue::Uuid(u) => IndexKeyPart::Text(u.to_string()),
            PgValue::Date(d) => IndexKeyPart::Int(d.signed_duration_since(chrono::NaiveDate::from_ymd_opt(1, 1, 1).unwrap()).num_days()),
            PgValue::Timestamp(ts) => IndexKeyPart::Int(ts.and_utc().timestamp_micros()),
            PgValue::TimestampTz(ts) => IndexKeyPart::Int(ts.timestamp_micros()),
            v => IndexKeyPart::Text(v.to_text()),
        }).collect();
        Self(parts)
    }
}

impl Index {
    pub fn new(
        name: impl Into<String>,
        table_schema: impl Into<String>,
        table_name: impl Into<String>,
        oid: Oid,
        method: IndexMethod,
        key_columns: Vec<String>,
        is_unique: bool,
        is_primary: bool,
    ) -> Self {
        Self {
            name: name.into(),
            table_schema: table_schema.into(),
            table_name: table_name.into(),
            oid,
            method,
            key_columns,
            key_exprs: Vec::new(),
            is_unique,
            is_primary,
            predicate: None,
            btree: std::collections::BTreeMap::new(),
        }
    }

    /// Insert a tuple into the index.
    pub fn insert(&mut self, key: IndexKey, ctid: u64) -> Result<()> {
        if self.is_unique {
            if let Some(existing) = self.btree.get(&key) {
                if !existing.is_empty() {
                    let col_list = self.key_columns.join(", ");
                    return Err(Error::Pg(PgError::unique_violation(
                        &self.table_name,
                        &self.name,
                        format!(
                            "Key ({col_list}) = ({}) already exists.",
                            key.0.iter().map(|p| match p {
                                IndexKeyPart::Text(s) => s.clone(),
                                IndexKeyPart::Int(i) => i.to_string(),
                                IndexKeyPart::Bool(b) => b.to_string(),
                                IndexKeyPart::Null => "NULL".to_string(),
                                IndexKeyPart::Bytes(_) => "<bytes>".to_string(),
                            }).collect::<Vec<_>>().join(", ")
                        ),
                    )));
                }
            }
        }
        self.btree.entry(key).or_default().push(ctid);
        Ok(())
    }

    /// Remove a ctid from the index.
    pub fn delete(&mut self, key: &IndexKey, ctid: u64) {
        if let Some(ctids) = self.btree.get_mut(key) {
            ctids.retain(|&c| c != ctid);
            if ctids.is_empty() {
                self.btree.remove(key);
            }
        }
    }

    /// Point lookup.
    pub fn lookup(&self, key: &IndexKey) -> Vec<u64> {
        self.btree.get(key).cloned().unwrap_or_default()
    }

    /// Range scan.
    pub fn range_scan(
        &self,
        lower: Option<&IndexKey>,
        upper: Option<&IndexKey>,
    ) -> Vec<u64> {
        use std::ops::Bound::*;
        let lower_bound = match lower {
            None => Unbounded,
            Some(k) => Included(k),
        };
        let upper_bound = match upper {
            None => Unbounded,
            Some(k) => Included(k),
        };
        self.btree
            .range((lower_bound, upper_bound))
            .flat_map(|(_, ctids)| ctids.iter().copied())
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// View
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct View {
    pub name: String,
    pub schema: String,
    pub oid: Oid,
    pub query: String,
    pub columns: Vec<ColumnDef>,
    pub is_materialized: bool,
    /// Cached result for materialized views.
    pub cached_result: Option<Vec<Vec<Option<PgValue>>>>,
}

impl View {
    pub fn new(
        name: impl Into<String>,
        schema: impl Into<String>,
        oid: Oid,
        query: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            schema: schema.into(),
            oid,
            query: query.into(),
            columns: Vec::new(),
            is_materialized: false,
            cached_result: None,
        }
    }

    pub fn materialized(mut self) -> Self {
        self.is_materialized = true;
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Schema
// ─────────────────────────────────────────────────────────────────────────────

/// A schema containing tables, indexes, sequences, and views.
#[derive(Debug)]
pub struct Schema {
    pub name: String,
    pub oid: Oid,
    pub owner: String,
    pub tables: HashMap<String, Arc<RwLock<Table>>>,
    pub indexes: HashMap<String, Arc<RwLock<Index>>>,
    pub sequences: HashMap<String, Arc<Sequence>>,
    pub views: HashMap<String, Arc<RwLock<View>>>,
    /// Enum types defined in this schema.
    pub enum_types: HashMap<String, Vec<String>>,  // type_name → labels
    /// Composite types.
    pub composite_types: HashMap<String, Vec<ColumnDef>>,
}

impl Schema {
    pub fn new(name: impl Into<String>, oid: Oid, owner: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            oid,
            owner: owner.into(),
            tables: HashMap::new(),
            indexes: HashMap::new(),
            sequences: HashMap::new(),
            views: HashMap::new(),
            enum_types: HashMap::new(),
            composite_types: HashMap::new(),
        }
    }

    pub fn table(&self, name: &str) -> Option<Arc<RwLock<Table>>> {
        let name_lower = name.to_lowercase();
        self.tables.get(&name_lower).cloned()
    }

    pub fn index(&self, name: &str) -> Option<Arc<RwLock<Index>>> {
        let name_lower = name.to_lowercase();
        self.indexes.get(&name_lower).cloned()
    }

    pub fn sequence(&self, name: &str) -> Option<Arc<Sequence>> {
        let name_lower = name.to_lowercase();
        self.sequences.get(&name_lower).cloned()
    }

    pub fn view(&self, name: &str) -> Option<Arc<RwLock<View>>> {
        let name_lower = name.to_lowercase();
        self.views.get(&name_lower).cloned()
    }

    pub fn add_table(&mut self, table: Table) {
        self.tables.insert(table.name.to_lowercase(), Arc::new(RwLock::new(table)));
    }

    pub fn add_index(&mut self, index: Index) {
        self.indexes.insert(index.name.to_lowercase(), Arc::new(RwLock::new(index)));
    }

    pub fn add_sequence(&mut self, seq: Sequence) {
        self.sequences.insert(seq.name.to_lowercase(), Arc::new(seq));
    }

    pub fn add_view(&mut self, view: View) {
        self.views.insert(view.name.to_lowercase(), Arc::new(RwLock::new(view)));
    }

    pub fn drop_table(&mut self, name: &str) -> bool {
        self.tables.remove(&name.to_lowercase()).is_some()
    }

    pub fn drop_index(&mut self, name: &str) -> bool {
        self.indexes.remove(&name.to_lowercase()).is_some()
    }

    pub fn drop_view(&mut self, name: &str) -> bool {
        self.views.remove(&name.to_lowercase()).is_some()
    }

    pub fn drop_sequence(&mut self, name: &str) -> bool {
        self.sequences.remove(&name.to_lowercase()).is_some()
    }

    /// Find all indexes for a given table.
    pub fn indexes_for_table(&self, table_name: &str) -> Vec<Arc<RwLock<Index>>> {
        let table_lower = table_name.to_lowercase();
        self.indexes
            .values()
            .filter(|idx| idx.read().table_name.to_lowercase() == table_lower)
            .cloned()
            .collect()
    }

    /// Table names in this schema.
    pub fn table_names(&self) -> Vec<String> {
        self.tables.keys().cloned().collect()
    }
}
