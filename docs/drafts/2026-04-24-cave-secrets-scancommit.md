---
crate: cave-secrets
upstream_repo: trufflesecurity/trufflehog
upstream_file: pkg/sources/git/git.go
upstream_fn: ScanCommit
status: draft
tier: 1
created_at: 2026-04-24T18:14:10.084388+00:00
---

## Upstream reference

`trufflesecurity/trufflehog` → `pkg/sources/git/git.go` → `ScanCommit`

## Failing test

```rust
#[tokio::test]
async fn test_scancommit_detects_secrets_in_commit() {
    use cave_secrets::{scancommit, Secret, SecretType};
    use std::path::Path;
    use tempfile::TempDir;
    use std::fs;
    use std::process::Command;

    // Create a temporary git repo
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let repo_path = temp_dir.path();
    
    // Initialize git repo
    Command::new("git")
        .arg("init")
        .current_dir(repo_path)
        .output()
        .expect("Failed to init git repo");

    // Configure git user for commits
    Command::new("git")
        .arg("config")
        .arg("user.email")
        .arg("test@example.com")
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git email");
    Command::new("git")
        .arg("config")
        .arg("user.name")
        .arg("Test User")
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git name");

    // Create a file with a fake AWS key
    let secret_content = "AWS_SECRET_ACCESS_KEY=AKIAIOSFODNN7EXAMPLE";
    fs::write(repo_path.join("config.txt"), secret_content).expect("Failed to write config file");

    // Commit the file
    Command::new("git")
        .arg("add")
        .arg("config.txt")
        .current_dir(repo_path)
        .output()
        .expect("Failed to add file");
    Command::new("git")
        .arg("commit")
        .arg("-m")
        .arg("Add config")
        .current_dir(repo_path)
        .output()
        .expect("Failed to commit");

    // Get the commit hash
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(repo_path)
        .output()
        .expect("Failed to get HEAD commit");
    let commit_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Scan the commit
    let secrets = scancommit(repo_path, &commit_hash).await.unwrap();

    // Assert at least one secret is detected
    assert!(!secrets.is_empty(), "Expected at least one secret to be detected");
    
    // Check that the secret contains the AWS key pattern
    let has_aws_key = secrets.iter().any(|s| {
        matches!(s.secret_type, SecretType::AWS) &&
        s.raw.contains("AKIAIOSFODNN7EXAMPLE")
    });
    assert!(has_aws_key, "Expected to find AWS secret with key AKIAIOSFODNN7EXAMPLE");
}
```

## Implementation skeleton

```rust
pub async fn scancommit(repo_path: &Path, commit_hash: &str) -> Result<Vec<Secret>, Box<dyn std::error::Error + Send + Sync>> {
    todo!("Tier 2")
}
```
