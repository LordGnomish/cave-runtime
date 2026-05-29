// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Domain models for cave-chat — compatible with LibreChat.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Channel ───────────────────────────────────────────────────────────────────

/// A chat channel (public, private, direct-message, or announcement).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Channel {
    pub id: Uuid,
    pub name: String,
    pub channel_type: ChannelType,
    /// Member user IDs (string so the caller owns the ID space).
    pub members: Vec<String>,
    pub archived: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Channel {
    pub fn new(name: impl Into<String>, channel_type: ChannelType) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            channel_type,
            members: vec![],
            archived: false,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    Public,
    Private,
    Direct,
    Announcement,
}

// ── Message ───────────────────────────────────────────────────────────────────

/// A chat message, optionally part of a thread.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub author_id: String,
    pub content: String,
    pub message_type: MessageType,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub reactions: Vec<Reaction>,
    /// If set, this message is a reply to the given root message.
    pub thread_root_id: Option<Uuid>,
    /// IDs of thread replies (populated on the root message).
    pub thread_replies: Vec<Uuid>,
}

impl Message {
    pub fn new(channel_id: Uuid, author_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            channel_id,
            author_id: author_id.into(),
            content: content.into(),
            message_type: MessageType::Text,
            created_at: Utc::now(),
            edited_at: None,
            reactions: vec![],
            thread_root_id: None,
            thread_replies: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Text,
    System,
    File,
    Alert,
}

/// A per-emoji reaction on a message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reaction {
    pub emoji: String,
    /// User IDs who reacted with this emoji.
    pub users: Vec<String>,
}

// ── Presence ──────────────────────────────────────────────────────────────────

/// Real-time presence state for a user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserPresence {
    pub user_id: String,
    pub status: PresenceStatus,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {
    Online,
    Away,
    DoNotDisturb,
    Offline,
}

// ── Stats ─────────────────────────────────────────────────────────────────────

/// Aggregated chat statistics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatStats {
    pub total_channels: u64,
    pub total_messages: u64,
    pub active_users: u64,
    pub messages_today: u64,
}

// ── Legacy aliases kept for compatibility with engine.rs ─────────────────────

/// Legacy room model — kept so engine.rs continues to compile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatRoom {
    pub id: Uuid,
    pub name: String,
    pub room_type: RoomType,
    pub members: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RoomType {
    Direct,
    Group,
    Incident,
    Announcement,
}
