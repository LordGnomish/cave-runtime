---
crate: cave-apiserver
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apiserver/pkg/endpoints/handlers/update.go
upstream_fn: updateHandler
status: draft
tier: 1
created_at: 2026-04-24T17:26:53.823425+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apiserver/pkg/endpoints/handlers/update.go` → `updateHandler`

## Failing test

```rust
#[tokio::test]
async fn test_updatehandler_success() {
    use cave_apiserver::storage::Storage;
    use cave_apiserver::types::{ObjectMeta, RawExtension, ResourceVersion, UpdateOptions};
    use cave_apiserver::runtime::Scheme;
    use cave_apiserver::endpoints::handlers::updatehandler;
    use cave_apiserver::storage::etcd::EtcdStorage;
    use cave_apiserver::storage::Watchable;
    use cave_apiserver::storage::Object;
    use cave_apiserver::storage::ObjectMetaBuilder;
    use cave_apiserver::storage::ResourceVersionBuilder;
    use cave_apiserver::storage::UpdateOptionsBuilder;
    use cave_apiserver::storage::UpdateOptionsType;
    use cave_apiserver::storage::UpdateOptionsStrategy;
    use cave_apiserver::storage::UpdateOptionsStrategyType;
    use cave_apiserver::storage::UpdateOptionsStrategyBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManager;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeBuilder;
    use cave_apiserver::storage::UpdateOptionsStrategyFieldManagerTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeTypeType
```

## Implementation skeleton

```rust

```
