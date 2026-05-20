// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Full Grafana v10 data model — panels, variables, annotations, alerting,
//! datasources, folders, permissions, users, orgs, teams.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Org / User / Team ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Org {
    pub id: i64,
    pub name: String,
    pub address: OrgAddress,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct OrgAddress {
    pub address1: String,
    pub address2: String,
    pub city: String,
    pub zip_code: String,
    pub state: String,
    pub country: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: i64,
    pub uid: String,
    pub name: String,
    pub email: String,
    pub login: String,
    pub org_id: i64,
    pub org_role: OrgRole,
    pub is_admin: bool,
    pub is_disabled: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    #[serde(default)]
    pub last_seen_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub avatar_url: String,
    #[serde(default)]
    pub theme: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub id: i64,
    pub uid: String,
    pub org_id: i64,
    pub name: String,
    pub email: String,
    pub avatar_url: String,
    pub member_count: i64,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TeamMember {
    pub org_id: i64,
    pub team_id: i64,
    pub user_id: i64,
    pub login: String,
    pub name: String,
    pub email: String,
    pub avatar_url: String,
    pub labels: Vec<String>,
    pub permission: TeamPermission,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub enum TeamPermission {
    #[default]
    Member = 0,
    Admin = 4,
}

// ─── Role / Permission ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum OrgRole {
    #[default]
    Viewer,
    Editor,
    Admin,
}

impl std::fmt::Display for OrgRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrgRole::Viewer => write!(f, "Viewer"),
            OrgRole::Editor => write!(f, "Editor"),
            OrgRole::Admin => write!(f, "Admin"),
        }
    }
}

