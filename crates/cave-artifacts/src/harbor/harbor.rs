// SPDX-License-Identifier: AGPL-3.0-or-later
//! Harbor-compatible project/admin models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Projects ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub public: bool,
    pub owner_name: String,
    pub description: String,
    pub repo_count: i64,
    pub creation_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
    pub metadata: ProjectMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_content_trust: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prevent_vul: Option<String>,
    /// Minimum blocking severity: "low"|"medium"|"high"|"critical"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_scan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reuse_sys_cve_allowlist: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub project_name: String,
    pub public: Option<bool>,
    pub metadata: Option<ProjectMetadata>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProjectRequest {
    pub metadata: Option<ProjectMetadata>,
    pub public: Option<bool>,
    pub description: Option<String>,
}

// ── Repositories ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id: Uuid,
    pub name: String,
    pub project_id: Uuid,
    pub description: String,
    pub artifact_count: i64,
    pub pull_count: i64,
    pub creation_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

// ── Robot Accounts ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RobotAccount {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub level: String, // "project" | "system"
    pub project_id: Option<Uuid>,
    pub expires_at: Option<DateTime<Utc>>,
    pub disabled: bool,
    pub permissions: Vec<RobotPermission>,
    pub creation_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RobotPermission {
    pub kind: String,      // "project"
    pub namespace: String, // project name or "*"
    pub access: Vec<AccessPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessPolicy {
    pub resource: String, // "repository", "artifact", "tag", etc.
    pub action: String,   // "push", "pull", "delete", "read", "create", "list", "*"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>, // "allow" (default)
}

#[derive(Debug, Deserialize)]
pub struct CreateRobotRequest {
    pub name: String,
    pub description: String,
    pub level: String,
    /// Days until expiry; -1 = never
    pub duration: Option<i64>,
    pub permissions: Vec<RobotPermission>,
}

#[derive(Debug, Serialize)]
pub struct CreateRobotResponse {
    pub id: Uuid,
    pub name: String,
    pub secret: String,
    pub creation_time: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

// ── Vulnerability Scanning ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub artifact_digest: String,
    pub scan_status: ScanStatus,
    pub severity: VulnSeverity,
    pub scanner: ScannerInfo,
    pub vulnerabilities: Vec<VulnItem>,
    pub start_time: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    NotScanned,
    Queued,
    Running,
    Success,
    Error,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VulnSeverity {
    None,
    Unknown,
    Negligible,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannerInfo {
    pub name: String,
    pub vendor: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnItem {
    pub id: String, // CVE-xxxx-xxxx
    pub package: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix_version: Option<String>,
    pub severity: VulnSeverity,
    pub description: String,
    pub links: Vec<String>,
    pub layer: Option<VulnLayer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnLayer {
    pub digest: String,
    pub diff_id: String,
}

// ── Replication ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationPolicy {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src_registry: Option<RegistryEndpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_registry: Option<RegistryEndpoint>,
    pub dest_namespace: String,
    pub dest_namespace_replace_count: Option<i32>,
    pub trigger: ReplicationTrigger,
    pub filters: Vec<ReplicationFilter>,
    pub deletion: bool,
    #[serde(rename = "override")]
    pub override_dest: bool,
    pub enabled: bool,
    pub speed: Option<i32>, // KB/s
    pub creation_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEndpoint {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub credential_type: String, // "basic" | "bearer" | "oauth"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_key: Option<String>,
    pub insecure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationTrigger {
    pub trigger_type: String, // "manual" | "scheduled" | "event_based"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationFilter {
    #[serde(rename = "type")]
    pub filter_type: String, // "name" | "tag" | "label" | "resource"
    pub value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoration: Option<String>, // "matches" | "excludes"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationExecution {
    pub id: Uuid,
    pub policy_id: Uuid,
    pub status: String, // "InProgress" | "Succeed" | "Failed" | "Stopped"
    pub trigger: String,
    pub start_time: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<DateTime<Utc>>,
    pub succeeded: i64,
    pub failed: i64,
    pub in_progress: i64,
    pub stopped: i64,
}

// ── Tag Retention ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub id: Uuid,
    pub project_id: Uuid,
    pub scope: RetentionScope,
    pub trigger: RetentionTrigger,
    pub rules: Vec<RetentionRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionScope {
    pub level: String, // "project"
    #[serde(rename = "ref")]
    pub ref_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionTrigger {
    pub kind: String, // "Schedule"
    pub settings: Option<HashMap<String, serde_json::Value>>,
    pub references: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionRule {
    pub disabled: bool,
    pub action: String,   // "retain"
    pub template: String, // "latestPushedK" | "latestActiveK" | "nDaysSinceLastPush" | etc.
    pub tag_selectors: Vec<RetentionSelector>,
    pub scope_selectors: HashMap<String, Vec<RetentionSelector>>,
    pub params: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionSelector {
    pub kind: String,       // "doublestar" | "regexp"
    pub decoration: String, // "matches" | "excludes"
    pub pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extras: Option<String>,
}

// ── Immutable Tag Rules ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImmutableTagRule {
    pub id: Uuid,
    pub project_id: Uuid,
    pub disabled: bool,
    pub tag_selectors: Vec<RetentionSelector>,
    pub scope_selectors: HashMap<String, Vec<RetentionSelector>>,
}

// ── Webhooks ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPolicy {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub description: String,
    pub targets: Vec<WebhookTarget>,
    pub event_types: Vec<String>,
    pub enabled: bool,
    pub creation_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookTarget {
    pub notify_type: String, // "http" | "slack"
    pub endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,
    pub skip_cert_verify: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookLog {
    pub id: Uuid,
    pub policy_id: Uuid,
    pub event_type: String,
    pub notify_type: String,
    pub status: i32,
    pub address: String,
    pub creation_time: DateTime<Utc>,
}

// ── Quotas ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quota {
    pub id: Uuid,
    #[serde(rename = "ref")]
    pub ref_info: QuotaRef,
    pub hard: QuotaLimits,
    pub used: QuotaLimits,
    pub creation_date: DateTime<Utc>,
    pub update_date: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaRef {
    pub id: i64,
    pub kind: String, // "project"
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaLimits {
    pub count: i64,
    pub storage: i64, // bytes, -1 = unlimited
}

#[derive(Debug, Deserialize)]
pub struct UpdateQuotaRequest {
    pub hard: QuotaLimits,
}

// ── Audit Log ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: i64,
    pub username: String,
    pub resource: String,
    pub resource_type: String,
    pub operation: String,
    pub op_time: DateTime<Utc>,
}

// ── Labels ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub color: String,
    pub scope: String, // "g" (global) | "p" (project-scoped)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Uuid>,
    pub creation_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateLabelRequest {
    pub name: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub scope: String,
    pub project_id: Option<Uuid>,
}

// ── P2P Preheat ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreheatPolicy {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub description: String,
    pub provider_id: Uuid,
    pub filters: Option<serde_json::Value>,
    pub trigger: Option<serde_json::Value>,
    pub enabled: bool,
    pub creation_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreheatProvider {
    pub id: Uuid,
    pub name: String,
    pub endpoint: String,
    #[serde(rename = "authMode")]
    pub auth_mode: String, // "none" | "basic" | "bearer"
    pub enabled: bool,
    pub status: String,
}

// ── LDAP / OIDC Auth Config ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LdapConfig {
    pub url: String,
    pub search_dn: String,
    pub search_password: String,
    pub base_dn: String,
    pub filter: String,
    pub uid: String,
    pub scope: i32,
    pub timeout: i32,
    pub verify_cert: bool,
    pub group_base_dn: String,
    pub group_filter: String,
    pub group_gid: String,
    pub group_admin_dn: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcConfig {
    pub endpoint: String,
    pub client_id: String,
    pub client_secret: String,
    pub groups_claim: Option<String>,
    pub admin_group: Option<String>,
    pub verify_cert: bool,
    pub auto_onboard: bool,
    pub user_claim: String,
    pub name: String,
    pub scope: String,
    pub extra_redirect_params: Option<HashMap<String, String>>,
}

// ── System Info ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub registry_url: String,
    pub harbor_version: String,
    pub oci_version: String,
    pub auth_mode: String,
    pub primary_auth_mode: bool,
    pub project_creation_restriction: String,
    pub read_only: bool,
    pub with_notary: bool,
    pub with_trivy: bool,
    pub with_chartmuseum: bool,
    pub notification_enable: bool,
}
