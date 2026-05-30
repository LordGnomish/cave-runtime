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
