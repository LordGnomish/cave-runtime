// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for chat store and model layer.

use cave_chat::models::{Channel, ChannelType, Message, MessageType, Reaction, UserPresence, PresenceStatus, ChatStats};
use cave_chat::store::ChatStore;
use chrono::Utc;
use uuid::Uuid;

fn make_channel(name: &str) -> Channel {
    Channel {
        id: Uuid::new_v4(),
        name: name.to_string(),
        channel_type: ChannelType::Public,
        members: vec!["user-1".to_string()],
        archived: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn make_message(channel_id: Uuid, author: &str) -> Message {
    Message {
        id: Uuid::new_v4(),
        channel_id,
        author_id: author.to_string(),
        content: "hello world".to_string(),
        message_type: MessageType::Text,
        created_at: Utc::now(),
        edited_at: None,
        reactions: vec![],
        thread_root_id: None,
        thread_replies: vec![],
    }
}

// ── Channel CRUD ──────────────────────────────────────────────────────────────

#[test]
fn test_create_and_get_channel() {
    let store = ChatStore::new();
    let ch = make_channel("general");
    let id = ch.id;
    let created = store.create_channel(ch);
    assert_eq!(created.name, "general");
    let fetched = store.get_channel(&id).expect("channel should exist");
    assert_eq!(fetched.id, id);
}

#[test]
fn test_list_channels_empty() {
    let store = ChatStore::new();
    assert!(store.list_channels().is_empty());
}

#[test]
fn test_list_channels_returns_all() {
    let store = ChatStore::new();
    store.create_channel(make_channel("alpha"));
    store.create_channel(make_channel("beta"));
    assert_eq!(store.list_channels().len(), 2);
}

#[test]
fn test_archive_channel() {
    let store = ChatStore::new();
    let ch = make_channel("archive-me");
    let id = ch.id;
    store.create_channel(ch);
    let archived = store.archive_channel(&id).expect("should archive");
    assert!(archived.archived);
}

#[test]
fn test_add_remove_member() {
    let store = ChatStore::new();
    let ch = make_channel("team");
    let id = ch.id;
    store.create_channel(ch);
    let updated = store.add_member(&id, "user-2".to_string()).expect("add");
    assert!(updated.members.contains(&"user-2".to_string()));
    let removed = store.remove_member(&id, "user-2").expect("remove");
    assert!(!removed.members.contains(&"user-2".to_string()));
}

// ── Message CRUD ──────────────────────────────────────────────────────────────

#[test]
fn test_create_and_get_message() {
    let store = ChatStore::new();
    let ch = make_channel("general");
    let cid = ch.id;
    store.create_channel(ch);
    let msg = make_message(cid, "alice");
    let mid = msg.id;
    let created = store.create_message(msg);
    assert_eq!(created.author_id, "alice");
    let fetched = store.get_message(&mid).expect("message should exist");
    assert_eq!(fetched.id, mid);
}

#[test]
fn test_delete_message() {
    let store = ChatStore::new();
    let ch = make_channel("general");
    let cid = ch.id;
    store.create_channel(ch);
    let msg = make_message(cid, "bob");
    let mid = msg.id;
    store.create_message(msg);
    let deleted = store.delete_message(&mid).expect("should delete");
    assert_eq!(deleted.id, mid);
    assert!(store.get_message(&mid).is_none());
}

#[test]
fn test_get_channel_messages_ordered() {
    let store = ChatStore::new();
    let ch = make_channel("general");
    let cid = ch.id;
    store.create_channel(ch);
    for i in 0..5u64 {
        let mut msg = make_message(cid, "alice");
        // ensure distinct timestamps
        msg.created_at = chrono::DateTime::from_timestamp(1_700_000_000 + i as i64, 0).unwrap();
        store.create_message(msg);
    }
    let msgs = store.get_channel_messages(&cid, 10, None);
    assert_eq!(msgs.len(), 5);
    // returned in descending order (newest first)
    for i in 0..msgs.len() - 1 {
        assert!(msgs[i].created_at >= msgs[i + 1].created_at);
    }
}

#[test]
fn test_get_channel_messages_limit() {
    let store = ChatStore::new();
    let ch = make_channel("general");
    let cid = ch.id;
    store.create_channel(ch);
    for i in 0..10u64 {
        let mut msg = make_message(cid, "alice");
        msg.created_at = chrono::DateTime::from_timestamp(1_700_000_000 + i as i64, 0).unwrap();
        store.create_message(msg);
    }
    let msgs = store.get_channel_messages(&cid, 3, None);
    assert_eq!(msgs.len(), 3);
}

// ── Thread ────────────────────────────────────────────────────────────────────

#[test]
fn test_thread_replies() {
    let store = ChatStore::new();
    let ch = make_channel("dev");
    let cid = ch.id;
    store.create_channel(ch);
    let root = make_message(cid, "alice");
    let root_id = root.id;
    store.create_message(root);
    let mut reply = make_message(cid, "bob");
    reply.thread_root_id = Some(root_id);
    store.create_message(reply);
    let thread = store.get_thread(&root_id);
    assert_eq!(thread.len(), 1);
    assert_eq!(thread[0].author_id, "bob");
}

// ── Reactions ─────────────────────────────────────────────────────────────────

#[test]
fn test_add_reaction_to_message() {
    let store = ChatStore::new();
    let ch = make_channel("general");
    let cid = ch.id;
    store.create_channel(ch);
    let msg = make_message(cid, "carol");
    let mid = msg.id;
    store.create_message(msg);
    let updated = store.add_reaction(&mid, "👍".to_string(), "dave".to_string()).expect("ok");
    assert_eq!(updated.reactions.len(), 1);
    assert_eq!(updated.reactions[0].emoji, "👍");
}

#[test]
fn test_reaction_no_duplicate_user() {
    let store = ChatStore::new();
    let ch = make_channel("general");
    let cid = ch.id;
    store.create_channel(ch);
    let msg = make_message(cid, "carol");
    let mid = msg.id;
    store.create_message(msg);
    store.add_reaction(&mid, "❤️".to_string(), "dave".to_string());
    let updated = store.add_reaction(&mid, "❤️".to_string(), "dave".to_string()).expect("ok");
    assert_eq!(updated.reactions[0].users.len(), 1);
}

// ── Search ────────────────────────────────────────────────────────────────────

#[test]
fn test_search_messages_full_text() {
    let store = ChatStore::new();
    let ch = make_channel("general");
    let cid = ch.id;
    store.create_channel(ch);
    let mut msg1 = make_message(cid, "alice");
    msg1.content = "deployment failed in production".to_string();
    let mut msg2 = make_message(cid, "bob");
    msg2.content = "everything looks good".to_string();
    store.create_message(msg1);
    store.create_message(msg2);
    let results = store.search_messages("deployment", None);
    assert_eq!(results.len(), 1);
    assert!(results[0].content.contains("deployment"));
}

#[test]
fn test_search_messages_case_insensitive() {
    let store = ChatStore::new();
    let ch = make_channel("ops");
    let cid = ch.id;
    store.create_channel(ch);
    let mut msg = make_message(cid, "alice");
    msg.content = "Production Deploy succeeded".to_string();
    store.create_message(msg);
    let results = store.search_messages("production deploy", None);
    assert_eq!(results.len(), 1);
}

// ── Presence ─────────────────────────────────────────────────────────────────

#[test]
fn test_set_and_get_presence() {
    let store = ChatStore::new();
    let pres = UserPresence {
        user_id: "alice".to_string(),
        status: PresenceStatus::Online,
        last_seen: Utc::now(),
    };
    store.set_presence(pres);
    let fetched = store.get_presence("alice").expect("should exist");
    assert!(matches!(fetched.status, PresenceStatus::Online));
}

#[test]
fn test_presence_update() {
    let store = ChatStore::new();
    store.set_presence(UserPresence {
        user_id: "bob".to_string(),
        status: PresenceStatus::Online,
        last_seen: Utc::now(),
    });
    store.set_presence(UserPresence {
        user_id: "bob".to_string(),
        status: PresenceStatus::Away,
        last_seen: Utc::now(),
    });
    let fetched = store.get_presence("bob").expect("ok");
    assert!(matches!(fetched.status, PresenceStatus::Away));
}

// ── Stats ─────────────────────────────────────────────────────────────────────

#[test]
fn test_compute_stats() {
    let store = ChatStore::new();
    let ch = make_channel("stats");
    let cid = ch.id;
    store.create_channel(ch);
    store.create_message(make_message(cid, "alice"));
    store.set_presence(UserPresence {
        user_id: "alice".to_string(),
        status: PresenceStatus::Online,
        last_seen: Utc::now(),
    });
    let stats: ChatStats = store.compute_stats();
    assert_eq!(stats.total_channels, 1);
    assert_eq!(stats.total_messages, 1);
    assert_eq!(stats.active_users, 1);
}
