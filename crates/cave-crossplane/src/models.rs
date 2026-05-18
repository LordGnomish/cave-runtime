// SPDX-License-Identifier: AGPL-3.0-or-later
//! Data models for cave-crossplane.

use std::collections::HashMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── XRD ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Xrd {
    pub id: Uuid,
    pub name: String,
    pub group: String,
    pub kind: String,
    pub list_kind: String,
    pub claim_kind: Option<String>,
    pub claim_list_kind: Option<String>,
    pub versions: Vec<XrdVersion>,
    pub scope: XrdScope,
    pub status: XrdStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrdVersion {
    pub name: String,
    pub served: bool,
    pub referenceable: bool,
    pub schema: Option<OpenApiV3Schema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiV3Schema {
    pub description: Option<String>,
    pub properties: HashMap<String, SchemaProperty>,
    pub required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaProperty {
    pub property_type: String,
    pub description: Option<String>,
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum XrdScope {
    Cluster,
    Namespaced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum XrdStatus {
    Offering,
    Rendering,
    Unknown,
}

// ── Composition ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Composition {
    pub id: Uuid,
    pub name: String,
    pub composite_type_ref: TypeRef,
    pub resources: Vec<ComposedResource>,
    pub pipeline: Vec<PipelineStep>,
    pub mode: CompositionMode,
    pub patch_sets: Vec<PatchSet>,
    pub status: CompositionStatus,
    pub revision: u32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeRef {
    pub api_version: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposedResource {
    pub name: String,
    pub base: serde_json::Value,
    pub patches: Vec<Patch>,
    pub connection_details: Vec<ConnectionDetail>,
    pub readiness_checks: Vec<ReadinessCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patch {
    pub patch_type: PatchType,
    pub from_field_path: Option<String>,
    pub to_field_path: Option<String>,
    pub transforms: Vec<Transform>,
    pub patch_set_name: Option<String>,
    pub combine: Option<CombineSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombineSpec {
    pub variables: Vec<CombineVariable>,
    pub strategy: CombineStrategy,
    pub string: Option<CombineStringSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombineVariable {
    pub from_field_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CombineStrategy {
    String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombineStringSpec {
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PatchType {
    FromCompositeFieldPath,
    ToCompositeFieldPath,
    CombineFromComposite,
    CombineToComposite,
    FromEnvironmentFieldPath,
    ToEnvironmentFieldPath,
    PatchSet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transform {
    pub transform_type: TransformType,
    pub math: Option<MathTransform>,
    pub map: Option<HashMap<String, String>>,
    pub string: Option<StringTransform>,
    pub convert: Option<ConvertTransform>,
    pub match_tf: Option<MatchTransform>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransformType {
    Map,
    Math,
    String,
    Convert,
    Match,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MathTransform {
    pub multiply: Option<f64>,
    pub clamp_min: Option<f64>,
    pub clamp_max: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StringTransform {
    pub kind: StringTransformType,
    pub format: Option<String>,
    pub regexp: Option<RegexpConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StringTransformType {
    Format,
    Convert,
    TrimPrefix,
    TrimSuffix,
    Regexp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegexpConfig {
    pub match_pattern: String,
    pub group: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertTransform {
    pub to_type: String,
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchTransform {
    pub patterns: Vec<MatchTransformPattern>,
    pub fallback_value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchTransformPattern {
    pub match_type: MatchType,
    pub literal: Option<String>,
    pub regexp: Option<String>,
    pub result: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MatchType {
    Literal,
    Regexp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchSet {
    pub name: String,
    pub patches: Vec<Patch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionDetail {
    pub name: String,
    pub connection_detail_type: ConnectionDetailType,
    pub from_field_path: Option<String>,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectionDetailType {
    FromFieldPath,
    FromConnectionSecretKey,
    FromValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessCheck {
    pub check_type: ReadinessCheckType,
    pub field_path: Option<String>,
    pub match_string: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReadinessCheckType {
    NonEmpty,
    MatchString,
    MatchTrue,
    MatchFalse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStep {
    pub step: String,
    pub function_ref: FunctionRef,
    pub input: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionRef {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompositionMode {
    Resources,
    Pipeline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompositionStatus {
    Available,
    Degraded,
}

// ── Composite Resource ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositeResource {
    pub id: Uuid,
    pub name: String,
    pub namespace: Option<String>,
    pub kind: String,
    pub api_version: String,
    pub spec: serde_json::Value,
    pub status: CompositeStatus,
    pub composition_ref: Option<String>,
    pub claim_ref: Option<ClaimRef>,
    pub synced_resources: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompositeStatus {
    Ready,
    Creating,
    Deleting,
    Unready,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimRef {
    pub namespace: String,
    pub name: String,
}

// ── Claim ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub kind: String,
    pub api_version: String,
    pub spec: serde_json::Value,
    pub status: ClaimStatus,
    pub composite_ref: Option<String>,
    pub sync_status: ClaimSyncStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClaimStatus {
    Ready,
    Waiting,
    Deleting,
    Unready,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClaimSyncStatus {
    Synced,
    OutOfSync,
    Unknown,
}

// ── Provider ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: Uuid,
    pub name: String,
    pub package: String,
    pub provider_type: ProviderType,
    pub revision: String,
    pub status: ProviderStatus,
    pub managed_resource_types: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProviderType {
    Official,
    Community,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProviderStatus {
    Installed,
    NotInstalled,
    Unhealthy,
}

// ── Reconcile ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconcileItem {
    pub id: Uuid,
    pub resource_kind: String,
    pub resource_name: String,
    pub namespace: Option<String>,
    pub status: ReconcileStatus,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReconcileStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeletionPolicy {
    Delete,
    Orphan,
}

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateXrdRequest {
    pub name: String,
    pub group: String,
    pub kind: String,
    pub claim_kind: Option<String>,
    pub scope: XrdScope,
    pub versions: Vec<XrdVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCompositionRequest {
    pub name: String,
    pub composite_type_ref: TypeRef,
    pub resources: Vec<ComposedResource>,
    pub pipeline: Vec<PipelineStep>,
    pub mode: CompositionMode,
    pub patch_sets: Vec<PatchSet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateClaimRequest {
    pub name: String,
    pub namespace: String,
    pub kind: String,
    pub api_version: String,
    pub spec: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProviderRequest {
    pub name: String,
    pub package: String,
    pub provider_type: ProviderType,
}
