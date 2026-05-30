// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Alert-group invitations — a faithful port of grafana/oncall
//! `engine/apps/alerts/models/invitation.py` (v1.10.0).
//!
//! An [`Invitation`] invites a user to join work on an alert group. Re-inviting
//! the same user deactivates the prior invitation (logged upstream as
//! `TYPE_RE_INVITE`); a fresh invite is `TYPE_INVITE`. Each invitation carries
//! an `attempt` counter and a backoff schedule that paces re-notification.

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

/// `Invitation.ATTEMPTS_LIMIT`.
pub const ATTEMPTS_LIMIT: i32 = 10;

/// `Invitation.time_deltas_by_attempts`, in seconds: 6m, 16m, 31m, 1h1m, 3h1m.
const TIME_DELTAS_BY_ATTEMPTS_SECS: [i64; 5] = [
    6 * 60,         // 6m
    16 * 60,        // 16m
    31 * 60,        // 31m
    60 * 60 + 60,   // 1h1m
    3 * 3600 + 60,  // 3h1m
];

/// Port of `Invitation.get_delay_by_attempt` — the backoff for re-notifying an
/// invitee. Attempts past the end of the table reuse the final (largest) delay.
pub fn get_delay_by_attempt(attempt: usize) -> Duration {
    let last = TIME_DELTAS_BY_ATTEMPTS_SECS[TIME_DELTAS_BY_ATTEMPTS_SECS.len() - 1];
    let secs = TIME_DELTAS_BY_ATTEMPTS_SECS.get(attempt).copied().unwrap_or(last);
    Duration::seconds(secs)
}

/// Outcome of [`InvitationStore::invite_user`], mirroring the upstream log
/// record type chosen (`TYPE_INVITE` vs `TYPE_RE_INVITE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvitationOutcome {
    /// No prior active invitation existed — `TYPE_INVITE`.
    Invite,
    /// A prior active invitation was deactivated first — `TYPE_RE_INVITE`.
    ReInvite,
}

/// Port of the `Invitation` model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitation {
    pub id: Uuid,
    pub alert_id: Uuid,
    pub author: String,
    pub invitee: String,
    pub is_active: bool,
    pub attempt: i32,
    pub created_at: DateTime<Utc>,
}

impl Invitation {
    /// `Invitation.attempts_left` property.
    pub fn attempts_left(&self) -> i32 {
        ATTEMPTS_LIMIT - self.attempt
    }
}

/// Errors raised by invitation operations.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvitationError {
    /// `stop_invitation`: no invitation with that id exists.
    #[error("invitation not found")]
    NotFound,
}

/// In-memory collection of invitations (the runtime layer; persistence is a
/// store concern, exactly as cave-oncall keeps alerts/schedules in memory).
#[derive(Debug, Default)]
pub struct InvitationStore {
    invitations: HashMap<Uuid, Invitation>,
}

impl InvitationStore {
    /// Port of `Invitation.invite_user`. If an active invitation already exists
    /// for `(invitee, alert_id)` it is deactivated (re-invite); a new active
    /// invitation is then created. Returns the chosen outcome and the new id.
    pub fn invite_user(
        &mut self,
        invitee: &str,
        alert_id: Uuid,
        author: &str,
    ) -> (InvitationOutcome, Uuid) {
        let existing: Option<Uuid> = self
            .invitations
            .values()
            .find(|i| i.is_active && i.invitee == invitee && i.alert_id == alert_id)
            .map(|i| i.id);

        let outcome = if let Some(prev) = existing {
            if let Some(inv) = self.invitations.get_mut(&prev) {
                inv.is_active = false;
            }
            InvitationOutcome::ReInvite
        } else {
            InvitationOutcome::Invite
        };

        let id = Uuid::new_v4();
        self.invitations.insert(
            id,
            Invitation {
                id,
                alert_id,
                author: author.to_string(),
                invitee: invitee.to_string(),
                is_active: true,
                attempt: 0,
                created_at: Utc::now(),
            },
        );
        (outcome, id)
    }

    /// Port of `Invitation.stop_invitation` — deactivate by id.
    pub fn stop_invitation(&mut self, id: Uuid) -> Result<(), InvitationError> {
        match self.invitations.get_mut(&id) {
            Some(inv) => {
                inv.is_active = false;
                Ok(())
            }
            None => Err(InvitationError::NotFound),
        }
    }

    /// Fetch an invitation by id.
    pub fn get(&self, id: Uuid) -> Option<&Invitation> {
        self.invitations.get(&id)
    }

    /// Active invitations for an alert group.
    pub fn active_for(&self, alert_id: Uuid) -> Vec<&Invitation> {
        self.invitations
            .values()
            .filter(|i| i.is_active && i.alert_id == alert_id)
            .collect()
    }
}
