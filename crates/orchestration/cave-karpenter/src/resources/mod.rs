// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of pkg/utils/resources from kubernetes-sigs/karpenter v1.12.1 (sha
// ed490e8) plus the sidecar-aware pod-request algorithm it delegates to
// (k8s.io/component-helpers/resource). Apache-2.0 upstream; see NOTICE.
//
//! Resource arithmetic over `ResourceList` (a map of resource-name → quantity),
//! and the effective pod-request computation (`Ceiling`) that accounts for
//! regular containers, restartable sidecar init containers, sequential init
//! containers, and pod overhead.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

/// A Kubernetes resource quantity, stored as an exact integer count of
/// nano-units (1e-9). This represents both decimal SI ("100m", "2", "1k") and
/// binary SI ("1Gi", "1024Mi") forms exactly for the magnitudes used in pod
/// scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Quantity {
    nano: i128,
}

impl Quantity {
    /// `resource.MustParse` — parse a quantity string. Unparseable input
    /// yields zero (upstream panics on invalid; we are lenient as inputs are
    /// pre-validated elsewhere).
    pub fn parse(value: &str) -> Quantity {
        let s = value.trim();
        let (mant_str, num, den) = split_suffix(s);

        let neg = mant_str.starts_with('-');
        let mant_str = mant_str.trim_start_matches(['+', '-']);
        let (int_part, frac_part) = match mant_str.split_once('.') {
            Some((a, b)) => (a, b),
            None => (mant_str, ""),
        };
        let digits = format!("{int_part}{frac_part}");
        let mant: i128 = if digits.is_empty() {
            0
        } else {
            digits.parse().unwrap_or(0)
        };
        let scale = frac_part.len() as u32;

        let mut nano = mant * num * 1_000_000_000i128 / (den * 10i128.pow(scale));
        if neg {
            nano = -nano;
        }
        Quantity { nano }
    }

    pub fn from_nano(nano: i128) -> Quantity {
        Quantity { nano }
    }

    pub fn add(&self, other: &Quantity) -> Quantity {
        Quantity {
            nano: self.nano + other.nano,
        }
    }

    pub fn sub(&self, other: &Quantity) -> Quantity {
        Quantity {
            nano: self.nano - other.nano,
        }
    }

    pub fn cmp_to(&self, other: &Quantity) -> Ordering {
        self.nano.cmp(&other.nano)
    }

    pub fn is_zero(&self) -> bool {
        self.nano == 0
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Concise decimal rendering: whole units when integral, else milli.
        if self.nano % 1_000_000_000 == 0 {
            write!(f, "{}", self.nano / 1_000_000_000)
        } else if self.nano % 1_000_000 == 0 {
            write!(f, "{}m", self.nano / 1_000_000)
        } else {
            write!(f, "{}n", self.nano)
        }
    }
}

/// Split a quantity string into its mantissa and SI multiplier (numer/denom).
fn split_suffix(s: &str) -> (&str, i128, i128) {
    const BIN: &[(&str, i128)] = &[
        ("Ki", 1 << 10),
        ("Mi", 1 << 20),
        ("Gi", 1 << 30),
        ("Ti", 1 << 40),
        ("Pi", 1 << 50),
        ("Ei", 1 << 60),
    ];
    for (suf, f) in BIN {
        if s.ends_with(suf) {
            return (&s[..s.len() - suf.len()], *f, 1);
        }
    }
    const DEC: &[(&str, i128, i128)] = &[
        ("n", 1, 1_000_000_000),
        ("u", 1, 1_000_000),
        ("m", 1, 1_000),
        ("k", 1_000, 1),
        ("M", 1_000_000, 1),
        ("G", 1_000_000_000, 1),
        ("T", 1_000_000_000_000, 1),
        ("P", 1_000_000_000_000_000, 1),
        ("E", 1_000_000_000_000_000_000, 1),
    ];
    for (suf, n, d) in DEC {
        if s.ends_with(suf) {
            return (&s[..s.len() - suf.len()], *n, *d);
        }
    }
    (s, 1, 1)
}

/// A map of resource-name → quantity (`corev1.ResourceList`).
pub type ResourceList = BTreeMap<String, Quantity>;

