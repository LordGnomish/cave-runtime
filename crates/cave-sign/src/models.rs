use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignedArtifact {
    pub id: Uuid,
    pub artifact_digest: String,
    pub artifact_type: ArtifactType,
    pub signature: String,
    pub signer_identity: String,
    pub signed_at: DateTime<Utc>,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    ContainerImage,
    Binary,
    Chart,
    Sbom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifyResult {
    pub artifact_digest: String,
    pub valid: bool,
    pub signer: Option<String>,
    pub reason: Option<String>,
}
