---
crate: cave-kubelet
upstream_repo: kubernetes/kubernetes
upstream_file: pkg/kubelet/pod/pod_manager.go
upstream_fn: GetPodByUID
status: draft
tier: 1
created_at: 2026-04-24T17:32:59.510498+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `pkg/kubelet/pod/pod_manager.go` → `GetPodByUID`

## Failing test

```rust
#[tokio::test]
async fn test_getpodbyuid() {
    use cave_kubelet::pod::manager::PodManager;
    use cave_kubelet::pod::types::Pod;
    use cave_kubelet::pod::uid::PodUID;
    use std::collections::HashMap;
    use std::time::Duration;

    // Create a pod manager
    let manager = PodManager::new();

    // Create a sample pod
    let uid = PodUID::new("test-uid-123".to_string());
    let pod = Pod {
        metadata: Default::default(),
        spec: Default::default(),
        status: Default::default(),
        uid: uid.clone(),
        creation_timestamp: Default::default(),
        namespace: "default".to_string(),
        name: "test-pod".to_string(),
        annotations: HashMap::new(),
        labels: HashMap::new(),
        phase: Default::default(),
        container_statuses: Vec::new(),
        init_container_statuses: Vec::new(),
        qos_class: Default::default(),
        host_ip: None,
        pod_ip: None,
        start_time: None,
        conditions: Vec::new(),
        ephemeral_containers: Vec::new(),
        share_process_namespace: None,
        security_context: None,
        set_hostname_as_fqdn: None,
        os: None,
        host_network: false,
        host_pid: false,
        host_ipc: false,
        dns_policy: Default::default(),
        service_account_name: "".to_string(),
        automount_service_account_token: None,
        node_name: "".to_string(),
        restart_policy: Default::default(),
        termination_grace_period_seconds: None,
        active_deadline_seconds: None,
        affinity: None,
        scheduler_name: "".to_string(),
        tolerations: Vec::new(),
        priority: None,
        priority_class_name: "".to_string(),
        preemption_policy: None,
        overhead: None,
        resource_claims: Vec::new(),
        topology_spread_constraints: Vec::new(),
        runtime_class_name: None,
        readiness_gates: Vec::new(),
        status_conditions: Vec::new(),
        finalizers: Vec::new(),
        owner_references: Vec::new(),
        managed_fields: Vec::new(),
        resource_version: "".to_string(),
        uid: uid.clone(),
        generation: 0,
        namespace_uid: "".to_string(),
        deletion_timestamp: None,
        deletion_grace_period_seconds: None,
        labels_checksum: None,
        annotations_checksum: None,
        pod_cidr: None,
        pod_cidrs: Vec::new(),
        host_aliases: Vec::new(),
        image_pull_secrets: Vec::new(),
        subdomain: "".to_string(),
        fqdn: None,
        hostname: None,
        affinity_checksum: None,
        tolerations_checksum: None,
        topology_spread_constraints_checksum: None,
        resource_claims_checksum: None,
        os_checksum: None,
        security_context_checksum: None,
        share_process_namespace_checksum: None,
        dns_config_checksum: None,
        host_aliases_checksum: None,
        image_pull_secrets_checksum: None,
        subdomain_checksum: None,
        hostname_checksum: None,
        finalizers_checksum: None,
        owner_references_checksum: None,
        managed_fields_checksum: None,
        resource_version_checksum: None,
        uid_checksum: None,
        namespace_uid_checksum: None,
        deletion_timestamp_checksum: None,
        deletion_grace_period_seconds_checksum: None,
        labels_checksum_v2: None,
        annotations_checksum_v2: None,
        affinity_checksum_v2: None,
        tolerations_checksum_v2: None,
        topology_spread_constraints_checksum_v2: None,
        resource_claims_checksum_v2: None,
        os_checksum_v2: None,
        security_context_checksum_v2: None,
        share_process_namespace_checksum_v2: None,
        dns_config_checksum_v2: None,
        host_aliases_checksum_v2: None,
        image_pull_secrets_checksum_v2: None,
        subdomain_checksum_v2: None,
        hostname_checksum_v2: None,
        finalizers_checksum_v2: None,
        owner_references_checksum_v2: None,
        managed_fields_checksum_v2: None,
        resource_version_checksum_v2: None,
        uid_checksum_v2: None,
        namespace_uid_checksum_v2: None,
        deletion_timestamp_checksum_v2: None,
        deletion_grace_period_seconds_checksum_v2: None,
    };

    // Add pod to manager
    manager.add_pod(pod.clone());

    // Wait a tiny bit to ensure async operations complete
    tokio::time::sleep(Duration::from_millis(1)).await;

    // Call getpodbyuid
    let result = manager.getpodbyuid(&uid).await;

    // Assert expected behavior
    assert!(result.is_some());
    let retrieved_pod = result.unwrap();
    assert_eq!(retrieved_pod.uid, uid);
    assert_eq!(retrieved_pod.name, "test-pod");
    assert_eq!(retrieved_pod.namespace, "default");
}
```

## Implementation skeleton

```rust
pub async fn getpodbyuid(&self, uid: &PodUID) -> Option<Pod> {
    todo!("Tier 2")
}
```
