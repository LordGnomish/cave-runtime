---
crate: cave-cri
upstream_repo: kubernetes/cri-api
upstream_file: pkg/apis/runtime/v1/api.proto
upstream_fn: RunPodSandbox
status: draft
tier: 1
created_at: 2026-04-24T17:28:30.388441+00:00
---

## Upstream reference

`kubernetes/cri-api` → `pkg/apis/runtime/v1/api.proto` → `RunPodSandbox`

## Failing test

```rust
#[tokio::test]
async fn test_runpodsandbox_success() {
    use cave_cri::api::runtime::v1::{
        RunPodSandboxRequest, RunPodSandboxResponse, PodSandboxConfig, Metadata,
        LinuxPodSandboxConfig, LinuxSandboxConfig, RuntimeHandler,
    };
    use cave_cri::CriClient;
    use std::collections::HashMap;

    // Mock client that captures request and returns expected response
    struct MockClient {
        expected_request: RunPodSandboxRequest,
    }

    #[async_trait::async_trait]
    impl CriClient for MockClient {
        async fn run_pod_sandbox(
            &self,
            request: RunPodSandboxRequest,
        ) -> Result<RunPodSandboxResponse, anyhow::Error> {
            assert_eq!(request, self.expected_request);
            Ok(RunPodSandboxResponse {
                pod_sandbox_id: "sandbox-123".to_string(),
            })
        }
    }

    let config = PodSandboxConfig {
        metadata: Metadata {
            name: "test-pod".to_string(),
            uid: "uid-456".to_string(),
            namespace: "default".to_string(),
            attempt: 1,
        },
        hostname: "test-host".to_string(),
        linux: Some(LinuxPodSandboxConfig {
            cgroup_parent: "/kubepods".to_string(),
            security_context: Some(LinuxSandboxConfig {
                namespace_options: Some(
                    cave_cri::api::runtime::v1::NamespaceOption {
                        network: 2, // NamespaceMode::NODE
                        pid: 1,     // NamespaceMode::POD
                        uts: 2,     // NamespaceMode::NODE
                        ..Default::default()
                    }
                ),
                seccomp_profile_path: "/var/lib/seccomp/profiles/default.json".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let request = RunPodSandboxRequest {
        config: Some(config),
        runtime_handler: "runc".to_string(),
    };

    let client = MockClient {
        expected_request: request.clone(),
    };

    let response = cave_cri::runpodsandbox(client, request).await.unwrap();
    assert_eq!(response.pod_sandbox_id, "sandbox-123");
}
```

## Implementation skeleton

```rust
pub async fn runpodsandbox<C: CriClient>(
    client: C,
    request: RunPodSandboxRequest,
) -> Result<RunPodSandboxResponse, anyhow::Error> {
    todo!("Tier 2")
}
```
