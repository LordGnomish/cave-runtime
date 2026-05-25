// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/notification/NotificationRouter.java
//
//! Notification framework router — given a `Notification` and a set of
//! `NotificationRule`s, return the rules that match. Mirrors upstream
//! `NotificationRouter.resolveRules(Notification)` (the pure decision
//! step before transport dispatch).

use super::{Notification, NotificationLevel, NotificationRule};

fn level_weight(l: NotificationLevel) -> u8 {
    match l {
        NotificationLevel::Informational => 0,
        NotificationLevel::Warning => 1,
        NotificationLevel::Error => 2,
    }
}

/// Returns the subset of `rules` that would fire for this `notification`:
/// (a) rule must be enabled,
/// (b) `notify_on` must contain the notification group,
/// (c) notification level ≥ `min_level`.
pub fn resolve_rules<'a>(
    rules: &'a [NotificationRule],
    notification: &Notification,
) -> Vec<&'a NotificationRule> {
    rules
        .iter()
        .filter(|r| {
            r.enabled
                && r.notify_on.contains(&notification.group)
                && level_weight(notification.level) >= level_weight(r.min_level)
        })
        .collect()
}

/// True when at least one rule fires. Quick boolean for short-circuit
/// callers that only need to know "should we notify?".
pub fn should_notify(rules: &[NotificationRule], notification: &Notification) -> bool {
    !resolve_rules(rules, notification).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::{
        Notification, NotificationGroup, NotificationLevel, NotificationRule, PublisherKind,
    };
    use uuid::Uuid;

    fn rule(
        enabled: bool,
        on: Vec<NotificationGroup>,
        min: NotificationLevel,
        kind: PublisherKind,
    ) -> NotificationRule {
        NotificationRule {
            uuid: Uuid::new_v4(),
            name: "r".into(),
            enabled,
            notify_on: on,
            min_level: min,
            publisher: kind,
        }
    }

    fn note(group: NotificationGroup, level: NotificationLevel) -> Notification {
        Notification {
            group,
            level,
            title: "t".into(),
            content: "c".into(),
            payload: None,
        }
    }

    #[test]
    fn disabled_rule_does_not_match() {
        let r = rule(
            false,
            vec![NotificationGroup::PolicyViolation],
            NotificationLevel::Informational,
            PublisherKind::Console,
        );
        let n = note(NotificationGroup::PolicyViolation, NotificationLevel::Error);
        assert!(resolve_rules(&[r], &n).is_empty());
    }

    #[test]
    fn rule_must_subscribe_to_group() {
        let r = rule(
            true,
            vec![NotificationGroup::NewVulnerability],
            NotificationLevel::Informational,
            PublisherKind::Console,
        );
        let n = note(NotificationGroup::PolicyViolation, NotificationLevel::Error);
        assert!(resolve_rules(&[r], &n).is_empty());
    }

    #[test]
    fn level_gate_blocks_below_threshold() {
        let r = rule(
            true,
            vec![NotificationGroup::PolicyViolation],
            NotificationLevel::Error,
            PublisherKind::Console,
        );
        let n = note(NotificationGroup::PolicyViolation, NotificationLevel::Warning);
        assert!(resolve_rules(&[r], &n).is_empty());
    }

    #[test]
    fn level_gate_inclusive_at_threshold() {
        let r = rule(
            true,
            vec![NotificationGroup::PolicyViolation],
            NotificationLevel::Warning,
            PublisherKind::Console,
        );
        let n = note(NotificationGroup::PolicyViolation, NotificationLevel::Warning);
        assert_eq!(resolve_rules(&[r], &n).len(), 1);
    }

    #[test]
    fn level_gate_includes_higher() {
        let r = rule(
            true,
            vec![NotificationGroup::PolicyViolation],
            NotificationLevel::Warning,
            PublisherKind::Console,
        );
        let n = note(NotificationGroup::PolicyViolation, NotificationLevel::Error);
        assert_eq!(resolve_rules(&[r], &n).len(), 1);
    }

    #[test]
    fn should_notify_returns_true_when_any_match() {
        let r = rule(
            true,
            vec![NotificationGroup::NewVulnerability],
            NotificationLevel::Informational,
            PublisherKind::Console,
        );
        let n = note(NotificationGroup::NewVulnerability, NotificationLevel::Informational);
        assert!(should_notify(&[r], &n));
    }

    #[test]
    fn should_notify_returns_false_when_none_match() {
        let n = note(NotificationGroup::NewVulnerability, NotificationLevel::Error);
        assert!(!should_notify(&[], &n));
    }

    #[test]
    fn multiple_rules_all_match_returned() {
        let r1 = rule(
            true,
            vec![NotificationGroup::NewVulnerability],
            NotificationLevel::Informational,
            PublisherKind::Console,
        );
        let r2 = rule(
            true,
            vec![NotificationGroup::NewVulnerability, NotificationGroup::PolicyViolation],
            NotificationLevel::Warning,
            PublisherKind::Webhook {
                url: "https://example/".into(),
            },
        );
        let n = note(NotificationGroup::NewVulnerability, NotificationLevel::Warning);
        let rules = [r1, r2];
        let matched = resolve_rules(&rules, &n);
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn rules_with_multiple_groups_match_any_group() {
        let r = rule(
            true,
            vec![NotificationGroup::NewVulnerability, NotificationGroup::PolicyViolation],
            NotificationLevel::Informational,
            PublisherKind::Console,
        );
        let n1 = note(NotificationGroup::PolicyViolation, NotificationLevel::Informational);
        let n2 = note(NotificationGroup::NewVulnerability, NotificationLevel::Informational);
        let n3 = note(NotificationGroup::BomConsumed, NotificationLevel::Informational);
        assert_eq!(resolve_rules(std::slice::from_ref(&r), &n1).len(), 1);
        assert_eq!(resolve_rules(std::slice::from_ref(&r), &n2).len(), 1);
        assert_eq!(resolve_rules(std::slice::from_ref(&r), &n3).len(), 0);
    }
}
