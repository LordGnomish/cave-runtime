// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Conversation-tree fork / path reconstruction.
//!
//! Faithful line-port of danny-avila/LibreChat v0.7.6
//! `api/server/utils/import/fork.js` — the pure tree-walking algorithms used by
//! the fork / import path to reconstruct a conversation subtree from a flat
//! message list. The DB-bound parts of `forkConversation` / `duplicateConversation`
//! (getConvo / getMessages / saveBatch) are intentionally NOT ported here — those
//! are persistence concerns. Only the in-process tree algorithms live in-crate:
//!
//!   - `getAllMessagesUpToParent`  -> [`get_all_messages_up_to_parent`]  (INCLUDE_BRANCHES)
//!   - `getMessagesUpToTargetLevel`-> [`get_messages_up_to_target_level`] (TARGET_LEVEL)
//!   - `splitAtTargetLevel`        -> [`split_at_target_level`]
//!
//! LibreChat models a message tree by `messageId` + `parentMessageId`, where the
//! sentinel `Constants.NO_PARENT` ("00000000-0000-0000-0000-000000000000")
//! marks a root message.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Sentinel parent id marking a root message.
///
/// Mirrors LibreChat `Constants.NO_PARENT`
/// (`packages/data-provider/src/parsers.ts`): `'00000000-0000-0000-0000-000000000000'`.
pub const NO_PARENT: &str = "00000000-0000-0000-0000-000000000000";

/// A node in a conversation message tree.
///
/// Carries only the fields the fork/import tree algorithms read (`messageId`,
/// `parentMessageId`) plus the text payload, so a port is faithful without
/// dragging in the full `TMessage` shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConvoNode {
    #[serde(rename = "messageId")]
    pub message_id: String,
    #[serde(rename = "parentMessageId")]
    pub parent_message_id: String,
    pub text: String,
}

impl ConvoNode {
    /// Build a node. `parent_message_id` should be [`NO_PARENT`] for a root.
    pub fn new(
        message_id: impl Into<String>,
        parent_message_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            message_id: message_id.into(),
            parent_message_id: parent_message_id.into(),
            text: text.into(),
        }
    }
}

/// Retrieves all messages up to the root from the target message, plus siblings
/// along the path, excluding the children of the target message.
///
/// Faithful port of `getAllMessagesUpToParent` (fork.js L174-208) — used by the
/// `ForkOptions.INCLUDE_BRANCHES` fork option.
pub fn get_all_messages_up_to_parent(
    messages: &[ConvoNode],
    target_message_id: &str,
) -> Vec<ConvoNode> {
    let by_id: HashMap<&str, &ConvoNode> =
        messages.iter().map(|m| (m.message_id.as_str(), m)).collect();

    let target = match by_id.get(target_message_id) {
        Some(t) => *t,
        None => return Vec::new(),
    };

    let mut path_to_root: HashSet<String> = HashSet::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut current: Option<&ConvoNode> = Some(target);

    while let Some(node) = current {
        if visited.contains(&node.message_id) {
            break;
        }
        visited.insert(node.message_id.clone());
        path_to_root.insert(node.message_id.clone());

        let parent_id = node.parent_message_id.as_str();
        if parent_id == NO_PARENT {
            break;
        }
        current = by_id.get(parent_id).copied();
    }

    // Include all messages that are in the path or whose parent is in the path,
    // exclude children of the target message (but always include the target).
    messages
        .iter()
        .filter(|msg| {
            (path_to_root.contains(&msg.message_id) && msg.message_id != target_message_id)
                || (path_to_root.contains(&msg.parent_message_id)
                    && msg.parent_message_id != target_message_id)
                || msg.message_id == target_message_id
        })
        .cloned()
        .collect()
}

