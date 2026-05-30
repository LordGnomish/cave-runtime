// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Upstream changelog watch + hot-patch ingestion — ADR-SELF-IMPROVE-001.
//!
//! Cave pins every upstream to an exact version (the always-latest gate,
//! tracked by `cave-llm-tracker`). This module compares the pinned version
//! against the latest the tracker reports, classifies the bump
//! ([`BumpKind`]), and turns each detected [`UpstreamUpdate`] into a
//! prioritised [`PortProposal`] queued for human-reviewed porting
//! ([`HotPatchQueue`]) — the "propose a port when a new release lands" half
//! of the self-improvement mandate.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::HermesError;

/// A semantic version triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parse `"v1.2.3"` or `"1.2.3"`. Requires exactly three numeric fields.
    pub fn parse(s: &str) -> crate::error::Result<Self> {
        let s = s.trim().trim_start_matches(['v', 'V']);
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(HermesError::SelfImprove(format!(
                "version '{s}' is not major.minor.patch"
            )));
        }
        let parse_part = |p: &str| {
            p.parse::<u64>()
                .map_err(|_| HermesError::SelfImprove(format!("non-numeric version field '{p}'")))
        };
        Ok(Version::new(
            parse_part(parts[0])?,
            parse_part(parts[1])?,
            parse_part(parts[2])?,
        ))
    }

    pub fn is_newer_than(&self, other: &Version) -> bool {
        self > other
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The kind of version bump between two versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BumpKind {
    Major,
    Minor,
    Patch,
}

impl BumpKind {
    /// Classify the bump from `from` to `to`, or `None` if `to` is not newer.
    pub fn classify(from: &Version, to: &Version) -> Option<Self> {
        if !to.is_newer_than(from) {
            return None;
        }
        if to.major != from.major {
            Some(BumpKind::Major)
        } else if to.minor != from.minor {
            Some(BumpKind::Minor)
        } else {
            Some(BumpKind::Patch)
        }
    }
}

/// Port priority for a [`PortProposal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    Low,
    Medium,
    High,
}

impl Priority {
    fn rank(&self) -> u8 {
        match self {
            Priority::High => 2,
            Priority::Medium => 1,
            Priority::Low => 0,
        }
    }
}

/// A detected upstream version change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpstreamUpdate {
    pub name: String,
    pub repo: String,
    pub from: Version,
    pub to: Version,
    pub kind: BumpKind,
}

/// Watches tracked upstreams against the latest versions the tracker reports.
#[derive(Debug, Default)]
pub struct ChangelogWatcher {
    tracked: BTreeMap<String, (String, Version)>,
}

impl ChangelogWatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an upstream with its currently pinned version.
    pub fn track(&mut self, name: &str, repo: &str, current: Version) -> &mut Self {
        self.tracked
            .insert(name.to_string(), (repo.to_string(), current));
        self
    }

    /// Compare each tracked upstream against `latest` (name → latest version,
    /// as reported by cave-llm-tracker) and emit an update per outdated pin.
    pub fn scan(&self, latest: &[(&str, Version)]) -> Vec<UpstreamUpdate> {
        let mut out = Vec::new();
        for (name, latest_ver) in latest {
            let Some((repo, current)) = self.tracked.get(*name) else {
                continue;
            };
            if let Some(kind) = BumpKind::classify(current, latest_ver) {
                out.push(UpstreamUpdate {
                    name: (*name).to_string(),
                    repo: repo.clone(),
                    from: *current,
                    to: *latest_ver,
                    kind,
                });
            }
        }
        out
    }
}

/// A queued, prioritised proposal to port an upstream bump.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortProposal {
    pub upstream: String,
    pub repo: String,
    pub from: Version,
    pub to: Version,
    pub priority: Priority,
    pub action: String,
}

impl PortProposal {
    pub fn from_update(u: &UpstreamUpdate) -> Self {
        let priority = match u.kind {
            BumpKind::Major => Priority::High,
            BumpKind::Minor => Priority::Medium,
            BumpKind::Patch => Priority::Low,
        };
        Self {
            upstream: u.name.clone(),
            repo: u.repo.clone(),
            from: u.from,
            to: u.to,
            priority,
            action: format!(
                "review {} → {} ({:?}); port new/changed functions, bump parity manifest pin",
                u.from, u.to, u.kind
            ),
        }
    }

    fn key(&self) -> String {
        format!("{}@{}", self.upstream, self.to)
    }
}

/// Dedup-on-enqueue, drain-by-priority queue of port proposals.
#[derive(Debug, Default)]
pub struct HotPatchQueue {
    items: Vec<PortProposal>,
}

