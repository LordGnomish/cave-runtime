// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Line-port of Keycloak composite-role resolution (Apache-2.0, v26.6.2):
//   server-spi/.../models/utils/RoleUtils.java
//       expandCompositeRoles(Set), expandCompositeRolesStream(role, visited), hasRole(Set, target)
//   server-spi-private/.../models/utils/KeycloakModelUtils.java
//       searchFor(role, composite, visited)  -- the engine of RoleModel.hasRole
//
// Keycloak resolves composite roles over the live `RoleModel` graph. Here the graph
// is made explicit (`RoleGraph`: role -> direct composite children) so the *algorithm*
// can be ported and tested byte-for-byte, independent of the persistence backend.
// A role `isComposite()` iff it has at least one composite child, matching upstream.

use std::collections::{HashMap, HashSet, VecDeque};

/// Role identifier. Keycloak uses the role's UUID for equality / visited-tracking
/// (`RoleAdapter.equals`, `searchFor` keys on `getId()`); we mirror that with an
/// opaque string id.
pub type RoleId = String;

/// Directed graph of roles to their *direct* composite child roles.
#[derive(Debug, Default, Clone)]
pub struct RoleGraph {
    /// `parent -> {direct composite children}`.
    composites: HashMap<RoleId, HashSet<RoleId>>,
}

impl RoleGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `child` as a direct composite of `parent`.
    pub fn add_composite(&mut self, parent: impl Into<RoleId>, child: impl Into<RoleId>) {
        self.composites
            .entry(parent.into())
            .or_default()
            .insert(child.into());
    }

    /// `RoleModel.isComposite()` — true iff the role has at least one composite child.
    pub fn is_composite(&self, role: &str) -> bool {
        self.composites.get(role).is_some_and(|c| !c.is_empty())
    }

    /// `RoleModel.getCompositesStream()` — the direct composite children.
    fn composites_of(&self, role: &str) -> impl Iterator<Item = &RoleId> {
        self.composites.get(role).into_iter().flatten()
    }

    /// `KeycloakModelUtils.searchFor(role, composite, visited)`:
    ///
    /// ```text
    /// if visited.contains(composite.id) return false;
    /// visited.add(composite.id);
    /// if (!composite.isComposite()) return false;
    /// compositeRoles = composite.getCompositesStream();
    /// return compositeRoles.contains(role)
    ///     || compositeRoles.anyMatch(x -> x.isComposite() && searchFor(role, x, visited));
    /// ```
    fn search_for(&self, role: &str, composite: &str, visited: &mut HashSet<RoleId>) -> bool {
        if visited.contains(composite) {
            return false;
        }
        visited.insert(composite.to_string());

        if !self.is_composite(composite) {
            return false;
        }

        // Collect once; `contains` then recursive `anyMatch`, preserving upstream order.
        let children: Vec<RoleId> = self.composites_of(composite).cloned().collect();
        if children.iter().any(|c| c == role) {
            return true;
        }
        children
            .iter()
            .any(|x| self.is_composite(x) && self.search_for(role, x, visited))
    }

    /// `RoleModel.hasRole(target)` — `this.equals(target) || searchFor(target, this, {})`.
    pub fn role_has_role(&self, role: &str, target: &str) -> bool {
        role == target || self.search_for(target, role, &mut HashSet::new())
    }

    /// `RoleUtils.expandCompositeRolesStream(role, visited)` — iterative DFS that emits
    /// `role` and every composite reachable from it, tracking `visited` to break cycles.
    /// Faithful to upstream: the *seed* role is emitted but not pre-added to `visited`;
    /// only its (transitively reached) children are added.
    fn expand_one(&self, role: &str, visited: &mut HashSet<RoleId>, out: &mut Vec<RoleId>) {
        if visited.contains(role) {
            return;
        }
        // Upstream uses an ArrayDeque as `stack.add` (addLast) / `stack.pop` (removeFirst).
        let mut stack: VecDeque<RoleId> = VecDeque::new();
        stack.push_back(role.to_string());

        while let Some(current) = stack.pop_front() {
            out.push(current.clone());
            if self.is_composite(&current) {
                for r in self.composites_of(&current).cloned().collect::<Vec<_>>() {
                    if !visited.contains(&r) {
                        visited.insert(r.clone());
                        stack.push_back(r);
                    }
                }
            }
        }
    }

    /// `RoleUtils.expandCompositeRoles(Set<RoleModel>)` — new set with composite roles
    /// expanded. The `visited` set is shared across all roots, exactly as upstream.
    pub fn expand_composite_roles(&self, roots: &[RoleId]) -> HashSet<RoleId> {
        let mut visited: HashSet<RoleId> = HashSet::new();
        let mut out: Vec<RoleId> = Vec::new();
        for r in roots {
            self.expand_one(r, &mut visited, &mut out);
        }
        out.into_iter().collect()
    }

    /// `RoleUtils.hasRole(Set<RoleModel> roles, RoleModel target)`:
    /// `roles.contains(target) || any mapping.hasRole(target)`.
    pub fn set_has_role(&self, roles: &HashSet<RoleId>, target: &str) -> bool {
        roles.contains(target) || roles.iter().any(|m| self.role_has_role(m, target))
    }
}

