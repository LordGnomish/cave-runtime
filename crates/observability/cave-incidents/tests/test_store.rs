// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for IncidentStore — incident CRUD, responder management, postmortem CRUD,
//! schedule management, metrics computation.

use cave_incidents::engine;
use cave_incidents::models::{
    AddResponderRequest, CreateIncidentRequest, CreatePostMortemRequest, EscalationPolicy,
    EscalationStep, EscalationTarget, IncidentSeverity, IncidentStatus, OnCallSchedule,
    OnCallUser, PostMortemStatus, Responder, ResponderRole, RotationType, ScheduleLayer,
};
use cave_incidents::store::IncidentStore;
use chrono::Utc;
use uuid::Uuid;

fn make_request(title: &str, severity: IncidentSeverity) -> CreateIncidentRequest {
    CreateIncidentRequest {
        title: title.to_string(),
        description: "Test description".to_string(),
        severity,
        created_by: Uuid::new_v4(),
        tags: vec!["test".to_string()],
    }
}

fn make_schedule(name: &str) -> OnCallSchedule {
    let user = OnCallUser {
        id: Uuid::new_v4(),
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
        phone: None,
        notification_prefs: vec![],
    };
    let layer = ScheduleLayer {
        id: Uuid::new_v4(),
        name: "Primary".to_string(),
        rotation_type: RotationType::Weekly,
        rotation_period_days: 7,
        users: vec![user],
        current_index: 0,
        starts_at: Utc::now() - chrono::Duration::days(1),
    };
    OnCallSchedule {
        id: Uuid::new_v4(),
        name: name.to_string(),
        timezone: "UTC".to_string(),
        layers: vec![layer],
    }
}

#[test]
fn test_store_create_and_get() {
    let store = IncidentStore::new();
    let req = make_request("DB Down", IncidentSeverity::P1);
    let incident = engine::create_incident(req);
    let id = incident.id;
    store.create(incident);
    let fetched = store.get(id).expect("incident should be found");
    assert_eq!(fetched.title, "DB Down");
    assert_eq!(fetched.severity, IncidentSeverity::P1);
}

#[test]
fn test_store_get_missing_returns_none() {
    let store = IncidentStore::new();
    assert!(store.get(Uuid::new_v4()).is_none());
}

#[test]
fn test_store_update_incident() {
    let store = IncidentStore::new();
    let req = make_request("API Slow", IncidentSeverity::P2);
    let mut incident = engine::create_incident(req);
    let id = incident.id;
    store.create(incident.clone());

    // Acknowledge and update
    let user = Uuid::new_v4();
    engine::acknowledge(&mut incident, user).unwrap();
    let updated = store.update(incident);
    assert!(updated);
    let fetched = store.get(id).unwrap();
    assert_eq!(fetched.status, IncidentStatus::Acknowledged);
}

#[test]
fn test_store_update_nonexistent_returns_false() {
    let store = IncidentStore::new();
    let incident = engine::create_incident(make_request("X", IncidentSeverity::P3));
    assert!(!store.update(incident)); // never created → false
}

#[test]
fn test_store_list_all() {
    let store = IncidentStore::new();
    store.create(engine::create_incident(make_request("I1", IncidentSeverity::P1)));
    store.create(engine::create_incident(make_request("I2", IncidentSeverity::P2)));
    store.create(engine::create_incident(make_request("I3", IncidentSeverity::P3)));
    let list = store.list();
    assert_eq!(list.len(), 3);
}

#[test]
fn test_store_list_open_only() {
    let store = IncidentStore::new();
    let mut i1 = engine::create_incident(make_request("Open", IncidentSeverity::P2));
    let mut i2 = engine::create_incident(make_request("Resolved", IncidentSeverity::P2));
    let user = Uuid::new_v4();
    engine::acknowledge(&mut i2, user).unwrap();
    engine::resolve(&mut i2, user, "Fixed".to_string()).unwrap();
    store.create(i1.clone());
    store.create(i2);

    let open = store.list_open();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].title, "Open");
}

#[test]
fn test_store_list_by_severity() {
    let store = IncidentStore::new();
    store.create(engine::create_incident(make_request("P1-a", IncidentSeverity::P1)));
    store.create(engine::create_incident(make_request("P1-b", IncidentSeverity::P1)));
    store.create(engine::create_incident(make_request("P3-a", IncidentSeverity::P3)));

    let p1 = store.list_by_severity(&IncidentSeverity::P1);
    assert_eq!(p1.len(), 2);
    let p3 = store.list_by_severity(&IncidentSeverity::P3);
    assert_eq!(p3.len(), 1);
    let p4 = store.list_by_severity(&IncidentSeverity::P4);
    assert_eq!(p4.len(), 0);
}

#[test]
fn test_store_delete_incident() {
    let store = IncidentStore::new();
    let incident = engine::create_incident(make_request("Temp", IncidentSeverity::P4));
    let id = incident.id;
    store.create(incident);
    assert!(store.get(id).is_some());
    let deleted = store.delete(id);
    assert!(deleted);
    assert!(store.get(id).is_none());
}

