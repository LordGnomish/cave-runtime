---
crate: cave-secrets
upstream_repo: trufflesecurity/trufflehog
upstream_file: pkg/sources/filesystem/filesystem.go
upstream_fn: ScanFile
status: draft
tier: 1
created_at: 2026-04-24T17:16:31.722837+00:00
---

## Upstream reference

`trufflesecurity/trufflehog` → `pkg/sources/filesystem/filesystem.go` → `ScanFile`

## Failing test

```rust
#[tokio::test]
async fn test_scanfile_scans_file_and_emits_chunks() {
    use cave_secrets::{Chunk, ChunkType, Source, SourceType};
    use std::fs;
    use tempfile::TempDir;

    // Create a temporary directory and file with some content
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("secret.txt");
    let content = "This is a test file.\nIt contains a fake AWS key: AKIAIOSFODNN7EXAMPLE\nAnd some more text.";
    fs::write(&file_path, content).expect("Failed to write test file");

    // Create a Source for the file
    let source = Source {
        source_type: SourceType::Filesystem,
        name: Some("test_filesystem".to_string()),
        config: serde_json::json!({"path": file_path.to_str().unwrap()}),
    };

    // Collect emitted chunks
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut chunk_stream = cave_secrets::scanfile(&source).await.unwrap();
    
    while let Some(chunk) = chunk_stream.next().await {
        chunks.push(chunk);
    }

    // Assert we got at least one chunk
    assert!(!chunks.is_empty(), "Expected at least one chunk to be emitted");

    // Assert the first chunk contains the expected content
    let first_chunk = &chunks[0];
    assert_eq!(first_chunk.data, content);
    assert_eq!(first_chunk.source_id, source.id());
    assert_eq!(first_chunk.source_name, source.name);
    assert_eq!(first_chunk.source_type, source.source_type);
    assert_eq!(first_chunk.chunk_type, ChunkType::Text);
}
```

## Implementation skeleton

```rust
pub async fn scanfile(source: &Source) -> Result<impl Stream<Item = Chunk>, Box<dyn std::error::Error + Send + Sync>> {
    todo!("Tier 2")
}
```
