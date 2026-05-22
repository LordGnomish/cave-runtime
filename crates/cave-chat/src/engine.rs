// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{Message, Reaction};
use uuid::Uuid;

pub fn is_member(room_members: &[Uuid], user_id: &Uuid) -> bool {
    room_members.contains(user_id)
}

pub fn message_count(messages: &[Message]) -> usize {
    messages.len()
}

pub fn messages_by_author<'a>(messages: &'a [Message], author_id: &Uuid) -> Vec<&'a Message> {
    messages
        .iter()
        .filter(|m| &m.author_id == author_id)
        .collect()
}

pub fn add_reaction(message: &mut Message, emoji: &str, user_id: Uuid) {
    if let Some(r) = message.reactions.iter_mut().find(|r| r.emoji == emoji) {
        if !r.user_ids.contains(&user_id) {
            r.user_ids.push(user_id);
        }
    } else {
        message.reactions.push(Reaction {
            emoji: emoji.to_string(),
            user_ids: vec![user_id],
        });
    }
}

pub fn total_reactions(message: &Message) -> usize {
    message.reactions.iter().map(|r| r.user_ids.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Message, MessageType};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_message(author_id: Uuid) -> Message {
        Message {
            id: Uuid::new_v4(),
            room_id: Uuid::new_v4(),
            author_id,
            content: "Hello!".to_string(),
            message_type: MessageType::Text,
            created_at: Utc::now(),
            edited_at: None,
            reactions: vec![],
        }
    }

    #[test]
    fn test_is_member_true() {
        let user = Uuid::new_v4();
        let members = vec![Uuid::new_v4(), user, Uuid::new_v4()];
        assert!(is_member(&members, &user));
    }

    #[test]
    fn test_is_member_false() {
        let user = Uuid::new_v4();
        let members = vec![Uuid::new_v4(), Uuid::new_v4()];
        assert!(!is_member(&members, &user));
    }

    #[test]
    fn test_messages_by_author() {
        let author = Uuid::new_v4();
        let other = Uuid::new_v4();
        let messages = vec![
            make_message(author),
            make_message(other),
            make_message(author),
        ];
        let by_author = messages_by_author(&messages, &author);
        assert_eq!(by_author.len(), 2);
        for m in &by_author {
            assert_eq!(m.author_id, author);
        }
    }

    #[test]
    fn test_add_reaction_new() {
        let mut msg = make_message(Uuid::new_v4());
        let user = Uuid::new_v4();
        add_reaction(&mut msg, "👍", user);
        assert_eq!(msg.reactions.len(), 1);
        assert_eq!(msg.reactions[0].emoji, "👍");
        assert_eq!(msg.reactions[0].user_ids.len(), 1);
    }

    #[test]
    fn test_add_reaction_existing_emoji_no_duplicate() {
        let mut msg = make_message(Uuid::new_v4());
        let user = Uuid::new_v4();
        add_reaction(&mut msg, "👍", user);
        add_reaction(&mut msg, "👍", user); // same user again — no duplicate
        assert_eq!(msg.reactions.len(), 1);
        assert_eq!(msg.reactions[0].user_ids.len(), 1);
    }

    #[test]
    fn test_total_reactions() {
        let mut msg = make_message(Uuid::new_v4());
        add_reaction(&mut msg, "👍", Uuid::new_v4());
        add_reaction(&mut msg, "👍", Uuid::new_v4());
        add_reaction(&mut msg, "❤️", Uuid::new_v4());
        assert_eq!(total_reactions(&msg), 3);
    }
}
