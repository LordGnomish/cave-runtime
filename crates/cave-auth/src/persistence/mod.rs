// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 model/jpa/src/main/java/org/keycloak/models/jpa/
//
// Port of Keycloak's JPA persistence layer. The existing cave-auth
// modules (`IdentityProviderStore`, `RealmStore`, …) keep state in
// `Arc<RwLock<HashMap<…>>>`; this module adds a real persistence trait
// and two implementations (in-memory + rusqlite) that the upper layers
// can swap in once the orchestrator wires the backend at startup.

//! Persistent storage for cave-auth realms, users, clients, roles,
//! groups, identity providers, and authentication flows.
//!
//! ```ignore
//! use cave_auth::persistence::{PersistenceBackend, RdbmsBackend, RealmEntity};
//!
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let backend = RdbmsBackend::open("/var/lib/cave-auth/state.sqlite")?;
//! let realm = backend.create_realm(RealmEntity::new("master")).await?;
//! assert!(backend.get_realm_by_name("master").await?.is_some());
//! # Ok(())
//! # }
//! ```

pub mod backend;
pub mod entities;
pub mod in_memory_backend;
pub mod migration;
pub mod rdbms_backend;
pub mod txn;

pub use backend::{PersistenceBackend, PersistenceError, Result};
pub use entities::{
    AuditFields, AuthFlowEntity, ClientEntity, FlowExecution, GroupEntity, IdentityProviderEntity,
    IdpMapper, RealmEntity, RoleEntity, UserCredential, UserEntity,
};
pub use in_memory_backend::{InMemoryBackend, InMemoryTxn};
pub use migration::{
    AppliedMigration, Migration, MigrationExecutor, baseline_migrations, fnv1a_64,
};
pub use rdbms_backend::RdbmsBackend;
pub use txn::Transaction;
