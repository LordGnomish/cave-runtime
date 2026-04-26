//! LocalPublisher — port of LocalPublisher from backstage-plugin-techdocs-node.
//!
//! Upstream: techdocs-node/src/publishing/local.ts — LocalPublisher class

use super::{Publisher, TechDocsError};
use crate::models::{EntityName, TechDocsMetadata};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// Publishes docs to the local filesystem.
///
/// Upstream: LocalPublisher in @backstage/plugin-techdocs-node
///
/// Directory layout: `output_dir/{namespace}/{kind}/{name}/`
pub struct LocalPublisher {
    output_dir: PathBuf,
}

impl LocalPublisher {
    /// Create a new LocalPublisher that stores docs under `output_dir`.
    pub fn new(output_dir: impl Into<PathBuf>) -> Self {
        Self {
            output_dir: output_dir.into(),
        }
    }

    /// Canonical path for an entity's docs root.
    ///
    /// Upstream: getEntityRootDir(entity) in local.ts
    fn entity_dir(&self, entity: &EntityName) -> PathBuf {
        self.output_dir
            .join(&entity.namespace)
            .join(&entity.kind)
            .join(&entity.name)
    }
}

#[async_trait]
impl Publisher for LocalPublisher {
    /// Copy `docs_path/*` into `output_dir/namespace/kind/name/`.
    ///
    /// Upstream: publish(entity, directory) in local.ts
    async fn publish(
        &self,
        entity: &EntityName,
        docs_path: &Path,
    ) -> Result<(), TechDocsError> {
        let dest = self.entity_dir(entity);
        tokio::fs::create_dir_all(&dest).await?;
        copy_dir_recursive(docs_path, &dest).await?;
        Ok(())
    }

    /// Read and parse `techdocs_metadata.json` from the entity docs directory.
    ///
    /// Upstream: fetchTechDocsMetadata(entityName) in local.ts
    async fn fetch_metadata(&self, entity: &EntityName) -> Result<TechDocsMetadata, TechDocsError> {
        let path = self.entity_dir(entity).join("techdocs_metadata.json");
        if !path.exists() {
            return Err(TechDocsError::NotFound(format!(
                "techdocs_metadata.json not found for {}/{}/{}",
                entity.namespace, entity.kind, entity.name
            )));
        }
        let bytes = tokio::fs::read(&path).await?;
        let metadata: TechDocsMetadata = serde_json::from_slice(&bytes)?;
        Ok(metadata)
    }

    /// Return true if the entity docs directory exists and is non-empty.
    ///
    /// Upstream: hasDocsBeenGenerated(entityName) in local.ts
    async fn has_docs(&self, entity: &EntityName) -> Result<bool, TechDocsError> {
        let dir = self.entity_dir(entity);
        if !dir.exists() {
            return Ok(false);
        }
        let mut entries = tokio::fs::read_dir(&dir).await?;
        Ok(entries.next_entry().await?.is_some())
    }

    /// Read a file at `output_dir/namespace/kind/name/{path}`.
    ///
    /// Upstream: fetchStaticFile(entity, path) in local.ts
    async fn read_file(&self, entity: &EntityName, path: &str) -> Result<Vec<u8>, TechDocsError> {
        let file_path = self.entity_dir(entity).join(path);
        if !file_path.exists() {
            return Err(TechDocsError::NotFound(format!(
                "static file not found: {}/{}/{}/{}",
                entity.namespace, entity.kind, entity.name, path
            )));
        }
        let bytes = tokio::fs::read(&file_path).await?;
        Ok(bytes)
    }
}

