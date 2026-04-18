use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustDomain {
    pub id: Uuid,
    pub name: String,
    pub spiffe_id: String,
    pub bundle: Option<Bundle>,
    pub status: TrustDomainStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub trust_domain: String,
    pub jwt_authorities: Vec<JwtAuthority>,
    pub x509_authorities: Vec<X509Authority>,
    pub sequence_number: u64,
    pub refresh_hint_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtAuthority {
    pub public_key_pem: String,
    pub key_id: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct X509Authority {
    pub asn1: String,
    pub tainted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrustDomainStatus {
    Active,
    Inactive,
    Federating,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationEntry {
    pub id: Uuid,
    pub entry_id: String,
    pub spiffe_id: SpiffeId,
    pub parent_id: SpiffeId,
    pub selectors: Vec<Selector>,
    pub dns_names: Vec<String>,
    pub federates_with: Vec<String>,
    pub admin: bool,
    pub downstream: bool,
    pub ttl_secs: u32,
    pub store_svid: bool,
    pub revision_number: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiffeId {
    pub trust_domain: String,
    pub path: String,
}

impl SpiffeId {
    pub fn to_uri(&self) -> String {
        format!("spiffe://{}{}", self.trust_domain, self.path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selector {
    pub selector_type: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct X509Svid {
    pub id: Uuid,
    pub spiffe_id: String,
    pub cert_chain_pem: String,
    pub private_key_pem: String,
    pub bundle: String,
    pub hint: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub issued_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtSvid {
    pub id: Uuid,
    pub spiffe_id: String,
    pub token: String,
    pub hint: Option<String>,
    pub audience: Vec<String>,
    pub expires_at: DateTime<Utc>,
    pub issued_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpireAgent {
    pub id: Uuid,
    pub agent_id: String,
    pub spiffe_id: String,
    pub node_name: String,
    pub namespace: String,
    pub attestation_type: AttestationType,
    pub status: AgentStatus,
    pub serial_number: String,
    pub can_reattest: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AttestationType {
    K8sPsat,
    K8sSat,
    AwsIid,
    X509Pop,
    JoinToken,
    Tpm,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Active,
    Banned,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationRelationship {
    pub id: Uuid,
    pub trust_domain: String,
    pub bundle_endpoint_url: String,
    pub bundle_endpoint_profile: BundleEndpointProfile,
    pub status: FederationStatus,
    pub last_bundle_refresh: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BundleEndpointProfile {
    HttpsWeb,
    HttpsSpiffe,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FederationStatus {
    Active,
    Refreshing,
    Failed,
    Unknown,
}

// Request types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTrustDomainRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRegistrationEntryRequest {
    pub spiffe_id: SpiffeId,
    pub parent_id: SpiffeId,
    pub selectors: Vec<Selector>,
    pub dns_names: Option<Vec<String>>,
    pub federates_with: Option<Vec<String>>,
    pub ttl_secs: Option<u32>,
    pub admin: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MintX509SvidRequest {
    pub spiffe_id: String,
    pub ttl_secs: Option<u32>,
    pub dns_names: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MintJwtSvidRequest {
    pub spiffe_id: String,
    pub audience: Vec<String>,
    pub ttl_secs: Option<u32>,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestAgentRequest {
    pub node_name: String,
    pub namespace: String,
    pub attestation_type: AttestationType,
    pub spiffe_id_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFederationRequest {
    pub trust_domain: String,
    pub bundle_endpoint_url: String,
    pub bundle_endpoint_profile: Option<BundleEndpointProfile>,
}
