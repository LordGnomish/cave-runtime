// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Nested folder service — line-port of grafana/grafana
//! `pkg/services/folder` (model.go + folderimpl/folder.go).
//!
//! Implements the standalone nested-folder tree that the dashboard store only
//! covered shallowly: parent/child hierarchy, root-first ancestor walks,
//! subtree height, full-path breadcrumbs, depth-limited create/move with
//! circular-reference detection, and cascading delete.

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
