// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pure-function business-logic helpers (no I/O).

use crate::models::{Message, Reaction};

/// Returns true if `user_id` (string) is in the members slice.
pub fn is_member_str(room_members: &[String], user_id: &str) -> bool {
    room_members.iter().any(|m| m == user_id)
}

/// Count total messages.
pub fn message_count(messages: &[Message]) -> usize {
    messages.len()
}

/// Filter messages by author.
pub fn messages_by_author<'a>(messages: &'a [Message], author_id: &str) -> Vec<&'a Message> {
    messages
        .iter()
        .filter(|m| m.author_id == author_id)
        .collect()
}

/// Add a reaction (idempotent per user+emoji pair).
pub fn add_reaction(message: &mut Message, emoji: &str, user_id: String) {
    if let Some(r) = message.reactions.iter_mut().find(|r| r.emoji == emoji) {
        if !r.users.contains(&user_id) {
            r.users.push(user_id);
        }
    } else {
        message.reactions.push(Reaction {
            emoji: emoji.to_string(),
            users: vec![user_id],
        });
    }
}

/// Sum of all reaction users across all emoji groups.
pub fn total_reactions(message: &Message) -> usize {
    message.reactions.iter().map(|r| r.users.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Message, MessageType};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_message(author_id: &str) -> Message {
        Message {
            id: Uuid::new_v4(),
            channel_id: Uuid::new_v4(),
            author_id: author_id.to_string(),
            content: "Hello!".to_string(),
            message_type: MessageType::Text,
            created_at: Utc::now(),
            edited_at: None,
            reactions: vec![],
            thread_root_id: None,
            thread_replies: vec![],
        }
    }

    #[test]
    fn test_is_member_str_true() {
        let members = vec!["alice".to_string(), "bob".to_string()];
        assert!(is_member_str(&members, "alice"));
    }

    #[test]
    fn test_is_member_str_false() {
        let members = vec!["alice".to_string()];
        assert!(!is_member_str(&members, "charlie"));
    }

    #[test]
    fn test_messages_by_author() {
        let messages = vec![
            make_message("alice"),
            make_message("bob"),
            make_message("alice"),
        ];
        let by_alice = messages_by_author(&messages, "alice");
        assert_eq!(by_alice.len(), 2);
        for m in &by_alice {
            assert_eq!(m.author_id, "alice");
        }
    }

    #[test]
    fn test_add_reaction_new() {
        let mut msg = make_message("alice");
        add_reaction(&mut msg, "👍", "bob".to_string());
        assert_eq!(msg.reactions.len(), 1);
        assert_eq!(msg.reactions[0].emoji, "👍");
        assert_eq!(msg.reactions[0].users.len(), 1);
    }

    #[test]
    fn test_add_reaction_existing_emoji_no_duplicate() {
        let mut msg = make_message("alice");
        add_reaction(&mut msg, "👍", "bob".to_string());
        add_reaction(&mut msg, "👍", "bob".to_string()); // same user again — no duplicate
        assert_eq!(msg.reactions.len(), 1);
        assert_eq!(msg.reactions[0].users.len(), 1);
    }

    #[test]
    fn test_add_reaction_second_user_same_emoji() {
        let mut msg = make_message("alice");
        add_reaction(&mut msg, "👍", "bob".to_string());
        add_reaction(&mut msg, "👍", "carol".to_string());
        assert_eq!(msg.reactions.len(), 1);
        assert_eq!(msg.reactions[0].users.len(), 2);
    }

    #[test]
    fn test_total_reactions() {
        let mut msg = make_message("alice");
        add_reaction(&mut msg, "👍", "bob".to_string());
        add_reaction(&mut msg, "👍", "carol".to_string());
        add_reaction(&mut msg, "❤️", "dave".to_string());
        assert_eq!(total_reactions(&msg), 3);
    }
}
