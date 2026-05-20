// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: goharbor/harbor src/common/rbac/{const.go,role.go,project/*.go}
//
//! Harbor RBAC — project role hierarchy + permission checks.
//!
//! Harbor v2 defines five roles per project, in descending privilege order:
//!
//! - `ProjectAdmin` — full project ownership (manage members, retention, ...).
//! - `Maintainer`   — push + scan + manage immutable tag rules.
//! - `Developer`    — push artifacts and read scans.
//! - `Guest`        — pull only.
//! - `LimitedGuest` — pull restricted to assigned artifacts (rare).
//!
//! Each role expands to a fixed set of `ResourceAction` pairs; the
//! `has_permission` check is `role.permissions().contains(&want)`.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    ProjectAdmin,
    Maintainer,
    Developer,
    Guest,
    LimitedGuest,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ProjectAdmin => "projectadmin",
            Self::Maintainer => "maintainer",
            Self::Developer => "developer",
            Self::Guest => "guest",
            Self::LimitedGuest => "limitedguest",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "projectadmin" | "project_admin" | "admin" => Self::ProjectAdmin,
            "maintainer" => Self::Maintainer,
            "developer" => Self::Developer,
            "guest" => Self::Guest,
            "limitedguest" | "limited_guest" => Self::LimitedGuest,
            _ => return None,
        })
    }

    /// Numeric rank (higher = more privileged). Lets callers compare roles
    /// without spelling out the whole table.
    pub fn rank(&self) -> u8 {
        match self {
            Self::ProjectAdmin => 5,
            Self::Maintainer => 4,
            Self::Developer => 3,
            Self::Guest => 2,
            Self::LimitedGuest => 1,
        }
    }

    pub fn permissions(&self) -> HashSet<Permission> {
        let mut p = HashSet::new();
        // LimitedGuest baseline.
        p.insert(perm(Resource::Repository, Action::Pull));
        if self.rank() < Role::Guest.rank() {
            return p;
        }
        // Guest adds full repository pull.
        p.insert(perm(Resource::Artifact, Action::Read));
        p.insert(perm(Resource::Tag, Action::Read));
        p.insert(perm(Resource::ScanReport, Action::Read));
        if self.rank() < Role::Developer.rank() {
            return p;
        }
        // Developer adds push + scan trigger.
        p.insert(perm(Resource::Repository, Action::Push));
        p.insert(perm(Resource::Artifact, Action::Create));
        p.insert(perm(Resource::Tag, Action::Create));
        p.insert(perm(Resource::Scan, Action::Create));
        if self.rank() < Role::Maintainer.rank() {
            return p;
        }
        // Maintainer adds retention / immutability / robot / scan-policy.
        p.insert(perm(Resource::Tag, Action::Delete));
        p.insert(perm(Resource::ImmutableRule, Action::Manage));
        p.insert(perm(Resource::Retention, Action::Manage));
        p.insert(perm(Resource::Robot, Action::Manage));
        p.insert(perm(Resource::ScanPolicy, Action::Manage));
        if self.rank() < Role::ProjectAdmin.rank() {
            return p;
        }
        // ProjectAdmin adds member + project + webhook + replication management.
        p.insert(perm(Resource::Member, Action::Manage));
        p.insert(perm(Resource::Project, Action::Manage));
        p.insert(perm(Resource::Webhook, Action::Manage));
        p.insert(perm(Resource::Replication, Action::Manage));
        p.insert(perm(Resource::Repository, Action::Delete));
        p
    }
}

