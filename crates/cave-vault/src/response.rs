// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use axum::{
    Json,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct VaultResponse {
    pub request_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub lease_id: String,
    pub renewable: bool,
    pub lease_duration: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap_info: Option<WrapInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthInfo>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WrapInfo {
    pub token: String,
    pub accessor: String,
    pub ttl: i64,
    pub creation_time: String,
    pub creation_path: String,
    pub wrapped_accessor: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AuthInfo {
    pub client_token: String,
    pub accessor: String,
    pub policies: Vec<String>,
    pub token_policies: Vec<String>,
    pub metadata: std::collections::HashMap<String, String>,
    pub lease_duration: i64,
    pub renewable: bool,
    pub entity_id: String,
    pub token_type: String,
    pub orphan: bool,
}

impl VaultResponse {
    pub fn new() -> Self {
        Self {
            request_id: Uuid::new_v4().to_string(),
            lease_id: String::new(),
            renewable: false,
            lease_duration: 0,
            data: None,
            wrap_info: None,
            warnings: None,
            auth: None,
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn with_auth(mut self, auth: AuthInfo) -> Self {
        self.auth = Some(auth);
        self
    }

    pub fn with_lease(mut self, lease_id: String, ttl: i64, renewable: bool) -> Self {
        self.lease_id = lease_id;
        self.lease_duration = ttl;
        self.renewable = renewable;
        self
    }
}

impl IntoResponse for VaultResponse {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}