/// Group identifier (Keycloak keys groups by UUID).
pub type GroupId = String;

/// Group hierarchy with per-group role mappings. Mirrors the slice of `GroupModel`
/// the `RoleUtils` membership helpers depend on: a `parent` pointer and direct role
/// mappings. Role *resolution* delegates to a [`RoleGraph`] so composite expansion is
/// shared with the role algorithms above.
#[derive(Debug, Default, Clone)]
pub struct GroupGraph {
    /// `child -> parent` group id.
    parent: HashMap<GroupId, GroupId>,
    /// `group -> {directly mapped role ids}`.
    roles: HashMap<GroupId, HashSet<RoleId>>,
}

impl GroupGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set `parent` as the parent of `child` (`GroupModel.setParent`).
    pub fn set_parent(&mut self, child: impl Into<GroupId>, parent: impl Into<GroupId>) {
        self.parent.insert(child.into(), parent.into());
    }

    /// Map `role` directly onto `group` (`GroupModel.grantRole`).
    pub fn assign_role(&mut self, group: impl Into<GroupId>, role: impl Into<RoleId>) {
        self.roles.entry(group.into()).or_default().insert(role.into());
    }

    /// `GroupModel.getParent()` — `None` at the top of the hierarchy.
    fn parent_of(&self, group: &str) -> Option<&GroupId> {
        self.parent.get(group)
    }

    fn roles_of(&self, group: &str) -> HashSet<RoleId> {
        self.roles.get(group).cloned().unwrap_or_default()
    }

    /// `RoleUtils.isMember(Stream<GroupModel> groups, targetGroup)` — true if `target`
    /// is in `groups` directly, or is an ancestor of any of them via the parent chain.
    pub fn is_member(&self, groups: &[GroupId], target: &str) -> bool {
        let set: HashSet<&GroupId> = groups.iter().collect();
        if set.contains(&target.to_string()) {
            return true;
        }
        groups.iter().any(|g| {
            let mut child: &str = g;
            while let Some(p) = self.parent_of(child) {
                if p == target {
                    return true;
                }
                child = p;
            }
            false
        })
    }

    /// `RoleUtils.isDirectMember(groups, target)` — exact id match only.
    pub fn is_direct_member(&self, groups: &[GroupId], target: &str) -> bool {
        groups.iter().any(|g| g == target)
    }

    /// `GroupModel.hasRole(role)`:
    /// `RoleUtils.hasRole(ownRoles, role) || (parent != null && parent.hasRole(role))`.
    /// Walks the parent chain so inherited roles count, and delegates composite
    /// expansion to `roles`.
    pub fn group_has_role(&self, group: &str, target: &str, roles: &RoleGraph) -> bool {
        if roles.set_has_role(&self.roles_of(group), target) {
            return true;
        }
        match self.parent_of(group) {
            Some(p) => self.group_has_role(p, target, roles),
            None => false,
        }
    }

    /// `RoleUtils.hasRoleFromGroup(group, target, checkParentGroup)`:
    /// ```text
    /// if (group.hasRole(target)) return true;
    /// if (checkParentGroup) { parent = group.getParent();
    ///     return parent != null && hasRoleFromGroup(parent, target, true); }
    /// return false;
    /// ```
    pub fn has_role_from_group(
        &self,
        group: &str,
        target: &str,
        check_parent_group: bool,
        roles: &RoleGraph,
    ) -> bool {
        if self.group_has_role(group, target, roles) {
            return true;
        }
        if check_parent_group {
            return match self.parent_of(group) {
                Some(p) => self.has_role_from_group(p, target, true, roles),
                None => false,
            };
        }
        false
    }
}
