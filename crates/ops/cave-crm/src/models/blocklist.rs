// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM Blocklist — `packages/twenty-server/src/modules/blocklist/standard-objects/blocklist.workspace-entity.ts`
//!
//! A per-workspace-member list of e-mail handles whose inbound/outbound
//! messages and calendar events the sync layer must drop. Twenty stores a
//! single nullable `handle` TEXT column plus the owning `workspaceMember`
//! relation; the matching semantics (exact address vs. whole-domain rule)
//! live in the messaging-import filter, ported here as `is_blocked`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Blocklist workspace-entity — mirrors Twenty's `BlocklistWorkspaceEntity`.
///
/// `handle` is `TEXT | null` upstream; a value of either a full e-mail
/// address (`spam@evil.test`) or a domain rule (`@evil.test` / `evil.test`)
/// is accepted, matching Twenty's blocklist UI validation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Blocklist {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// Twenty `workspaceMemberId` — the member who owns this rule.
    pub workspace_member_id: Uuid,
    /// Twenty `handle` (TEXT, nullable).
    pub handle: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member() -> (Uuid, Uuid) {
        (Uuid::new_v4(), Uuid::new_v4())
    }

    #[test]
    fn new_carries_workspace_and_member() {
        let (ws, m) = member();
        let b = Blocklist::new(ws, m, "spam@evil.test");
        assert_eq!(b.workspace_id, ws);
        assert_eq!(b.workspace_member_id, m);
        assert_eq!(b.handle.as_deref(), Some("spam@evil.test"));
    }

    #[test]
    fn normalized_handle_is_lowercased_and_trimmed() {
        let (ws, m) = member();
        let b = Blocklist::new(ws, m, "  Spam@Evil.TEST  ");
        assert_eq!(b.normalized_handle().as_deref(), Some("spam@evil.test"));
    }

    #[test]
    fn exact_email_rule_blocks_only_that_address() {
        let (ws, m) = member();
        let b = Blocklist::new(ws, m, "spam@evil.test");
        assert!(!b.is_domain_rule());
        assert!(b.is_blocked("spam@evil.test"));
        assert!(b.is_blocked("SPAM@EVIL.TEST")); // case-insensitive
        assert!(!b.is_blocked("ham@evil.test")); // same domain, different local
        assert!(!b.is_blocked("spam@good.test"));
    }

    #[test]
    fn at_prefixed_domain_rule_blocks_whole_domain() {
        let (ws, m) = member();
        let b = Blocklist::new(ws, m, "@evil.test");
        assert!(b.is_domain_rule());
        assert!(b.is_blocked("spam@evil.test"));
        assert!(b.is_blocked("ham@EVIL.test"));
        assert!(!b.is_blocked("spam@good.test"));
    }

    #[test]
    fn bare_domain_rule_blocks_whole_domain() {
        let (ws, m) = member();
        let b = Blocklist::new(ws, m, "evil.test");
        assert!(b.is_domain_rule());
        assert!(b.is_blocked("anyone@evil.test"));
        assert!(!b.is_blocked("anyone@evil.test.attacker.com"));
    }

    #[test]
    fn empty_or_null_handle_blocks_nothing() {
        let (ws, m) = member();
        let mut b = Blocklist::new(ws, m, "");
        assert!(!b.is_blocked("spam@evil.test"));
        b.handle = None;
        assert!(!b.is_blocked("spam@evil.test"));
    }

    #[test]
    fn serializes_null_handle_as_json_null() {
        let (ws, m) = member();
        let mut b = Blocklist::new(ws, m, "spam@evil.test");
        b.handle = None;
        let j = serde_json::to_value(&b).unwrap();
        assert!(j["handle"].is_null());
    }
}
