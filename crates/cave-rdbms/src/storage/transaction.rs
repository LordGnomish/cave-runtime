//! Transaction management.

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
}

impl Transaction {
    pub fn new() -> Self {
        Transaction {
            state: TransactionState::Idle,
            savepoints: Vec::new(),
        }
    }

    pub fn begin(&mut self) {
        self.state = TransactionState::Active;
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
}