/// `Merge` — sum a list of resource lists into a single list.
pub fn merge(lists: &[ResourceList]) -> ResourceList {
    let mut result = ResourceList::new();
    for list in lists {
        merge_into(&mut result, list);
    }
    result
}

/// `MergeInto` — sum `src` into `dest` in place.
pub fn merge_into(dest: &mut ResourceList, src: &ResourceList) {
    for (name, q) in src {
        let current = dest.get(name).copied().unwrap_or_default();
        dest.insert(name.clone(), current.add(q));
    }
}

/// `Subtract` — `lhs - rhs`, per resource. Missing rhs keys leave lhs intact.
pub fn subtract(lhs: &ResourceList, rhs: &ResourceList) -> ResourceList {
    let mut result = lhs.clone();
    for (name, q) in lhs {
        if let Some(rhs_value) = rhs.get(name) {
            result.insert(name.clone(), q.sub(rhs_value));
        }
    }
    result
}

/// `MaxResources` — component-wise maximum across the lists.
pub fn max_resources(lists: &[ResourceList]) -> ResourceList {
    let mut result = ResourceList::new();
    for list in lists {
        max_into(&mut result, list);
    }
    result
}

fn max_into(dest: &mut ResourceList, src: &ResourceList) {
    for (name, q) in src {
        match dest.get(name) {
            Some(existing) if existing.cmp_to(q) != Ordering::Less => {}
            _ => {
                dest.insert(name.clone(), *q);
            }
        }
    }
}

pub fn is_zero(q: &Quantity) -> bool {
    q.is_zero()
}

pub fn cmp(lhs: &Quantity, rhs: &Quantity) -> Ordering {
    lhs.cmp_to(rhs)
}

/// `Fits` — every candidate resource is ≤ the corresponding total. A negative
/// total never fits.
pub fn fits(candidate: &ResourceList, total: &ResourceList) -> bool {
    let zero = Quantity::default();
    for q in total.values() {
        if zero.cmp_to(q) == Ordering::Greater {
            return false;
        }
    }
    for (name, q) in candidate {
        let avail = total.get(name).copied().unwrap_or_default();
        if q.cmp_to(&avail) == Ordering::Greater {
            return false;
        }
    }
    true
}

// ---- Pod model + Ceiling (effective pod requests/limits) ----

/// A container's resource requirements. `restart_policy_always` marks a
/// restartable init container (a "sidecar").
#[derive(Debug, Clone, Default)]
pub struct Container {
    pub requests: ResourceList,
    pub limits: ResourceList,
    pub restart_policy_always: bool,
}

/// Minimal pod shape for resource computation.
#[derive(Debug, Clone, Default)]
pub struct Pod {
    pub containers: Vec<Container>,
    pub init_containers: Vec<Container>,
    pub overhead: ResourceList,
}

/// Effective requests + limits, computed with the same logic as the scheduler.
#[derive(Debug, Clone, Default)]
pub struct ResourceRequirements {
    pub requests: ResourceList,
    pub limits: ResourceList,
}

/// `Ceiling` — effective resource requirements for a pod.
pub fn ceiling(pod: &Pod) -> ResourceRequirements {
    ResourceRequirements {
        requests: effective(pod, |c| &c.requests),
        limits: effective(pod, |c| &c.limits),
    }
}

/// `PodRequests` (requests selector) / `PodLimits` (limits selector). Sums the
/// regular containers, then folds in init containers: restartable sidecars
/// accumulate into the running total *and* the init-peak, while sequential
/// init containers peak at their own request plus the running sidecar total.
fn effective(pod: &Pod, sel: impl Fn(&Container) -> &ResourceList) -> ResourceList {
    let mut reqs = merge(&pod.containers.iter().map(|c| sel(c).clone()).collect::<Vec<_>>());
    let mut restartable = ResourceList::new();
    let mut init_max = ResourceList::new();

    for ic in &pod.init_containers {
        let creqs = sel(ic);
        if ic.restart_policy_always {
            merge_into(&mut reqs, creqs);
            merge_into(&mut restartable, creqs);
            max_into(&mut init_max, &restartable);
        } else {
            let mut tmp = creqs.clone();
            merge_into(&mut tmp, &restartable);
            max_into(&mut init_max, &tmp);
        }
    }
    max_into(&mut reqs, &init_max);
    merge_into(&mut reqs, &pod.overhead);
    reqs
}
