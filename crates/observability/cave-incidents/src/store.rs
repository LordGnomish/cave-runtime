// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory store for incidents, schedules, policies, and postmortems.

use crate::models::{
    EscalationPolicy, Incident, IncidentMetrics, IncidentSeverity, IncidentStatus, OnCallSchedule,
    OnCallUser, PostMortem, Responder, TimelineEntry,
};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

pub struct IncidentStore {
    incidents: RwLock<HashMap<Uuid, Incident>>,
    schedules: RwLock<HashMap<Uuid, OnCallSchedule>>,
    policies: RwLock<HashMap<Uuid, EscalationPolicy>>,
    postmortems: RwLock<HashMap<Uuid, PostMortem>>,
}

impl Default for IncidentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl IncidentStore {
    pub fn new() -> Self {
        Self {
            incidents: RwLock::new(HashMap::new()),
            schedules: RwLock::new(HashMap::new()),
            policies: RwLock::new(HashMap::new()),
            postmortems: RwLock::new(HashMap::new()),
        }
    }

    // ── Incident CRUD ─────────────────────────────────────────────────────────

    pub fn create(&self, incident: Incident) {
        let mut map = self.incidents.write().unwrap();
        map.insert(incident.id, incident);
    }

    pub fn get(&self, id: Uuid) -> Option<Incident> {
        let map = self.incidents.read().unwrap();
        map.get(&id).cloned()
    }

    pub fn update(&self, incident: Incident) -> bool {
        let mut map = self.incidents.write().unwrap();
        if map.contains_key(&incident.id) {
            map.insert(incident.id, incident);
            true
        } else {
            false
        }
    }

    pub fn delete(&self, id: Uuid) -> bool {
        let mut map = self.incidents.write().unwrap();
        map.remove(&id).is_some()
    }

    pub fn list(&self) -> Vec<Incident> {
        let map = self.incidents.read().unwrap();
        map.values().cloned().collect()
    }

    pub fn list_open(&self) -> Vec<Incident> {
        let map = self.incidents.read().unwrap();
        map.values()
            .filter(|i| {
                matches!(i.status, IncidentStatus::Open | IncidentStatus::Acknowledged)
            })
            .cloned()
            .collect()
    }

    pub fn list_by_severity(&self, severity: &IncidentSeverity) -> Vec<Incident> {
        let map = self.incidents.read().unwrap();
        map.values()
            .filter(|i| &i.severity == severity)
            .cloned()
            .collect()
    }

    pub fn append_timeline(&self, incident_id: Uuid, entry: TimelineEntry) -> bool {
        let mut map = self.incidents.write().unwrap();
        if let Some(incident) = map.get_mut(&incident_id) {
            incident.timeline.push(entry);
            incident.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    pub fn add_responder(&self, incident_id: Uuid, responder: Responder) -> bool {
        let mut map = self.incidents.write().unwrap();
        if let Some(incident) = map.get_mut(&incident_id) {
            incident.responders.push(responder);
            incident.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    // ── Metrics ───────────────────────────────────────────────────────────────

    pub fn metrics(&self) -> IncidentMetrics {
        let map = self.incidents.read().unwrap();
        let incidents: Vec<&Incident> = map.values().collect();
        crate::engine::compute_metrics_from_refs(&incidents)
    }

    // ── Schedule CRUD ─────────────────────────────────────────────────────────

    pub fn add_schedule(&self, schedule: OnCallSchedule) {
        let mut map = self.schedules.write().unwrap();
        map.insert(schedule.id, schedule);
    }

    pub fn get_schedule(&self, id: Uuid) -> Option<OnCallSchedule> {
        let map = self.schedules.read().unwrap();
        map.get(&id).cloned()
    }

    pub fn list_schedules(&self) -> Vec<OnCallSchedule> {
        let map = self.schedules.read().unwrap();
        map.values().cloned().collect()
    }

    pub fn current_on_call(&self, schedule_id: Uuid) -> Option<OnCallUser> {
        let map = self.schedules.read().unwrap();
        let schedule = map.get(&schedule_id)?;
        let engine = crate::oncall::OnCallEngine::new();
        engine.current_oncall(schedule, Utc::now()).cloned()
    }

    // ── Escalation Policy CRUD ────────────────────────────────────────────────

    pub fn add_policy(&self, policy: EscalationPolicy) {
        let mut map = self.policies.write().unwrap();
        map.insert(policy.id, policy);
    }

    pub fn get_policy(&self, id: Uuid) -> Option<EscalationPolicy> {
        let map = self.policies.read().unwrap();
        map.get(&id).cloned()
    }

    pub fn list_policies(&self) -> Vec<EscalationPolicy> {
        let map = self.policies.read().unwrap();
        map.values().cloned().collect()
    }

    // ── PostMortem CRUD ───────────────────────────────────────────────────────

    pub fn create_postmortem(&self, pm: PostMortem) {
        let mut map = self.postmortems.write().unwrap();
        map.insert(pm.id, pm);
    }

    pub fn get_postmortem(&self, id: Uuid) -> Option<PostMortem> {
        let map = self.postmortems.read().unwrap();
        map.get(&id).cloned()
    }

    pub fn update_postmortem(&self, pm: PostMortem) -> bool {
        let mut map = self.postmortems.write().unwrap();
        if map.contains_key(&pm.id) {
            map.insert(pm.id, pm);
            true
        } else {
            false
        }
    }

    pub fn list_postmortems(&self) -> Vec<PostMortem> {
        let map = self.postmortems.read().unwrap();
        map.values().cloned().collect()
    }
}
