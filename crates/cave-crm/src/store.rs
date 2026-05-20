// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! In-memory CRM store.
//!
//! Twenty backs persistence with PostgreSQL via TypeORM. The cave-crm
//! MVP exposes the same surface against an in-memory implementation
//! (HashMap-per-entity inside a workspace-keyed outer map) so that
//! integration tests can spin up a tenant without a real database. A
//! Postgres-backed store using `cave-rdbms-operator` lands in v0.2.
//!
//! Multi-tenant semantics — every read/write takes a `workspace_id`
//! and is filtered before yielding rows. Cross-tenant leakage is the
//! single most important correctness invariant.

use crate::indexes::IndexSet;
use crate::models::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct CrmStore {
    pub workspaces: Arc<RwLock<HashMap<Uuid, Workspace>>>,
    pub workspace_members: Arc<RwLock<HashMap<Uuid, WorkspaceMember>>>,
    pub users: Arc<RwLock<HashMap<Uuid, User>>>,

    pub people: Arc<RwLock<HashMap<Uuid, Person>>>,
    pub companies: Arc<RwLock<HashMap<Uuid, Company>>>,
    pub opportunities: Arc<RwLock<HashMap<Uuid, Opportunity>>>,
    pub leads: Arc<RwLock<HashMap<Uuid, Lead>>>,

    pub pipeline_steps: Arc<RwLock<HashMap<Uuid, PipelineStep>>>,
    pub notes: Arc<RwLock<HashMap<Uuid, Note>>>,
    pub tasks: Arc<RwLock<HashMap<Uuid, Task>>>,
    pub activity_targets: Arc<RwLock<HashMap<Uuid, ActivityTarget>>>,

    pub calendar_events: Arc<RwLock<HashMap<Uuid, CalendarEvent>>>,
    pub calendar_attendees: Arc<RwLock<HashMap<Uuid, CalendarEventAttendee>>>,

    pub views: Arc<RwLock<HashMap<Uuid, View>>>,
    pub api_keys: Arc<RwLock<HashMap<Uuid, ApiKey>>>,

    pub object_metadata: Arc<RwLock<HashMap<Uuid, ObjectMetadata>>>,
    pub field_metadata: Arc<RwLock<HashMap<Uuid, FieldMetadata>>>,

    /// Custom-field values, keyed by `(workspace_id, object_row_id,
    /// field_metadata_id)` → JSON-encoded value. JSON because Twenty
    /// allows arbitrary `FieldKind` schemas.
    pub custom_field_values: Arc<RwLock<HashMap<(Uuid, Uuid, Uuid), String>>>,

    pub indexes: Arc<RwLock<IndexSet>>,
}

