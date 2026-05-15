// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: postgres/postgres@c372fbbd8e911f2412b80a8c39d7079366565d67 src/backend/access/transam/xact.c
//! Transaction management — state, savepoints, and SQL-92 isolation levels.

/// SQL-92 isolation level. PostgreSQL accepts `READ UNCOMMITTED` as a
/// synonym of `READ COMMITTED` (default). `SERIALIZABLE` is the
/// strongest level. Mirrors `XactIsoLevel` from
/// `src/backend/access/transam/xact.c` (`enum XactIsoLevel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IsolationLevel {
    /// PG aliases this to `ReadCommitted` (per `XactIsoLevel` mapping).
    ReadUncommitted,
    /// Default per `default_transaction_isolation = "read committed"`.
    #[default]
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

impl IsolationLevel {
    /// Parses an isolation phrase exactly the way PG accepts it in
    /// `SET TRANSACTION ISOLATION LEVEL ...` (see
    /// `src/backend/parser/gram.y` `transaction_mode_item`). Case-insensitive,
    /// whitespace-collapsed. Rejects unknown phrases.
    pub fn parse(phrase: &str) -> Result<Self, String> {
        // Collapse runs of whitespace, uppercase. Mirrors PG's lexer.
        let canon: String = phrase
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_uppercase();
        match canon.as_str() {
            "READ UNCOMMITTED" => Ok(Self::ReadUncommitted),
            "READ COMMITTED" => Ok(Self::ReadCommitted),
            "REPEATABLE READ" => Ok(Self::RepeatableRead),
            "SERIALIZABLE" => Ok(Self::Serializable),
            _ => Err(format!("unrecognised isolation level: {phrase:?}")),
        }
    }

