// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Nested folder service — line-port of grafana/grafana
//! `pkg/services/folder` (model.go + folderimpl/folder.go).
//!
//! Implements the standalone nested-folder tree that the dashboard store only
//! covered shallowly: parent/child hierarchy, root-first ancestor walks,
//! subtree height, full-path breadcrumbs, depth-limited create/move with
//! circular-reference detection, and cascading delete.

use std::collections::HashMap;

/// Maximum number of ancestors a folder's parent may have — `model.go`
/// `MaxNestedFolderDepth`.
pub const MAX_NESTED_FOLDER_DEPTH: usize = 4;
/// `model.go` `GeneralFolderUID`.
pub const GENERAL_FOLDER_UID: &str = "general";
/// `model.go` `RootFolderUID` (the empty-string root).
pub const ROOT_FOLDER_UID: &str = "";
/// `model.go` `SharedWithMeFolderUID`.
pub const SHARED_WITH_ME_FOLDER_UID: &str = "sharedwithme";

/// Folder operation errors, mirroring `model.go`'s `errutil` errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderError {
    NotFound,
    CircularReference,
    MaximumDepthReached,
    AlreadyExists,
    ParentNotFound,
}

/// A single folder node in the nested-folder tree (`folder.Folder` subset).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderNode {
    pub uid: String,
    pub title: String,
    /// `None` = root-level (parent_uid == "").
    pub parent_uid: Option<String>,
}

/// In-memory nested-folder tree. Equivalent to the `folder.Service` operating
/// over a single org.
#[derive(Debug, Clone, Default)]
pub struct FolderTree {
    nodes: HashMap<String, FolderNode>,
}

