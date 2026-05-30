// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD integration test for conversation-tree fork/path reconstruction.
//!
//! Faithful port of danny-avila/LibreChat v0.7.6
//! `api/server/utils/import/fork.js`:
//!   - getAllMessagesUpToParent (INCLUDE_BRANCHES)
//!   - getMessagesUpToTargetLevel (TARGET_LEVEL)
//!   - splitAtTargetLevel
//!
//! These are the pure tree-walking algorithms the LibreChat fork/import path
//! uses to reconstruct a conversation subtree from a flat message list.

use cave_chat::conversation_tree::{
    get_all_messages_up_to_parent, get_messages_up_to_target_level, split_at_target_level,
    ConvoNode, NO_PARENT,
};

/// Build the same fixture tree the upstream fork.spec.js uses:
///
///   root (NO_PARENT)
///    └─ child1
///        ├─ grandchild1
///        └─ grandchild2
///    └─ child2
fn fixture() -> Vec<ConvoNode> {
    vec![
        ConvoNode::new("root", NO_PARENT, "root text"),
        ConvoNode::new("child1", "root", "child1 text"),
        ConvoNode::new("child2", "root", "child2 text"),
        ConvoNode::new("grandchild1", "child1", "gc1 text"),
        ConvoNode::new("grandchild2", "child1", "gc2 text"),
    ]
}

fn ids(nodes: &[ConvoNode]) -> Vec<String> {
    let mut v: Vec<String> = nodes.iter().map(|n| n.message_id.clone()).collect();
    v.sort();
    v
}

#[test]
fn all_messages_up_to_parent_includes_path_and_siblings_but_not_target_children() {
    // INCLUDE_BRANCHES from grandchild1: path = root -> child1 -> grandchild1.
    // Include path nodes, their siblings (child2 sibling of child1; grandchild2
    // sibling of grandchild1), the target itself, but NOT children of the target.
    let tree = fixture();
    let result = get_all_messages_up_to_parent(&tree, "grandchild1");
    let got = ids(&result);
    // root, child1, child2 (sibling of child1), grandchild1 (target), grandchild2 (sibling)
    assert_eq!(
        got,
        vec![
            "child1".to_string(),
            "child2".to_string(),
            "grandchild1".to_string(),
            "grandchild2".to_string(),
            "root".to_string(),
        ]
    );
}

#[test]
fn all_messages_up_to_parent_missing_target_returns_empty() {
    let tree = fixture();
    assert!(get_all_messages_up_to_parent(&tree, "nope").is_empty());
}

#[test]
fn messages_up_to_target_level_returns_levels_down_to_target() {
    // TARGET_LEVEL from child1: BFS from roots collecting every node down to and
    // including the level where the target appears.
    let tree = fixture();
    let result = get_messages_up_to_target_level(&tree, "child1");
    let got = ids(&result);
    // root level + child level (child1, child2). grandchildren are deeper -> excluded.
    assert_eq!(
        got,
        vec!["child1".to_string(), "child2".to_string(), "root".to_string()]
    );
}

#[test]
fn messages_up_to_target_level_target_at_root_returns_root_level() {
    let tree = fixture();
    let result = get_messages_up_to_target_level(&tree, "root");
    // target is at root level -> just the root-level set
    let got = ids(&result);
    assert_eq!(got, vec!["root".to_string()]);
}

#[test]
fn split_at_target_level_reparents_target_level_to_root() {
    // Split at child level: child1 and child2 become roots (NO_PARENT),
    // their descendants kept, root dropped (above the target level).
    let tree = fixture();
    let result = split_at_target_level(&tree, "child1");
    // child1, child2 reparented; grandchild1, grandchild2 kept; root removed.
    let got = ids(&result);
    assert_eq!(
        got,
        vec![
            "child1".to_string(),
            "child2".to_string(),
            "grandchild1".to_string(),
            "grandchild2".to_string(),
        ]
    );
    // The target-level nodes must now point at NO_PARENT.
    for n in &result {
        if n.message_id == "child1" || n.message_id == "child2" {
            assert_eq!(n.parent_message_id, NO_PARENT);
        }
    }
    // descendants keep their parent
    let gc1 = result.iter().find(|n| n.message_id == "grandchild1").unwrap();
    assert_eq!(gc1.parent_message_id, "child1");
}
