---
crate: cave-etcd
upstream_repo: etcd-io/etcd
upstream_file: pkg/pathutil/path.go
upstream_fn: CompactPath
status: draft
tier: 1
created_at: 2026-04-23T07:07:52.895376+00:00
---

## Upstream reference

`etcd-io/etcd` → `pkg/pathutil/path.go` → `CompactPath`

## Failing test

```rust
#[tokio::test]
async fn test_compactpath() {
    use cave_etcd::compactpath;

    // Test case 1: Normal path with trailing slash
    assert_eq!(compactpath("/foo/bar/"), "/foo/bar");

    // Test case 2: Root path
    assert_eq!(compactpath("/"), "/");

    // Test case 3: Path without trailing slash
    assert_eq!(compactpath("/foo/bar"), "/foo/bar");

    // Test case 4: Nested path with multiple slashes
    assert_eq!(compactpath("/a/b/c/"), "/a/b/c");

    // Test case 5: Path with trailing slashes (more than one)
    assert_eq!(compactpath("/foo///"), "/foo");

    // Test case 6: Empty string (edge case)
    assert_eq!(compactpath(""), "");

    // Test case 7: Path with leading slashes only
    assert_eq!(compactpath("///"), "");
}
```

## Implementation skeleton

```rust
pub fn compactpath(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }

    let mut result = String::from(path);
    // Remove trailing slashes, but preserve at least one slash for root path
    while result.len() > 1 && result.ends_with('/') {
        result.pop();
    }

    // Collapse multiple consecutive slashes into one (except at the beginning)
    let mut chars: Vec<char> = result.chars().collect();
    let mut write_idx = 0;
    let mut prev_slash = false;

    for &c in chars.iter() {
        if c == '/' {
            if !prev_slash {
                chars[write_idx] = c;
                write_idx += 1;
                prev_slash = true;
            }
        } else {
            chars[write_idx] = c;
            write_idx += 1;
            prev_slash = false;
        }
    }

    let compacted = chars[..write_idx].iter().collect::<String>();

    // Ensure root path "/" is preserved
    if compacted.is_empty() {
        "/".to_string()
    } else {
        compacted
    }
}
```
