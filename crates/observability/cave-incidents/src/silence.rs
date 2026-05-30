// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Incident-group silence state machine and computed status precedence.
//!
//! Faithful line-port of grafana/oncall v1.10.0
//! `engine/apps/alerts/models/alert_group.py`:
//!   - `status` property         — resolved > acknowledged > silenced > new
//!   - `is_silenced_forever`     — `silenced and silenced_until is None`
//!   - `is_silenced_for_period`  — `silenced and silenced_until is not None`
//!   - `silence(**kwargs)`       — only mutates when not already silenced
//!   - `un_silence()`            — clears the silence fields, stamps `restarted_at`
//!
//! This models whether *this* incident group is muted and until when. The
//! Alertmanager routing-tree / inhibit-rule silences remain in cave-alerts; the
//! distributed unsilence-timer firing remains in cave-net. Here we only port the
//! pure in-crate state-transition + status-precedence logic.

use chrono::{DateTime, Utc};

/// Computed group status, ported from `AlertGroup.status` (range(4): NEW,
/// ACKNOWLEDGED, RESOLVED, SILENCED). Note the precedence ordering encoded in
/// the upstream `status` property — resolved/acknowledged win over silenced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputedStatus {
    New,
    Acknowledged,
    Resolved,
    Silenced,
}

/// The silence-relevant slice of an incident group's state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupSilenceState {
    pub resolved: bool,
    pub acknowledged: bool,
    pub silenced: bool,
    /// `None` while silenced == false, or while silenced *forever*.
    pub silenced_until: Option<DateTime<Utc>>,
    pub silenced_at: Option<DateTime<Utc>>,
    /// Stamped by `un_silence` (upstream: `restarted_at = timezone.now()`).
    pub restarted_at: Option<DateTime<Utc>>,
}

impl GroupSilenceState {
    /// Port of `AlertGroup.is_silenced_forever`.
    pub fn is_silenced_forever(&self) -> bool {
        self.silenced && self.silenced_until.is_none()
    }

    /// Port of `AlertGroup.is_silenced_for_period`.
    pub fn is_silenced_for_period(&self) -> bool {
        self.silenced && self.silenced_until.is_some()
    }

    /// Port of the `AlertGroup.status` property. Precedence is exactly upstream's:
    /// resolved first, then acknowledged, then silenced, else new.
    pub fn status(&self) -> ComputedStatus {
        if self.resolved {
            ComputedStatus::Resolved
        } else if self.acknowledged {
            ComputedStatus::Acknowledged
        } else if self.silenced {
            ComputedStatus::Silenced
        } else {
            ComputedStatus::New
        }
    }

    /// Port of `AlertGroup.silence`. Only mutates when not already silenced, so a
    /// second silence call cannot clobber the original `silenced_until`. `until`
    /// of `None` means silenced *forever*.
    pub fn silence(&mut self, until: Option<DateTime<Utc>>) {
        if !self.silenced {
            self.silenced = true;
            self.silenced_until = until;
            if self.silenced_at.is_none() {
                self.silenced_at = Some(Utc::now());
            }
        }
    }

    /// Port of `AlertGroup.un_silence` — clears every silence field and stamps
    /// `restarted_at`.
    pub fn un_silence(&mut self) {
        self.silenced_until = None;
        self.silenced_at = None;
        self.silenced = false;
        self.restarted_at = Some(Utc::now());
    }

    /// Whether the silence is still in effect at `now`. A forever silence is
    /// always active; a for-period silence is active only until `silenced_until`.
    /// (Upstream relies on the `filter_active` queryset which excludes
    /// `silenced=True, silenced_until__isnull=True` forever-silences and, for
    /// period silences, on the unsilence task firing at `silenced_until`.)
    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        if !self.silenced {
            return false;
        }
        match self.silenced_until {
            None => true,                 // silenced forever
            Some(until) => now < until,   // silenced for a period
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn default_is_new_and_not_silenced() {
        let s = GroupSilenceState::default();
        assert_eq!(s.status(), ComputedStatus::New);
        assert!(!s.silenced);
    }

    #[test]
    fn forever_silence_active_until_unsilenced() {
        let mut s = GroupSilenceState::default();
        s.silence(None);
        assert!(s.is_active_at(Utc::now() + Duration::days(365)));
        s.un_silence();
        assert!(!s.is_active_at(Utc::now()));
    }
}