/// Retrieves all messages from the roots down to (and including) the level at
/// which the target message lives, via a breadth-first level walk.
///
/// Faithful port of `getMessagesUpToTargetLevel` (fork.js L216-278) — used by
/// the default `ForkOptions.TARGET_LEVEL` fork option.
pub fn get_messages_up_to_target_level(
    messages: &[ConvoNode],
    target_message_id: &str,
) -> Vec<ConvoNode> {
    if messages.len() == 1 && messages[0].message_id == target_message_id {
        return messages.to_vec();
    }

    // Map of parentMessageId -> children messages.
    let mut parent_to_children: HashMap<&str, Vec<&ConvoNode>> = HashMap::new();
    for message in messages {
        parent_to_children
            .entry(message.parent_message_id.as_str())
            .or_default()
            .push(message);
    }

    let target = match messages.iter().find(|m| m.message_id == target_message_id) {
        Some(t) => t,
        None => return Vec::new(), // upstream logs "Target message not found."
    };

    let mut visited: HashSet<String> = HashSet::new();

    let root_messages: Vec<&ConvoNode> = parent_to_children
        .get(NO_PARENT)
        .cloned()
        .unwrap_or_default();
    let mut current_level: Vec<&ConvoNode> = if !root_messages.is_empty() {
        root_messages.clone()
    } else {
        vec![target]
    };

    // Preserve first-seen order while de-duplicating, matching JS `Set` semantics.
    let mut order: Vec<&ConvoNode> = Vec::new();
    let mut in_results: HashSet<String> = HashSet::new();
    for n in &current_level {
        if in_results.insert(n.message_id.clone()) {
            order.push(n);
        }
    }

    // If the target is at root level, return the root-level set.
    if current_level
        .iter()
        .any(|m| m.message_id == target_message_id)
        && target.parent_message_id == NO_PARENT
    {
        return order.into_iter().cloned().collect();
    }

    let mut target_found = false;
    while !target_found && !current_level.is_empty() {
        let mut next_level: Vec<&ConvoNode> = Vec::new();
        for node in &current_level {
            if visited.contains(&node.message_id) {
                continue; // cycle guard
            }
            visited.insert(node.message_id.clone());
            if let Some(children) = parent_to_children.get(node.message_id.as_str()) {
                for child in children {
                    if visited.contains(&child.message_id) {
                        continue; // cycle guard
                    }
                    next_level.push(child);
                    if in_results.insert(child.message_id.clone()) {
                        order.push(child);
                    }
                    if child.message_id == target_message_id {
                        target_found = true;
                    }
                }
            }
        }
        current_level = next_level;
    }

    order.into_iter().cloned().collect()
}

/// Splits the conversation at the target message level: the target, its siblings,
/// and all descendants are kept; everything above the target level is dropped, and
/// target-level messages are reparented to [`NO_PARENT`].
///
/// Faithful port of `splitAtTargetLevel` (fork.js L287-354).
pub fn split_at_target_level(
    messages: &[ConvoNode],
    target_message_id: &str,
) -> Vec<ConvoNode> {
    // Map of parentMessageId -> children messages.
    let mut parent_to_children: HashMap<&str, Vec<&ConvoNode>> = HashMap::new();
    for message in messages {
        parent_to_children
            .entry(message.parent_message_id.as_str())
            .or_default()
            .push(message);
    }

    if !messages.iter().any(|m| m.message_id == target_message_id) {
        return Vec::new(); // upstream logs "Target message not found."
    }

    // Assign a level to every reachable message via a level-by-level walk.
    let mut level_map: HashMap<String, usize> = HashMap::new();
    let root_messages: Vec<&ConvoNode> = parent_to_children
        .get(NO_PARENT)
        .cloned()
        .unwrap_or_default();
    for m in &root_messages {
        level_map.insert(m.message_id.clone(), 0);
    }

    let mut current_level: Vec<&ConvoNode> = root_messages;
    let mut current_level_index: usize = 0;
    while !current_level.is_empty() {
        let mut next_level: Vec<&ConvoNode> = Vec::new();
        for node in &current_level {
            if let Some(children) = parent_to_children.get(node.message_id.as_str()) {
                for child in children {
                    next_level.push(child);
                    level_map.insert(child.message_id.clone(), current_level_index + 1);
                }
            }
        }
        current_level = next_level;
        current_level_index += 1;
    }

    let target_level = match level_map.get(target_message_id) {
        Some(l) => *l,
        None => return Vec::new(), // "Target level not found."
    };

    // Keep messages at or below the target level; reparent the target level to root.
    messages
        .iter()
        .filter_map(|msg| {
            let message_level = level_map.get(&msg.message_id).copied();
            match message_level {
                Some(level) if level < target_level => None,
                Some(level) if level == target_level => {
                    let mut cloned = msg.clone();
                    cloned.parent_message_id = NO_PARENT.to_string();
                    Some(cloned)
                }
                _ => Some(msg.clone()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Vec<ConvoNode> {
        vec![
            ConvoNode::new("root", NO_PARENT, "r"),
            ConvoNode::new("c1", "root", "c1"),
            ConvoNode::new("c2", "root", "c2"),
            ConvoNode::new("gc1", "c1", "gc1"),
        ]
    }

    #[test]
    fn up_to_parent_single_node() {
        let single = vec![ConvoNode::new("only", NO_PARENT, "x")];
        let r = get_all_messages_up_to_parent(&single, "only");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].message_id, "only");
    }

    #[test]
    fn target_level_single_node() {
        let single = vec![ConvoNode::new("only", NO_PARENT, "x")];
        let r = get_messages_up_to_target_level(&single, "only");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn split_drops_above_and_keeps_below() {
        let r = split_at_target_level(&tree(), "c1");
        let ids: HashSet<&str> = r.iter().map(|n| n.message_id.as_str()).collect();
        assert!(ids.contains("c1"));
        assert!(ids.contains("c2"));
        assert!(ids.contains("gc1"));
        assert!(!ids.contains("root"));
    }

    #[test]
    fn split_missing_target_returns_empty() {
        assert!(split_at_target_level(&tree(), "ghost").is_empty());
    }
}
