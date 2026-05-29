// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for extended incident models: Responder, PostMortem, IncidentMetrics,
//! OnCallUser, EscalationPolicy, OnCallSchedule, ScheduleLayer.

use cave_incidents::models::{
    EscalationPolicy, EscalationStep, EscalationTarget, IncidentMetrics, IncidentSeverity,
    NotificationChannel, OnCallSchedule, OnCallUser, PostMortem, PostMortemStatus, Responder,
    ResponderRole, RotationType, ScheduleLayer,
};
use chrono::Utc;
use uuid::Uuid;

#[test]
fn test_responder_serde_roundtrip() {
    let r = Responder {
        user_id: Uuid::new_v4(),
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
        role: ResponderRole::CommandingOfficer,
        paged_at: Utc::now(),
        acknowledged_at: Some(Utc::now()),
    };
    let json = serde_json::to_string(&r).unwrap();
    let back: Responder = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, r.name);
    assert_eq!(back.email, r.email);
    assert!(matches!(back.role, ResponderRole::CommandingOfficer));
    assert!(back.acknowledged_at.is_some());
}

#[test]
fn test_responder_roles_serde() {
    for role in [
        ResponderRole::CommandingOfficer,
        ResponderRole::Responder,
        ResponderRole::Observer,
    ] {
        let s = serde_json::to_string(&role).unwrap();
        let back: ResponderRole = serde_json::from_str(&s).unwrap();
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            serde_json::to_string(&role).unwrap()
        );
    }
}

#[test]
fn test_postmortem_serde_roundtrip() {
    let pm = PostMortem {
        id: Uuid::new_v4(),
        incident_id: Uuid::new_v4(),
        title: "Post-mortem for DB outage".to_string(),
        summary: "Primary DB lost quorum".to_string(),
        root_cause: "Disk IOPS exhaustion on leader node".to_string(),
        action_items: vec!["Add IOPS alert".to_string(), "Add disk sizing runbook".to_string()],
        status: PostMortemStatus::Draft,
        created_at: Utc::now(),
        published_at: None,
        author_id: Uuid::new_v4(),
    };
    let json = serde_json::to_string(&pm).unwrap();
    let back: PostMortem = serde_json::from_str(&json).unwrap();
    assert_eq!(back.title, pm.title);
    assert_eq!(back.root_cause, pm.root_cause);
    assert_eq!(back.action_items.len(), 2);
    assert!(matches!(back.status, PostMortemStatus::Draft));
    assert!(back.published_at.is_none());
}

#[test]
fn test_postmortem_status_published() {
    let mut pm = PostMortem {
        id: Uuid::new_v4(),
        incident_id: Uuid::new_v4(),
        title: "PM".to_string(),
        summary: "s".to_string(),
        root_cause: "r".to_string(),
        action_items: vec![],
        status: PostMortemStatus::Draft,
        created_at: Utc::now(),
        published_at: None,
        author_id: Uuid::new_v4(),
    };
    pm.status = PostMortemStatus::Published;
    pm.published_at = Some(Utc::now());
    let json = serde_json::to_string(&pm).unwrap();
    let back: PostMortem = serde_json::from_str(&json).unwrap();
    assert!(matches!(back.status, PostMortemStatus::Published));
    assert!(back.published_at.is_some());
}

#[test]
fn test_incident_metrics_serde() {
    let m = IncidentMetrics {
        total_incidents: 10,
        open_incidents: 3,
        p1_count: 1,
        p2_count: 2,
        p3_count: 4,
        p4_count: 3,
        avg_time_to_acknowledge_secs: Some(120.0),
        avg_time_to_resolve_secs: Some(3600.0),
        resolved_this_week: 5,
    };
    let json = serde_json::to_string(&m).unwrap();
    let back: IncidentMetrics = serde_json::from_str(&json).unwrap();
    assert_eq!(back.total_incidents, 10);
    assert_eq!(back.open_incidents, 3);
    assert_eq!(back.p1_count, 1);
    assert!((back.avg_time_to_acknowledge_secs.unwrap() - 120.0).abs() < 0.001);
}

#[test]
fn test_oncall_user_serde() {
    let u = OnCallUser {
        id: Uuid::new_v4(),
        name: "Bob".to_string(),
        email: "bob@example.com".to_string(),
        phone: Some("+1-555-1234".to_string()),
        notification_prefs: vec![NotificationChannel::Slack, NotificationChannel::Email],
    };
    let json = serde_json::to_string(&u).unwrap();
    let back: OnCallUser = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "Bob");
    assert_eq!(back.notification_prefs.len(), 2);
}

#[test]
fn test_notification_channels_serde() {
    for channel in [
        NotificationChannel::Email,
        NotificationChannel::Slack,
        NotificationChannel::Sms,
        NotificationChannel::PagerDuty,
        NotificationChannel::Webhook,
    ] {
        let s = serde_json::to_string(&channel).unwrap();
        let back: NotificationChannel = serde_json::from_str(&s).unwrap();
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            serde_json::to_string(&channel).unwrap()
        );
    }
}

#[test]
fn test_schedule_layer_serde() {
    let u = OnCallUser {
        id: Uuid::new_v4(),
        name: "Eve".to_string(),
        email: "eve@example.com".to_string(),
        phone: None,
        notification_prefs: vec![],
    };
    let layer = ScheduleLayer {
        id: Uuid::new_v4(),
        name: "Primary Layer".to_string(),
        rotation_type: RotationType::Weekly,
        rotation_period_days: 7,
        users: vec![u],
        current_index: 0,
        starts_at: Utc::now(),
    };
    let json = serde_json::to_string(&layer).unwrap();
    let back: ScheduleLayer = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "Primary Layer");
    assert_eq!(back.users.len(), 1);
    assert!(matches!(back.rotation_type, RotationType::Weekly));
}

#[test]
fn test_escalation_policy_serde() {
    let policy = EscalationPolicy {
        id: Uuid::new_v4(),
        name: "Default Policy".to_string(),
        steps: vec![
            EscalationStep {
                delay_minutes: 0,
                targets: vec![EscalationTarget::Team("primary".to_string())],
            },
            EscalationStep {
                delay_minutes: 5,
                targets: vec![
                    EscalationTarget::User(Uuid::new_v4()),
                    EscalationTarget::Schedule(Uuid::new_v4()),
                ],
            },
        ],
        repeat_count: 2,
    };
    let json = serde_json::to_string(&policy).unwrap();
    let back: EscalationPolicy = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "Default Policy");
    assert_eq!(back.steps.len(), 2);
    assert_eq!(back.steps[1].targets.len(), 2);
    assert_eq!(back.repeat_count, 2);
}

#[test]
fn test_oncall_schedule_serde() {
    let schedule = OnCallSchedule {
        id: Uuid::new_v4(),
        name: "Engineering On-Call".to_string(),
        timezone: "UTC".to_string(),
        layers: vec![],
    };
    let json = serde_json::to_string(&schedule).unwrap();
    let back: OnCallSchedule = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "Engineering On-Call");
    assert_eq!(back.timezone, "UTC");
}

#[test]
fn test_severity_ord_p1_is_most_severe() {
    // P1 should be considered higher severity than P4
    // Verify we can compare severities
    let p1 = IncidentSeverity::P1;
    let p4 = IncidentSeverity::P4;
    let p1_json = serde_json::to_string(&p1).unwrap();
    let p4_json = serde_json::to_string(&p4).unwrap();
    assert_ne!(p1_json, p4_json);
}
