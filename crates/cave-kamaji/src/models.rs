// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantSpec {
    pub kubernetes_version: String,
    pub data_store: String,
    pub replicas: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TenantPhase {
    Provisioning,
    Running,
    Upgrading,
    Deleting,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantStatus {
    pub phase: TenantPhase,
    pub api_server_endpoint: Option<String>,
    pub ready: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantControlPlane {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub spec: TenantSpec,
    pub status: TenantStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub namespace: String,
    pub spec: TenantSpec,
}
