---
crate: cave-etcd
upstream_repo: etcd-io/etcd
upstream_file: server/etcdserver/api/v3rpc/maintenance.go
upstream_fn: Defragment
status: draft
tier: 1
created_at: 2026-04-24T18:14:54.870856+00:00
---

## Upstream reference

`etcd-io/etcd` → `server/etcdserver/api/v3rpc/maintenance.go` → `Defragment`

## Failing test

```rust
#[tokio::test]
async fn test_defragment_success() {
    use cave_etcd::DefragmentRequest;
    use cave_etcd::DefragmentResponse;
    use cave_etcd::Error;
    use std::path::PathBuf;

    // Simulate a valid etcd data directory
    let data_dir = PathBuf::from("/tmp/cave_etcd_test_data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let _file = std::fs::File::create(data_dir.join("member/snap/db")).unwrap();

    let req = DefragmentRequest {
        data_dir: data_dir.clone(),
    };

    // Mock the defragmentation logic by ensuring no error is returned
    // and the data directory path is preserved
    match cave_etcd::defragment(req).await {
        Ok(resp) => {
            assert_eq!(resp.data_dir, data_dir);
        }
        Err(e) => panic!("Defragmentation failed unexpectedly: {:?}", e),
    }

    // Cleanup
    std::fs::remove_dir_all(&data_dir).unwrap();
}
```

## Implementation skeleton

```rust
pub async fn defragment(req: DefragmentRequest) -> Result<DefragmentResponse, Error> {
    todo!("Tier 2")
}
```
