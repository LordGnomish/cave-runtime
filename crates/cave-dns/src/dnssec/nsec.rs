// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! NSEC denial-of-existence (RFC 4034 §4).

use serde::{Deserialize, Serialize};

/// A parsed NSEC record: a link between the owner name and the next name
/// in canonical zone order plus the type-bitmap of records present at the
/// owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nsec {
    pub owner: String,
    pub next_name: String,
    pub type_bitmap: Vec<u16>,
}

impl Nsec {
    pub fn new(owner: impl Into<String>, next: impl Into<String>, types: Vec<u16>) -> Self {
        let mut t = types;
        t.sort_unstable();
        t.dedup();
        Self {
            owner: owner.into(),
            next_name: next.into(),
            type_bitmap: t,
        }
    }

    /// Does this NSEC prove that `name` does NOT exist in the zone? An
    /// NSEC covers a name when owner < name < next_name in canonical order.
    pub fn covers(&self, name: &str) -> bool {
        canonical_lt(&self.owner, name) && canonical_lt(name, &self.next_name)
    }

    /// Does this NSEC prove that `qtype` does NOT exist at `owner`?
    pub fn proves_no_type(&self, owner: &str, qtype: u16) -> bool {
        canonical_eq(&self.owner, owner) && !self.type_bitmap.contains(&qtype)
    }

    pub fn has_type(&self, qtype: u16) -> bool {
        self.type_bitmap.contains(&qtype)
    }
}

fn canonical(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let stripped = lower.trim_end_matches('.');
    stripped.to_string()
}

fn canonical_eq(a: &str, b: &str) -> bool {
    canonical(a) == canonical(b)
}

/// Canonical DNS name ordering — compare labels right-to-left.
fn canonical_lt(a: &str, b: &str) -> bool {
    let a_canon = canonical(a);
    let b_canon = canonical(b);
    let ar: Vec<&str> = a_canon.split('.').rev().collect();
    let br: Vec<&str> = b_canon.split('.').rev().collect();
    ar < br
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nsec_covers_lexicographically_between() {
        let n = Nsec::new("alpha.example.com.", "gamma.example.com.", vec![1, 28]);
        assert!(n.covers("beta.example.com."));
        assert!(!n.covers("zeta.example.com."));
    }

    #[test]
    fn nsec_does_not_cover_boundaries() {
        let n = Nsec::new("alpha.example.com.", "gamma.example.com.", vec![1]);
        assert!(!n.covers("alpha.example.com."));
        assert!(!n.covers("gamma.example.com."));
    }

    #[test]
    fn nsec_proves_no_type_when_owner_matches_and_bit_absent() {
        let n = Nsec::new("svc.example.com.", "tmp.example.com.", vec![1, 16]);
        assert!(n.proves_no_type("svc.example.com.", 28));
        assert!(!n.proves_no_type("svc.example.com.", 1));
        assert!(!n.proves_no_type("other.example.com.", 28));
    }

    #[test]
    fn nsec_dedups_and_sorts_type_bitmap() {
        let n = Nsec::new("a.example.com.", "b.example.com.", vec![28, 1, 16, 1, 28]);
        assert_eq!(n.type_bitmap, vec![1, 16, 28]);
    }

    #[test]
    fn nsec_canonical_eq_is_dot_and_case_insensitive() {
        let n = Nsec::new("HOST.example.com", "next.example.com.", vec![1]);
        assert!(n.proves_no_type("host.EXAMPLE.com.", 28));
        assert!(n.has_type(1));
    }
}