impl HotPatchQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue a proposal. Returns `false` (no-op) if an identical
    /// `(upstream, to-version)` is already queued.
    pub fn enqueue(&mut self, proposal: PortProposal) -> bool {
        if self.items.iter().any(|p| p.key() == proposal.key()) {
            return false;
        }
        self.items.push(proposal);
        true
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Drain all proposals, highest priority first (stable within a tier).
    pub fn drain_by_priority(&mut self) -> Vec<PortProposal> {
        let mut drained = std::mem::take(&mut self.items);
        drained.sort_by(|a, b| b.priority.rank().cmp(&a.priority.rank()));
        drained
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parses_with_or_without_v_prefix() {
        assert_eq!(Version::parse("v1.2.3").unwrap(), Version::new(1, 2, 3));
        assert_eq!(Version::parse("1.2.3").unwrap(), Version::new(1, 2, 3));
        assert!(Version::parse("not.a.version").is_err());
        assert!(Version::parse("1.2").is_err());
    }

    #[test]
    fn version_ordering_is_semver() {
        assert!(Version::new(2, 0, 0).is_newer_than(&Version::new(1, 9, 9)));
        assert!(Version::new(1, 3, 0).is_newer_than(&Version::new(1, 2, 9)));
        assert!(Version::new(1, 2, 5).is_newer_than(&Version::new(1, 2, 4)));
        assert!(!Version::new(1, 2, 3).is_newer_than(&Version::new(1, 2, 3)));
    }

    #[test]
    fn bump_kind_classification() {
        assert_eq!(
            BumpKind::classify(&Version::new(1, 2, 3), &Version::new(2, 0, 0)),
            Some(BumpKind::Major)
        );
        assert_eq!(
            BumpKind::classify(&Version::new(1, 2, 3), &Version::new(1, 3, 0)),
            Some(BumpKind::Minor)
        );
        assert_eq!(
            BumpKind::classify(&Version::new(1, 2, 3), &Version::new(1, 2, 4)),
            Some(BumpKind::Patch)
        );
        assert_eq!(
            BumpKind::classify(&Version::new(1, 2, 3), &Version::new(1, 2, 3)),
            None
        );
    }

    #[test]
    fn watcher_scans_only_outdated_upstreams() {
        let mut w = ChangelogWatcher::new();
        w.track("cave-hermes", "NousResearch/hermes-agent", Version::new(2026, 5, 16));
        w.track("cave-local-llm", "ollama/ollama", Version::new(0, 5, 0));
        let updates = w.scan(&[
            ("cave-hermes", Version::new(2026, 5, 16)), // unchanged
            ("cave-local-llm", Version::new(0, 6, 0)),  // minor bump
        ]);
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].name, "cave-local-llm");
        assert_eq!(updates[0].kind, BumpKind::Minor);
        assert_eq!(updates[0].from, Version::new(0, 5, 0));
        assert_eq!(updates[0].to, Version::new(0, 6, 0));
    }

    #[test]
    fn port_proposal_priority_tracks_bump_kind() {
        let major = PortProposal::from_update(&UpstreamUpdate {
            name: "x".into(),
            repo: "r".into(),
            from: Version::new(1, 0, 0),
            to: Version::new(2, 0, 0),
            kind: BumpKind::Major,
        });
        assert_eq!(major.priority, Priority::High);
        let patch = PortProposal::from_update(&UpstreamUpdate {
            name: "x".into(),
            repo: "r".into(),
            from: Version::new(1, 0, 0),
            to: Version::new(1, 0, 1),
            kind: BumpKind::Patch,
        });
        assert_eq!(patch.priority, Priority::Low);
    }

    #[test]
    fn queue_dedups_same_target_version() {
        let mut q = HotPatchQueue::new();
        let p = PortProposal::from_update(&UpstreamUpdate {
            name: "x".into(),
            repo: "r".into(),
            from: Version::new(1, 0, 0),
            to: Version::new(1, 1, 0),
            kind: BumpKind::Minor,
        });
        assert!(q.enqueue(p.clone()));
        assert!(!q.enqueue(p), "same (name, to) is a no-op");
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn queue_drains_high_priority_first() {
        let mut q = HotPatchQueue::new();
        q.enqueue(PortProposal::from_update(&UpstreamUpdate {
            name: "low".into(),
            repo: "r".into(),
            from: Version::new(1, 0, 0),
            to: Version::new(1, 0, 1),
            kind: BumpKind::Patch,
        }));
        q.enqueue(PortProposal::from_update(&UpstreamUpdate {
            name: "high".into(),
            repo: "r".into(),
            from: Version::new(1, 0, 0),
            to: Version::new(2, 0, 0),
            kind: BumpKind::Major,
        }));
        let drained = q.drain_by_priority();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].upstream, "high");
        assert_eq!(drained[1].upstream, "low");
    }
}
