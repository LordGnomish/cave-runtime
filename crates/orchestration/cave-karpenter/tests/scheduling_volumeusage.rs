// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of the pure, k8s-client-independent core of
// pkg/scheduling/volumeusage.go from kubernetes-sigs/karpenter v1.12.1
// (sha ed490e8): the `Volumes` set-map (Add/Union/Insert) plus the
// per-node `VolumeUsage` limit tracker (NewVolumeUsage/AddLimit/Add/
// ExceedsLimits/DeletePod).
//
// The k8s-client-bound resolvers (GetVolumes / ResolveDriver / driverFromSC /
// driverFromVolume) are scope-cut per ADR-RUNTIME-KARPENTER-CLOUD-AGNOSTIC-001
// — they require a live controller-runtime client + CSI translation lib and
// carry no cloud-agnostic behaviour. The reservation/limit math below is the
// portable kernel the scheduler relies on.

use cave_karpenter::scheduling::volumeusage::{VolumeUsage, Volumes};

// ---- Volumes (map[provisioner] -> set[pvcID]) --------------------------------

#[test]
fn add_creates_set_and_inserts() {
    let mut v = Volumes::new();
    v.add("ebs.csi.aws.com", "default/pvc-1");
    assert_eq!(v.count("ebs.csi.aws.com"), 1);
    assert!(v.contains("ebs.csi.aws.com", "default/pvc-1"));
}

#[test]
fn add_is_set_idempotent() {
    let mut v = Volumes::new();
    v.add("ebs.csi.aws.com", "default/pvc-1");
    v.add("ebs.csi.aws.com", "default/pvc-1");
    assert_eq!(v.count("ebs.csi.aws.com"), 1, "duplicate pvcID collapses");
}

#[test]
fn add_tracks_distinct_pvcs_per_driver() {
    let mut v = Volumes::new();
    v.add("ebs.csi.aws.com", "default/pvc-1");
    v.add("ebs.csi.aws.com", "default/pvc-2");
    v.add("efs.csi.aws.com", "default/pvc-3");
    assert_eq!(v.count("ebs.csi.aws.com"), 2);
    assert_eq!(v.count("efs.csi.aws.com"), 1);
}

#[test]
fn union_is_non_mutating_and_merges() {
    let mut a = Volumes::new();
    a.add("d1", "a");
    let mut b = Volumes::new();
    b.add("d1", "b");
    b.add("d2", "c");
    let u = a.union(&b);
    // union result merges both
    assert_eq!(u.count("d1"), 2);
    assert_eq!(u.count("d2"), 1);
    // originals untouched
    assert_eq!(a.count("d1"), 1);
    assert_eq!(a.count("d2"), 0);
    assert_eq!(b.count("d1"), 1);
}

#[test]
fn insert_mutates_self() {
    let mut a = Volumes::new();
    a.add("d1", "a");
    let mut b = Volumes::new();
    b.add("d1", "b");
    b.add("d2", "c");
    a.insert(&b);
    assert_eq!(a.count("d1"), 2);
    assert_eq!(a.count("d2"), 1);
}

// ---- VolumeUsage -------------------------------------------------------------

#[test]
fn exceeds_limits_ok_when_no_limit_registered() {
    let mut u = VolumeUsage::new();
    let mut vols = Volumes::new();
    vols.add("ebs.csi.aws.com", "pvc-1");
    vols.add("ebs.csi.aws.com", "pvc-2");
    // no AddLimit call → unlimited → never exceeds
    assert!(u_exceeds(&mut u, &vols).is_ok());
}

#[test]
fn exceeds_limits_ok_at_exactly_the_limit() {
    // upstream guard is `len(volumes) > limit` (strict) — equal is allowed.
    let mut u = VolumeUsage::new();
    u.add_limit("ebs.csi.aws.com", 2);
    let mut vols = Volumes::new();
    vols.add("ebs.csi.aws.com", "pvc-1");
    vols.add("ebs.csi.aws.com", "pvc-2");
    assert!(u.exceeds_limits(&vols).is_ok());
}

#[test]
fn exceeds_limits_err_over_the_limit() {
    let mut u = VolumeUsage::new();
    u.add_limit("ebs.csi.aws.com", 1);
    let mut vols = Volumes::new();
    vols.add("ebs.csi.aws.com", "pvc-1");
    vols.add("ebs.csi.aws.com", "pvc-2");
    let err = u.exceeds_limits(&vols).expect_err("2 > 1 must exceed");
    assert_eq!(err.provisioner, "ebs.csi.aws.com");
    assert_eq!(err.volume_count, 2);
    assert_eq!(err.volume_limit, 1);
}

#[test]
fn exceeds_limits_accounts_for_already_added_usage() {
    // existing usage unions with the candidate volumes before comparing
    let mut u = VolumeUsage::new();
    u.add_limit("ebs.csi.aws.com", 2);
    let mut existing = Volumes::new();
    existing.add("ebs.csi.aws.com", "pvc-1");
    existing.add("ebs.csi.aws.com", "pvc-2");
    u.add("ns/pod-a", existing);

    let mut candidate = Volumes::new();
    candidate.add("ebs.csi.aws.com", "pvc-3");
    // 3 distinct > 2 → exceed
    assert!(u.exceeds_limits(&candidate).is_err());
}

#[test]
fn delete_pod_recomputes_usage_from_remaining_pods() {
    let mut u = VolumeUsage::new();
    let mut a = Volumes::new();
    a.add("d", "pvc-1");
    let mut b = Volumes::new();
    b.add("d", "pvc-2");
    u.add("ns/pod-a", a);
    u.add("ns/pod-b", b);
    assert_eq!(u.volumes().count("d"), 2);

    u.delete_pod("ns/pod-a");
    assert_eq!(u.volumes().count("d"), 1);
    assert!(u.volumes().contains("d", "pvc-2"));
    assert!(!u.volumes().contains("d", "pvc-1"));
}

#[test]
fn delete_pod_preserves_pvc_shared_by_another_pod() {
    // both pods reference the same pvcID under the same driver; deleting one
    // must keep the pvc because the survivor still mounts it.
    let mut u = VolumeUsage::new();
    let mut a = Volumes::new();
    a.add("d", "shared");
    let mut b = Volumes::new();
    b.add("d", "shared");
    u.add("ns/pod-a", a);
    u.add("ns/pod-b", b);
    assert_eq!(u.volumes().count("d"), 1);

    u.delete_pod("ns/pod-a");
    assert_eq!(u.volumes().count("d"), 1);
    assert!(u.volumes().contains("d", "shared"));
}

// helper that borrows mutably only to satisfy lifetimes in the no-limit case
fn u_exceeds(u: &mut VolumeUsage, vols: &Volumes) -> Result<(), cave_karpenter::scheduling::volumeusage::VolumeLimitExceeded> {
    u.exceeds_limits(vols)
}
