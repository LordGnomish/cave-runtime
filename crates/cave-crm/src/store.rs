// SPDX-License-Identifier: AGPL-3.0-or-later
//! In-memory store placeholder. PostgreSQL-backed implementation lands in v0.2
//! (cave-rdbms-operator), driven by Twenty's Postgres schema.

use crate::models::{Activity, Company, Opportunity, Person};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct CrmStore {
    pub people: Arc<RwLock<HashMap<Uuid, Person>>>,
    pub companies: Arc<RwLock<HashMap<Uuid, Company>>>,
    pub opportunities: Arc<RwLock<HashMap<Uuid, Opportunity>>>,
    pub activities: Arc<RwLock<HashMap<Uuid, Activity>>>,
}
