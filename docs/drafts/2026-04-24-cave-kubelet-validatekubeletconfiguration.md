---
crate: cave-kubelet
upstream_repo: kubernetes/kubernetes
upstream_file: pkg/kubelet/apis/config/validation/validation.go
upstream_fn: ValidateKubeletConfiguration
status: draft
tier: 1
created_at: 2026-04-24T16:38:55.762533+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `pkg/kubelet/apis/config/validation/validation.go` → `ValidateKubeletConfiguration`

## Failing test

```rust
#[tokio::test]
async fn test_validate_kubelet_configuration() {
    use cave_kubelet::apis::config::v1beta1::KubeletConfiguration;
    use cave_kubelet::validation::validatekubeletconfiguration;
    use std::collections::HashMap;

    // Valid minimal config
    let valid_config = KubeletConfiguration {
        api_version: Some("kubelet.config.k8s.io/v1beta1".to_string()),
        kind: Some("KubeletConfiguration".to_string()),
        static_pod_path: Some("/etc/kubernetes/manifests".to_string()),
        sync_frequency: Some(10000000000), // 10s in nanoseconds
        file_check_frequency: Some(20000000000), // 20s
        address: Some("0.0.0.0".to_string()),
        port: Some(10250),
        tls_cert_file: Some("/var/lib/kubelet/pki/kubelet.crt".to_string()),
        tls_private_key_file: Some("/var/lib/kubelet/pki/kubelet.key".to_string()),
        authentication: Some(
            serde_json::from_value(serde_json::json!({
                "anonymous": {"enabled": false},
                "webhook": {"cacheTTL": "2m0s", "enabled": true},
                "x509": {"clientCAFile": "/var/lib/kubelet/pki/ca.crt"}
            })).unwrap()
        ),
        authorization: Some(
            serde_json::from_value(serde_json::json!({
                "mode": "Webhook",
                "webhook": {"cacheAuthorizedTTL": "5m0s", "cacheUnauthorizedTTL": "30s"}
            })).unwrap()
        ),
        cluster_dns: Some(vec!["10.96.0.10".to_string()]),
        cluster_domain: Some("cluster.local".to_string()),
        cgroup_driver: Some("systemd".to_string()),
        container_runtime_endpoint: Some("unix:///run/containerd/containerd.sock".to_string()),
        image_service_endpoint: Some("unix:///run/containerd/containerd.sock".to_string()),
        max_pods: Some(110),
        pod_infra_container_image: Some("k8s.gcr.io/pause:3.9".to_string()),
        eviction_hard: Some(HashMap::from([
            ("memory.available".to_string(), "100Mi".to_string()),
            ("nodefs.available".to_string(), "10%".to_string()),
            ("imagefs.available".to_string(), "15%".to_string()),
        ])),
        ..Default::default()
    };

    let result = validatekubeletconfiguration(&valid_config).await;
    assert!(result.is_ok(), "Valid config should pass validation: {:?}", result);

    // Invalid config: empty cluster_dns
    let invalid_config = KubeletConfiguration {
        cluster_dns: Some(vec![]),
        ..valid_config.clone()
    };

    let result = validatekubeletconfiguration(&invalid_config).await;
    assert!(result.is_err(), "Empty cluster_dns should fail validation");
    let err = result.unwrap_err();
    assert!(err.to_string().contains("cluster_dns"), "Error should mention cluster_dns");

    // Invalid config: invalid port (out of range)
    let invalid_port_config = KubeletConfiguration {
        port: Some(0),
        ..valid_config.clone()
    };

    let result = validatekubeletconfiguration(&invalid_port_config).await;
    assert!(result.is_err(), "Port 0 should fail validation");
    let err = result.unwrap_err();
    assert!(err.to_string().contains("port"), "Error should mention port");

    // Invalid config: invalid eviction_hard threshold format
    let invalid_eviction_config = KubeletConfiguration {
        eviction_hard: Some(HashMap::from([
            ("memory.available".to_string(), "invalid".to_string()),
        ])),
        ..valid_config
    };

    let result = validatekubeletconfiguration(&invalid_eviction_config).await;
    assert!(result.is_err(), "Invalid eviction threshold should fail");
}
```

## Implementation skeleton

```rust
pub async fn validatekubeletconfiguration(
    config: &KubeletConfiguration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    todo!("Tier 2");
}
```