impl std::str::FromStr for OrgRole {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Viewer" | "viewer" => Ok(OrgRole::Viewer),
            "Editor" | "editor" => Ok(OrgRole::Editor),
            "Admin" | "admin" => Ok(OrgRole::Admin),
            other => Err(format!("unknown role: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PermissionLevel {
    View = 1,
    Edit = 2,
    Admin = 4,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DashboardPermission {
    pub id: i64,
    pub dashboard_id: i64,
    pub folder_id: Option<i64>,
    pub user_id: Option<i64>,
    pub team_id: Option<i64>,
    pub role: Option<OrgRole>,
    pub permission: PermissionLevel,
    pub permission_name: String,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

// ─── API Keys / Service Accounts ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiKey {
    pub id: i64,
    pub org_id: i64,
    pub name: String,
    pub role: OrgRole,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    #[serde(default)]
    pub expires: Option<DateTime<Utc>>,
    /// Hashed key — never returned in API responses after creation
    #[serde(skip_serializing)]
    pub key_hash: String,
    /// Only present in create response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceAccount {
    pub id: i64,
    pub uid: String,
    pub org_id: i64,
    pub name: String,
    pub login: String,
    pub role: OrgRole,
    pub is_disabled: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub avatar_url: String,
    pub tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceAccountToken {
    pub id: i64,
    pub name: String,
    pub created: DateTime<Utc>,
    #[serde(default)]
    pub expires: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

// ─── Folder ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: i64,
    pub uid: String,
    pub org_id: i64,
    pub title: String,
    pub url: String,
    pub has_acl: bool,
    pub can_save: bool,
    pub can_edit: bool,
    pub can_admin: bool,
    pub can_delete: bool,
    pub created_by: String,
    pub created: DateTime<Utc>,
    pub updated_by: String,
    pub updated: DateTime<Utc>,
    pub version: i64,
    #[serde(default)]
    pub parent_uid: Option<String>,
}

impl Folder {
    pub fn new(id: i64, org_id: i64, uid: &str, title: &str) -> Self {
        let now = Utc::now();
        Self {
            id,
            uid: uid.to_string(),
            org_id,
            title: title.to_string(),
            url: format!("/dashboards/f/{uid}/"),
            has_acl: false,
            can_save: true,
            can_edit: true,
            can_admin: true,
            can_delete: true,
            created_by: "admin".into(),
            created: now,
            updated_by: "admin".into(),
            updated: now,
            version: 1,
            parent_uid: None,
        }
    }
}

// ─── DataSource ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum DataSourceType {
    Prometheus,
    Loki,
    Jaeger,
    Tempo,
    Postgres,
    Elasticsearch,
    InfluxDb,
    Graphite,
    Mysql,
    Mssql,
    Cloudwatch,
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for DataSourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Prometheus => write!(f, "prometheus"),
            Self::Loki => write!(f, "loki"),
            Self::Jaeger => write!(f, "jaeger"),
            Self::Tempo => write!(f, "tempo"),
            Self::Postgres => write!(f, "postgres"),
            Self::Elasticsearch => write!(f, "elasticsearch"),
            Self::InfluxDb => write!(f, "influxdb"),
            Self::Graphite => write!(f, "graphite"),
            Self::Mysql => write!(f, "mysql"),
            Self::Mssql => write!(f, "mssql"),
            Self::Cloudwatch => write!(f, "cloudwatch"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DataSourceAccess {
    #[default]
    Proxy,
    Direct,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DataSource {
    pub id: i64,
    pub uid: String,
    pub org_id: i64,
    pub name: String,
    #[serde(rename = "type")]
    pub ds_type: DataSourceType,
    pub type_name: String,
    pub type_logo_url: String,
    pub access: DataSourceAccess,
    pub url: String,
    pub password: String,
    pub user: String,
    pub database: String,
    pub basic_auth: bool,
    pub basic_auth_user: String,
    pub basic_auth_password: String,
    pub with_credentials: bool,
    pub is_default: bool,
    pub json_data: serde_json::Value,
    pub secure_json_fields: HashMap<String, bool>,
    pub version: i64,
    pub read_only: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

impl DataSource {
    pub fn new(
        id: i64,
        org_id: i64,
        uid: &str,
        name: &str,
        ds_type: DataSourceType,
        url: &str,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            uid: uid.to_string(),
            org_id,
            name: name.to_string(),
            type_name: ds_type.to_string(),
            type_logo_url: format!("public/app/plugins/datasource/{}/img/logo.svg", ds_type),
            ds_type,
            access: DataSourceAccess::Proxy,
            url: url.to_string(),
            password: String::new(),
            user: String::new(),
            database: String::new(),
            basic_auth: false,
            basic_auth_user: String::new(),
            basic_auth_password: String::new(),
            with_credentials: false,
            is_default: false,
            json_data: serde_json::json!({}),
            secure_json_fields: HashMap::new(),
            version: 1,
            read_only: false,
            created: now,
            updated: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceHealthStatus {
    pub status: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
}

// ─── Panel ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PanelType {
    Graph,
    Stat,
    Gauge,
    Table,
    BarGauge,
    PieChart,
    Heatmap,
    Logs,
    Traces,
    Text,
    AlertList,
    DashboardList,
    News,
    Histogram,
    StateTimeline,
    StatusHistory,
    Candlestick,
    XyChart,
    Geomap,
    Canvas,
    Flamegraph,
    Trend,
    Row,
}

impl std::fmt::Display for PanelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PanelType::Graph => "graph",
            PanelType::Stat => "stat",
            PanelType::Gauge => "gauge",
            PanelType::Table => "table",
            PanelType::BarGauge => "bargauge",
            PanelType::PieChart => "piechart",
            PanelType::Heatmap => "heatmap",
            PanelType::Logs => "logs",
            PanelType::Traces => "traces",
            PanelType::Text => "text",
            PanelType::AlertList => "alertlist",
            PanelType::DashboardList => "dashlist",
            PanelType::News => "news",
            PanelType::Histogram => "histogram",
            PanelType::StateTimeline => "state-timeline",
            PanelType::StatusHistory => "status-history",
            PanelType::Candlestick => "candlestick",
            PanelType::XyChart => "xychart",
            PanelType::Geomap => "geomap",
            PanelType::Canvas => "canvas",
            PanelType::Flamegraph => "flamegraph",
            PanelType::Trend => "trend",
            PanelType::Row => "row",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GridPos {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    #[serde(default)]
    pub static_pos: bool,
}

/// Datasource reference inside a panel target
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DatasourceRef {
    pub uid: String,
    #[serde(rename = "type")]
    pub ds_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    /// Raw query expression (PromQL, LogQL, SQL, etc.)
    pub expr: String,
    /// Legend format template
    #[serde(default)]
    pub legend_format: String,
    /// Reference ID within the panel (A, B, C…)
    pub ref_id: String,
    /// Datasource for this specific target (overrides panel datasource)
    #[serde(default)]
    pub datasource: Option<DatasourceRef>,
    /// Whether this query is hidden (used for calculations only)
    #[serde(default)]
    pub hide: bool,
    /// Instant vs range query (Prometheus)
    #[serde(default)]
    pub instant: bool,
    /// Range query flag
    #[serde(default)]
    pub range: bool,
    /// Additional query-specific fields
    #[serde(default, flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Panel transformation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PanelTransformation {
    pub id: TransformationType,
    pub options: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TransformationType {
    Reduce,
    Merge,
    FilterFieldsByName,
    FilterByValue,
    Organize,
    CalculateField,
    GroupBy,
    SortBy,
    RenameByRegex,
    Concatenate,
    ConvertFieldType,
    Limit,
    SeriesToRows,
    JoinByField,
    LabelsToFields,
    FieldLookup,
    Histogram,
    Spatial,
    Regression,
}

/// Panel threshold step
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThresholdStep {
    pub color: String,
    pub value: Option<f64>,
}

/// Panel field config overrides
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FieldConfig {
    pub defaults: FieldConfigDefaults,
    #[serde(default)]
    pub overrides: Vec<FieldConfigOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FieldConfigDefaults {
    #[serde(default)]
    pub color: Option<FieldColorConfig>,
    #[serde(default)]
    pub custom: serde_json::Value,
    #[serde(default)]
    pub decimals: Option<i32>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub display_name_from_ds: String,
    #[serde(default)]
    pub filterable: bool,
    #[serde(default)]
    pub mappings: Vec<serde_json::Value>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub no_value: Option<String>,
    #[serde(default)]
    pub thresholds: Option<ThresholdsConfig>,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub links: Vec<DataLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FieldColorConfig {
    pub mode: String,
    #[serde(default)]
    pub fixed_color: Option<String>,
    #[serde(default)]
    pub series_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThresholdsConfig {
    pub mode: String, // "absolute" | "percentage"
    pub steps: Vec<ThresholdStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FieldConfigOverride {
    pub matcher: FieldMatcher,
    pub properties: Vec<FieldProperty>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FieldMatcher {
    pub id: String,
    pub options: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FieldProperty {
    pub id: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DataLink {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub target_blank: bool,
}

/// Alert configuration embedded in a panel (legacy alerting)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PanelAlert {
    pub id: i64,
    pub dashboard_id: i64,
    pub panel_id: i64,
    pub name: String,
    pub state: AlertState,
    pub conditions: Vec<AlertCondition>,
    pub for_duration: String,
    pub frequency: String,
    pub no_data_state: NoDataState,
    pub exec_err_state: ExecErrState,
    pub notifications: Vec<AlertNotificationRef>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AlertNotificationRef {
    pub id: i64,
    pub uid: String,
}

/// Main panel model
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Panel {
    pub id: i32,
    pub title: String,
    #[serde(rename = "type")]
    pub panel_type: PanelType,
    pub grid_pos: GridPos,
    #[serde(default)]
    pub datasource: Option<DatasourceRef>,
    #[serde(default)]
    pub targets: Vec<Target>,
    #[serde(default)]
    pub transformations: Vec<PanelTransformation>,
    #[serde(default)]
    pub field_config: FieldConfig,
    #[serde(default)]
    pub options: serde_json::Value,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub transparent: bool,
    #[serde(default)]
    pub links: Vec<DataLink>,
    #[serde(default)]
    pub repeat: Option<String>,
    #[serde(default)]
    pub repeat_direction: Option<String>,
    #[serde(default)]
    pub max_data_points: Option<i64>,
    #[serde(default)]
    pub interval: Option<String>,
    #[serde(default)]
    pub time_from: Option<String>,
    #[serde(default)]
    pub time_shift: Option<String>,
    #[serde(default)]
    pub alert: Option<PanelAlert>,
    /// Sub-panels (for row collapsed state)
    #[serde(default)]
    pub panels: Vec<Panel>,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub hide_time_override: bool,
    #[serde(default)]
    pub cache_timeout: Option<String>,
    #[serde(default)]
    pub query_caching_ttl: Option<i64>,
    #[serde(default)]
    pub plugin_version: Option<String>,
}

// ─── Variable ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum VariableType {
    Query,
    Custom,
    Textbox,
    Constant,
    Datasource,
    Interval,
    AdhocFilters,
    GroupBy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum VariableRefresh {
    #[default]
    Never,
    OnDashboardLoad,
    OnTimeRangeChanged,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum VariableHide {
    #[default]
    DontHide,
    HideLabel,
    HideVariable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum VariableSort {
    #[default]
    Disabled,
    AlphabeticalAsc,
    AlphabeticalDesc,
    NumericalAsc,
    NumericalDesc,
    AlphabeticalCaseInsensitiveAsc,
    AlphabeticalCaseInsensitiveDesc,
    NaturalAsc,
    NaturalDesc,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct VariableOption {
    pub text: serde_json::Value, // string or Vec<String> for multi
    pub value: serde_json::Value,
    #[serde(default)]
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    pub name: String,
    pub label: String,
    #[serde(rename = "type")]
    pub var_type: VariableType,
    #[serde(default)]
    pub description: String,
    pub hide: VariableHide,
    pub refresh: VariableRefresh,
    pub sort: VariableSort,
    /// For query variables: the PromQL/LogQL query to run
    #[serde(default)]
    pub query: serde_json::Value,
    /// For query variables: the datasource to run query against
    #[serde(default)]
    pub datasource: Option<DatasourceRef>,
    /// For custom/interval: comma-separated values
    #[serde(default)]
    pub options: Vec<VariableOption>,
    /// Current selected value
    pub current: VariableOption,
    #[serde(default)]
    pub multi: bool,
    #[serde(default)]
    pub include_all: bool,
    #[serde(default)]
    pub all_value: Option<String>,
    #[serde(default)]
    pub regex: String,
    #[serde(default)]
    pub values_text: String,
    #[serde(default)]
    pub skip_url_sync: bool,
}

// ─── Annotation ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnnotationBuiltinType {
    Dashboard,
    Alert,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationQuery {
    pub name: String,
    #[serde(default)]
    pub datasource: Option<DatasourceRef>,
    pub enable: bool,
    pub hide: bool,
    pub icon_color: String,
    #[serde(rename = "type")]
    pub annotation_type: String, // "dashboard" | "alert" | custom
    #[serde(default)]
    pub expr: String,
    #[serde(default)]
    pub step: String,
    #[serde(default)]
    pub title_format: String,
    #[serde(default)]
    pub text_format: String,
    #[serde(default)]
    pub tag_keys: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// An annotation instance stored in the database
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    pub id: i64,
    pub alert_id: i64,
    pub alert_name: String,
    pub dashboard_id: i64,
    pub dashboard_uid: String,
    pub panel_id: i64,
    pub user_id: i64,
    pub user_name: String,
    pub new_state: String,
    pub prev_state: String,
    pub time: i64,     // Unix ms
    pub time_end: i64, // Unix ms
    pub text: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    pub login: String,
    pub email: String,
    pub avatar_url: String,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

// ─── TimeRange / Dashboard ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TimeRange {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub raw: TimeRangeRaw,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TimeRangeRaw {
    pub from: String,
    pub to: String,
}

/// Full dashboard model (Grafana JSON format)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub uid: String,
    pub org_id: i64,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub style: String,
    pub schema_version: i32,
    pub version: i64,
    pub revision: i64,
    #[serde(default)]
    pub time: TimeRange,
    #[serde(default)]
    pub timepicker: serde_json::Value,
    #[serde(default)]
    pub refresh: String,
    pub panels: Vec<Panel>,
    #[serde(default)]
    pub templating: Templating,
    #[serde(default)]
    pub annotations: DashboardAnnotations,
    #[serde(default)]
    pub links: Vec<DashboardLink>,
    #[serde(default)]
    pub editable: bool,
    #[serde(default)]
    pub graph_tooltip: i32,
    #[serde(default)]
    pub live_now: bool,
    #[serde(default)]
    pub week_start: String,
    #[serde(default)]
    pub fiscal_year_start_month: i32,
    // Folder info (not in JSON body but returned by API)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_uid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_url: Option<String>,
    // Metadata
    pub slug: String,
    pub url: String,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub updated_by: String,
    #[serde(default)]
    pub is_starred: bool,
    #[serde(default)]
    pub is_snapshot: bool,
    #[serde(default)]
    pub expires: Option<DateTime<Utc>>,
    #[serde(default)]
    pub overwrite: bool,
}

impl Dashboard {
    pub fn slug_from_title(title: &str) -> String {
        title
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    }

    pub fn new(id: i64, org_id: i64, title: &str) -> Self {
        let uid = Uuid::new_v4().to_string().replace('-', "")[..12].to_string();
        let slug = Self::slug_from_title(title);
        let now = Utc::now();
        Self {
            id: Some(id),
            uid: uid.clone(),
            org_id,
            title: title.to_string(),
            description: String::new(),
            tags: vec![],
            style: "dark".into(),
            schema_version: 39,
            version: 1,
            revision: 1,
            time: TimeRange {
                from: "now-6h".into(),
                to: "now".into(),
                raw: TimeRangeRaw {
                    from: "now-6h".into(),
                    to: "now".into(),
                },
            },
            timepicker: serde_json::json!({}),
            refresh: "".into(),
            panels: vec![],
            templating: Templating::default(),
            annotations: DashboardAnnotations::default(),
            links: vec![],
            editable: true,
            graph_tooltip: 0,
            live_now: false,
            week_start: String::new(),
            fiscal_year_start_month: 0,
            folder_id: None,
            folder_uid: None,
            folder_title: None,
            folder_url: None,
            slug: slug.clone(),
            url: format!("/d/{uid}/{slug}"),
            created: now,
            updated: now,
            created_by: "admin".into(),
            updated_by: "admin".into(),
            is_starred: false,
            is_snapshot: false,
            expires: None,
            overwrite: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Templating {
    pub list: Vec<Variable>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DashboardAnnotations {
    pub list: Vec<AnnotationQuery>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DashboardLink {
    pub title: String,
    pub url: String,
    #[serde(rename = "type")]
    pub link_type: String, // "link" | "dashboards"
    pub icon: String,
    pub tooltip: String,
    pub tags: Vec<String>,
    pub target_blank: bool,
    pub include_vars: bool,
    pub keep_time: bool,
    pub as_dropdown: bool,
}

// ─── Dashboard Version (history) ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DashboardVersion {
    pub id: i64,
    pub dashboard_id: i64,
    pub parent_version: i64,
    pub restored_from: i64,
    pub version: i64,
    pub created: DateTime<Utc>,
    pub created_by: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ─── Snapshot ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub id: i64,
    pub uid: String,
    pub name: String,
    pub org_id: i64,
    pub key: String,
    pub delete_key: String,
    pub url: String,
    pub external: bool,
    pub external_url: String,
    pub expires: DateTime<Utc>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub dashboard: serde_json::Value,
}

// ─── Playlist ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: i64,
    pub uid: String,
    pub name: String,
    pub interval: String,
    pub org_id: i64,
    pub items: Vec<PlaylistItem>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistItem {
    pub id: i64,
    pub playlist_id: i64,
    #[serde(rename = "type")]
    pub item_type: String, // "dashboard_by_id" | "dashboard_by_tag" | "dashboard_by_uid"
    pub value: String,
    pub order: i32,
    pub title: String,
}

// ─── Unified Alerting ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "PascalCase")]
pub enum AlertState {
    #[default]
    Normal,
    Pending,
    Firing,
    Error,
    NoData,
    Inactive,
}

impl std::fmt::Display for AlertState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertState::Normal => write!(f, "Normal"),
            AlertState::Pending => write!(f, "Pending"),
            AlertState::Firing => write!(f, "Firing"),
            AlertState::Error => write!(f, "Error"),
            AlertState::NoData => write!(f, "NoData"),
            AlertState::Inactive => write!(f, "Inactive"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "PascalCase")]
pub enum NoDataState {
    #[default]
    NoData,
    Alerting,
    Ok,
    KeepState,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "PascalCase")]
pub enum ExecErrState {
    #[default]
    Alerting,
    Error,
    Ok,
    KeepState,
}

/// Grafana Unified Alerting rule (maps to /api/ruler/grafana/api/v1/rules)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlertRule {
    pub id: i64,
    pub uid: String,
    pub org_id: i64,
    pub folder_uid: String,
    pub rule_group: String,
    pub title: String,
    pub condition: String, // ref_id of the condition query
    pub data: Vec<AlertRuleQuery>,
    pub no_data_state: NoDataState,
    pub exec_err_state: ExecErrState,
    pub for_duration: String,
    pub annotations: HashMap<String, String>,
    pub labels: HashMap<String, String>,
    pub is_paused: bool,
    pub updated: DateTime<Utc>,
    pub created: DateTime<Utc>,
    #[serde(default)]
    pub state: AlertState,
    #[serde(default)]
    pub health: String,
    #[serde(default)]
    pub last_evaluation: Option<DateTime<Utc>>,
    #[serde(default)]
    pub evaluation_time: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlertRuleQuery {
    pub ref_id: String,
    pub query_type: String,
    pub relative_time_range: RelativeTimeRange,
    pub datasource_uid: String,
    pub model: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RelativeTimeRange {
    pub from: i64, // seconds
    pub to: i64,   // seconds
}

/// Rule group
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuleGroup {
    pub name: String,
    pub folder_uid: String,
    pub interval: i64, // evaluation interval in seconds
    pub rules: Vec<AlertRule>,
}

// ─── Contact Points ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContactPointType {
    Webhook,
    Email,
    Slack,
    PagerDuty,
    Opsgenie,
    Telegram,
    Teams,
    VictorOps,
    Alertmanager,
    Hipchat,
    Line,
    Kafka,
    GoogleHangoutsChat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContactPoint {
    pub uid: String,
    pub name: String,
    #[serde(rename = "type")]
    pub cp_type: ContactPointType,
    pub settings: serde_json::Value,
    pub disable_resolve_message: bool,
    pub send_reminder: bool,
    pub frequency: String,
}

// ─── Notification Policy ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPolicy {
    pub receiver: String,
    #[serde(default)]
    pub group_by: Vec<String>,
    #[serde(default)]
    pub continue_policy: bool,
    #[serde(default)]
    pub matchers: Vec<Matcher>,
    #[serde(default)]
    pub group_wait: Option<String>,
    #[serde(default)]
    pub group_interval: Option<String>,
    #[serde(default)]
    pub repeat_interval: Option<String>,
    #[serde(default)]
    pub mute_time_intervals: Vec<String>,
    #[serde(default)]
    pub routes: Vec<NotificationPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Matcher {
    pub name: String,
    pub value: String,
    #[serde(rename = "isEqual")]
    pub is_equal: bool,
    #[serde(rename = "isRegex")]
    pub is_regex: bool,
}

// ─── Silence ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Silence {
    pub id: String,
    pub status: SilenceStatus,
    pub updated_at: DateTime<Utc>,
    pub comment: String,
    pub created_by: String,
    pub ends_at: DateTime<Utc>,
    pub starts_at: DateTime<Utc>,
    pub matchers: Vec<SilenceMatcher>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SilenceStatus {
    pub state: String, // "active" | "pending" | "expired"
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SilenceMatcher {
    pub is_equal: bool,
    pub is_regex: bool,
    pub name: String,
    pub value: String,
}

// ─── Mute Timing ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MuteTiming {
    pub name: String,
    pub time_intervals: Vec<TimeInterval>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TimeInterval {
    #[serde(default)]
    pub times: Vec<TimeIntervalRange>,
    #[serde(default)]
    pub days_of_month: Vec<String>,
    #[serde(default)]
    pub months: Vec<String>,
    #[serde(default)]
    pub weekdays: Vec<String>,
    #[serde(default)]
    pub years: Vec<String>,
    #[serde(default)]
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TimeIntervalRange {
    pub start_minute: i32,
    pub end_minute: i32,
}

// ─── Alert Group ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlertGroup {
    pub labels: HashMap<String, String>,
    pub receiver: AlertReceiver,
    pub alerts: Vec<AlertInstance>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AlertReceiver {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlertInstance {
    pub state: AlertState,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub value: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: Option<DateTime<Utc>>,
    pub generator_url: String,
    pub fingerprint: String,
    pub silence_urls: Vec<String>,
    pub dashboard_url: Option<String>,
    pub panel_url: Option<String>,
    pub values: Option<HashMap<String, f64>>,
    pub evaluations: Option<AlertEvaluations>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AlertEvaluations {
    pub avg_interval: f64,
    pub count: i64,
    pub last_evaluation_time: Option<DateTime<Utc>>,
    pub evaluated_at: Option<DateTime<Utc>>,
}

// ─── Legacy Alerting ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlertCondition {
    #[serde(rename = "type")]
    pub condition_type: String, // "query"
    pub query: AlertConditionQuery,
    pub reducer: AlertReducer,
    pub evaluator: AlertEvaluator,
    pub operator: AlertOperator,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AlertConditionQuery {
    pub params: Vec<String>, // [ref_id, "5m", "now"]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AlertReducer {
    #[serde(rename = "type")]
    pub reducer_type: String, // "avg" | "min" | "max" | "sum" | "count" | "last"
    pub params: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AlertEvaluator {
    #[serde(rename = "type")]
    pub eval_type: String, // "gt" | "lt" | "eq" | "gte" | "lte" | "within_range" | "outside_range" | "no_value"
    pub params: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AlertOperator {
    #[serde(rename = "type")]
    pub op_type: String, // "and" | "or"
}

/// Legacy notification channel
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlertNotificationChannel {
    pub id: i64,
    pub uid: String,
    pub org_id: i64,
    pub name: String,
    #[serde(rename = "type")]
    pub channel_type: String,
    pub is_default: bool,
    pub send_reminder: bool,
    pub disable_resolve_message: bool,
    pub frequency: String,
    pub settings: serde_json::Value,
    pub secure_settings: HashMap<String, String>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

// ─── API Request/Response types ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertDashboardRequest {
    pub dashboard: serde_json::Value,
    #[serde(default)]
    pub folder_id: Option<i64>,
    #[serde(default)]
    pub folder_uid: Option<String>,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertDashboardResponse {
    pub id: i64,
    pub uid: String,
    pub url: String,
    pub status: String,
    pub version: i64,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub query: Option<String>,
    pub tag: Vec<String>,
    #[serde(rename = "type")]
    pub result_type: Option<String>, // "dash-db" | "dash-folder"
    pub dashboard_ids: Vec<i64>,
    pub dashboard_uids: Vec<String>,
    pub folder_ids: Vec<i64>,
    pub folder_uids: Vec<String>,
    pub starred: Option<bool>,
    pub limit: Option<i64>,
    pub page: Option<i64>,
    pub sort: Option<String>,
    pub org_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub id: i64,
    pub uid: String,
    pub title: String,
    pub uri: String,
    pub url: String,
    pub slug: String,
    pub r#type: String,
    pub tags: Vec<String>,
    pub is_starred: bool,
    pub folder_id: Option<i64>,
    pub folder_uid: Option<String>,
    pub folder_title: Option<String>,
    pub folder_url: Option<String>,
    pub sort_meta: i64,
    pub sort_meta_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderRequest {
    pub uid: Option<String>,
    pub title: String,
    #[serde(default)]
    pub parent_uid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDataSourceRequest {
    pub name: String,
    #[serde(rename = "type")]
    pub ds_type: DataSourceType,
    pub url: String,
    #[serde(default)]
    pub access: DataSourceAccess,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub json_data: serde_json::Value,
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(default)]
    pub basic_auth: bool,
    #[serde(default)]
    pub basic_auth_user: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub database: String,
    #[serde(default)]
    pub org_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSnapshotRequest {
    pub dashboard: serde_json::Value,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub expires: Option<i64>,
    #[serde(default)]
    pub external: bool,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub delete_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAnnotationRequest {
    #[serde(default)]
    pub dashboard_uid: Option<String>,
    #[serde(default)]
    pub panel_id: Option<i64>,
    pub time: i64,
    #[serde(default)]
    pub time_end: Option<i64>,
    pub tags: Vec<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateOrgRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserRequest {
    pub name: String,
    pub email: String,
    pub login: String,
    pub password: String,
    #[serde(default)]
    pub org_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub role: OrgRole,
    #[serde(default)]
    pub seconds_to_live: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DsQueryRequest {
    pub queries: Vec<DsQuery>,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DsQuery {
    pub ref_id: String,
    pub datasource: DatasourceRef,
    #[serde(flatten)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DsQueryResponse {
    pub results: HashMap<String, QueryResult>,
}

// ─── Query / DataFrame ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub frames: Vec<DataFrame>,
    pub status: i32,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DataFrame {
    pub schema: DataFrameSchema,
    pub data: DataFrameData,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DataFrameSchema {
    pub ref_id: String,
    pub name: String,
    pub fields: Vec<FieldSchema>,
    #[serde(default)]
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FieldSchema {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String, // "time" | "number" | "string" | "boolean"
    #[serde(default)]
    pub type_info: Option<serde_json::Value>,
    #[serde(default)]
    pub labels: Option<HashMap<String, String>>,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DataFrameData {
    pub values: Vec<Vec<serde_json::Value>>,
    #[serde(default)]
    pub entities: Option<serde_json::Value>,
}