fn perm(resource: Resource, action: Action) -> Permission {
    Permission { resource, action }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    Repository,
    Artifact,
    Tag,
    Scan,
    ScanReport,
    ScanPolicy,
    ImmutableRule,
    Retention,
    Robot,
    Member,
    Project,
    Webhook,
    Replication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Read,
    Create,
    Push,
    Pull,
    Delete,
    Manage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Permission {
    pub resource: Resource,
    pub action: Action,
}

#[derive(Debug, Clone)]
pub struct ProjectMember {
    pub subject: String,
    pub role: Role,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectAcl {
    members: Vec<ProjectMember>,
}

impl ProjectAcl {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_member(&mut self, subject: impl Into<String>, role: Role) {
        let s = subject.into();
        self.members.retain(|m| m.subject != s);
        self.members.push(ProjectMember { subject: s, role });
    }
    pub fn remove_member(&mut self, subject: &str) {
        self.members.retain(|m| m.subject != subject);
    }
    pub fn role_for(&self, subject: &str) -> Option<Role> {
        self.members
            .iter()
            .find(|m| m.subject == subject)
            .map(|m| m.role)
    }
    pub fn members(&self) -> &[ProjectMember] {
        &self.members
    }
    pub fn has_permission(&self, subject: &str, want: &Permission) -> bool {
        let Some(role) = self.role_for(subject) else {
            return false;
        };
        role.permissions().contains(want)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_rank_order() {
        assert!(Role::ProjectAdmin.rank() > Role::Maintainer.rank());
        assert!(Role::Maintainer.rank() > Role::Developer.rank());
        assert!(Role::Developer.rank() > Role::Guest.rank());
        assert!(Role::Guest.rank() > Role::LimitedGuest.rank());
    }

    #[test]
    fn limitedguest_can_only_pull_repository() {
        let p = Role::LimitedGuest.permissions();
        assert!(p.contains(&perm(Resource::Repository, Action::Pull)));
        assert!(!p.contains(&perm(Resource::Repository, Action::Push)));
    }

    #[test]
    fn guest_inherits_limitedguest_plus_read_metadata() {
        let p = Role::Guest.permissions();
        assert!(p.contains(&perm(Resource::Repository, Action::Pull)));
        assert!(p.contains(&perm(Resource::Artifact, Action::Read)));
        assert!(p.contains(&perm(Resource::ScanReport, Action::Read)));
        assert!(!p.contains(&perm(Resource::Repository, Action::Push)));
    }

    #[test]
    fn developer_can_push_but_not_manage_retention() {
        let p = Role::Developer.permissions();
        assert!(p.contains(&perm(Resource::Repository, Action::Push)));
        assert!(p.contains(&perm(Resource::Artifact, Action::Create)));
        assert!(!p.contains(&perm(Resource::Retention, Action::Manage)));
    }

    #[test]
    fn maintainer_manages_retention_and_immutability() {
        let p = Role::Maintainer.permissions();
        assert!(p.contains(&perm(Resource::Retention, Action::Manage)));
        assert!(p.contains(&perm(Resource::ImmutableRule, Action::Manage)));
        assert!(p.contains(&perm(Resource::Robot, Action::Manage)));
        // Not project admin.
        assert!(!p.contains(&perm(Resource::Member, Action::Manage)));
    }

    #[test]
    fn projectadmin_can_delete_repository() {
        let p = Role::ProjectAdmin.permissions();
        assert!(p.contains(&perm(Resource::Repository, Action::Delete)));
        assert!(p.contains(&perm(Resource::Member, Action::Manage)));
        assert!(p.contains(&perm(Resource::Replication, Action::Manage)));
    }

    #[test]
    fn parse_role_case_insensitive() {
        assert_eq!(Role::parse("PROJECTADMIN"), Some(Role::ProjectAdmin));
        assert_eq!(Role::parse("admin"), Some(Role::ProjectAdmin));
        assert_eq!(Role::parse("maintainer"), Some(Role::Maintainer));
        assert_eq!(Role::parse("nope"), None);
    }

    #[test]
    fn acl_role_for_returns_member_role() {
        let mut acl = ProjectAcl::new();
        acl.add_member("alice", Role::Maintainer);
        acl.add_member("bob", Role::Guest);
        assert_eq!(acl.role_for("alice"), Some(Role::Maintainer));
        assert_eq!(acl.role_for("bob"), Some(Role::Guest));
        assert_eq!(acl.role_for("charlie"), None);
    }

    #[test]
    fn acl_add_replaces_existing_role() {
        let mut acl = ProjectAcl::new();
        acl.add_member("alice", Role::Guest);
        acl.add_member("alice", Role::Developer);
        assert_eq!(acl.role_for("alice"), Some(Role::Developer));
        assert_eq!(acl.members().len(), 1);
    }

    #[test]
    fn acl_has_permission_checks_role() {
        let mut acl = ProjectAcl::new();
        acl.add_member("dev", Role::Developer);
        assert!(acl.has_permission("dev", &perm(Resource::Repository, Action::Push)));
        assert!(!acl.has_permission("dev", &perm(Resource::Member, Action::Manage)));
    }

    #[test]
    fn acl_has_permission_unknown_subject_denied() {
        let acl = ProjectAcl::new();
        assert!(!acl.has_permission("nobody", &perm(Resource::Repository, Action::Pull)));
    }

    #[test]
    fn acl_remove_drops_member() {
        let mut acl = ProjectAcl::new();
        acl.add_member("alice", Role::Maintainer);
        acl.remove_member("alice");
        assert!(acl.role_for("alice").is_none());
    }

    #[test]
    fn permission_serde_roundtrip() {
        let p = perm(Resource::Repository, Action::Push);
        let j = serde_json::to_string(&p).unwrap();
        let back: Permission = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }
}
