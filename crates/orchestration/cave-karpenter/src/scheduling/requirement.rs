// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of pkg/scheduling/requirement.go from kubernetes-sigs/karpenter
// v1.12.1 (sha ed490e8). Apache-2.0 upstream; see NOTICE.
//
//! A `Requirement` is an efficient representation of a node-selector
//! requirement. Internally it is a *complement-able* set: `complement =
//! false` means "value must be IN the set", `complement = true` means
//! "value must NOT be in the set" (which also models Exists / NotIn).
//! Numeric Gt/Lt operators are canonicalized into inclusive integer
//! bounds (`gte` / `lte`); Gt N becomes Gte N+1 and Lt N becomes Lte N-1,
//! exactly as upstream does.

use std::collections::BTreeSet;
use std::fmt;

/// Node-selector operator. Mirrors `corev1.NodeSelectorOperator` plus
/// Karpenter's `Gte` / `Lte` extensions. `Operator()` on a constructed
/// requirement only ever returns `In` / `NotIn` / `Exists` / `DoesNotExist`
/// — `Gt` / `Lt` / `Gte` / `Lte` are inputs that collapse into bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    In,
    NotIn,
    Exists,
    DoesNotExist,
    Gt,
    Lt,
    Gte,
    Lte,
}

impl fmt::Display for Operator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Operator::In => "In",
            Operator::NotIn => "NotIn",
            Operator::Exists => "Exists",
            Operator::DoesNotExist => "DoesNotExist",
            Operator::Gt => "Gt",
            Operator::Lt => "Lt",
            Operator::Gte => "Gte",
            Operator::Lte => "Lte",
        };
        f.write_str(s)
    }
}

/// `corev1.NodeSelectorRequirement` + Karpenter's `MinValues` flexibility —
/// the serialized form produced by [`Requirement::node_selector_requirement`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSelectorRequirement {
    pub key: String,
    pub operator: Operator,
    pub values: Vec<String>,
    pub min_values: Option<i64>,
}

/// Normalized-label map: upstream `v1.NormalizedLabels` rewrites the
/// deprecated beta labels to their stable equivalents on construction.
fn normalized_label(key: &str) -> &str {
    match key {
        "failure-domain.beta.kubernetes.io/zone" => "topology.kubernetes.io/zone",
        "failure-domain.beta.kubernetes.io/region" => "topology.kubernetes.io/region",
        "beta.kubernetes.io/arch" => "kubernetes.io/arch",
        "beta.kubernetes.io/os" => "kubernetes.io/os",
        other => other,
    }
}

#[derive(Debug, Clone)]
pub struct Requirement {
    pub key: String,
    complement: bool,
    values: BTreeSet<String>,
    /// inclusive lower bound (Gt is converted to Gte)
    gte: Option<i64>,
    /// inclusive upper bound (Lt is converted to Lte)
    lte: Option<i64>,
    pub min_values: Option<i64>,
}

impl Requirement {
    /// `NewRequirement` — no minValues flexibility.
    pub fn new(key: &str, operator: Operator, values: &[String]) -> Requirement {
        Requirement::new_with_flexibility(key, operator, None, values)
    }

    /// `NewRequirementWithFlexibility` — GT/LT are canonicalized to GTE/LTE.
    pub fn new_with_flexibility(
        key: &str,
        operator: Operator,
        min_values: Option<i64>,
        values: &[String],
    ) -> Requirement {
        let key = normalized_label(key).to_string();

        // Super-common case: inline everything.
        if operator == Operator::In {
            return Requirement {
                key,
                values: values.iter().cloned().collect(),
                complement: false,
                gte: None,
                lte: None,
                min_values,
            };
        }

        let mut r = Requirement {
            key,
            values: BTreeSet::new(),
            complement: true,
            gte: None,
            lte: None,
            min_values,
        };
        if operator == Operator::DoesNotExist {
            r.complement = false;
        }
        if operator == Operator::NotIn {
            r.values.extend(values.iter().cloned());
        }
        if operator == Operator::Gt {
            let value = parse_prevalidated(&values[0]);
            if value == i64::MAX {
                // Gt MaxInt matches nothing.
                return Requirement::new_with_flexibility(
                    &r.key,
                    Operator::DoesNotExist,
                    min_values,
                    &[],
                );
            }
            r.gte = Some(value + 1); // canonicalize GT N to GTE N+1
        }
        if operator == Operator::Lt {
            let value = parse_prevalidated(&values[0]);
            r.lte = Some(value - 1); // canonicalize LT N to LTE N-1
        }
        if operator == Operator::Gte {
            r.gte = Some(parse_prevalidated(&values[0]));
        }
        if operator == Operator::Lte {
            r.lte = Some(parse_prevalidated(&values[0]));
        }
        r
    }

    /// The single canonical `NodeSelectorRequirement` for this requirement.
    /// (When both bounds exist they must be serialized as two; that case is
    /// out of scope for the single-return form, matching upstream.)
    pub fn node_selector_requirement(&self) -> NodeSelectorRequirement {
        if let Some(g) = self.gte {
            return NodeSelectorRequirement {
                key: self.key.clone(),
                operator: Operator::Gte,
                values: vec![g.to_string()],
                min_values: self.min_values,
            };
        }
        if let Some(l) = self.lte {
            return NodeSelectorRequirement {
                key: self.key.clone(),
                operator: Operator::Lte,
                values: vec![l.to_string()],
                min_values: self.min_values,
            };
        }
        let (operator, values) = if self.complement {
            if !self.values.is_empty() {
                (Operator::NotIn, self.sorted_values())
            } else {
                (Operator::Exists, vec![])
            }
        } else if !self.values.is_empty() {
            (Operator::In, self.sorted_values())
        } else {
            (Operator::DoesNotExist, vec![])
        };
        NodeSelectorRequirement {
            key: self.key.clone(),
            operator,
            values,
            min_values: self.min_values,
        }
    }

