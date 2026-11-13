// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM messaging workspace-entities —
//! `packages/twenty-server/src/modules/messaging/common/standard-objects/`
//! (Message, MessageThread, MessageChannel, MessageParticipant,
//! MessageChannelMessageAssociation).
//!
//! These are the *models* — the data shapes + enums that back Twenty's
//! email/SMS surfacing inside the CRM. The IMAP/Gmail/Outlook *sync workers*
//! (OAuth, message-list fetch, throttling) remain a separate scope cut
//! (`packages/.../messaging/services/google-gmail-sync.service.ts`); this
//! module closes the message-entity gap that `Activity` only approximated.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Direction of a message on a channel — lives on the
/// MessageChannelMessageAssociation join entity in Twenty.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MessageDirection {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MessageChannelType {
    Email,
    Sms,
    EmailGroup,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MessageChannelSyncStatus {
    NotSynced,
    Ongoing,
    Active,
    FailedInsufficientPermissions,
    FailedUnknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MessageChannelVisibility {
    /// Only that a message exists is shared.
    Metadata,
    /// Subject line is shared, body hidden.
    Subject,
    /// Full message shared.
    ShareEverything,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MessageParticipantRole {
    From,
    To,
    Cc,
    Bcc,
}

/// A conversation grouping (RFC 822 thread).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageThread {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub subject: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MessageThread {
    pub fn new(workspace_id: Uuid, subject: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            subject: subject.into(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// A single message within a thread.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// RFC 822 `Message-ID` header — the external dedup key.
    pub header_message_id: Option<String>,
    pub subject: String,
    pub text: String,
    pub received_at: Option<DateTime<Utc>>,
    pub message_thread_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Message {
    pub fn new(workspace_id: Uuid, message_thread_id: Uuid, subject: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            header_message_id: None,
            subject: subject.into(),
            text: String::new(),
            received_at: None,
            message_thread_id: Some(message_thread_id),
            created_at: now,
            updated_at: now,
        }
    }
}

/// A participant on a message (the join to Person; external addresses keep
/// only `handle` + `display_name` with a null `person_id`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageParticipant {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub message_id: Uuid,
    pub role: MessageParticipantRole,
    pub handle: String,
    pub display_name: String,
    pub person_id: Option<Uuid>,
    pub workspace_member_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl MessageParticipant {
    pub fn new(
        workspace_id: Uuid,
        message_id: Uuid,
        role: MessageParticipantRole,
        handle: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            message_id,
            role,
            handle: handle.into(),
            display_name: String::new(),
            person_id: None,
            workspace_member_id: None,
            created_at: Utc::now(),
        }
    }
}

/// A connected mailbox / SMS line that sync workers populate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageChannel {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub connected_account_id: Uuid,
    pub channel_type: MessageChannelType,
    pub handle: String,
    pub visibility: MessageChannelVisibility,
    pub is_contact_auto_creation_enabled: bool,
    pub is_sync_enabled: bool,
    pub sync_status: MessageChannelSyncStatus,
    pub sync_cursor: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MessageChannel {
    pub fn new(
        workspace_id: Uuid,
        connected_account_id: Uuid,
        channel_type: MessageChannelType,
        handle: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            connected_account_id,
            channel_type,
            handle: handle.into(),
            visibility: MessageChannelVisibility::ShareEverything,
            is_contact_auto_creation_enabled: true,
            is_sync_enabled: false,
            sync_status: MessageChannelSyncStatus::NotSynced,
            sync_cursor: None,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Junction between a Message and the MessageChannel it arrived on, carrying
/// sync metadata + flow direction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageChannelMessageAssociation {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub message_channel_id: Uuid,
    pub message_id: Uuid,
    pub direction: MessageDirection,
    pub message_external_id: Option<String>,
    pub message_thread_external_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl MessageChannelMessageAssociation {
    pub fn new(
        workspace_id: Uuid,
        message_channel_id: Uuid,
        message_id: Uuid,
        direction: MessageDirection,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            message_channel_id,
            message_id,
            direction,
            message_external_id: None,
            message_thread_external_id: None,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_serializes_screaming() {
        assert_eq!(
            serde_json::to_string(&MessageDirection::Incoming).unwrap(),
            "\"INCOMING\""
        );
        assert_eq!(
            serde_json::to_string(&MessageDirection::Outgoing).unwrap(),
            "\"OUTGOING\""
        );
    }

    #[test]
    fn channel_type_includes_email_group() {
        assert_eq!(
            serde_json::to_string(&MessageChannelType::EmailGroup).unwrap(),
            "\"EMAIL_GROUP\""
        );
        assert_eq!(
            serde_json::to_string(&MessageChannelType::Email).unwrap(),
            "\"EMAIL\""
        );
    }

    #[test]
    fn participant_role_casings() {
        assert_eq!(
            serde_json::to_string(&MessageParticipantRole::From).unwrap(),
            "\"FROM\""
        );
        assert_eq!(
            serde_json::to_string(&MessageParticipantRole::Bcc).unwrap(),
            "\"BCC\""
        );
    }

    #[test]
    fn channel_defaults_to_not_synced() {
        let ws = Uuid::new_v4();
        let acct = Uuid::new_v4();
        let ch = MessageChannel::new(ws, acct, MessageChannelType::Email, "ada@acme.com");
        assert_eq!(ch.sync_status, MessageChannelSyncStatus::NotSynced);
        assert_eq!(ch.visibility, MessageChannelVisibility::ShareEverything);
        assert!(!ch.is_sync_enabled);
        assert_eq!(ch.handle, "ada@acme.com");
    }

    #[test]
    fn thread_and_message_link() {
        let ws = Uuid::new_v4();
        let thread = MessageThread::new(ws, "Q4 planning");
        let msg = Message::new(ws, thread.id, "Re: Q4 planning");
        assert_eq!(msg.message_thread_id, Some(thread.id));
        assert_eq!(msg.subject, "Re: Q4 planning");
        assert!(msg.received_at.is_none());
    }

    #[test]
    fn participant_links_message_and_optional_person() {
        let ws = Uuid::new_v4();
        let msg_id = Uuid::new_v4();
        let p = MessageParticipant::new(ws, msg_id, MessageParticipantRole::To, "bob@acme.com");
        assert_eq!(p.message_id, msg_id);
        assert_eq!(p.role, MessageParticipantRole::To);
        assert_eq!(p.handle, "bob@acme.com");
        assert!(p.person_id.is_none()); // unresolved external address
    }

    #[test]
    fn association_carries_direction_and_links() {
        let ws = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let message_id = Uuid::new_v4();
        let assoc = MessageChannelMessageAssociation::new(
            ws,
            channel_id,
            message_id,
            MessageDirection::Incoming,
        );
        assert_eq!(assoc.message_channel_id, channel_id);
        assert_eq!(assoc.message_id, message_id);
        assert_eq!(assoc.direction, MessageDirection::Incoming);
    }

    #[test]
    fn sync_status_serializes_failed_variants() {
        assert_eq!(
            serde_json::to_string(&MessageChannelSyncStatus::FailedInsufficientPermissions)
                .unwrap(),
            "\"FAILED_INSUFFICIENT_PERMISSIONS\""
        );
    }
}
