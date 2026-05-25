// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/connections/jpa/JpaConnectionProvider.java
//
// Port of Keycloak's per-request JPA transaction handle. Keycloak wraps
// the EntityManager in a `KeycloakTransaction`; we expose the same
// commit / rollback contract over `Box<dyn Transaction>`.

//! Transaction abstraction shared by every [`PersistenceBackend`].

use async_trait::async_trait;

/// A handle to a single in-flight transaction. Drop without
/// commit/rollback is equivalent to rollback in the in-memory backend
/// and to ROLLBACK in the SQL-backed implementations (mirroring JDBC
/// semantics).
///
/// Closing this trait consumes the box (`self: Box<Self>`) which means
/// callers cannot accidentally double-close.
#[async_trait]
pub trait Transaction: Send + Sync {
    async fn commit(self: Box<Self>) -> Result<(), crate::persistence::backend::PersistenceError>;
    async fn rollback(self: Box<Self>)
    -> Result<(), crate::persistence::backend::PersistenceError>;
    /// Inspection helper for in-memory backend tests; SQL backends may
    /// return `None`.
    fn is_in_memory(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::backend::PersistenceError;
    use std::sync::{Arc, Mutex};

    struct DummyTxn {
        committed: Arc<Mutex<bool>>,
        rolled_back: Arc<Mutex<bool>>,
    }

    #[async_trait]
    impl Transaction for DummyTxn {
        async fn commit(self: Box<Self>) -> Result<(), PersistenceError> {
            *self.committed.lock().unwrap() = true;
            Ok(())
        }
        async fn rollback(self: Box<Self>) -> Result<(), PersistenceError> {
            *self.rolled_back.lock().unwrap() = true;
            Ok(())
        }
        fn is_in_memory(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn dummy_commit_marks_committed() {
        let committed = Arc::new(Mutex::new(false));
        let rolled_back = Arc::new(Mutex::new(false));
        let t: Box<dyn Transaction> = Box::new(DummyTxn {
            committed: committed.clone(),
            rolled_back: rolled_back.clone(),
        });
        t.commit().await.unwrap();
        assert!(*committed.lock().unwrap());
        assert!(!*rolled_back.lock().unwrap());
    }

    #[tokio::test]
    async fn dummy_rollback_marks_rollback() {
        let committed = Arc::new(Mutex::new(false));
        let rolled_back = Arc::new(Mutex::new(false));
        let t: Box<dyn Transaction> = Box::new(DummyTxn {
            committed: committed.clone(),
            rolled_back: rolled_back.clone(),
        });
        t.rollback().await.unwrap();
        assert!(*rolled_back.lock().unwrap());
        assert!(!*committed.lock().unwrap());
    }

    #[tokio::test]
    async fn is_in_memory_default_false() {
        struct Other;
        #[async_trait]
        impl Transaction for Other {
            async fn commit(self: Box<Self>) -> Result<(), PersistenceError> {
                Ok(())
            }
            async fn rollback(self: Box<Self>) -> Result<(), PersistenceError> {
                Ok(())
            }
        }
        let o: Box<dyn Transaction> = Box::new(Other);
        assert!(!o.is_in_memory());
    }
}
