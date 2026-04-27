//! NodePort allocator — parity tests against k8s v1.36.0.
//!
//! Upstream: `pkg/registry/core/service/portallocator/allocator.go`.
//! Default range 30000-32767 — see Kubernetes
//! `cmd/kube-apiserver/app/options/options.go` (DefaultServiceNodePortRange).

use cave_kube_proxy::{
    KubeProxyError, NodePortAllocator, DEFAULT_MAX_NODE_PORT, DEFAULT_MIN_NODE_PORT,
};

const TENANT: &str = "tenant-acme-prod";

/// Cite: Kubernetes `cmd/kube-apiserver/app/options/options.go`
/// (DefaultServiceNodePortRange = `{Base: 30000, Size: 2768}`) — the
/// default NodePort range is exactly 30000-32767 inclusive (2768 ports).
#[test]
fn default_range_is_30000_to_32767_inclusive() {
    let a = NodePortAllocator::new(TENANT);
    assert_eq!(a.min, 30_000);
    assert_eq!(a.max, 32_767);
    assert_eq!(a.capacity(), 2_768);
    assert_eq!(DEFAULT_MIN_NODE_PORT, 30_000);
    assert_eq!(DEFAULT_MAX_NODE_PORT, 32_767);
    assert_eq!(a.tenant_id, TENANT);
}

/// Cite: `pkg/registry/core/service/portallocator/allocator.go:123`
/// (Allocate) — explicit allocation succeeds when the port is in
/// range and free.
#[test]
fn allocate_in_range_succeeds_then_marks_allocated() {
    let mut a = NodePortAllocator::new(TENANT);
    a.allocate(31_080).unwrap();
    assert!(a.has(31_080));
    assert_eq!(a.used(), 1);
    assert_eq!(a.free(), 2_767);
}

/// Cite: `pkg/registry/core/service/portallocator/allocator.go:43`
/// (ErrAllocated) + `:47` (ErrNotInRange).
#[test]
fn allocate_out_of_range_or_double_allocate_errors() {
    let mut a = NodePortAllocator::new(TENANT);
    let err = a.allocate(80).unwrap_err();
    assert!(matches!(err, KubeProxyError::PortNotInRange { port: 80, .. }));

    let err = a.allocate(40_000).unwrap_err();
    assert!(matches!(err, KubeProxyError::PortNotInRange { port: 40_000, .. }));

    a.allocate(30_500).unwrap();
    let err = a.allocate(30_500).unwrap_err();
    assert!(matches!(err, KubeProxyError::PortAlreadyAllocated(30_500)));
}

/// Cite: `pkg/registry/core/service/portallocator/allocator.go:156`
/// (AllocateNext) — auto-allocate yields the lowest free port and
/// then advances on each subsequent call.
#[test]
fn allocate_next_yields_lowest_free_port_in_order() {
    let mut a = NodePortAllocator::new(TENANT);
    let p1 = a.allocate_next().unwrap();
    let p2 = a.allocate_next().unwrap();
    let p3 = a.allocate_next().unwrap();
    assert_eq!(p1, 30_000);
    assert_eq!(p2, 30_001);
    assert_eq!(p3, 30_002);

    a.release(30_001);
    let next = a.allocate_next().unwrap();
    assert_eq!(next, 30_001, "released port re-used as the new lowest free");
}

/// Cite: `pkg/registry/core/service/portallocator/allocator.go:185`
/// (Release) — releasing a previously-allocated port frees the slot;
/// releasing an unallocated port is a no-op (returns `false`).
#[test]
fn release_frees_port_and_unallocated_release_is_noop() {
    let mut a = NodePortAllocator::new(TENANT);
    a.allocate(30_500).unwrap();
    assert!(a.release(30_500));
    assert!(!a.has(30_500));
    assert!(!a.release(30_500), "second release is a no-op");
    assert!(!a.release(31_999), "never-allocated port is also a no-op");
}

/// Cite: `pkg/registry/core/service/portallocator/allocator.go:156`
/// (AllocateNext, ErrFull branch) — once the entire range is
/// exhausted, AllocateNext returns ErrFull (mapped to
/// `PortRangeExhausted` here).
#[test]
fn exhausted_range_returns_port_range_exhausted() {
    let mut a = NodePortAllocator::with_range(TENANT, 30_000, 30_002).unwrap();
    a.allocate_next().unwrap();
    a.allocate_next().unwrap();
    a.allocate_next().unwrap();
    let err = a.allocate_next().unwrap_err();
    assert_eq!(err, KubeProxyError::PortRangeExhausted);
    assert_eq!(a.used(), 3);
    assert_eq!(a.free(), 0);
}
