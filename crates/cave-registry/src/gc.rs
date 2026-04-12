//! Garbage collection — removes blobs not referenced by any manifest.

use crate::store::RegistryStore;
use std::sync::Arc;
use tracing::{info, warn};

pub struct GarbageCollector {
    store: Arc<RegistryStore>,
}

impl GarbageCollector {
    pub fn new(store: Arc<RegistryStore>) -> Self {
        Self { store }
    }

    /// Delete all blobs that are not referenced by any stored manifest.
    /// Returns the number of blobs collected.
    pub async fn collect(&self) -> usize {
        let unreferenced = self.store.unreferenced_blobs().await;
        let count = unreferenced.len();
        if count == 0 {
            info!(target: "cave_registry::gc", "garbage collection: nothing to collect");
            return 0;
        }
        for digest in &unreferenced {
            if self.store.delete_blob(digest).await {
                info!(target: "cave_registry::gc", %digest, "collected unreferenced blob");
            } else {
                warn!(target: "cave_registry::gc", %digest, "blob disappeared before gc could remove it");
            }
        }
        info!(target: "cave_registry::gc", count, "garbage collection complete");
        count
    }

    /// Spawn a background task that runs GC every `interval`.
    pub fn spawn_periodic(store: Arc<RegistryStore>, interval: std::time::Duration) {
        tokio::spawn(async move {
            let gc = GarbageCollector::new(store);
            loop {
                tokio::time::sleep(interval).await;
                gc.collect().await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_gc_removes_unreferenced_blob() {
        let store = Arc::new(RegistryStore::new());
        let gc = GarbageCollector::new(Arc::clone(&store));

        // Add a blob with no manifest referencing it.
        store.put_blob(b"orphan data".to_vec(), None).await.unwrap();
        let before = store.unreferenced_blobs().await;
        assert_eq!(before.len(), 1);

        let collected = gc.collect().await;
        assert_eq!(collected, 1);

        let after = store.unreferenced_blobs().await;
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn test_gc_keeps_referenced_blobs() {
        use crate::types::MEDIA_MANIFEST_V2;
        let store = Arc::new(RegistryStore::new());
        let gc = GarbageCollector::new(Arc::clone(&store));

        // Put a blob and reference it from a manifest.
        let data = b"layer data";
        let digest = store.put_blob(data.to_vec(), None).await.unwrap();

        let manifest_json = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": MEDIA_MANIFEST_V2,
            "config": { "mediaType": "application/vnd.docker.container.image.v1+json", "size": 0, "digest": digest },
            "layers": []
        });
        let manifest_bytes = serde_json::to_vec(&manifest_json).unwrap();
        store
            .put_manifest("myrepo", "latest", MEDIA_MANIFEST_V2.to_string(), manifest_bytes)
            .await
            .unwrap();

        let collected = gc.collect().await;
        assert_eq!(collected, 0, "referenced blob must not be collected");
        assert!(store.blob_exists(&digest).await);
    }
}