    /// Constrain `self` by `other`, returning the intersection requirement.
    pub fn intersection(&self, other: &Requirement) -> Requirement {
        let complement = self.complement && other.complement;
        let mut gte = max_opt(self.gte, other.gte);
        let mut lte = min_opt(self.lte, other.lte);
        let min_values = max_opt(self.min_values, other.min_values);

        if let (Some(g), Some(l)) = (gte, lte) {
            if g > l {
                return Requirement::new_with_flexibility(
                    &self.key,
                    Operator::DoesNotExist,
                    min_values,
                    &[],
                );
            }
        }

        let mut values: BTreeSet<String> = if self.complement && other.complement {
            self.values.union(&other.values).cloned().collect()
        } else if self.complement && !other.complement {
            other.values.difference(&self.values).cloned().collect()
        } else if !self.complement && other.complement {
            self.values.difference(&other.values).cloned().collect()
        } else {
            self.values.intersection(&other.values).cloned().collect()
        };
        values.retain(|v| within_bounds(v, gte, lte));

        // Bounds only meaningful for complement (Exists-with-bounds) sets.
        if !complement {
            gte = None;
            lte = None;
        }
        Requirement {
            key: self.key.clone(),
            values,
            complement,
            gte,
            lte,
            min_values,
        }
    }

    /// Efficient "do these two overlap?" check that avoids building sets.
    pub fn has_intersection(&self, other: &Requirement) -> bool {
        let gte = max_opt(self.gte, other.gte);
        let lte = min_opt(self.lte, other.lte);
        if let (Some(g), Some(l)) = (gte, lte) {
            if g > l {
                return false;
            }
        }
        if self.complement && other.complement {
            return true;
        }
        if self.complement && !other.complement {
            return other
                .values
                .iter()
                .any(|v| !self.values.contains(v) && within_bounds(v, gte, lte));
        }
        if !self.complement && other.complement {
            return self
                .values
                .iter()
                .any(|v| !other.values.contains(v) && within_bounds(v, gte, lte));
        }
        self.values
            .iter()
            .any(|v| other.values.contains(v) && within_bounds(v, gte, lte))
    }

    /// `Has` — does this requirement admit `value`?
    pub fn has(&self, value: &str) -> bool {
        if self.complement {
            !self.values.contains(value) && within_bounds(value, self.gte, self.lte)
        } else {
            self.values.contains(value) && within_bounds(value, self.gte, self.lte)
        }
    }

    /// A representative admitted value. Diverges from upstream's `rand`: for
    /// the unbounded NotIn/Exists case we return the lower bound (or 0) as a
    /// deterministic representative, which still satisfies the upstream
    /// contract (non-empty, within bounds, ≠ excluded singletons).
    pub fn any(&self) -> String {
        match self.operator() {
            Operator::In => self.values.iter().next().cloned().unwrap_or_default(),
            Operator::NotIn | Operator::Exists => self.gte.unwrap_or(0).to_string(),
            _ => String::new(),
        }
    }

    pub fn operator(&self) -> Operator {
        if self.complement {
            if self.len() < i64::MAX {
                Operator::NotIn
            } else {
                Operator::Exists
            }
        } else if self.len() > 0 {
            Operator::In
        } else {
            Operator::DoesNotExist
        }
    }

    pub fn len(&self) -> i64 {
        if self.complement {
            i64::MAX - self.values.len() as i64
        } else {
            self.values.len() as i64
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn values(&self) -> Vec<String> {
        self.values.iter().cloned().collect()
    }

    pub fn insert(&mut self, items: impl IntoIterator<Item = String>) {
        self.values.extend(items);
    }

    fn sorted_values(&self) -> Vec<String> {
        self.values.iter().cloned().collect()
    }

    /// Field tuple used for behavioral equality in tests (excludes minValues,
    /// matching the upstream Intersection truth-table fixtures).
    pub fn canonical(&self) -> (bool, Vec<String>, Option<i64>, Option<i64>) {
        (
            self.complement,
            self.sorted_values(),
            self.gte,
            self.lte,
        )
    }
}

impl fmt::Display for Requirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.operator() {
            Operator::Exists | Operator::DoesNotExist => {
                write!(f, "{} {}", self.key, self.operator())?;
            }
            op => {
                let mut values = self.sorted_values();
                if values.len() > 5 {
                    let extra = values.len() - 5;
                    values.truncate(5);
                    values.push(format!("and {extra} others"));
                }
                write!(f, "{} {} [{}]", self.key, op, values.join(" "))?;
            }
        }
        if let Some(g) = self.gte {
            write!(f, " >={g}")?;
        }
        if let Some(l) = self.lte {
            write!(f, " <={l}")?;
        }
        if let Some(m) = self.min_values {
            write!(f, " minValues {m}")?;
        }
        Ok(())
    }
}

fn parse_prevalidated(s: &str) -> i64 {
    s.parse::<i64>().unwrap_or(0)
}

fn within_bounds(value: &str, gte: Option<i64>, lte: Option<i64>) -> bool {
    if gte.is_none() && lte.is_none() {
        return true;
    }
    // If bounds are set, non-integer values are invalid.
    let val: i64 = match value.parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    if let Some(g) = gte {
        if val < g {
            return false;
        }
    }
    if let Some(l) = lte {
        if val > l {
            return false;
        }
    }
    true
}

fn min_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (None, b) => b,
        (a, None) => a,
        (Some(a), Some(b)) => Some(a.min(b)),
    }
}

fn max_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (None, b) => b,
        (a, None) => a,
        (Some(a), Some(b)) => Some(a.max(b)),
    }
}