/// Recursively copy all files from `src` into `dst`.
///
/// Upstream: copies docs directory in local.ts publish()
async fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(src.to_path_buf(), dst.to_path_buf())];

    while let Some((src_dir, dst_dir)) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&src_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let file_type = entry.file_type().await?;
            let src_path = entry.path();
            let dst_path = dst_dir.join(entry.file_name());

            if file_type.is_dir() {
                tokio::fs::create_dir_all(&dst_path).await?;
                stack.push((src_path, dst_path));
            } else {
                tokio::fs::copy(&src_path, &dst_path).await?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_entity() -> EntityName {
        EntityName::new("default", "Component", "my-service")
    }

    /// publish() creates output_dir/namespace/kind/name/
    ///
    /// Upstream: local.test.ts — "publish creates directory"
    #[tokio::test]
    async fn publish_creates_directory() {
        let tmp = TempDir::new().unwrap();
        let publisher = LocalPublisher::new(tmp.path());

        // Create a source docs directory with one file
        let src = TempDir::new().unwrap();
        tokio::fs::write(src.path().join("index.html"), b"<html/>").await.unwrap();

        let entity = test_entity();
        publisher.publish(&entity, src.path()).await.unwrap();

        let dest = tmp.path().join("default").join("Component").join("my-service");
        assert!(dest.exists(), "entity docs directory must exist after publish");
    }

    /// has_docs() returns false for a non-existent entity.
    ///
    /// Upstream: local.test.ts — "hasDocsBeenGenerated returns false when entity has no docs"
    #[tokio::test]
    async fn has_docs_false_when_empty() {
        let tmp = TempDir::new().unwrap();
        let publisher = LocalPublisher::new(tmp.path());
        let entity = test_entity();

        let result = publisher.has_docs(&entity).await.unwrap();
        assert!(!result, "has_docs must be false when no docs have been published");
    }

    /// has_docs() returns true after publish().
    ///
    /// Upstream: local.test.ts — "hasDocsBeenGenerated returns true after publish"
    #[tokio::test]
    async fn has_docs_true_after_publish() {
        let tmp = TempDir::new().unwrap();
        let publisher = LocalPublisher::new(tmp.path());

        let src = TempDir::new().unwrap();
        tokio::fs::write(src.path().join("index.html"), b"<html/>").await.unwrap();

        let entity = test_entity();
        publisher.publish(&entity, src.path()).await.unwrap();

        let result = publisher.has_docs(&entity).await.unwrap();
        assert!(result, "has_docs must be true after publish");
    }

    /// fetch_metadata() returns NotFound when no metadata file exists.
    ///
    /// Upstream: local.test.ts — "fetchTechDocsMetadata throws NotFound"
    #[tokio::test]
    async fn fetch_metadata_not_found() {
        let tmp = TempDir::new().unwrap();
        let publisher = LocalPublisher::new(tmp.path());
        let entity = test_entity();

        let result = publisher.fetch_metadata(&entity).await;
        assert!(
            matches!(result, Err(TechDocsError::NotFound(_))),
            "fetch_metadata must return NotFound when no metadata file exists"
        );
    }

    /// fetch_metadata() parses techdocs_metadata.json correctly.
    ///
    /// Upstream: local.test.ts — "fetchTechDocsMetadata returns metadata"
    #[tokio::test]
    async fn fetch_metadata_reads_json() {
        let tmp = TempDir::new().unwrap();
        let publisher = LocalPublisher::new(tmp.path());

        // Publish a source directory containing techdocs_metadata.json
        let src = TempDir::new().unwrap();
        let meta = serde_json::json!({
            "site_name": "My Service",
            "site_description": "Docs for my service",
            "etag": "abc123",
            "build_timestamp": 1_700_000_000i64,
            "files": ["index.html", "api.html"]
        });
        tokio::fs::write(
            src.path().join("techdocs_metadata.json"),
            serde_json::to_vec(&meta).unwrap(),
        )
        .await
        .unwrap();

        let entity = test_entity();
        publisher.publish(&entity, src.path()).await.unwrap();

        let result = publisher.fetch_metadata(&entity).await.unwrap();
        assert_eq!(result.site_name, "My Service");
        assert_eq!(result.etag, "abc123");
        assert_eq!(result.build_timestamp, 1_700_000_000);
        assert_eq!(result.files, vec!["index.html", "api.html"]);
    }

    /// read_file() returns NotFound for a missing file.
    ///
    /// Upstream: local.test.ts — "fetchStaticFile throws NotFound"
    #[tokio::test]
    async fn read_file_not_found() {
        let tmp = TempDir::new().unwrap();
        let publisher = LocalPublisher::new(tmp.path());
        let entity = test_entity();

        let result = publisher.read_file(&entity, "index.html").await;
        assert!(
            matches!(result, Err(TechDocsError::NotFound(_))),
            "read_file must return NotFound for a missing file"
        );
    }

    /// read_file() returns correct bytes after publish.
    ///
    /// Upstream: local.test.ts — "fetchStaticFile returns correct content"
    #[tokio::test]
    async fn read_file_returns_content() {
        let tmp = TempDir::new().unwrap();
        let publisher = LocalPublisher::new(tmp.path());

        let src = TempDir::new().unwrap();
        tokio::fs::write(src.path().join("index.html"), b"hello techdocs")
            .await
            .unwrap();

        let entity = test_entity();
        publisher.publish(&entity, src.path()).await.unwrap();

        let bytes = publisher.read_file(&entity, "index.html").await.unwrap();
        assert_eq!(bytes, b"hello techdocs");
    }

    /// publish() copies all files from the source directory.
    ///
    /// Upstream: local.test.ts — "publish copies all files"
    #[tokio::test]
    async fn publish_copies_all_files() {
        let tmp = TempDir::new().unwrap();
        let publisher = LocalPublisher::new(tmp.path());

        let src = TempDir::new().unwrap();
        tokio::fs::write(src.path().join("index.html"), b"index").await.unwrap();
        tokio::fs::write(src.path().join("api.html"), b"api").await.unwrap();
        tokio::fs::write(src.path().join("style.css"), b"body{}").await.unwrap();

        let entity = test_entity();
        publisher.publish(&entity, src.path()).await.unwrap();

        let dest = tmp.path().join("default").join("Component").join("my-service");
        assert!(dest.join("index.html").exists());
        assert!(dest.join("api.html").exists());
        assert!(dest.join("style.css").exists());

        let content = tokio::fs::read(dest.join("api.html")).await.unwrap();
        assert_eq!(content, b"api");
    }
}
