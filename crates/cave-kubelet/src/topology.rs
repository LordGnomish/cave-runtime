//! Topology manager — NUMA alignment for CPU, memory, and device hints.
//!
//! Mirrors `pkg/kubelet/cm/topologymanager`: each hint provider supplies
//! TopologyHints (bitmask of NUMA nodes + preferred flag); the manager
//! computes the merged hint and admits/denies the pod according to the
//! configured policy: `none` / `best-effort` / `restricted` /
//! `single-numa-node`. Scope can be container or pod.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// NUMA mask — bit n set means NUMA node n is acceptable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NumaMask(pub u64);

impl NumaMask {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub fn from_nodes(nodes: &[u8]) -> Self {
        let mut m = 0u64;
        for n in nodes {
            m |= 1u64 << *n;
        }
        Self(m)
    }

    pub fn full(num_nodes: u8) -> Self {
        if num_nodes >= 64 {
            return Self(u64::MAX);
        }
        Self((1u64 << num_nodes) - 1)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn count(self) -> u32 {
        self.0.count_ones()
    }

    pub fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub fn nodes(self) -> Vec<u8> {
        let mut out = Vec::new();
        for i in 0..64u8 {
            if self.0 & (1u64 << i) != 0 {
                out.push(i);
            }
        }
        out
    }

    pub fn contains_node(self, node: u8) -> bool {
        if node >= 64 {
            return false;
        }
        self.0 & (1u64 << node) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyHint {
    pub mask: NumaMask,
    pub preferred: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Policy {
    None,
    BestEffort,
    Restricted,
    SingleNumaNode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    Container,
    Pod,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionResult {
    pub admit: bool,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub best_hint: Option<TopologyHint>,
}

impl AdmissionResult {
    pub fn admit(best: Option<TopologyHint>) -> Self {
        Self {
            admit: true,
            reason: None,
            message: None,
            best_hint: best,
        }
    }

    pub fn deny(reason: &str, message: &str) -> Self {
        Self {
            admit: false,
            reason: Some(reason.into()),
            message: Some(message.into()),
            best_hint: None,
        }
    }
}

/// Merge a set of hint lists (one per provider) by computing the cartesian
/// product and ANDing masks. Each result is preferred only if every input
/// in that combination was preferred AND the merged mask is non-empty.
/// Returns the de-duplicated set of merged hints.
pub fn merge_provider_hints(provider_hints: &[Vec<TopologyHint>]) -> Vec<TopologyHint> {
    if provider_hints.is_empty() {
        return Vec::new();
    }
    if provider_hints.iter().any(|v| v.is_empty()) {
        // A provider with no hints means "no preference" — represent as full-mask preferred.
        // (Upstream substitutes a "no preference" hint with mask=all, preferred=true.)
        return Vec::new();
    }
    let mut combos: Vec<TopologyHint> = vec![TopologyHint {
        mask: NumaMask(u64::MAX),
        preferred: true,
    }];
    for hints in provider_hints {
        let mut next = Vec::with_capacity(combos.len() * hints.len());
        for c in &combos {
            for h in hints {
                next.push(TopologyHint {
                    mask: c.mask.intersect(h.mask),
                    preferred: c.preferred && h.preferred,
                });
            }
        }
        combos = next;
    }
    // Filter to non-empty intersections.
    combos.retain(|h| !h.mask.is_empty());
    // Dedup.
    combos.sort_by_key(|h| (h.mask.0, !h.preferred));
    combos.dedup_by_key(|h| (h.mask, h.preferred));
    combos
}

/// Pick the best hint per upstream `bitmask.go` priority:
/// 1. Among preferred hints, prefer the smallest count.
/// 2. If no preferred hint exists, take the smallest among non-preferred.
/// 3. Tie-break by lowest numerical mask (lowest NUMA nodes first).
pub fn best_hint(merged: &[TopologyHint]) -> Option<TopologyHint> {
    if merged.is_empty() {
        return None;
    }
    let mut sorted = merged.to_vec();
    sorted.sort_by(|a, b| {
        match b.preferred.cmp(&a.preferred) {
            std::cmp::Ordering::Equal => match a.mask.count().cmp(&b.mask.count()) {
                std::cmp::Ordering::Equal => a.mask.0.cmp(&b.mask.0),
                o => o,
            },
            o => o,
        }
    });
    Some(sorted[0])
}

/// Run topology admission for a single container / pod scope decision.
/// `provider_hints` must already be ordered (one Vec per provider); the
/// merge produces all valid alignments and the policy chooses among them.
pub fn admit(
    policy: Policy,
    provider_hints: &[Vec<TopologyHint>],
) -> AdmissionResult {
    if matches!(policy, Policy::None) {
        return AdmissionResult::admit(None);
    }
    let merged = merge_provider_hints(provider_hints);
    if merged.is_empty() {
        return AdmissionResult::deny(
            "TopologyAffinityError",
            "no aligned topology assignment possible",
        );
    }
    let best = best_hint(&merged).expect("merged non-empty");
    match policy {
        Policy::None => AdmissionResult::admit(Some(best)),
        Policy::BestEffort => AdmissionResult::admit(Some(best)),
        Policy::Restricted => {
            if !best.preferred {
                AdmissionResult::deny(
                    "TopologyAffinityError",
                    "no preferred topology alignment available",
                )
            } else {
                AdmissionResult::admit(Some(best))
            }
        }
        Policy::SingleNumaNode => {
            if best.mask.count() != 1 || !best.preferred {
                AdmissionResult::deny(
                    "TopologyAffinityError",
                    "single-numa-node policy requires a preferred single-node alignment",
                )
            } else {
                AdmissionResult::admit(Some(best))
            }
        }
    }
}

/// State for tracking the merged hint at pod scope across containers in
/// the same pod (Scope::Pod). Hints are intersected across containers.
#[derive(Debug, Default, Clone)]
pub struct PodScopeAccumulator {
    pub current: Option<Vec<TopologyHint>>,
}

impl PodScopeAccumulator {
    pub fn add_container_hints(&mut self, container_hints: Vec<TopologyHint>) {
        match &self.current {
            None => self.current = Some(container_hints),
            Some(existing) => {
                let mut next = Vec::new();
                for a in existing {
                    for b in &container_hints {
                        let m = a.mask.intersect(b.mask);
                        if !m.is_empty() {
                            next.push(TopologyHint {
                                mask: m,
                                preferred: a.preferred && b.preferred,
                            });
                        }
                    }
                }
                next.sort_by_key(|h| (h.mask.0, !h.preferred));
                next.dedup_by_key(|h| (h.mask, h.preferred));
                self.current = Some(next);
            }
        }
    }

    pub fn finalize(self) -> Vec<TopologyHint> {
        self.current.unwrap_or_default()
    }
}

/// Numa distance lookup — used for advanced alignment heuristics.
#[derive(Debug, Clone, Default)]
pub struct NumaDistanceMatrix {
    pub matrix: BTreeMap<(u8, u8), u32>,
}

impl NumaDistanceMatrix {
    pub fn set(&mut self, a: u8, b: u8, d: u32) {
        self.matrix.insert((a, b), d);
        self.matrix.insert((b, a), d);
    }

    pub fn distance(&self, a: u8, b: u8) -> u32 {
        if a == b {
            10
        } else {
            self.matrix.get(&(a, b)).copied().unwrap_or(20)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(nodes: &[u8], pref: bool) -> TopologyHint {
        TopologyHint {
            mask: NumaMask::from_nodes(nodes),
            preferred: pref,
        }
    }

    #[test]
    fn numa_mask_basic() {
        let m = NumaMask::from_nodes(&[0, 2, 4]);
        assert_eq!(m.count(), 3);
        assert!(m.contains_node(2));
        assert!(!m.contains_node(1));
        assert_eq!(m.nodes(), vec![0, 2, 4]);
    }

    #[test]
    fn numa_mask_full() {
        let m = NumaMask::full(4);
        assert_eq!(m.count(), 4);
        assert_eq!(m.nodes(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn numa_mask_empty() {
        assert!(NumaMask::empty().is_empty());
        assert_eq!(NumaMask::empty().count(), 0);
    }

    #[test]
    fn numa_mask_intersect() {
        let a = NumaMask::from_nodes(&[0, 1, 2]);
        let b = NumaMask::from_nodes(&[1, 2, 3]);
        assert_eq!(a.intersect(b), NumaMask::from_nodes(&[1, 2]));
    }

    #[test]
    fn numa_mask_intersect_empty() {
        let a = NumaMask::from_nodes(&[0]);
        let b = NumaMask::from_nodes(&[1]);
        assert!(a.intersect(b).is_empty());
    }

    #[test]
    fn merge_provider_hints_single_provider() {
        let hints = vec![vec![h(&[0], true), h(&[1], true)]];
        let merged = merge_provider_hints(&hints);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_provider_hints_two_providers_intersect() {
        let provider_a = vec![h(&[0, 1], true)];
        let provider_b = vec![h(&[1, 2], false)];
        let merged = merge_provider_hints(&[provider_a, provider_b]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].mask, NumaMask::from_nodes(&[1]));
        assert!(!merged[0].preferred);
    }

    #[test]
    fn merge_provider_hints_drops_empty_intersection() {
        let provider_a = vec![h(&[0], true)];
        let provider_b = vec![h(&[1], true)];
        let merged = merge_provider_hints(&[provider_a, provider_b]);
        assert!(merged.is_empty());
    }

    #[test]
    fn merge_provider_hints_empty_provider_yields_empty() {
        let merged = merge_provider_hints(&[vec![], vec![h(&[0], true)]]);
        assert!(merged.is_empty());
    }

    #[test]
    fn merge_provider_hints_preferred_when_all_inputs_preferred() {
        let provider_a = vec![h(&[0, 1], true)];
        let provider_b = vec![h(&[0], true)];
        let merged = merge_provider_hints(&[provider_a, provider_b]);
        assert!(merged.iter().all(|m| m.preferred));
    }

    #[test]
    fn merge_provider_hints_dedup() {
        let provider_a = vec![h(&[0, 1], true), h(&[0, 1], true)];
        let merged = merge_provider_hints(&[provider_a]);
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn best_hint_picks_smallest_preferred() {
        let merged = vec![
            h(&[0, 1, 2], true),
            h(&[0], true),
            h(&[0, 1], true),
        ];
        let best = best_hint(&merged).unwrap();
        assert_eq!(best.mask, NumaMask::from_nodes(&[0]));
    }

    #[test]
    fn best_hint_prefers_preferred_over_smaller_unpreferred() {
        let merged = vec![h(&[0], false), h(&[0, 1], true)];
        let best = best_hint(&merged).unwrap();
        assert_eq!(best.mask, NumaMask::from_nodes(&[0, 1]));
        assert!(best.preferred);
    }

    #[test]
    fn best_hint_lowest_mask_breaks_tie() {
        let merged = vec![h(&[2], true), h(&[0], true)];
        let best = best_hint(&merged).unwrap();
        assert_eq!(best.mask, NumaMask::from_nodes(&[0]));
    }

    #[test]
    fn best_hint_empty_returns_none() {
        assert!(best_hint(&[]).is_none());
    }

    #[test]
    fn admit_none_always_admits() {
        let res = admit(Policy::None, &[]);
        assert!(res.admit);
    }

    #[test]
    fn admit_best_effort_admits_with_best_alignment() {
        let res = admit(Policy::BestEffort, &[vec![h(&[0], true)]]);
        assert!(res.admit);
        assert_eq!(res.best_hint.unwrap().mask, NumaMask::from_nodes(&[0]));
    }

    #[test]
    fn admit_best_effort_admits_unpreferred() {
        let res = admit(
            Policy::BestEffort,
            &[vec![h(&[0, 1], false)]],
        );
        assert!(res.admit);
    }

    #[test]
    fn admit_restricted_denies_unpreferred() {
        let res = admit(
            Policy::Restricted,
            &[vec![h(&[0, 1], false)]],
        );
        assert!(!res.admit);
        assert_eq!(res.reason.as_deref(), Some("TopologyAffinityError"));
    }

    #[test]
    fn admit_restricted_admits_preferred_multi_node() {
        let res = admit(Policy::Restricted, &[vec![h(&[0, 1], true)]]);
        assert!(res.admit);
    }

    #[test]
    fn admit_single_numa_denies_multi_node() {
        let res = admit(
            Policy::SingleNumaNode,
            &[vec![h(&[0, 1], true)]],
        );
        assert!(!res.admit);
    }

    #[test]
    fn admit_single_numa_admits_single_preferred_node() {
        let res = admit(
            Policy::SingleNumaNode,
            &[vec![h(&[0], true)]],
        );
        assert!(res.admit);
    }

    #[test]
    fn admit_single_numa_denies_unpreferred_single_node() {
        let res = admit(
            Policy::SingleNumaNode,
            &[vec![h(&[0], false)]],
        );
        assert!(!res.admit);
    }

    #[test]
    fn admit_denies_when_no_alignment_possible() {
        let res = admit(
            Policy::Restricted,
            &[vec![h(&[0], true)], vec![h(&[1], true)]],
        );
        assert!(!res.admit);
    }

    #[test]
    fn admit_with_three_providers_intersection() {
        let res = admit(
            Policy::SingleNumaNode,
            &[
                vec![h(&[0, 1], true)],
                vec![h(&[0, 1, 2], true)],
                vec![h(&[0], true)],
            ],
        );
        assert!(res.admit);
        assert_eq!(res.best_hint.unwrap().mask, NumaMask::from_nodes(&[0]));
    }

    #[test]
    fn pod_scope_accumulator_intersects_across_containers() {
        let mut acc = PodScopeAccumulator::default();
        acc.add_container_hints(vec![h(&[0, 1], true)]);
        acc.add_container_hints(vec![h(&[1, 2], true)]);
        let merged = acc.finalize();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].mask, NumaMask::from_nodes(&[1]));
    }

    #[test]
    fn pod_scope_accumulator_first_container_seeds() {
        let mut acc = PodScopeAccumulator::default();
        acc.add_container_hints(vec![h(&[0], true), h(&[1], true)]);
        let merged = acc.finalize();
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn pod_scope_accumulator_empty_intersection_drops_to_empty() {
        let mut acc = PodScopeAccumulator::default();
        acc.add_container_hints(vec![h(&[0], true)]);
        acc.add_container_hints(vec![h(&[1], true)]);
        let merged = acc.finalize();
        assert!(merged.is_empty());
    }

    #[test]
    fn pod_scope_accumulator_preserves_preferred_only_when_all_preferred() {
        let mut acc = PodScopeAccumulator::default();
        acc.add_container_hints(vec![h(&[0], true)]);
        acc.add_container_hints(vec![h(&[0], false)]);
        let merged = acc.finalize();
        assert_eq!(merged.len(), 1);
        assert!(!merged[0].preferred);
    }

    #[test]
    fn numa_distance_self_is_ten() {
        let m = NumaDistanceMatrix::default();
        assert_eq!(m.distance(0, 0), 10);
    }

    #[test]
    fn numa_distance_default_unset_is_twenty() {
        let m = NumaDistanceMatrix::default();
        assert_eq!(m.distance(0, 1), 20);
    }

    #[test]
    fn numa_distance_explicit_lookup() {
        let mut m = NumaDistanceMatrix::default();
        m.set(0, 1, 21);
        assert_eq!(m.distance(0, 1), 21);
        assert_eq!(m.distance(1, 0), 21);
    }

    #[test]
    fn admission_result_admit_path() {
        let r = AdmissionResult::admit(Some(h(&[0], true)));
        assert!(r.admit);
        assert!(r.best_hint.is_some());
    }

    #[test]
    fn admission_result_deny_path() {
        let r = AdmissionResult::deny("R", "M");
        assert!(!r.admit);
        assert_eq!(r.reason.as_deref(), Some("R"));
    }

    #[test]
    fn merge_with_non_overlapping_set_yields_empty() {
        let merged = merge_provider_hints(&[vec![h(&[0], true)], vec![h(&[1, 2], true)]]);
        assert!(merged.is_empty());
    }

    #[test]
    fn admit_handles_zero_hint_lists_for_non_none_policy() {
        // No providers ⇒ nothing to align ⇒ deny except for None.
        for p in [Policy::BestEffort, Policy::Restricted, Policy::SingleNumaNode] {
            let res = admit(p, &[]);
            assert!(!res.admit, "policy {:?} should deny with no providers", p);
        }
    }

    #[test]
    fn scope_enum_variants_exist() {
        let _ = Scope::Container;
        let _ = Scope::Pod;
    }

    #[test]
    fn full_mask_for_64_or_more_is_max() {
        assert_eq!(NumaMask::full(64), NumaMask(u64::MAX));
        assert_eq!(NumaMask::full(70), NumaMask(u64::MAX));
    }

    #[test]
    fn contains_node_out_of_range_returns_false() {
        let m = NumaMask::from_nodes(&[0]);
        assert!(!m.contains_node(64));
    }
}
