---
crate: cave-kubelet
upstream_repo: kubernetes/kubernetes
upstream_file: pkg/kubelet/config/config.go
upstream_fn: channelConfig
status: draft
tier: 1
created_at: 2026-04-24T16:39:17.878683+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `pkg/kubelet/config/config.go` → `channelConfig`

## Failing test

```rust
#[tokio::test]
async fn test_channelconfig_with_realistic_inputs() {
    use cave_kubelet::channelconfig;
    use std::time::Duration;

    // Simulate realistic configuration values
    let config = cave_kubelet::config::Config {
        sync_frequency: Duration::from_secs(10),
        file_check_frequency: Duration::from_secs(20),
        http_check_frequency: Duration::from_secs(20),
        manifest_url: Some("http://example.com/manifests".to_string()),
        ..Default::default()
    };

    let (config_channel, _) = channelconfig(&config).await;

    // Verify that the channel is non-blocking and can receive the config
    tokio::time::sleep(Duration::from_millis(100)).await;
    let received = config_channel.try_recv();
    assert!(received.is_ok(), "Config channel should be populated");
    let received_config = received.unwrap();
    assert_eq!(received_config.sync_frequency, Duration::from_secs(10));
    assert_eq!(received_config.file_check_frequency, Duration::from_secs(20));
    assert_eq!(received_config.http_check_frequency, Duration::from_secs(20));
    assert_eq!(received_config.manifest_url, Some("http://example.com/manifests".to_string()));
}
```

## Implementation skeleton

```rust
pub async fn channelconfig(config: &cave_kubelet::config::Config) -> (tokio::sync::watch::Sender<cave_kubelet::config::Config>, tokio::sync::watch::Receiver<cave_kubelet::config::Config>) {
    todo!("Tier 2")
}
```
