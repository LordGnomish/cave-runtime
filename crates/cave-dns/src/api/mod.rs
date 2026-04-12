pub mod records;
pub mod zones;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::error::DnsError;

// ─── Shared API types ────────────────────────────────────────────────────────

/// Generic success wrapper.
#[derive(Serialize)]
pub struct ApiOk<T: Serialize> {
    pub data: T,
}

/// Generic error wrapper.
#[derive(Serialize)]
pub struct ApiError {
    pub error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
    }
}

impl From<DnsError> for ApiError {
    fn from(e: DnsError) -> Self {
        ApiError {
            error: e.to_string(),
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

// ─── Zone API types ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ZoneDto {
    pub name: String,
    pub zone_type: String,
    pub serial: u32,
    pub record_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateZoneRequest {
    pub name: String,
    pub zone_type: Option<String>,
    pub file: Option<String>,
}

// ─── Record API types ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RecordDto {
    pub id: String,
    pub name: String,
    pub ttl: u32,
    pub class: String,
    pub record_type: String,
    pub rdata: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateRecordRequest {
    pub name: String,
    pub ttl: u32,
    #[serde(default = "default_class")]
    pub class: String,
    pub record_type: String,
    pub rdata: String,
}

fn default_class() -> String {
    "IN".into()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateRecordRequest {
    pub ttl: Option<u32>,
    pub rdata: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BatchRecordRequest {
    #[serde(default)]
    pub create: Vec<CreateRecordRequest>,
    #[serde(default)]
    pub delete: Vec<String>,
}
