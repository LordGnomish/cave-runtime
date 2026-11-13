// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Restore resource-ordering engine.
//!
//! Faithful line-port of two pure pieces of Velero's restore path:
//!   * `pkg/types/priority.go` — `Priorities` struct + `Set` (parse) + `String`.
//!   * `pkg/restore/restore.go` — `getOrderedResources`.
//!
//! Together these decide the order Kubernetes resources are restored in: a fixed
//! list of high-priority resources first, then every other backed-up resource in
//! alphabetical order, then a fixed list of low-priority resources last. This is
//! pure in-memory engine logic — no plugin RPC, no discovery, no persistence.

/// Separator token used in the `Priorities` flag string (`-`).
const PRIORITY_SEPARATOR: &str = "-";

/// Defines the desired order of resource operations: resources in `high` are handled
/// first, resources in `low` are handled last, and everything else is handled
/// alphabetically in between. Port of Velero `types.Priorities`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Priorities {
    /// Resources handled first, in the given order.
    pub high: Vec<String>,
    /// Resources handled last, in the given order.
    pub low: Vec<String>,
}

impl Priorities {
    /// Returns the flag-string representation. Port of `Priorities.String`:
    /// high priorities joined by `,`, then (if any low priorities) the separator
    /// and the low priorities.
    pub fn to_priority_string(&self) -> String {
        let mut priorities = self.high.clone();
        if !self.low.is_empty() {
            priorities.push(PRIORITY_SEPARATOR.to_string());
            priorities.extend(self.low.iter().cloned());
        }
        priorities.join(",")
    }

    /// Parses the provided comma-separated string into a `Priorities`. Port of
    /// `Priorities.Set`. The single token `-` separates the high-priority prefix
    /// from the low-priority suffix; more than one separator is an error.
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut p = Priorities::default();
        if s.is_empty() {
            return Ok(p);
        }
        let strs: Vec<String> = s.split(',').map(|x| x.to_string()).collect();

        let mut separator_index: i64 = -1;
        for (i, str_) in strs.iter().enumerate() {
            if str_ == PRIORITY_SEPARATOR {
                if separator_index > -1 {
                    return Err(format!(
                        "multiple priority separator {PRIORITY_SEPARATOR:?} found"
                    ));
                }
                separator_index = i as i64;
            }
        }

        // has no separator
        if separator_index == -1 {
            p.high = strs;
            return Ok(p);
        }
        // start with separator
        if separator_index == 0 {
            // contains only separator
            if strs.len() == 1 {
                return Ok(p);
            }
            p.low = strs[1..].to_vec();
            return Ok(p);
        }
        // end with separator
        if separator_index as usize == strs.len() - 1 {
            p.high = strs[..strs.len() - 1].to_vec();
            return Ok(p);
        }
        // separator in the middle
        let idx = separator_index as usize;
        p.high = strs[..idx].to_vec();
        p.low = strs[idx + 1..].to_vec();
        Ok(p)
    }
}

/// Returns an ordered list of resource identifiers to restore. The list begins with
/// all of the high-priority resources (in order), ends with all of the low-priority
/// resources (in order), and an alphabetized list of the remaining backed-up
/// resources (with the prioritized ones removed) is placed in the middle.
///
/// Port of Velero `getOrderedResources` (pkg/restore/restore.go).
pub fn get_ordered_resources(priorities: &Priorities, backup_resources: &[String]) -> Vec<String> {
    use std::collections::BTreeSet;

    let mut priority_set: BTreeSet<&String> = BTreeSet::new();
    for p in &priorities.high {
        priority_set.insert(p);
    }
    for p in &priorities.low {
        priority_set.insert(p);
    }

    // pick the prioritized resources out
    let mut ordered_backup_resources: Vec<String> = backup_resources
        .iter()
        .filter(|r| !priority_set.contains(*r))
        .cloned()
        .collect();
    // alphabetize resources in the backup
    ordered_backup_resources.sort();

    let mut list = priorities.high.clone();
    list.extend(ordered_backup_resources);
    list.extend(priorities.low.iter().cloned());
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_parse_string() {
        let p = Priorities::parse("p0,p1,-,p9").unwrap();
        assert_eq!(p.to_priority_string(), "p0,p1,-,p9");
    }
}
