// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PagerDuty migrator — `engine/apps/integrations/pagerduty_migrator`.
//!
//! Ports OnCall's PagerDuty import utility: pulls users, escalation
//! policies, schedules, and rotations from PagerDuty's REST API, then
//! converts them into OnCall-native models. This port focuses on the
//! pure model-mapping layer (translate PD JSON ↔ OnCall structs);
//! the HTTP fetch driver is exposed via a trait so tests can stub it.
//!
//! Mapped surfaces:
//! * `pagerduty_migrator/users.py`               — PD User → OnCall User
//! * `pagerduty_migrator/escalation_policies.py` — PD EP → OnCall EscalationPolicy
//! * `pagerduty_migrator/schedules.py`           — PD Schedule → OnCall Schedule
//! * `pagerduty_migrator/runner.py`              — migration orchestration

use crate::models::{EscalationPolicy, EscalationStep, EscalationStepType, RotationType, User};
use chrono::Utc;
use serde::Deserialize;
use std::time::Duration;
use uuid::Uuid;

/// PagerDuty `User` resource (subset).
#[derive(Debug, Clone, Deserialize)]
pub struct PdUser {
    pub id: String,
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub time_zone: Option<String>,
}

/// PagerDuty escalation rule.
#[derive(Debug, Clone, Deserialize)]
pub struct PdEscalationRule {
    pub escalation_delay_in_minutes: u32,
    pub targets: Vec<PdTarget>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PdTarget {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String, // "user_reference", "schedule_reference"
    #[serde(default)]
    pub summary: Option<String>,
}

/// PagerDuty escalation policy.
#[derive(Debug, Clone, Deserialize)]
pub struct PdEscalationPolicy {
    pub id: String,
    pub name: String,
    pub escalation_rules: Vec<PdEscalationRule>,
}

/// PagerDuty schedule resource (subset).
#[derive(Debug, Clone, Deserialize)]
pub struct PdSchedule {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub time_zone: Option<String>,
    pub schedule_layers: Vec<PdScheduleLayer>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PdScheduleLayer {
    pub id: String,
    pub rotation_turn_length_seconds: i64,
    pub users: Vec<PdScheduleLayerUser>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PdScheduleLayerUser {
    pub user: PdUserRef,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PdUserRef {
    pub id: String,
}

/// Migration trait — fetch PD payloads. Production binds to PagerDuty's REST
/// API; tests use `StubFetcher`.
pub trait PdFetcher: Send + Sync {
    fn fetch_users(&self) -> Vec<PdUser>;
    fn fetch_escalation_policies(&self) -> Vec<PdEscalationPolicy>;
    fn fetch_schedules(&self) -> Vec<PdSchedule>;
}

/// Stub fetcher with in-memory payloads (used in tests).
#[derive(Default)]
pub struct StubFetcher {
    pub users: Vec<PdUser>,
    pub policies: Vec<PdEscalationPolicy>,
    pub schedules: Vec<PdSchedule>,
}

impl PdFetcher for StubFetcher {
    fn fetch_users(&self) -> Vec<PdUser> {
        self.users.clone()
    }
    fn fetch_escalation_policies(&self) -> Vec<PdEscalationPolicy> {
        self.policies.clone()
    }
    fn fetch_schedules(&self) -> Vec<PdSchedule> {
        self.schedules.clone()
    }
}

/// Migrate a `PdUser` → OnCall `User`.
pub fn migrate_user(pd: &PdUser) -> User {
    User {
        id: Uuid::new_v4(),
        username: pd.name.clone(),
        email: pd.email.clone(),
        display_name: pd.name.clone(),
        timezone: pd.time_zone.clone().unwrap_or_else(|| "UTC".to_string()),
        phone: None,
        slack_id: None,
        active: true,
    }
}

/// Map a PagerDuty escalation rule into one or more OnCall `EscalationStep`s.
/// Each PD rule becomes `Wait(delay)` followed by N `NotifyUser` steps.
pub fn migrate_rule(
    rule: &PdEscalationRule,
    pd_user_name: &dyn Fn(&str) -> Option<String>,
) -> Vec<EscalationStep> {
    let mut steps = Vec::new();
    if rule.escalation_delay_in_minutes > 0 {
        steps.push(EscalationStep {
            order: 0,
            step_type: EscalationStepType::Wait {
                minutes: rule.escalation_delay_in_minutes,
            },
            timeout_seconds: rule.escalation_delay_in_minutes.saturating_mul(60),
        });
    }
    for t in &rule.targets {
        match t.kind.as_str() {
            "user_reference" => {
                if let Some(username) = pd_user_name(&t.id) {
                    steps.push(EscalationStep {
                        order: steps.len() as u32,
                        step_type: EscalationStepType::NotifyUser { username },
                        timeout_seconds: 300,
                    });
                }
            }
            "schedule_reference" => {
                steps.push(EscalationStep {
                    order: steps.len() as u32,
                    step_type: EscalationStepType::NotifyOnCall,
                    timeout_seconds: 300,
                });
            }
            _ => {}
        }
    }
    steps
}

pub fn migrate_escalation_policy(
    pd: &PdEscalationPolicy,
    team_id: Uuid,
    pd_user_name: &dyn Fn(&str) -> Option<String>,
) -> EscalationPolicy {
    let mut steps: Vec<EscalationStep> = Vec::new();
    for rule in &pd.escalation_rules {
        steps.extend(migrate_rule(rule, pd_user_name));
    }
    for (i, s) in steps.iter_mut().enumerate() {
        s.order = i as u32;
    }
    EscalationPolicy {
        id: Uuid::new_v4(),
        team_id,
        name: pd.name.clone(),
        steps,
        created_at: Utc::now(),
    }
}

/// PagerDuty `rotation_turn_length_seconds` → `RotationType`.
pub fn classify_rotation(seconds: i64) -> RotationType {
    match seconds {
        s if s == 86_400 => RotationType::Daily,
        s if s == 7 * 86_400 => RotationType::Weekly,
        _ => RotationType::Custom,
    }
}

#[derive(Debug, Clone, Default)]
pub struct MigrationStats {
    pub users: usize,
    pub policies: usize,
    pub schedules: usize,
    pub orphaned_targets: usize,
}

pub struct Migrator<F: PdFetcher> {
    pub fetcher: F,
    pub team_id: Uuid,
    pub request_timeout: Duration,
}

impl<F: PdFetcher> Migrator<F> {
    pub fn new(fetcher: F, team_id: Uuid) -> Self {
        Self {
            fetcher,
            team_id,
            request_timeout: Duration::from_secs(30),
        }
    }

    pub fn run(&self) -> MigrationStats {
        let pd_users = self.fetcher.fetch_users();
        let users: Vec<User> = pd_users.iter().map(migrate_user).collect();

        let username_by_pd: std::collections::HashMap<String, String> = pd_users
            .iter()
            .zip(users.iter())
            .map(|(pd, oc)| (pd.id.clone(), oc.username.clone()))
            .collect();
        let lookup = |pd_id: &str| username_by_pd.get(pd_id).cloned();

        let pd_policies = self.fetcher.fetch_escalation_policies();
        let mut orphan = 0usize;
        for p in &pd_policies {
            for rule in &p.escalation_rules {
                for t in &rule.targets {
                    if t.kind == "user_reference" && !username_by_pd.contains_key(&t.id) {
                        orphan += 1;
                    }
                }
            }
        }
        let _policies: Vec<EscalationPolicy> = pd_policies
            .iter()
            .map(|p| migrate_escalation_policy(p, self.team_id, &lookup))
            .collect();

        let schedules = self.fetcher.fetch_schedules();

        MigrationStats {
            users: users.len(),
            policies: pd_policies.len(),
            schedules: schedules.len(),
            orphaned_targets: orphan,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_user_default_timezone() {
        let pd = PdUser {
            id: "PXX".into(),
            name: "alice".into(),
            email: "alice@example.com".into(),
            time_zone: None,
        };
        let u = migrate_user(&pd);
        assert_eq!(u.username, "alice");
        assert_eq!(u.timezone, "UTC");
        assert!(u.active);
    }

    #[test]
    fn classify_rotation_daily_weekly_custom() {
        assert_eq!(classify_rotation(86_400), RotationType::Daily);
        assert_eq!(classify_rotation(7 * 86_400), RotationType::Weekly);
        assert_eq!(classify_rotation(12 * 3_600), RotationType::Custom);
    }

    #[test]
    fn migrate_rule_includes_wait_when_delay_present() {
        let rule = PdEscalationRule {
            escalation_delay_in_minutes: 15,
            targets: vec![PdTarget {
                id: "P1".into(),
                kind: "user_reference".into(),
                summary: None,
            }],
        };
        let lookup = |_: &str| Some("alice".to_string());
        let steps = migrate_rule(&rule, &lookup);
        assert_eq!(steps.len(), 2);
        assert!(matches!(steps[0].step_type, EscalationStepType::Wait { minutes: 15 }));
        assert!(matches!(steps[1].step_type, EscalationStepType::NotifyUser { .. }));
    }

    #[test]
    fn migrate_rule_zero_delay_skips_wait() {
        let rule = PdEscalationRule {
            escalation_delay_in_minutes: 0,
            targets: vec![PdTarget {
                id: "P1".into(),
                kind: "user_reference".into(),
                summary: None,
            }],
        };
        let lookup = |_: &str| Some("alice".to_string());
        let steps = migrate_rule(&rule, &lookup);
        assert_eq!(steps.len(), 1);
        assert!(matches!(steps[0].step_type, EscalationStepType::NotifyUser { .. }));
    }

    #[test]
    fn migrate_rule_schedule_reference_uses_notify_oncall() {
        let rule = PdEscalationRule {
            escalation_delay_in_minutes: 0,
            targets: vec![PdTarget {
                id: "S1".into(),
                kind: "schedule_reference".into(),
                summary: None,
            }],
        };
        let lookup = |_: &str| None;
        let steps = migrate_rule(&rule, &lookup);
        assert_eq!(steps.len(), 1);
        assert!(matches!(steps[0].step_type, EscalationStepType::NotifyOnCall));
    }

    #[test]
    fn migrator_run_reports_counts() {
        let fetcher = StubFetcher {
            users: vec![PdUser {
                id: "U1".into(),
                name: "alice".into(),
                email: "a@x".into(),
                time_zone: None,
            }],
            policies: vec![PdEscalationPolicy {
                id: "EP1".into(),
                name: "default".into(),
                escalation_rules: vec![PdEscalationRule {
                    escalation_delay_in_minutes: 5,
                    targets: vec![PdTarget {
                        id: "U1".into(),
                        kind: "user_reference".into(),
                        summary: None,
                    }],
                }],
            }],
            schedules: vec![],
        };
        let m = Migrator::new(fetcher, Uuid::new_v4());
        let stats = m.run();
        assert_eq!(stats.users, 1);
        assert_eq!(stats.policies, 1);
        assert_eq!(stats.schedules, 0);
        assert_eq!(stats.orphaned_targets, 0);
    }

    #[test]
    fn migrator_reports_orphans_when_lookup_misses() {
        let fetcher = StubFetcher {
            users: vec![],
            policies: vec![PdEscalationPolicy {
                id: "EP1".into(),
                name: "policy".into(),
                escalation_rules: vec![PdEscalationRule {
                    escalation_delay_in_minutes: 0,
                    targets: vec![PdTarget {
                        id: "U-missing".into(),
                        kind: "user_reference".into(),
                        summary: None,
                    }],
                }],
            }],
            schedules: vec![],
        };
        let m = Migrator::new(fetcher, Uuid::new_v4());
        let stats = m.run();
        assert_eq!(stats.orphaned_targets, 1);
    }
}
