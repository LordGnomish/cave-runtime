// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kafka ACL management — per topic, group, and cluster resources.

use crate::error::{StreamsError, StreamsResult};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

// ── Resource types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ResourceType {
    Any,
    Topic,
    Group,
    Cluster,
    TransactionalId,
    DelegationToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PatternType {
    /// Matches the exact resource name
    Literal,
    /// Matches any resource whose name starts with the pattern
    Prefixed,
    /// Matches any resource
    Any,
    /// Used in filters to match both Literal and Prefixed
    Match,
}

// ── Operation / permission ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Operation {
    Any,
    All,
    Read,
    Write,
    Create,
    Delete,
    Alter,
    Describe,
    ClusterAction,
    DescribeConfigs,
    AlterConfigs,
    IdempotentWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PermissionType {
    Any,
    Deny,
    Allow,
}

// ── ACL binding ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclBinding {
    pub resource_type: ResourceType,
    pub resource_name: String,
    pub pattern_type: PatternType,
    pub principal: String,
    pub host: String,
    pub operation: Operation,
    pub permission: PermissionType,
}

impl AclBinding {
    pub fn allow_topic_read(principal: &str, topic: &str) -> Self {
        Self {
            resource_type: ResourceType::Topic,
            resource_name: topic.to_string(),
            pattern_type: PatternType::Literal,
            principal: principal.to_string(),
            host: "*".to_string(),
            operation: Operation::Read,
            permission: PermissionType::Allow,
        }
    }

    pub fn allow_topic_write(principal: &str, topic: &str) -> Self {
        Self {
            resource_type: ResourceType::Topic,
            resource_name: topic.to_string(),
            pattern_type: PatternType::Literal,
            principal: principal.to_string(),
            host: "*".to_string(),
            operation: Operation::Write,
            permission: PermissionType::Allow,
        }
    }

    pub fn allow_group_read(principal: &str, group: &str) -> Self {
        Self {
            resource_type: ResourceType::Group,
            resource_name: group.to_string(),
            pattern_type: PatternType::Literal,
            principal: principal.to_string(),
            host: "*".to_string(),
            operation: Operation::Read,
            permission: PermissionType::Allow,
        }
    }
}

/// Filter used in DescribeAcls / DeleteAcls
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclFilter {
    pub resource_type: Option<ResourceType>,
    pub resource_name: Option<String>,
    pub pattern_type: Option<PatternType>,
    pub principal: Option<String>,
    pub host: Option<String>,
    pub operation: Option<Operation>,
    pub permission: Option<PermissionType>,
}

impl AclFilter {
    pub fn matches(&self, acl: &AclBinding) -> bool {
        if let Some(ref rt) = self.resource_type {
            if rt != &ResourceType::Any && rt != &acl.resource_type {
                return false;
            }
        }
        if let Some(ref rn) = self.resource_name {
            if !rn.is_empty() && rn != &acl.resource_name {
                return false;
            }
        }
        if let Some(ref principal) = self.principal {
            if !principal.is_empty() && principal != &acl.principal {
                return false;
            }
        }
        if let Some(ref op) = self.operation {
            if op != &Operation::Any && op != &acl.operation {
                return false;
            }
        }
        if let Some(ref perm) = self.permission {
            if perm != &PermissionType::Any && perm != &acl.permission {
                return false;
            }
        }
        true
    }
}

// ── ACL store ─────────────────────────────────────────────────────────────────

pub struct AclStore {
    /// key = "{resource_type}/{resource_name}" → bindings
    acls: DashMap<String, Vec<AclBinding>>,
}

impl AclStore {
    pub fn new() -> Self {
        Self {
            acls: DashMap::new(),
        }
    }

    fn key(rt: &ResourceType, name: &str) -> String {
        format!("{rt:?}/{name}")
    }

    pub fn create_acl(&self, binding: AclBinding) {
        let key = Self::key(&binding.resource_type, &binding.resource_name);
        self.acls
            .entry(key)
            .or_default()
            .push(binding);
    }

    pub fn create_acls(&self, bindings: Vec<AclBinding>) {
        for b in bindings {
            self.create_acl(b);
        }
    }