impl CrmStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bootstrap a fresh workspace with the standard pipeline steps,
    /// object metadata, and indexes that Twenty seeds on workspace
    /// creation.
    pub async fn bootstrap_workspace(&self, name: impl Into<String>) -> Workspace {
        let ws = Workspace::new(name);
        self.workspaces.write().await.insert(ws.id, ws.clone());

        let stages = PipelineStep::defaults(ws.id);
        let mut ps = self.pipeline_steps.write().await;
        for stage in stages {
            ps.insert(stage.id, stage);
        }
        drop(ps);

        let stds = ObjectMetadata::standards(ws.id);
        let mut om = self.object_metadata.write().await;
        for o in stds {
            om.insert(o.id, o);
        }
        drop(om);

        let mut idx = self.indexes.write().await;
        idx.seed_default_for_workspace(ws.id);
        drop(idx);

        ws
    }

    /// List opportunities, filtered to a workspace, ordered by `position`.
    pub async fn opportunities_in_workspace(&self, workspace_id: Uuid) -> Vec<Opportunity> {
        let mut out: Vec<Opportunity> = self
            .opportunities
            .read()
            .await
            .values()
            .filter(|o| o.workspace_id == workspace_id)
            .cloned()
            .collect();
        out.sort_by_key(|o| o.position);
        out
    }

    /// List opportunities in a given pipeline step (for kanban lane render).
    pub async fn opportunities_in_pipeline_step(
        &self,
        workspace_id: Uuid,
        pipeline_step_id: Uuid,
    ) -> Vec<Opportunity> {
        let mut out: Vec<Opportunity> = self
            .opportunities
            .read()
            .await
            .values()
            .filter(|o| o.workspace_id == workspace_id && o.pipeline_step_id == pipeline_step_id)
            .cloned()
            .collect();
        out.sort_by_key(|o| o.position);
        out
    }

    /// Convert a Lead → Company + Person + Opportunity. Mirrors the
    /// legacy `cave-erp/src/modules/crm.rs::convert_lead` behavior
    /// (now deprecated by ADR-145) on cave-crm's surface.
    pub async fn convert_lead(&self, lead_id: Uuid) -> Option<ConvertedLead> {
        let mut leads = self.leads.write().await;
        let lead = leads.get(&lead_id)?.clone();
        if lead.status == LeadStatus::Converted {
            return None;
        }

        let mut company = Company::new(lead.workspace_id, &lead.company);
        company.domain_name = extract_domain(&lead.email);

        let mut person = Person::new(lead.workspace_id, "", "");
        let (first, last) = split_name(&lead.contact_name);
        person.first_name = first;
        person.last_name = last;
        person.email = Some(lead.email.clone());
        person.phone = lead.phone.clone();
        person.company_id = Some(company.id);

        // Drop the leads lock before touching other stores to keep lock
        // order strictly entity-typed (avoids deadlock with concurrent
        // person/company/opportunity writers).
        drop(leads);

        self.companies
            .write()
            .await
            .insert(company.id, company.clone());
        self.people.write().await.insert(person.id, person.clone());

        // First pipeline step in the workspace becomes the new opp's lane.
        let first_step = self
            .pipeline_steps
            .read()
            .await
            .values()
            .filter(|s| s.workspace_id == lead.workspace_id)
            .min_by_key(|s| s.position)
            .map(|s| s.id);
        let pipeline_step_id = first_step.unwrap_or_else(Uuid::new_v4);

        let mut opp = Opportunity::new(
            lead.workspace_id,
            format!("Opp from {}", lead.name),
            pipeline_step_id,
        );
        opp.company_id = Some(company.id);
        opp.point_of_contact_id = Some(person.id);
        opp.owner_user_id = lead.assigned_user_id;

        self.opportunities.write().await.insert(opp.id, opp.clone());

        // Mark lead as converted.
        let mut leads_w = self.leads.write().await;
        if let Some(l) = leads_w.get_mut(&lead_id) {
            l.mark_converted();
        }

        Some(ConvertedLead {
            company,
            person,
            opportunity: opp,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ConvertedLead {
    pub company: Company,
    pub person: Person,
    pub opportunity: Opportunity,
}

fn extract_domain(email: &str) -> Option<String> {
    email.split_once('@').map(|(_, d)| d.to_string())
}

fn split_name(full: &str) -> (String, String) {
    let mut it = full.splitn(2, ' ');
    let first = it.next().unwrap_or("").to_string();
    let last = it.next().unwrap_or("").to_string();
    (first, last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bootstrap_workspace_seeds_pipeline_steps() {
        let s = CrmStore::new();
        let ws = s.bootstrap_workspace("Acme").await;
        let ps: Vec<_> = s
            .pipeline_steps
            .read()
            .await
            .values()
            .filter(|p| p.workspace_id == ws.id)
            .cloned()
            .collect();
        assert_eq!(ps.len(), 5);
    }

    #[tokio::test]
    async fn bootstrap_workspace_seeds_object_metadata() {
        let s = CrmStore::new();
        let ws = s.bootstrap_workspace("Acme").await;
        let om: Vec<_> = s
            .object_metadata
            .read()
            .await
            .values()
            .filter(|p| p.workspace_id == ws.id)
            .cloned()
            .collect();
        assert_eq!(om.len(), 8);
    }

    #[tokio::test]
    async fn opportunities_filtered_by_workspace() {
        let s = CrmStore::new();
        let ws1 = s.bootstrap_workspace("Acme").await;
        let ws2 = s.bootstrap_workspace("Other").await;
        let step1 = s
            .pipeline_steps
            .read()
            .await
            .values()
            .find(|p| p.workspace_id == ws1.id)
            .unwrap()
            .id;
        let mut o1 = Opportunity::new(ws1.id, "A", step1);
        o1.position = 0;
        let mut o2 = Opportunity::new(ws1.id, "B", step1);
        o2.position = 1;
        let step2 = s
            .pipeline_steps
            .read()
            .await
            .values()
            .find(|p| p.workspace_id == ws2.id)
            .unwrap()
            .id;
        let o3 = Opportunity::new(ws2.id, "Z", step2);

        for o in [&o1, &o2, &o3] {
            s.opportunities.write().await.insert(o.id, o.clone());
        }
        let out = s.opportunities_in_workspace(ws1.id).await;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "A"); // ordered by position
    }

    #[tokio::test]
    async fn convert_lead_creates_company_person_opportunity() {
        let s = CrmStore::new();
        let ws = s.bootstrap_workspace("Acme").await;
        let lead = Lead::new(
            ws.id,
            "Acme Q4",
            "Bob Smith",
            "bob@acme.com",
            "Acme Co",
            "Website",
        );
        let lead_id = lead.id;
        s.leads.write().await.insert(lead_id, lead);
        let converted = s.convert_lead(lead_id).await.expect("converts");
        assert_eq!(converted.company.name, "Acme Co");
        assert_eq!(converted.company.domain_name.as_deref(), Some("acme.com"));
        assert_eq!(converted.person.first_name, "Bob");
        assert_eq!(converted.person.last_name, "Smith");
        assert_eq!(converted.person.company_id, Some(converted.company.id));
        assert_eq!(converted.opportunity.company_id, Some(converted.company.id));
        // Lead is now Converted.
        let stored = s.leads.read().await.get(&lead_id).cloned().unwrap();
        assert_eq!(stored.status, LeadStatus::Converted);
    }

    #[tokio::test]
    async fn convert_already_converted_lead_returns_none() {
        let s = CrmStore::new();
        let ws = s.bootstrap_workspace("Acme").await;
        let mut lead = Lead::new(ws.id, "n", "Bob S", "b@b.c", "co", "src");
        lead.mark_converted();
        let id = lead.id;
        s.leads.write().await.insert(id, lead);
        assert!(s.convert_lead(id).await.is_none());
    }
}
