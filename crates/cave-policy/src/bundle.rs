//! Bundle system — package policies into versioned bundles.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// A versioned bundle of policies and data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub id: Uuid,
    pub name: String,
    pub version: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Rego policy modules keyed by module path (e.g. "policies/authz.rego").
    pub modules: HashMap<String, String>,
    /// External data loaded into `data`.
    pub data: serde_json::Value,
    pub roots: Vec<String>,
    pub status: BundleStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BundleStatus {
    Active,
    Pending,
    Error,
}

impl Bundle {
    pub fn new(name: String, version: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            version,
            created_at: now,
            updated_at: now,
            modules: HashMap::new(),
            data: serde_json::Value::Object(Default::default()),
            roots: vec![],
            status: BundleStatus::Pending,
        }
    }

    pub fn add_module(&mut self, path: impl Into<String>, src: impl Into<String>) {
        self.modules.insert(path.into(), src.into());
        self.updated_at = Utc::now();
    }

    pub fn activate(&mut self) {
        self.status = BundleStatus::Active;
        self.updated_at = Utc::now();
    }

    /// Serialize the bundle to JSON bytes (for upload / storage).
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Deserialize a bundle from JSON bytes (for download / loading).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// In-memory bundle store.
#[derive(Debug, Default)]
pub struct BundleStore {
    bundles: HashMap<Uuid, Bundle>,
}

impl BundleStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, bundle: Bundle) -> Uuid {
        let id = bundle.id;
        self.bundles.insert(id, bundle);
        id
    }

    pub fn get(&self, id: &Uuid) -> Option<&Bundle> {
        self.bundles.get(id)
    }

    pub fn get_mut(&mut self, id: &Uuid) -> Option<&mut Bundle> {
        self.bundles.get_mut(id)
    }

    pub fn remove(&mut self, id: &Uuid) -> Option<Bundle> {
        self.bundles.remove(id)
    }

    pub fn list(&self) -> Vec<&Bundle> {
        let mut v: Vec<&Bundle> = self.bundles.values().collect();
        v.sort_by_key(|b| b.created_at);
        v
    }

    pub fn active_bundles(&self) -> Vec<&Bundle> {
        self.list().into_iter()
            .filter(|b| b.status == BundleStatus::Active)
            .collect()
    }
}