    pub fn describe_acls(&self, filter: &AclFilter) -> Vec<AclBinding> {
        self.acls
            .iter()
            .flat_map(|e| e.value().clone())
            .filter(|acl| filter.matches(acl))
            .collect()
    }

    pub fn delete_acls(&self, filter: &AclFilter) -> Vec<AclBinding> {
        let mut deleted = Vec::new();
        for mut entry in self.acls.iter_mut() {
            let (keep, remove): (Vec<_>, Vec<_>) =
                entry.value().iter().cloned().partition(|acl| !filter.matches(acl));
            deleted.extend(remove);
            *entry.value_mut() = keep;
        }
        deleted
    }

    /// Check if a principal is allowed to perform an operation on a resource.
    pub fn is_allowed(
        &self,
        principal: &str,
        resource_type: &ResourceType,
        resource_name: &str,
        operation: &Operation,
    ) -> bool {
        let key = Self::key(resource_type, resource_name);
        let Some(bindings) = self.acls.get(&key) else {
            // Also check prefixed patterns
            return self.check_prefixed(principal, resource_type, resource_name, operation);
        };

        // Deny takes precedence
        let denied = bindings.iter().any(|b| {
            b.permission == PermissionType::Deny
                && (b.principal == "*" || b.principal == principal)
                && (b.operation == Operation::Any
                    || b.operation == Operation::All
                    || &b.operation == operation)
        });
        if denied {
            return false;
        }

        bindings.iter().any(|b| {
            b.permission == PermissionType::Allow
                && (b.principal == "*" || b.principal == principal)
                && (b.operation == Operation::Any
                    || b.operation == Operation::All
                    || &b.operation == operation)
        })
    }

    fn check_prefixed(
        &self,
        principal: &str,
        resource_type: &ResourceType,
        resource_name: &str,
        operation: &Operation,
    ) -> bool {
        for entry in self.acls.iter() {
            for binding in entry.value() {
                if binding.pattern_type == PatternType::Prefixed
                    && &binding.resource_type == resource_type
                    && resource_name.starts_with(&binding.resource_name)
                    && binding.permission == PermissionType::Allow
                    && (binding.principal == "*" || binding.principal == principal)
                    && (&binding.operation == operation
                        || binding.operation == Operation::Any
                        || binding.operation == Operation::All)
                {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_describe_acls() {
        let store = AclStore::new();
        store.create_acl(AclBinding::allow_topic_read("User:alice", "orders"));
        store.create_acl(AclBinding::allow_topic_write("User:bob", "orders"));

        let filter = AclFilter {
            resource_type: Some(ResourceType::Topic),
            resource_name: Some("orders".into()),
            pattern_type: None,
            principal: None,
            host: None,
            operation: None,
            permission: None,
        };
        let acls = store.describe_acls(&filter);
        assert_eq!(acls.len(), 2);
    }

    #[test]
    fn delete_acls() {
        let store = AclStore::new();
        store.create_acl(AclBinding::allow_topic_read("User:alice", "events"));
        let filter = AclFilter {
            resource_type: Some(ResourceType::Topic),
            resource_name: Some("events".into()),
            pattern_type: None,
            principal: Some("User:alice".into()),
            host: None,
            operation: Some(Operation::Read),
            permission: Some(PermissionType::Allow),
        };
        let deleted = store.delete_acls(&filter);
        assert_eq!(deleted.len(), 1);
        assert!(store.describe_acls(&AclFilter {
            resource_type: None, resource_name: None, pattern_type: None,
            principal: None, host: None, operation: None, permission: None
        }).is_empty());
    }

    #[test]
    fn is_allowed_check() {
        let store = AclStore::new();
        store.create_acl(AclBinding::allow_topic_read("User:alice", "data"));
        assert!(store.is_allowed("User:alice", &ResourceType::Topic, "data", &Operation::Read));
        assert!(!store.is_allowed("User:alice", &ResourceType::Topic, "data", &Operation::Write));
        assert!(!store.is_allowed("User:eve", &ResourceType::Topic, "data", &Operation::Read));
    }
}
