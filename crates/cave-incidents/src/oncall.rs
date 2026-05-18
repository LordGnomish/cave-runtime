// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{
    EscalationPolicy, EscalationStep, EscalationTarget, Incident, NotificationChannel,
    OnCallSchedule, OnCallUser, ResponderRole, RotationType, ScheduleLayer,
};
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use crate::models::Responder;

pub struct OnCallEngine;

impl OnCallEngine {
    pub fn new() -> Self {
        OnCallEngine
    }

    /// Get the current on-call user for a schedule at a given time.
    /// Uses the first layer with users; determines rotation index by elapsed periods.
    pub fn current_oncall<'a>(
        &self,
        schedule: &'a OnCallSchedule,
        at: DateTime<Utc>,
    ) -> Option<&'a OnCallUser> {
        let layer = schedule.layers.first()?;
        if layer.users.is_empty() {
            return None;
        }
        let index = rotation_index(layer, at);
        layer.users.get(index)
    }

    /// Get the next `count` on-call shifts for the first layer of a schedule.
    /// Returns (start, end, user_name).
    pub fn upcoming_shifts(
        &self,
        schedule: &OnCallSchedule,
        count: usize,
    ) -> Vec<(DateTime<Utc>, DateTime<Utc>, String)> {
        let Some(layer) = schedule.layers.first() else {
            return vec![];
        };
        if layer.users.is_empty() {
            return vec![];
        }

        let period_days = layer.rotation_period_days.max(1) as i64;
        let now = Utc::now();

        // Find the start of the current rotation period
        let elapsed_secs = (now - layer.starts_at).num_seconds().max(0);
        let period_secs = period_days * 86_400;
        let periods_elapsed = elapsed_secs / period_secs;
        let current_period_start = layer.starts_at + Duration::seconds(periods_elapsed * period_secs);

        let mut shifts = Vec::with_capacity(count);
        for i in 0..count {
            let shift_start = current_period_start + Duration::seconds(i as i64 * period_secs);
            let shift_end = shift_start + Duration::seconds(period_secs);
            let idx = ((periods_elapsed + i as i64) as usize + layer.current_index)
                % layer.users.len();
            let user_name = layer.users[idx].name.clone();
            shifts.push((shift_start, shift_end, user_name));
        }
        shifts
    }

    /// Page the on-call user for an incident (simulated).
    pub fn page_oncall(
        &self,
        incident: &Incident,
        schedule: &OnCallSchedule,
    ) -> Option<Responder> {
        let user = self.current_oncall(schedule, Utc::now())?;
        let now = Utc::now();
        tracing::info!(
            incident_id = %incident.id,
            user = %user.name,
            email = %user.email,
            "Paging on-call user for incident"
        );
        Some(Responder {
            user_id: user.id,
            name: user.name.clone(),
            email: user.email.clone(),
            role: ResponderRole::Responder,
            paged_at: now,
            acknowledged_at: None,
        })
    }

    /// Apply an escalation policy step (simulated — logs what would happen).
    pub fn escalate(
        &self,
        incident: &Incident,
        policy: &EscalationPolicy,
        step: usize,
    ) -> Vec<String> {
        let Some(escalation_step) = policy.steps.get(step) else {
            return vec![format!(
                "Step {} does not exist in policy '{}'",
                step, policy.name
            )];
        };

        let mut actions = Vec::new();
        for target in &escalation_step.targets {
            let action = match target {
                EscalationTarget::User(uid) => format!(
                    "Would page user {} for incident '{}' (severity: {:?})",
                    uid, incident.title, incident.severity
                ),
                EscalationTarget::Schedule(sid) => format!(
                    "Would page current on-call from schedule {} for incident '{}'",
                    sid, incident.title
                ),
                EscalationTarget::Team(team) => format!(
                    "Would notify team '{}' for incident '{}'",
                    team, incident.title
                ),
            };
            tracing::info!(
                incident_id = %incident.id,
                policy = %policy.name,
                step = step,
                "Escalation: {}",
                action
            );
            actions.push(action);
        }
        actions
    }
}

impl Default for OnCallEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn rotation_index(layer: &ScheduleLayer, at: DateTime<Utc>) -> usize {
    if layer.users.is_empty() {
        return 0;
    }
    let period_days = layer.rotation_period_days.max(1) as i64;
    let elapsed_secs = (at - layer.starts_at).num_seconds().max(0);
    let period_secs = period_days * 86_400;
    let periods_elapsed = (elapsed_secs / period_secs) as usize;
    (layer.current_index + periods_elapsed) % layer.users.len()
}

/// Build a default on-call schedule with 3 users in weekly rotation.
pub fn default_schedule() -> OnCallSchedule {
    let user_a = OnCallUser {
        id: Uuid::new_v4(),
        name: "Alice On-Call".to_string(),
        email: "alice@example.com".to_string(),
        phone: Some("+1-555-0001".to_string()),
        notification_prefs: vec![NotificationChannel::Slack, NotificationChannel::Email],
    };
    let user_b = OnCallUser {
        id: Uuid::new_v4(),
        name: "Bob On-Call".to_string(),
        email: "bob@example.com".to_string(),
        phone: Some("+1-555-0002".to_string()),
        notification_prefs: vec![NotificationChannel::Sms, NotificationChannel::Email],
    };
    let user_c = OnCallUser {
        id: Uuid::new_v4(),
        name: "Carol On-Call".to_string(),
        email: "carol@example.com".to_string(),
        phone: Some("+1-555-0003".to_string()),
        notification_prefs: vec![NotificationChannel::PagerDuty, NotificationChannel::Email],
    };

    let layer = ScheduleLayer {
        id: Uuid::new_v4(),
        name: "Primary".to_string(),
        rotation_type: RotationType::Weekly,
        rotation_period_days: 7,
        users: vec![user_a, user_b, user_c],
        current_index: 0,
        starts_at: Utc::now(),
    };

    OnCallSchedule {
        id: Uuid::new_v4(),
        name: "Default On-Call Schedule".to_string(),
        timezone: "UTC".to_string(),
        layers: vec![layer],
    }
}

/// Build a default escalation policy:
///   step 0: page primary on-call immediately
///   step 1: escalate to secondary after 5 min
///   step 2: page manager after 15 min
pub fn default_escalation_policy() -> EscalationPolicy {
    EscalationPolicy {
        id: Uuid::new_v4(),
        name: "Default Escalation Policy".to_string(),
        steps: vec![
            EscalationStep {
                delay_minutes: 0,
                targets: vec![EscalationTarget::Team("primary-oncall".to_string())],
            },
            EscalationStep {
                delay_minutes: 5,
                targets: vec![EscalationTarget::Team("secondary-oncall".to_string())],
            },
            EscalationStep {
                delay_minutes: 15,
                targets: vec![EscalationTarget::Team("engineering-managers".to_string())],
            },
        ],
        repeat_count: 3,
    }
}
