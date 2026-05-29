// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory chat store — channels, messages, presence, stats.

use crate::models::{
    Channel, ChatStats, Message, PresenceStatus, Reaction, UserPresence,
};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

/// Thread-safe in-memory store for all chat data.
#[derive(Default)]
pub struct ChatStore {
    channels: RwLock<HashMap<Uuid, Channel>>,
    messages: RwLock<HashMap<Uuid, Message>>,
    presence: RwLock<HashMap<String, UserPresence>>,
}

impl ChatStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Channels ──────────────────────────────────────────────────────────────

    /// Insert a new channel and return a clone of what was stored.
    pub fn create_channel(&self, channel: Channel) -> Channel {
        let mut channels = self.channels.write().unwrap();
        let c = channel.clone();
        channels.insert(channel.id, channel);
        c
    }

    /// Look up a channel by ID.
    pub fn get_channel(&self, id: &Uuid) -> Option<Channel> {
        self.channels.read().unwrap().get(id).cloned()
    }

    /// Return all channels (order unspecified).
    pub fn list_channels(&self) -> Vec<Channel> {
        self.channels.read().unwrap().values().cloned().collect()
    }

    /// Mark a channel as archived; returns the updated channel or `None` if not found.
    pub fn archive_channel(&self, id: &Uuid) -> Option<Channel> {
        let mut channels = self.channels.write().unwrap();
        if let Some(ch) = channels.get_mut(id) {
            ch.archived = true;
            ch.updated_at = Utc::now();
            return Some(ch.clone());
        }
        None
    }

    /// Add `user_id` to the channel member list (idempotent).
    pub fn add_member(&self, channel_id: &Uuid, user_id: String) -> Option<Channel> {
        let mut channels = self.channels.write().unwrap();
        if let Some(ch) = channels.get_mut(channel_id) {
            if !ch.members.contains(&user_id) {
                ch.members.push(user_id);
                ch.updated_at = Utc::now();
            }
            return Some(ch.clone());
        }
        None
    }

    /// Remove `user_id` from the channel member list.
    pub fn remove_member(&self, channel_id: &Uuid, user_id: &str) -> Option<Channel> {
        let mut channels = self.channels.write().unwrap();
        if let Some(ch) = channels.get_mut(channel_id) {
            ch.members.retain(|m| m != user_id);
            ch.updated_at = Utc::now();
            return Some(ch.clone());
        }
        None
    }

    // ── Messages ──────────────────────────────────────────────────────────────

    /// Store a message.  If it has a `thread_root_id`, registers it on the root.
    pub fn create_message(&self, message: Message) -> Message {
        let thread_root_id = message.thread_root_id;
        let reply_id = message.id;
        let m = message.clone();

        {
            let mut messages = self.messages.write().unwrap();
            messages.insert(m.id, message);
        }

        // Register as a thread reply on the root message.
        if let Some(root_id) = thread_root_id {
            let mut messages = self.messages.write().unwrap();
            if let Some(root) = messages.get_mut(&root_id) {
                if !root.thread_replies.contains(&reply_id) {
                    root.thread_replies.push(reply_id);
                }
            }
        }

        m
    }

    /// Fetch a message by ID.
    pub fn get_message(&self, id: &Uuid) -> Option<Message> {
        self.messages.read().unwrap().get(id).cloned()
    }

    /// Delete a message and return it, or `None` if not found.
    pub fn delete_message(&self, id: &Uuid) -> Option<Message> {
        self.messages.write().unwrap().remove(id)
    }

    /// Return up to `limit` messages in a channel, newest-first.
    /// If `before` is given, only messages older than that timestamp are included.
    pub fn get_channel_messages(
        &self,
        channel_id: &Uuid,
        limit: usize,
        before: Option<DateTime<Utc>>,
    ) -> Vec<Message> {
        let messages = self.messages.read().unwrap();
        let mut msgs: Vec<Message> = messages
            .values()
            .filter(|m| {
                m.channel_id == *channel_id
                    && before.map_or(true, |t| m.created_at < t)
            })
            .cloned()
            .collect();
        // newest-first
        msgs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        msgs.truncate(limit);
        msgs
    }

    /// Return all direct thread replies to `root_message_id`, oldest-first.
    pub fn get_thread(&self, root_message_id: &Uuid) -> Vec<Message> {
        let reply_ids: Vec<Uuid> = {
            let messages = self.messages.read().unwrap();
            match messages.get(root_message_id) {
                Some(root) => root.thread_replies.clone(),
                None => return vec![],
            }
        };
        let messages = self.messages.read().unwrap();
        let mut replies: Vec<Message> = reply_ids
            .iter()
            .filter_map(|id| messages.get(id).cloned())
            .collect();
        replies.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        replies
    }

    /// Add an emoji reaction from `user` to a message (idempotent per user).
    pub fn add_reaction(
        &self,
        message_id: &Uuid,
        emoji: String,
        user: String,
    ) -> Option<Message> {
        let mut messages = self.messages.write().unwrap();
        if let Some(msg) = messages.get_mut(message_id) {
            if let Some(reaction) = msg.reactions.iter_mut().find(|r| r.emoji == emoji) {
                if !reaction.users.contains(&user) {
                    reaction.users.push(user);
                }
            } else {
                msg.reactions.push(Reaction {
                    emoji,
                    users: vec![user],
                });
            }
            return Some(msg.clone());
        }
        None
    }

    /// Remove a user's reaction from a message.  Returns the updated message.
    pub fn remove_reaction(
        &self,
        message_id: &Uuid,
        emoji: &str,
        user: &str,
    ) -> Option<Message> {
        let mut messages = self.messages.write().unwrap();
        if let Some(msg) = messages.get_mut(message_id) {
            for reaction in msg.reactions.iter_mut() {
                if reaction.emoji == emoji {
                    reaction.users.retain(|u| u != user);
                    break;
                }
            }
            // Remove empty reaction groups
            msg.reactions.retain(|r| !r.users.is_empty());
            return Some(msg.clone());
        }
        None
    }

    /// Full-text search across message content (case-insensitive substring).
    /// Optionally scoped to a single channel.
    pub fn search_messages(&self, query: &str, channel_id: Option<Uuid>) -> Vec<Message> {
        let messages = self.messages.read().unwrap();
        let lower = query.to_lowercase();
        messages
            .values()
            .filter(|m| {
                let channel_match = channel_id.map_or(true, |id| m.channel_id == id);
                channel_match && m.content.to_lowercase().contains(&lower)
            })
            .cloned()
            .collect()
    }

    // ── Presence ──────────────────────────────────────────────────────────────

    /// Look up a user's presence record.
    pub fn get_presence(&self, user_id: &str) -> Option<UserPresence> {
        self.presence.read().unwrap().get(user_id).cloned()
    }

    /// Upsert a user's presence record.
    pub fn set_presence(&self, presence: UserPresence) -> UserPresence {
        let mut p = self.presence.write().unwrap();
        let ret = presence.clone();
        p.insert(presence.user_id.clone(), presence);
        ret
    }

    // ── Stats ─────────────────────────────────────────────────────────────────

    /// Compute aggregate statistics across the current store state.
    pub fn compute_stats(&self) -> ChatStats {
        let channels = self.channels.read().unwrap();
        let messages = self.messages.read().unwrap();
        let presence = self.presence.read().unwrap();

        let now = Utc::now();
        let today_start = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();

        let total_channels = channels.len() as u64;
        let total_messages = messages.len() as u64;
        let active_users = presence
            .values()
            .filter(|p| !matches!(p.status, PresenceStatus::Offline))
            .count() as u64;
        let messages_today = messages
            .values()
            .filter(|m| m.created_at >= today_start)
            .count() as u64;

        ChatStats {
            total_channels,
            total_messages,
            active_users,
            messages_today,
        }
    }
}
