---
crate: cave-cri
upstream_repo: kubernetes/cri-api
upstream_file: pkg/apis/runtime/v1/api.proto
upstream_fn: CreateContainer
status: draft
tier: 1
created_at: 2026-04-24T17:29:25.148744+00:00
---

## Upstream reference

`kubernetes/cri-api` → `pkg/apis/runtime/v1/api.proto` → `CreateContainer`

## Failing test

```rust
#[tokio::test]
async fn test_create_container_success() {
    use cave_cri::{CreateContainerRequest, CreateContainerResponse};
    use std::path::PathBuf;

    let request = CreateContainerRequest {
        pod_sandbox_id: "sandbox-123".to_string(),
        config: Some(cave_cri::ContainerConfig {
            metadata: Some(cave_cri::ContainerMetadata {
                name: "test-container".to_string(),
                attempt: 1,
            }),
            image: Some(cave_cri::Image {
                image: "nginx:latest".to_string(),
            }),
            command: vec!["nginx".to_string(), "-g".to_string(), "daemon off;".to_string()],
            args: vec![],
            working_dir: "/var/www".to_string(),
            envs: vec![
                cave_cri::KeyValue { key: "PORT".to_string(), value: "8080".to_string() },
                cave_cri::KeyValue { key: "HOST".to_string(), value: "0.0.0.0".to_string() },
            ],
            mounts: vec![
                cave_cri::Mount {
                    container_path: "/var/www/html".to_string(),
                    host_path: "/var/www/html".to_string(),
                    read_only: false,
                    selinux_relabel: false,
                    propagation: cave_cri::mount::MountPropagation::RPRIVATE as i32,
                }
            ],
            devices: vec![],
            annotations: std::collections::HashMap::new(),
            labels: std::collections::HashMap::new(),
            log_path: "/var/log/pods/default_test-container_1.log".to_string(),
            stdin: false,
            stdin_once: false,
            tty: false,
        }),
        sandbox_config: Some(cave_cri::PodSandboxConfig {
            metadata: Some(cave_cri::PodSandboxMetadata {
                name: "test-pod".to_string(),
                uid: "pod-uid-456".to_string(),
                namespace: "default".to_string(),
                attempt: 1,
            }),
            hostname: "test-pod".to_string(),
            log_directory: "/var/log/pods/default_test-pod".to_string(),
            dns_config: None,
            port_mappings: vec![],
            resources: None,
            selinux_options: None,
            windows_options: None,
            linux_options: None,
            annotations: std::collections::HashMap::new(),
            labels: std::collections::HashMap::new(),
        }),
        runtime_handler: "runc".to_string(),
        no_pivot: false,
    };

    let result = cave_cri::createcontainer(request).await;
    
    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response.container_id, "container-789");
    assert!(response.info.is_empty());
}
```

## Implementation skeleton

```rust
pub async fn createcontainer(_request: CreateContainerRequest) -> Result<CreateContainerResponse, anyhow::Error> {
    todo!("Tier 2")
}
```
