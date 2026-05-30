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
