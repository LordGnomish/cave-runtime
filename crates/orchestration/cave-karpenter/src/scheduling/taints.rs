// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Taint / toleration matching — port of `pkg/scheduling/taints.go` from
//! kubernetes-sigs/karpenter v1.12.1 (sha ed490e8).
//!
//! Karpenter wraps `[]corev1.Taint` as `Taints` and asks whether a pod's
//! tolerations cover every taint a candidate node would carry. The matcher
//! reproduces the upstream-kubernetes semantics that karpenter leans on
//! verbatim:
//!   * `(*Toleration).ToleratesTaint` (k8s.io/api/core/v1) — effect/key/
//!     operator matching, with empty effect/key acting as wildcards.
//!   * `(*Taint).MatchTaint` — key+effect equality (value-insensitive),
//!     used by `Merge` to avoid appending duplicate taints.
//!
//! `KnownEphemeralTaints` lists the taints expected on an initializing node;
//! the disruption/provisioning controllers ignore them while a
//! Karpenter-managed node is still uninitialized.

use std::fmt;
use std::sync::LazyLock;

/// Node taint effect. Mirrors `corev1.TaintEffect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    NoSchedule,
    PreferNoSchedule,
    NoExecute,
}

impl fmt::Display for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Effect::NoSchedule => "NoSchedule",
            Effect::PreferNoSchedule => "PreferNoSchedule",
            Effect::NoExecute => "NoExecute",
        };
        f.write_str(s)
    }
}

/// Toleration match operator. Mirrors `corev1.TolerationOperator`; an empty
/// operator upstream defaults to `Equal`, which is the default here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Operator {
    #[default]
    Equal,
    Exists,
}

/// A node taint. Mirrors `corev1.Taint`. `value` is optional (absent == the
/// empty string upstream).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taint {
    pub key: String,
    pub value: Option<String>,
    pub effect: Effect,
}

impl Taint {
    fn value_str(&self) -> &str {
        self.value.as_deref().unwrap_or("")
    }

    /// `(*Taint).MatchTaint`: two taints match when key and effect are equal.
    /// Value is intentionally ignored.
    pub fn matches_taint(&self, other: &Taint) -> bool {
        self.key == other.key && self.effect == other.effect
    }
}

impl fmt::Display for Taint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // mirrors pretty.Taint(taint) used in upstream error wrapping
        match &self.value {
            Some(v) if !v.is_empty() => write!(f, "{}={}:{}", self.key, v, self.effect),
            _ => write!(f, "{}:{}", self.key, self.effect),
        }
    }
}

/// A pod/node-affinity toleration. Mirrors `corev1.Toleration`. `key` absent
/// means "match any key"; `effect` absent means "match any effect".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toleration {
    pub key: Option<String>,
    pub operator: Operator,
    pub value: Option<String>,
    pub effect: Option<Effect>,
}

impl Toleration {
    fn value_str(&self) -> &str {
        self.value.as_deref().unwrap_or("")
    }

    /// `(*Toleration).ToleratesTaint` (upstream k8s):
    ///   * a non-empty toleration effect must equal the taint effect;
    ///   * a non-empty toleration key must equal the taint key;
    ///   * `Exists` then matches; `Equal` requires value equality.
    pub fn tolerates_taint(&self, taint: &Taint) -> bool {
        if let Some(effect) = self.effect {
            if effect != taint.effect {
                return false;
            }
        }
        if let Some(key) = &self.key {
            if key != &taint.key {
                return false;
            }
        }
        match self.operator {
            Operator::Equal => self.value_str() == taint.value_str(),
            Operator::Exists => true,
        }
    }
}

/// Raised by [`Taints::tolerates`] listing the taints no toleration covered.
/// Mirrors the multierr aggregation upstream returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaintIntolerance {
    pub untolerated: Vec<Taint>,
}

impl fmt::Display for TaintIntolerance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "did not tolerate taint(s): ")?;
        for (i, t) in self.untolerated.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{t}")?;
        }
        Ok(())
    }
}

impl std::error::Error for TaintIntolerance {}

/// Decorated alias for `[]corev1.Taint`. Mirrors upstream `Taints`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Taints(Vec<Taint>);

impl From<Vec<Taint>> for Taints {
    fn from(v: Vec<Taint>) -> Self {
        Taints(v)
    }
}

impl Taints {
    /// Iterate the underlying taints.
    pub fn iter(&self) -> std::slice::Iter<'_, Taint> {
        self.0.iter()
    }

    /// `ToleratesPod`: a pod tolerates the taints iff its tolerations do.
    pub fn tolerates_pod(&self, tolerations: &[Toleration]) -> Result<(), TaintIntolerance> {
        self.tolerates(tolerations)
    }

    /// `Tolerates`: `Ok` iff every taint is tolerated by at least one
    /// toleration; otherwise `Err` aggregating the untolerated taints.
    pub fn tolerates(&self, tolerations: &[Toleration]) -> Result<(), TaintIntolerance> {
        let untolerated: Vec<Taint> = self
            .0
            .iter()
            .filter(|taint| !tolerations.iter().any(|t| t.tolerates_taint(taint)))
            .cloned()
            .collect();
        if untolerated.is_empty() {
            Ok(())
        } else {
            Err(TaintIntolerance { untolerated })
        }
    }

    /// `Merge`: return a copy of `self` with every taint from `with` that is
    /// not already present (by [`Taint::matches_taint`], i.e. key+effect)
    /// appended. Existing entries are preserved verbatim.
    pub fn merge(&self, with: &Taints) -> Taints {
        let mut res = self.0.clone();
        for taint in &with.0 {
            if !res.iter().any(|t| taint.matches_taint(t)) {
                res.push(taint.clone());
            }
        }
        Taints(res)
    }
}

/// Taints expected on an initializing node; ignored while a Karpenter-managed
/// node is still uninitialized. Mirrors `KnownEphemeralTaints`.
pub static KNOWN_EPHEMERAL_TAINTS: LazyLock<Vec<Taint>> = LazyLock::new(|| {
    vec![
        Taint {
            key: "node.kubernetes.io/not-ready".into(),
            value: None,
            effect: Effect::NoSchedule,
        },
        Taint {
            key: "node.kubernetes.io/not-ready".into(),
            value: None,
            effect: Effect::NoExecute,
        },
        Taint {
            key: "node.kubernetes.io/unreachable".into(),
            value: None,
            effect: Effect::NoSchedule,
        },
        Taint {
            key: "node.cloudprovider.kubernetes.io/uninitialized".into(),
            value: Some("true".into()),
            effect: Effect::NoSchedule,
        },
        // v1.UnregisteredNoExecuteTaint (karpenter.sh/unregistered : NoExecute)
        Taint {
            key: "karpenter.sh/unregistered".into(),
            value: None,
            effect: Effect::NoExecute,
        },
    ]
});