    /// SQL-standard text form (round-trips with `parse`).
    pub fn as_sql(&self) -> &'static str {
        match self {
            Self::ReadUncommitted => "READ UNCOMMITTED",
            Self::ReadCommitted => "READ COMMITTED",
            Self::RepeatableRead => "REPEATABLE READ",
            Self::Serializable => "SERIALIZABLE",
        }
    }

    /// True iff this level forbids non-repeatable reads.
    /// (See PG manual §13.2.) `READ UNCOMMITTED`/`READ COMMITTED`
    /// allow them; `REPEATABLE READ` / `SERIALIZABLE` do not.
    pub fn prevents_non_repeatable_reads(&self) -> bool {
        matches!(self, Self::RepeatableRead | Self::Serializable)
    }

    /// True iff this level forbids phantom rows. Per PG (manual §13.2)
    /// the MVCC implementation gives `REPEATABLE READ` phantom-free
    /// reads too; only the SQL standard requires only `SERIALIZABLE`.
    pub fn prevents_phantom_reads(&self) -> bool {
        matches!(self, Self::RepeatableRead | Self::Serializable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    Idle,
    Active,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Transaction {
    pub state: TransactionState,
    pub savepoints: Vec<String>,
    /// Active isolation level. `BEGIN` resets to the session default
    /// (`ReadCommitted`); `SET TRANSACTION ISOLATION LEVEL <lvl>`
    /// overrides for the current txn only — see
    /// `src/backend/access/transam/xact.c::SetTransactionIsoLevel`.
    pub isolation: IsolationLevel,
    /// Per `READ ONLY` flag (`transaction_read_only` GUC).
    pub read_only: bool,
}

impl Transaction {
    pub fn new() -> Self {
        Transaction {
            state: TransactionState::Idle,
            savepoints: Vec::new(),
            isolation: IsolationLevel::default(),
            read_only: false,
        }
    }

    pub fn begin(&mut self) {
        self.state = TransactionState::Active;
        // BEGIN resets isolation to the session default. Subsequent
        // `SET TRANSACTION` may override.
        self.isolation = IsolationLevel::default();
        self.read_only = false;
    }

    pub fn commit(&mut self) {
        self.state = TransactionState::Idle;
        self.savepoints.clear();
    }

    pub fn rollback(&mut self) {
        self.state = TransactionState::Idle;
        self.savepoints.clear();
    }

    pub fn create_savepoint(&mut self, name: &str) {
        self.savepoints.push(name.to_string());
    }

    pub fn rollback_to_savepoint(&mut self, name: &str) -> bool {
        if let Some(pos) = self.savepoints.iter().position(|s| s == name) {
            self.savepoints.truncate(pos + 1);
            true
        } else {
            false
        }
    }

    /// PG rule: `SET TRANSACTION` only valid mid-txn. See `xact.c`:
    /// > `ERRCODE_ACTIVE_SQL_TRANSACTION` if no txn is in progress.
    pub fn set_isolation(&mut self, level: IsolationLevel) -> Result<(), String> {
        if self.state != TransactionState::Active {
            return Err("SET TRANSACTION can only run inside a transaction block".into());
        }
        // PG additionally forbids changing isolation *after* the first
        // query has been issued in the txn (`XACT_FLAGS_TXN_STARTED`).
        // We approximate that by refusing the change once any savepoint
        // has been created (a proxy for "txn has done work").
        if !self.savepoints.is_empty() {
            return Err(
                "SET TRANSACTION ISOLATION LEVEL must be called before any query"
                    .into(),
            );
        }
        self.isolation = level;
        Ok(())
    }

    pub fn set_read_only(&mut self, read_only: bool) -> Result<(), String> {
        if self.state != TransactionState::Active {
            return Err("SET TRANSACTION can only run inside a transaction block".into());
        }
        self.read_only = read_only;
        Ok(())
    }
}

impl Default for Transaction {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_lifecycle() {
        let mut tx = Transaction::new();
        assert_eq!(tx.state, TransactionState::Idle);
        tx.begin();
        assert_eq!(tx.state, TransactionState::Active);
        tx.commit();
        assert_eq!(tx.state, TransactionState::Idle);
    }

    #[test]
    fn test_transaction_savepoint() {
        let mut tx = Transaction::new();
        tx.begin();
        tx.create_savepoint("sp1");
        assert_eq!(tx.savepoints.len(), 1);
        tx.create_savepoint("sp2");
        assert_eq!(tx.savepoints.len(), 2);
        assert!(tx.rollback_to_savepoint("sp1"));
        assert_eq!(tx.savepoints.len(), 1);
    }

    #[test]
    fn test_transaction_rollback() {
        let mut tx = Transaction::new();
        tx.begin();
        tx.create_savepoint("sp1");
        tx.rollback();
        assert_eq!(tx.state, TransactionState::Idle);
        assert!(tx.savepoints.is_empty());
    }

    // ── Isolation-level tests ───────────────────────────────────────────────

    #[test]
    fn test_isolation_default_is_read_committed() {
        // Matches PG default `default_transaction_isolation = "read committed"`.
        assert_eq!(IsolationLevel::default(), IsolationLevel::ReadCommitted);
        let tx = Transaction::new();
        assert_eq!(tx.isolation, IsolationLevel::ReadCommitted);
    }

    #[test]
    fn test_isolation_parse_all_four_levels() {
        assert_eq!(
            IsolationLevel::parse("read uncommitted").unwrap(),
            IsolationLevel::ReadUncommitted
        );
        assert_eq!(
            IsolationLevel::parse("READ COMMITTED").unwrap(),
            IsolationLevel::ReadCommitted
        );
        assert_eq!(
            IsolationLevel::parse("Repeatable Read").unwrap(),
            IsolationLevel::RepeatableRead
        );
        assert_eq!(
            IsolationLevel::parse("serializable").unwrap(),
            IsolationLevel::Serializable
        );
    }

    #[test]
    fn test_isolation_parse_collapses_whitespace() {
        // PG's lexer treats multiple spaces / tabs as a single token gap.
        assert_eq!(
            IsolationLevel::parse("read   committed").unwrap(),
            IsolationLevel::ReadCommitted
        );
        assert_eq!(
            IsolationLevel::parse("read\tcommitted").unwrap(),
            IsolationLevel::ReadCommitted
        );
    }

    #[test]
    fn test_isolation_parse_rejects_garbage() {
        assert!(IsolationLevel::parse("snapshot").is_err());
        assert!(IsolationLevel::parse("").is_err());
    }

    #[test]
    fn test_isolation_as_sql_roundtrips() {
        for lvl in [
            IsolationLevel::ReadUncommitted,
            IsolationLevel::ReadCommitted,
            IsolationLevel::RepeatableRead,
            IsolationLevel::Serializable,
        ] {
            assert_eq!(IsolationLevel::parse(lvl.as_sql()).unwrap(), lvl);
        }
    }

    #[test]
    fn test_isolation_prevents_anomalies_per_pg_manual() {
        // PG manual §13.2 — in PG, REPEATABLE READ already prevents
        // phantom reads (stricter than SQL standard).
        assert!(!IsolationLevel::ReadUncommitted.prevents_non_repeatable_reads());
        assert!(!IsolationLevel::ReadCommitted.prevents_non_repeatable_reads());
        assert!(IsolationLevel::RepeatableRead.prevents_non_repeatable_reads());
        assert!(IsolationLevel::Serializable.prevents_non_repeatable_reads());

        assert!(!IsolationLevel::ReadCommitted.prevents_phantom_reads());
        assert!(IsolationLevel::RepeatableRead.prevents_phantom_reads());
        assert!(IsolationLevel::Serializable.prevents_phantom_reads());
    }

    #[test]
    fn test_set_isolation_outside_txn_errors() {
        // PG: ERRCODE_ACTIVE_SQL_TRANSACTION when no txn in progress.
        let mut tx = Transaction::new();
        assert!(tx.set_isolation(IsolationLevel::Serializable).is_err());
    }

    #[test]
    fn test_set_isolation_inside_txn_sets_level() {
        let mut tx = Transaction::new();
        tx.begin();
        tx.set_isolation(IsolationLevel::Serializable).unwrap();
        assert_eq!(tx.isolation, IsolationLevel::Serializable);
    }

    #[test]
    fn test_set_isolation_after_work_errors() {
        // PG: must come before any query. We proxy "query issued" via savepoint.
        let mut tx = Transaction::new();
        tx.begin();
        tx.create_savepoint("sp1");
        assert!(tx.set_isolation(IsolationLevel::Serializable).is_err());
    }

    #[test]
    fn test_begin_resets_isolation_to_default() {
        let mut tx = Transaction::new();
        tx.begin();
        tx.set_isolation(IsolationLevel::Serializable).unwrap();
        tx.commit();
        tx.begin();
        // New txn → reset to session default.
        assert_eq!(tx.isolation, IsolationLevel::ReadCommitted);
    }

    #[test]
    fn test_set_read_only_outside_txn_errors() {
        let mut tx = Transaction::new();
        assert!(tx.set_read_only(true).is_err());
    }

    #[test]
    fn test_set_read_only_inside_txn_toggles_flag() {
        let mut tx = Transaction::new();
        tx.begin();
        assert!(!tx.read_only);
        tx.set_read_only(true).unwrap();
        assert!(tx.read_only);
    }
}
