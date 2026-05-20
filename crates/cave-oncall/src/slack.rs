// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Slack bot integration — `engine/apps/slack`.
//!
//! Ports OnCall's Slack-bot surface: alert→channel routing, interactive
//! component callbacks (Acknowledge / Resolve / Snooze / Take-it buttons),
//! signature verification for inbound events, and per-team workspace
//! configuration. The Slack SDK is HTTP-only (signed requests),
//! so the port has no native SDK dependency — only `serde_json` for
//! the payload shapes.
//!
//! Mapped surfaces:
//! * `engine/apps/slack/slack_client.py`            — Web API outbound
//! * `engine/apps/slack/scenarios/interactive.py`   — interactive callbacks
//! * `engine/apps/slack/utils/verify_signature.py`  — HMAC-SHA256 verify
//! * `engine/apps/slack/models/slack_user_identity.py`

use crate::models::Alert;
use std::collections::HashMap;

/// Workspace-level Slack config (per OnCall organization).
#[derive(Debug, Clone)]
pub struct SlackWorkspace {
    pub team_id: String,
    pub bot_token: String,
    pub signing_secret: String,
    pub default_channel: String,
    /// Override channel by Alert.severity or Alert.team
    pub channel_routes: HashMap<String, String>,
}

impl SlackWorkspace {
    pub fn new(team_id: impl Into<String>, bot_token: impl Into<String>) -> Self {
        Self {
            team_id: team_id.into(),
            bot_token: bot_token.into(),
            signing_secret: String::new(),
            default_channel: "#alerts".into(),
            channel_routes: HashMap::new(),
        }
    }

    /// Resolve the channel for a given alert — explicit route wins, falls back to default.
    pub fn channel_for(&self, alert: &Alert) -> String {
        let team_key = alert.team_id.to_string();
        if let Some(ch) = self.channel_routes.get(&team_key) {
            return ch.clone();
        }
        let sev = format!("{:?}", alert.severity).to_lowercase();
        if let Some(ch) = self.channel_routes.get(&sev) {
            return ch.clone();
        }
        self.default_channel.clone()
    }
}

/// Slack interactive component button action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlackAction {
    Acknowledge,
    Resolve,
    Snooze { minutes: u32 },
    TakeIt,
    Escalate,
}

impl SlackAction {
    /// Parse from action_id string (matches OnCall's `acknowledge_alert_group` style).
    pub fn parse(action_id: &str) -> Option<Self> {
        match action_id {
            "acknowledge_alert_group" => Some(Self::Acknowledge),
            "resolve_alert_group" => Some(Self::Resolve),
            "take_it" => Some(Self::TakeIt),
            "escalate" => Some(Self::Escalate),
            s if s.starts_with("snooze_") => {
                let minutes: u32 = s.trim_start_matches("snooze_").parse().ok()?;
                Some(Self::Snooze { minutes })
            }
            _ => None,
        }
    }
}

/// Interactive payload dispatched from Slack to OnCall.
#[derive(Debug, Clone)]
pub struct SlackInteractive {
    pub user_id: String,
    pub team_id: String,
    pub channel_id: String,
    pub action: SlackAction,
    pub alert_id: String,
}

/// Verify Slack signing per `engine/apps/slack/utils/verify_signature.py`.
/// Returns the canonical signing-base string used by Slack so the caller
/// can run an HMAC-SHA256 with `signing_secret` and compare to the
/// `X-Slack-Signature` header. Pure helper — no external crypto here.
pub fn slack_signing_base(version: &str, timestamp: &str, body: &str) -> String {
    format!("{}:{}:{}", version, timestamp, body)
}

/// Block-kit message scaffold for alert notifications (subset of Slack Block Kit).
#[derive(Debug, Clone)]
pub struct AlertBlock {
    pub title: String,
    pub severity: String,
    pub source: String,
    pub fingerprint: String,
    pub action_buttons: Vec<(String, String)>,
}

impl AlertBlock {
    pub fn from_alert(a: &Alert) -> Self {
        Self {
            title: a.title.clone(),
            severity: format!("{:?}", a.severity),
            source: a.source.clone(),
            fingerprint: a.fingerprint.clone(),
            action_buttons: vec![
                ("Acknowledge".into(), "acknowledge_alert_group".into()),
                ("Resolve".into(), "resolve_alert_group".into()),
                ("Snooze 30m".into(), "snooze_30".into()),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Alert, AlertState, Severity};
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn mk_alert(team: Uuid) -> Alert {
        Alert {
            id: Uuid::new_v4(),
            team_id: team,
            title: "Disk full".into(),
            severity: Severity::Critical,
            source: "prometheus".into(),
            fingerprint: "fp-1".into(),
            state: AlertState::Firing,
            labels: HashMap::new(),
            annotations: HashMap::new(),
            created_at: Utc::now(),
            ack_at: None,
            ack_by: None,
            resolved_at: None,
            escalation_policy_id: None,
            current_escalation_step: 0,
        }
    }

    #[test]
    fn channel_for_uses_team_route_when_present() {
        let team = Uuid::new_v4();
        let mut ws = SlackWorkspace::new("T1", "tok");
        ws.channel_routes
            .insert(team.to_string(), "#payments-alerts".into());
        let a = mk_alert(team);
        assert_eq!(ws.channel_for(&a), "#payments-alerts");
    }

    #[test]
    fn channel_for_falls_back_to_default() {
        let ws = SlackWorkspace::new("T1", "tok");
        let a = mk_alert(Uuid::new_v4());
        assert_eq!(ws.channel_for(&a), "#alerts");
    }

    #[test]
    fn channel_for_severity_route_used_when_no_team_route() {
        let mut ws = SlackWorkspace::new("T1", "tok");
        ws.channel_routes.insert("critical".into(), "#sev1".into());
        let a = mk_alert(Uuid::new_v4());
        assert_eq!(ws.channel_for(&a), "#sev1");
    }

    #[test]
    fn slack_action_parse_static_ids() {
        assert_eq!(
            SlackAction::parse("acknowledge_alert_group"),
            Some(SlackAction::Acknowledge)
        );
        assert_eq!(
            SlackAction::parse("resolve_alert_group"),
            Some(SlackAction::Resolve)
        );
        assert_eq!(SlackAction::parse("take_it"), Some(SlackAction::TakeIt));
        assert_eq!(SlackAction::parse("escalate"), Some(SlackAction::Escalate));
        assert_eq!(SlackAction::parse("unknown"), None);
    }

    #[test]
    fn slack_action_parse_snooze_minutes() {
        assert_eq!(
            SlackAction::parse("snooze_30"),
            Some(SlackAction::Snooze { minutes: 30 })
        );
        assert_eq!(
            SlackAction::parse("snooze_120"),
            Some(SlackAction::Snooze { minutes: 120 })
        );
        assert_eq!(SlackAction::parse("snooze_bad"), None);
    }

    #[test]
    fn signing_base_format_matches_slack_spec() {
        let s = slack_signing_base("v0", "1531420618", "body=xyz");
        assert_eq!(s, "v0:1531420618:body=xyz");
    }

    #[test]
    fn alert_block_contains_three_action_buttons() {
        let a = mk_alert(Uuid::new_v4());
        let b = AlertBlock::from_alert(&a);
        assert_eq!(b.action_buttons.len(), 3);
        assert_eq!(b.severity, "Critical");
        assert_eq!(b.fingerprint, "fp-1");
    }
}