impl FolderTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, uid: &str) -> Option<&FolderNode> {
        self.nodes.get(uid)
    }

    /// Ancestors of `uid`, ordered root-first, excluding `uid` itself.
    /// Mirrors `GetParents` (the store walks up then `util.Reverse`).
    pub fn get_parents(&self, uid: &str) -> Vec<&FolderNode> {
        let mut chain: Vec<&FolderNode> = Vec::new();
        let mut cursor = self.nodes.get(uid).and_then(|f| f.parent_uid.clone());
        let mut guard = 0;
        while let Some(p) = cursor {
            let Some(node) = self.nodes.get(&p) else { break };
            chain.push(node);
            cursor = node.parent_uid.clone();
            guard += 1;
            if guard > MAX_NESTED_FOLDER_DEPTH + 1 {
                break; // defensive: corrupted cycle
            }
        }
        chain.reverse();
        chain
    }

    /// Direct children of `uid` (or root-level folders when `parent` is `None`).
    pub fn get_children(&self, parent: Option<&str>) -> Vec<&FolderNode> {
        let mut out: Vec<&FolderNode> = self
            .nodes
            .values()
            .filter(|f| f.parent_uid.as_deref() == parent)
            .collect();
        out.sort_by(|a, b| a.uid.cmp(&b.uid));
        out
    }

    /// All descendants of `uid` (breadth-first), excluding `uid` itself.
    pub fn get_descendants(&self, uid: &str) -> Vec<&FolderNode> {
        let mut out = Vec::new();
        let mut queue: Vec<String> = vec![uid.to_string()];
        let mut guard = 0;
        while let Some(cur) = queue.pop() {
            for child in self.get_children(Some(&cur)) {
                out.push(child);
                queue.push(child.uid.clone());
            }
            guard += 1;
            if guard > self.nodes.len() + 1 {
                break;
            }
        }
        out
    }

    /// Height of the subtree rooted at `uid` — the longest downward path
    /// length. A leaf has height 0. Mirrors `GetHeight` BFS semantics.
    pub fn get_height(&self, uid: &str) -> usize {
        let mut height: isize = -1;
        let mut queue: Vec<String> = vec![uid.to_string()];
        let mut guard = 0;
        while !queue.is_empty() {
            height += 1;
            let level = std::mem::take(&mut queue);
            for ele in level {
                for child in self.get_children(Some(&ele)) {
                    queue.push(child.uid.clone());
                }
            }
            guard += 1;
            if guard > self.nodes.len() + 2 {
                break;
            }
        }
        height.max(0) as usize
    }

    fn path_components(&self, uid: &str) -> Vec<&FolderNode> {
        let mut comps = self.get_parents(uid);
        if let Some(node) = self.nodes.get(uid) {
            comps.push(node);
        }
        comps
    }

    /// Full title breadcrumb, root → leaf, slash-joined (slashes in titles
    /// escaped, as in `setFullpath`).
    pub fn fullpath(&self, uid: &str) -> String {
        self.path_components(uid)
            .iter()
            .map(|f| f.title.replace('/', "\\/"))
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Full UID breadcrumb, root → leaf, slash-joined (`FullpathUIDs`).
    pub fn fullpath_uids(&self, uid: &str) -> String {
        self.path_components(uid)
            .iter()
            .map(|f| f.uid.clone())
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Create a folder. Faithful to `validateParent` + store create:
    /// duplicate-uid, unknown-parent, depth (`len(ancestors(parent)) >=
    /// MaxNestedFolderDepth`), and circular guards.
    pub fn create(
        &mut self,
        uid: &str,
        title: &str,
        parent_uid: Option<&str>,
    ) -> Result<(), FolderError> {
        if self.nodes.contains_key(uid) {
            return Err(FolderError::AlreadyExists);
        }
        if let Some(p) = parent_uid {
            if p == uid {
                return Err(FolderError::CircularReference);
            }
            if !self.nodes.contains_key(p) {
                return Err(FolderError::ParentNotFound);
            }
            let ancestors = self.get_parents(p);
            if ancestors.len() >= MAX_NESTED_FOLDER_DEPTH {
                return Err(FolderError::MaximumDepthReached);
            }
            for ancestor in ancestors {
                if ancestor.uid == uid {
                    return Err(FolderError::CircularReference);
                }
            }
        }
        self.nodes.insert(
            uid.to_string(),
            FolderNode {
                uid: uid.to_string(),
                title: title.to_string(),
                parent_uid: parent_uid.filter(|p| !p.is_empty()).map(|p| p.to_string()),
            },
        );
        Ok(())
    }

    /// Move a folder to a new parent (or to root with `None`). Faithful to
    /// `MoveLegacy`: into-self / into-descendant circular guards and the
    /// `height + len(parents) + 1 > MaxNestedFolderDepth` limit.
    pub fn move_folder(&mut self, uid: &str, new_parent: Option<&str>) -> Result<(), FolderError> {
        if !self.nodes.contains_key(uid) {
            return Err(FolderError::NotFound);
        }
        if let Some(np) = new_parent {
            if np == uid {
                return Err(FolderError::CircularReference);
            }
            if !self.nodes.contains_key(np) {
                return Err(FolderError::ParentNotFound);
            }
            // moving into one's own descendant is a cycle (GetHeight guard).
            if self.get_descendants(uid).iter().any(|f| f.uid == np) {
                return Err(FolderError::CircularReference);
            }
            let parents = self.get_parents(np);
            // also explicit: uid among the new parent's ancestors.
            if parents.iter().any(|f| f.uid == uid) {
                return Err(FolderError::CircularReference);
            }
            let height = self.get_height(uid);
            if height + parents.len() + 1 > MAX_NESTED_FOLDER_DEPTH {
                return Err(FolderError::MaximumDepthReached);
            }
        }
        let node = self.nodes.get_mut(uid).unwrap();
        node.parent_uid = new_parent.filter(|p| !p.is_empty()).map(|p| p.to_string());
        Ok(())
    }

    /// Delete a folder and all of its descendants. Returns the number of
    /// folders removed.
    pub fn delete(&mut self, uid: &str) -> Result<usize, FolderError> {
        if !self.nodes.contains_key(uid) {
            return Err(FolderError::NotFound);
        }
        let mut to_remove: Vec<String> =
            self.get_descendants(uid).iter().map(|f| f.uid.clone()).collect();
        to_remove.push(uid.to_string());
        let n = to_remove.len();
        for u in to_remove {
            self.nodes.remove(&u);
        }
        Ok(n)
    }

    /// Number of folders in the tree.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_folder_constants() {
        assert_eq!(MAX_NESTED_FOLDER_DEPTH, 4);
        assert_eq!(GENERAL_FOLDER_UID, "general");
        assert_eq!(ROOT_FOLDER_UID, "");
        assert_eq!(SHARED_WITH_ME_FOLDER_UID, "sharedwithme");
    }

    /// Build the chain a(root) → b → c → d.
    fn chain_abcd() -> FolderTree {
        let mut t = FolderTree::new();
        t.create("a", "A", None).unwrap();
        t.create("b", "B", Some("a")).unwrap();
        t.create("c", "C", Some("b")).unwrap();
        t.create("d", "D", Some("c")).unwrap();
        t
    }

    #[test]
    fn test_create_root_and_get_parents_root_first() {
        let t = chain_abcd();
        let parents: Vec<&str> = t.get_parents("d").iter().map(|f| f.uid.as_str()).collect();
        // GetParents returns ancestors root-first, excluding self.
        assert_eq!(parents, vec!["a", "b", "c"]);
        assert!(t.get_parents("a").is_empty());
    }

    #[test]
    fn test_create_fifth_level_ok_sixth_fails() {
        // create allows a parent with up to MaxNestedFolderDepth-1 ancestors,
        // so a/b/c/d/e (5 levels) is fine but the 6th create fails.
        let mut t = chain_abcd();
        t.create("e", "E", Some("d")).unwrap();
        assert_eq!(
            t.create("f", "F", Some("e")),
            Err(FolderError::MaximumDepthReached)
        );
    }

    #[test]
    fn test_create_duplicate_uid_fails() {
        let mut t = chain_abcd();
        assert_eq!(t.create("b", "B2", Some("a")), Err(FolderError::AlreadyExists));
    }

    #[test]
    fn test_create_unknown_parent_fails() {
        let mut t = FolderTree::new();
        assert_eq!(t.create("x", "X", Some("nope")), Err(FolderError::ParentNotFound));
    }

    #[test]
    fn test_get_height() {
        let t = chain_abcd();
        assert_eq!(t.get_height("a"), 3); // a→b→c→d
        assert_eq!(t.get_height("c"), 1);
        assert_eq!(t.get_height("d"), 0); // leaf
    }

    #[test]
    fn test_children_and_descendants() {
        let mut t = chain_abcd();
        t.create("b2", "B2", Some("a")).unwrap();
        let children: Vec<&str> = t.get_children(Some("a")).iter().map(|f| f.uid.as_str()).collect();
        assert!(children.contains(&"b") && children.contains(&"b2") && children.len() == 2);
        let desc: Vec<&str> = t.get_descendants("a").iter().map(|f| f.uid.as_str()).collect();
        for u in ["b", "c", "d", "b2"] {
            assert!(desc.contains(&u), "descendants missing {u}");
        }
        assert!(!desc.contains(&"a"));
        // top-level (root) children
        let top: Vec<&str> = t.get_children(None).iter().map(|f| f.uid.as_str()).collect();
        assert_eq!(top, vec!["a"]);
    }

    #[test]
    fn test_fullpath_and_fullpath_uids() {
        let t = chain_abcd();
        assert_eq!(t.fullpath("d"), "A/B/C/D");
        assert_eq!(t.fullpath_uids("d"), "a/b/c/d");
        assert_eq!(t.fullpath("a"), "A");
    }

    #[test]
    fn test_move_reassigns_parent() {
        let mut t = chain_abcd();
        t.create("x", "X", None).unwrap();
        t.move_folder("x", Some("a")).unwrap();
        assert_eq!(t.get("x").unwrap().parent_uid.as_deref(), Some("a"));
        assert_eq!(t.fullpath_uids("x"), "a/x");
    }

    #[test]
    fn test_move_into_self_is_circular() {
        let mut t = chain_abcd();
        assert_eq!(t.move_folder("a", Some("a")), Err(FolderError::CircularReference));
    }

    #[test]
    fn test_move_into_descendant_is_circular() {
        let mut t = chain_abcd();
        // moving a under b (b is a's child) would create a cycle.
        assert_eq!(t.move_folder("a", Some("b")), Err(FolderError::CircularReference));
    }

    #[test]
    fn test_move_depth_exceeded() {
        let mut t = chain_abcd();
        // x has a child y, so height(x) = 1.
        t.create("x", "X", None).unwrap();
        t.create("y", "Y", Some("x")).unwrap();
        // move x under d: height(x)=1 + parents(d)=3 + 1 = 5 > 4 → max depth.
        assert_eq!(t.move_folder("x", Some("d")), Err(FolderError::MaximumDepthReached));
        // move x under c: 1 + parents(c)=2 + 1 = 4, allowed.
        assert!(t.move_folder("x", Some("c")).is_ok());
    }

    #[test]
    fn test_move_to_root() {
        let mut t = chain_abcd();
        t.move_folder("c", None).unwrap();
        assert_eq!(t.get("c").unwrap().parent_uid, None);
        // c keeps its own subtree (d follows).
        assert_eq!(t.fullpath_uids("d"), "c/d");
    }

    #[test]
    fn test_delete_cascades() {
        let mut t = chain_abcd();
        let removed = t.delete("b").unwrap();
        assert_eq!(removed, 3); // b, c, d
        assert!(t.get("b").is_none());
        assert!(t.get("d").is_none());
        assert!(t.get("a").is_some());
    }

    #[test]
    fn test_move_unknown_folder_errors() {
        let mut t = chain_abcd();
        assert_eq!(t.move_folder("zzz", Some("a")), Err(FolderError::NotFound));
    }
}
