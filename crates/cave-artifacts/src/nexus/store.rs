// SPDX-License-Identifier: AGPL-3.0-or-later
//! In-memory storage backend for the Nexus module.
//!
//! All state is held behind RwLocks so handlers can safely fan out across
//! tokio tasks. Blob bytes are content-addressed by SHA-256 and dedupe
//! across assets — uploading the same payload twice references one blob.
//!
//! Persistent backends (PostgreSQL for metadata, object storage for blobs)
//! land later through the same trait surface; nothing in `routes.rs` should
//! depend on the in-memory shape directly.

use super::error::NexusError;
use super::models::{
    Asset, BlobRef, CleanupPolicy, Component, Repository, RepositoryType, RoutingRule,
};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct NexusStore {
    inner: RwLock<Inner>,
}

#[derive(Default)]
struct Inner {
    repositories: HashMap<String, Repository>,
    components: HashMap<Uuid, Component>,
    assets: HashMap<Uuid, Asset>,
    /// Index: repository_name -> path -> asset_id (raw lookup hot path).
    asset_paths: HashMap<String, HashMap<String, Uuid>>,
    blobs: HashMap<String, Vec<u8>>,
    /// Refcount for each blob; assets share blobs by sha256.
    blob_refs: HashMap<String, usize>,
    cleanup_policies: HashMap<String, CleanupPolicy>,
    routing_rules: HashMap<String, RoutingRule>,
}

