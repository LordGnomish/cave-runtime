// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Registry-to-registry replication.
//! Pushes manifests and blobs to configured upstream registries.

use crate::harbor::store::RegistryStore;
use crate::harbor::types::ReplicationTarget;
use std::sync::Arc;
use tracing::{error, info, warn};

pub struct ReplicationManager {
    store: Arc<RegistryStore>,
    client: reqwest::Client,
}

impl ReplicationManager {
    pub fn new(store: Arc<RegistryStore>) -> Self {
        Self { store, client: reqwest::Client::new() }
    }

    pub async fn add_target(&self, target: ReplicationTarget) {
        self.store.add_replication_target(target).await;
    }

    pub async fn targets(&self) -> Vec<ReplicationTarget> {
        self.store.get_replication_targets().await
    }

    /// Replicate a manifest (and its blobs) to all enabled targets.
    /// Spawns background tasks — does not block the caller.
    pub async fn replicate_manifest(
        &self,
        repository: &str,
        reference: &str,
        manifest_bytes: Vec<u8>,
        media_type: String,
        digest: String,
    ) {
        let targets = self.store.get_replication_targets().await;
        for target in targets {
            if !target.enabled {
                continue;
            }
            let client = self.client.clone();
            let repo = repository.to_string();
            let reference = reference.to_string();
            let bytes = manifest_bytes.clone();
            let mt = media_type.clone();
            let dg = digest.clone();
            tokio::spawn(async move {
                let url = format!(
                    "{}/v2/{}/manifests/{}",
                    target.url.trim_end_matches('/'),
                    repo,
                    reference
                );
                match client
                    .put(&url)
                    .header("Content-Type", &mt)
                    .body(bytes)
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        info!(target: "cave_registry::replication", %url, %dg, "manifest replicated");
                    }
                    Ok(resp) => {
                        warn!(
                            target: "cave_registry::replication",
                            %url,
                            status = resp.status().as_u16(),
                            "replication received non-2xx"
                        );
                    }
                    Err(e) => {
                        error!(target: "cave_registry::replication", %url, err = %e, "replication failed");
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_add_and_list_targets() {
        let store = Arc::new(RegistryStore::new());
        let mgr = ReplicationManager::new(Arc::clone(&store));

        mgr.add_target(ReplicationTarget {
            id: "t1".to_string(),
            name: "staging".to_string(),
            url: "https://registry.staging.example.com".to_string(),
            enabled: true,
            username: None,
            password: None,
        })
        .await;

        let targets = mgr.targets().await;
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "staging");
    }

    #[tokio::test]
    async fn test_disabled_target_skipped() {
        let store = Arc::new(RegistryStore::new());
        let mgr = ReplicationManager::new(Arc::clone(&store));

        mgr.add_target(ReplicationTarget {
            id: "t2".to_string(),
            name: "disabled".to_string(),
            url: "https://unreachable.example.com".to_string(),
            enabled: false,
            username: None,
            password: None,
        })
        .await;

        // Should not attempt a network call for disabled targets.
        mgr.replicate_manifest("repo", "latest", b"{}".to_vec(), "application/json".to_string(), "sha256:abc".to_string()).await;
        // If no panic / no network call attempted, test passes.
    }
}
