---
crate: cave-kubelet
upstream_repo: kubernetes/kubernetes
upstream_file: pkg/kubelet/prober/worker.go
upstream_fn: doProbe
status: draft
tier: 1
created_at: 2026-04-24T17:32:10.004024+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `pkg/kubelet/prober/worker.go` → `doProbe`

## Failing test

```rust
#[tokio::test]
async fn test_doprobe_success() {
    use cave_kubelet::prober::{Probe, ProbeResult, Prober};
    use std::time::Duration;
    use tokio::sync::mpsc;

    // Create a mock prober that always returns success
    struct MockProber;
    #[async_trait::async_trait]
    impl Prober for MockProber {
        async fn probe(&self, _pod: &str, _container: &str, _probe: &Probe) -> ProbeResult {
            ProbeResult::Success
        }
    }

    let (tx, mut rx) = mpsc::channel::<ProbeResult>(1);
    let probe = Probe::default();
    let pod = "test-pod";
    let container = "test-container";

    // Run doprobe in background
    let handle = tokio::spawn(async move {
        cave_kubelet::doprobe(
            Box::new(MockProber),
            pod,
            container,
            &probe,
            tx,
            Duration::from_secs(1),
        )
        .await
    });

    // Wait for result
    let result = rx.recv().await;
    handle.await.unwrap();

    assert_eq!(result, Some(ProbeResult::Success));
}
```

## Implementation skeleton

```rust
pub async fn doprobe(
    prober: Box<dyn Prober + Send + Sync>,
    pod: &str,
    container: &str,
    probe: &Probe,
    result_tx: tokio::sync::mpsc::Sender<ProbeResult>,
    timeout: std::time::Duration,
) {
    todo!("Tier 2");
}
```