#[test]
fn test_store_delete_nonexistent_returns_false() {
    let store = IncidentStore::new();
    assert!(!store.delete(Uuid::new_v4()));
}

#[test]
fn test_store_add_responder() {
    let store = IncidentStore::new();
    let incident = engine::create_incident(make_request("Network Down", IncidentSeverity::P1));
    let id = incident.id;
    store.create(incident);

    let responder = Responder {
        user_id: Uuid::new_v4(),
        name: "Bob".to_string(),
        email: "bob@example.com".to_string(),
        role: ResponderRole::Responder,
        paged_at: Utc::now(),
        acknowledged_at: None,
    };
    let added = store.add_responder(id, responder);
    assert!(added);

    let fetched = store.get(id).unwrap();
    assert_eq!(fetched.responders.len(), 1);
    assert_eq!(fetched.responders[0].name, "Bob");
}

#[test]
fn test_store_postmortem_crud() {
    let store = IncidentStore::new();
    let incident = engine::create_incident(make_request("DB Failure", IncidentSeverity::P1));
    let incident_id = incident.id;
    store.create(incident);

    let pm = cave_incidents::models::PostMortem {
        id: Uuid::new_v4(),
        incident_id,
        title: "Root cause analysis".to_string(),
        summary: "DB leader node crashed".to_string(),
        root_cause: "Out of disk space".to_string(),
        action_items: vec!["Add disk alert".to_string()],
        status: PostMortemStatus::Draft,
        created_at: Utc::now(),
        published_at: None,
        author_id: Uuid::new_v4(),
    };
    let pm_id = pm.id;

    store.create_postmortem(pm.clone());
    let fetched = store.get_postmortem(pm_id).expect("postmortem should exist");
    assert_eq!(fetched.title, "Root cause analysis");
    assert_eq!(fetched.action_items.len(), 1);

    let mut updated = fetched;
    updated.status = PostMortemStatus::Published;
    updated.published_at = Some(Utc::now());
    let ok = store.update_postmortem(updated);
    assert!(ok);

    let final_pm = store.get_postmortem(pm_id).unwrap();
    assert!(matches!(final_pm.status, PostMortemStatus::Published));

    let all = store.list_postmortems();
    assert_eq!(all.len(), 1);
}

#[test]
fn test_store_schedule_crud() {
    let store = IncidentStore::new();
    let sched = make_schedule("Eng On-Call");
    let sched_id = sched.id;
    store.add_schedule(sched.clone());

    let fetched = store.get_schedule(sched_id).expect("schedule should exist");
    assert_eq!(fetched.name, "Eng On-Call");
    assert_eq!(fetched.layers.len(), 1);
    assert_eq!(fetched.layers[0].users.len(), 1);

    let all = store.list_schedules();
    assert_eq!(all.len(), 1);
}

#[test]
fn test_store_current_on_call() {
    let store = IncidentStore::new();
    let sched = make_schedule("Primary");
    let sched_id = sched.id;
    store.add_schedule(sched);

    let on_call = store.current_on_call(sched_id);
    assert!(on_call.is_some());
    assert_eq!(on_call.unwrap().name, "Alice");
}

#[test]
fn test_store_policy_crud() {
    let store = IncidentStore::new();
    let policy = EscalationPolicy {
        id: Uuid::new_v4(),
        name: "Default".to_string(),
        steps: vec![EscalationStep {
            delay_minutes: 0,
            targets: vec![EscalationTarget::Team("primary".to_string())],
        }],
        repeat_count: 3,
    };
    let policy_id = policy.id;
    store.add_policy(policy);

    let fetched = store.get_policy(policy_id).expect("policy should exist");
    assert_eq!(fetched.name, "Default");

    let all = store.list_policies();
    assert_eq!(all.len(), 1);
}

#[test]
fn test_store_metrics_aggregation() {
    let store = IncidentStore::new();
    let user = Uuid::new_v4();

    let mut p1 = engine::create_incident(make_request("P1", IncidentSeverity::P1));
    let mut p2 = engine::create_incident(make_request("P2", IncidentSeverity::P2));
    let p3 = engine::create_incident(make_request("P3-open", IncidentSeverity::P3));

    // Resolve p1 and p2
    engine::acknowledge(&mut p1, user).unwrap();
    engine::resolve(&mut p1, user, "Fixed".to_string()).unwrap();
    engine::acknowledge(&mut p2, user).unwrap();
    engine::resolve(&mut p2, user, "Fixed too".to_string()).unwrap();

    store.create(p1);
    store.create(p2);
    store.create(p3);

    let m = store.metrics();
    assert_eq!(m.total_incidents, 3);
    assert_eq!(m.p1_count, 1);
    assert_eq!(m.p2_count, 1);
    assert_eq!(m.p3_count, 1);
    // p3 is open; p1 and p2 are resolved
    assert_eq!(m.open_incidents, 1);
    // resolved_this_week = 2 (both resolved just now)
    assert_eq!(m.resolved_this_week, 2);
    // MTTA should be Some (both p1 and p2 were acknowledged)
    assert!(m.avg_time_to_acknowledge_secs.is_some());
}