impl NexusStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Repositories ────────────────────────────────────────────────────

    pub fn create_repository(&self, repo: Repository) -> Result<Repository, NexusError> {
        let mut inner = self.inner.write().unwrap();
        if inner.repositories.contains_key(&repo.name) {
            return Err(NexusError::RepositoryAlreadyExists(repo.name));
        }
        if let RepositoryType::Group { member_names } = &repo.repo_type {
            for m in member_names {
                if !inner.repositories.contains_key(m) {
                    return Err(NexusError::GroupMemberMissing(m.clone()));
                }
            }
        }
        inner.repositories.insert(repo.name.clone(), repo.clone());
        Ok(repo)
    }

    pub fn get_repository(&self, name: &str) -> Result<Repository, NexusError> {
        self.inner
            .read()
            .unwrap()
            .repositories
            .get(name)
            .cloned()
            .ok_or_else(|| NexusError::RepositoryNotFound(name.to_string()))
    }

    pub fn list_repositories(&self) -> Vec<Repository> {
        let mut out: Vec<_> = self
            .inner
            .read()
            .unwrap()
            .repositories
            .values()
            .cloned()
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn update_repository(
        &self,
        name: &str,
        f: impl FnOnce(&mut Repository),
    ) -> Result<Repository, NexusError> {
        let mut inner = self.inner.write().unwrap();
        let repo = inner
            .repositories
            .get_mut(name)
            .ok_or_else(|| NexusError::RepositoryNotFound(name.to_string()))?;
        f(repo);
        repo.updated_at = Utc::now();
        Ok(repo.clone())
    }

    pub fn delete_repository(&self, name: &str) -> Result<(), NexusError> {
        let mut inner = self.inner.write().unwrap();
        let repo = inner
            .repositories
            .remove(name)
            .ok_or_else(|| NexusError::RepositoryNotFound(name.to_string()))?;
        // Cascade: drop all components, assets, blob refs in the repo.
        let component_ids: Vec<_> = inner
            .components
            .iter()
            .filter(|(_, c)| c.repository_id == repo.id)
            .map(|(id, _)| *id)
            .collect();
        for cid in component_ids {
            inner.components.remove(&cid);
        }
        let asset_ids: Vec<_> = inner
            .assets
            .iter()
            .filter(|(_, a)| a.repository_id == repo.id)
            .map(|(id, a)| (*id, a.blob.sha256.clone()))
            .collect();
        for (aid, sha) in asset_ids {
            inner.assets.remove(&aid);
            decref_blob(&mut inner, &sha);
        }
        inner.asset_paths.remove(name);
        Ok(())
    }

    // ── Components ──────────────────────────────────────────────────────

    pub fn create_component(&self, component: Component) -> Component {
        let mut inner = self.inner.write().unwrap();
        inner.components.insert(component.id, component.clone());
        component
    }

    pub fn get_component(&self, id: Uuid) -> Result<Component, NexusError> {
        self.inner
            .read()
            .unwrap()
            .components
            .get(&id)
            .cloned()
            .ok_or_else(|| NexusError::ComponentNotFound(id.to_string()))
    }

    pub fn list_components(&self, repository_name: Option<&str>) -> Vec<Component> {
        let inner = self.inner.read().unwrap();
        let mut out: Vec<_> = inner
            .components
            .values()
            .filter(|c| {
                repository_name
                    .map(|name| c.repository_name == name)
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| {
            a.repository_name
                .cmp(&b.repository_name)
                .then_with(|| a.name.cmp(&b.name))
        });
        out
    }

    pub fn delete_component(&self, id: Uuid) -> Result<(), NexusError> {
        let mut inner = self.inner.write().unwrap();
        let component = inner
            .components
            .remove(&id)
            .ok_or_else(|| NexusError::ComponentNotFound(id.to_string()))?;
        // Cascade assets belonging to the component.
        let asset_ids: Vec<_> = inner
            .assets
            .iter()
            .filter(|(_, a)| a.component_id == id)
            .map(|(id, a)| (*id, a.path.clone(), a.blob.sha256.clone()))
            .collect();
        for (aid, path, sha) in asset_ids {
            inner.assets.remove(&aid);
            if let Some(map) = inner.asset_paths.get_mut(&component.repository_name) {
                map.remove(&path);
            }
            decref_blob(&mut inner, &sha);
        }
        Ok(())
    }

    // ── Assets ──────────────────────────────────────────────────────────

    pub fn put_asset(&self, asset: Asset, bytes: Vec<u8>) -> Result<Asset, NexusError> {
        let mut inner = self.inner.write().unwrap();
        // Store blob (dedupe by sha256).
        let sha = asset.blob.sha256.clone();
        if !inner.blobs.contains_key(&sha) {
            // Defensive: only insert when bytes match the declared sha.
            let actual = sha256_hex(&bytes);
            if actual != sha {
                return Err(NexusError::InvalidPath(format!(
                    "sha mismatch: declared {sha}, actual {actual}"
                )));
            }
            inner.blobs.insert(sha.clone(), bytes);
        }
        *inner.blob_refs.entry(sha).or_insert(0) += 1;

        // Store asset and index by path.
        let repo_name = asset.repository_name.clone();
        let path = asset.path.clone();
        inner
            .asset_paths
            .entry(repo_name)
            .or_default()
            .insert(path, asset.id);
        inner.assets.insert(asset.id, asset.clone());
        Ok(asset)
    }

    pub fn get_asset(&self, id: Uuid) -> Result<Asset, NexusError> {
        self.inner
            .read()
            .unwrap()
            .assets
            .get(&id)
            .cloned()
            .ok_or_else(|| NexusError::AssetNotFound(id.to_string()))
    }

    pub fn get_asset_by_path(&self, repo_name: &str, path: &str) -> Result<Asset, NexusError> {
        let inner = self.inner.read().unwrap();
        let id = inner
            .asset_paths
            .get(repo_name)
            .and_then(|m| m.get(path))
            .copied()
            .ok_or_else(|| NexusError::AssetNotFound(format!("{repo_name}:{path}")))?;
        inner
            .assets
            .get(&id)
            .cloned()
            .ok_or_else(|| NexusError::AssetNotFound(id.to_string()))
    }

    pub fn list_assets(&self, repository_name: Option<&str>) -> Vec<Asset> {
        let inner = self.inner.read().unwrap();
        let mut out: Vec<_> = inner
            .assets
            .values()
            .filter(|a| {
                repository_name
                    .map(|name| a.repository_name == name)
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| a.repository_name.cmp(&b.repository_name).then_with(|| a.path.cmp(&b.path)));
        out
    }

    pub fn delete_asset(&self, id: Uuid) -> Result<(), NexusError> {
        let mut inner = self.inner.write().unwrap();
        let asset = inner
            .assets
            .remove(&id)
            .ok_or_else(|| NexusError::AssetNotFound(id.to_string()))?;
        if let Some(map) = inner.asset_paths.get_mut(&asset.repository_name) {
            map.remove(&asset.path);
        }
        decref_blob(&mut inner, &asset.blob.sha256);
        Ok(())
    }

    pub fn record_download(&self, id: Uuid) -> Result<(), NexusError> {
        let mut inner = self.inner.write().unwrap();
        let asset = inner
            .assets
            .get_mut(&id)
            .ok_or_else(|| NexusError::AssetNotFound(id.to_string()))?;
        asset.download_count += 1;
        asset.last_downloaded = Some(Utc::now());
        Ok(())
    }

    // ── Blobs ───────────────────────────────────────────────────────────

    pub fn read_blob(&self, sha256: &str) -> Result<Vec<u8>, NexusError> {
        self.inner
            .read()
            .unwrap()
            .blobs
            .get(sha256)
            .cloned()
            .ok_or_else(|| NexusError::BlobNotFound(sha256.to_string()))
    }

    pub fn store_blob(&self, bytes: Vec<u8>) -> BlobRef {
        let sha = sha256_hex(&bytes);
        let size = bytes.len() as u64;
        let mut inner = self.inner.write().unwrap();
        inner.blobs.entry(sha.clone()).or_insert(bytes);
        BlobRef { sha256: sha, size }
    }

    pub fn blob_count(&self) -> usize {
        self.inner.read().unwrap().blobs.len()
    }

    // ── Cleanup policies ────────────────────────────────────────────────

    pub fn create_cleanup_policy(&self, policy: CleanupPolicy) -> CleanupPolicy {
        let mut inner = self.inner.write().unwrap();
        inner.cleanup_policies.insert(policy.name.clone(), policy.clone());
        policy
    }

    pub fn get_cleanup_policy(&self, name: &str) -> Result<CleanupPolicy, NexusError> {
        self.inner
            .read()
            .unwrap()
            .cleanup_policies
            .get(name)
            .cloned()
            .ok_or_else(|| NexusError::CleanupPolicyNotFound(name.to_string()))
    }

    pub fn list_cleanup_policies(&self) -> Vec<CleanupPolicy> {
        let mut out: Vec<_> = self
            .inner
            .read()
            .unwrap()
            .cleanup_policies
            .values()
            .cloned()
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn delete_cleanup_policy(&self, name: &str) -> Result<(), NexusError> {
        self.inner
            .write()
            .unwrap()
            .cleanup_policies
            .remove(name)
            .ok_or_else(|| NexusError::CleanupPolicyNotFound(name.to_string()))?;
        Ok(())
    }

    // ── Routing rules ───────────────────────────────────────────────────

    pub fn create_routing_rule(&self, rule: RoutingRule) -> RoutingRule {
        let mut inner = self.inner.write().unwrap();
        inner.routing_rules.insert(rule.name.clone(), rule.clone());
        rule
    }

    pub fn get_routing_rule(&self, name: &str) -> Result<RoutingRule, NexusError> {
        self.inner
            .read()
            .unwrap()
            .routing_rules
            .get(name)
            .cloned()
            .ok_or_else(|| NexusError::RoutingRuleNotFound(name.to_string()))
    }

    pub fn list_routing_rules(&self) -> Vec<RoutingRule> {
        let mut out: Vec<_> = self
            .inner
            .read()
            .unwrap()
            .routing_rules
            .values()
            .cloned()
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn delete_routing_rule(&self, name: &str) -> Result<(), NexusError> {
        self.inner
            .write()
            .unwrap()
            .routing_rules
            .remove(name)
            .ok_or_else(|| NexusError::RoutingRuleNotFound(name.to_string()))?;
        Ok(())
    }

    // ── Helpers exposed for cleanup module ──────────────────────────────

    pub(super) fn assets_in_repo(&self, repo_name: &str) -> Vec<Asset> {
        let inner = self.inner.read().unwrap();
        inner
            .assets
            .values()
            .filter(|a| a.repository_name == repo_name)
            .cloned()
            .collect()
    }
}

fn decref_blob(inner: &mut Inner, sha: &str) {
    let entry = inner.blob_refs.entry(sha.to_string()).or_insert(1);
    *entry = entry.saturating_sub(1);
    if *entry == 0 {
        inner.blob_refs.remove(sha);
        inner.blobs.remove(sha);
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
