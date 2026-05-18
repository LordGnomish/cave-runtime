// SPDX-License-Identifier: AGPL-3.0-or-later
//! In-memory store for NodePools / NodeClaims / NodeClasses.
//!
//! Placeholder backing for the Karpenter scaffold. Persistence layer
//! (cave-rdbms-operator / cave-etcd) will be wired once the scheduler reaches
//! parity with upstream NodePool reconcile.

use std::collections::HashMap;
use std::sync::RwLock;

use crate::models::{NodeClaim, NodeClass, NodePool};

#[derive(Default)]
pub struct Store {
    pools: RwLock<HashMap<String, NodePool>>,
    claims: RwLock<HashMap<String, NodeClaim>>,
    classes: RwLock<HashMap<String, NodeClass>>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put_pool(&self, pool: NodePool) {
        self.pools.write().unwrap().insert(pool.name.clone(), pool);
    }

    pub fn get_pool(&self, name: &str) -> Option<NodePool> {
        self.pools.read().unwrap().get(name).cloned()
    }

    pub fn list_pools(&self) -> Vec<NodePool> {
        self.pools.read().unwrap().values().cloned().collect()
    }

    pub fn delete_pool(&self, name: &str) -> bool {
        self.pools.write().unwrap().remove(name).is_some()
    }

    pub fn put_claim(&self, claim: NodeClaim) {
        self.claims.write().unwrap().insert(claim.name.clone(), claim);
    }

    pub fn list_claims(&self) -> Vec<NodeClaim> {
        self.claims.read().unwrap().values().cloned().collect()
    }

    pub fn put_class(&self, class: NodeClass) {
        let key = format!("{}/{}", class.kind, class.name);
        self.classes.write().unwrap().insert(key, class);
    }

    pub fn list_classes(&self) -> Vec<NodeClass> {
        self.classes.read().unwrap().values().cloned().collect()
    }
}
