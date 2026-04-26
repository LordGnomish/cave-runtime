---
crate: cave-vault
upstream_repo: openbao/openbao
upstream_file: builtin/logical/kv/path_kv.go
upstream_fn: handleRead
status: draft
tier: 1
created_at: 2026-04-24T17:39:39.810486+00:00
---

## Upstream reference

`openbao/openbao` → `builtin/logical/kv/path_kv.go` → `handleRead`

## Failing test

```rust
#[tokio::test]
async fn test_handleread() {
    use cave_vault::logical::{Request, Response, Data};
    use cave_vault::vault::Secret;
    use std::collections::HashMap;

    // Setup: create a mock secret backend with test data
    let mut data = HashMap::new();
    data.insert("foo".to_string(), "bar".to_string());
    data.insert("nested".to_string(), serde_json::json!({"key": "value"}));
    
    let secret = Secret::new("kv/", data);
    
    // Create request to read existing key
    let mut req = Request::new("read", "kv/foo");
    req.data.insert("key".to_string(), "foo".to_string());
    
    // Mock backend that returns the secret
    let backend = cave_vault::vault::Backend::new(Box::new(move |req: &Request| {
        if req.operation == "read" && req.path == "kv/foo" {
            Response::ok(Some(secret.clone()))
        } else {
            Response::error("not found")
        }
    }));
    
    // Call handleread
    let result = cave_vault::logical::handleread(req, &backend).await;
    
    // Assert: successful response with correct data
    assert!(result.is_ok(), "handleread should succeed");
    let resp = result.unwrap();
    assert_eq!(resp.data.get("value"), Some(&serde_json::Value::String("bar".to_string())));
    
    // Test reading non-existent key
    let mut req2 = Request::new("read", "kv/nonexistent");
    req2.data.insert("key".to_string(), "nonexistent".to_string());
    
    let result2 = cave_vault::logical::handleread(req2, &backend).await;
    assert!(result2.is_err(), "reading non-existent key should fail");
}
```

## Implementation skeleton

```rust
pub async fn handleread(
    mut req: Request,
    backend: &Backend,
) -> Result<Response, String> {
    // Extract key from request data or path
    let key = req.data
        .remove("key")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .or_else(|| {
            // Try to extract key from path if not in data
            let path = req.path.strip_prefix("read/")?;
            Some(path.to_string())
        })
        .ok_or_else(|| "missing key parameter".to_string())?;

    // Construct path for backend lookup
    let full_path = format!("{}/{}", req.path.split('/').next().unwrap_or(""), key);
    
    // Create read request
    let read_req = Request::new("read", &full_path);
    
    // Call backend
    match backend.handle(&read_req).await {
        Ok(response) => {
            if let Some(secret) = response.data {
                // Extract data from secret
                let mut result_data = HashMap::new();
                for (k, v) in secret.data {
                    result_data.insert(k, v);
                }
                Ok(Response::ok(Some(result_data)))
            } else {
                Err(format!("key '{}' not found", key))
            }
        }
        Err(e) => Err(e),
    }
}
```
