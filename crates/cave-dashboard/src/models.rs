//! Data models for CAVE Dashboard.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Time ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub from: String,
    pub to: String,
}

impl Default for TimeRange {
    fn default() -> Self {
        Self { from: "now-6h".to_string(), to: "now".to_string() }
    }
}

// ─── Panel ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PanelType {
    Graph,
    Stat,
    Gauge,
    Table,
    BarChart,
    PieChart,
    Heatmap,
    Logs,
    AlertList,
}

impl Default for PanelType {
    fn default() -> Self { Self::Graph }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GridPos {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub ref_id: String,
    pub expr: String,
    pub datasource_uid: Option<String>,
    pub legend_format: Option<String>,
    pub interval: Option<String>,
}

impl Default for Target {
    fn default() -> Self {
        Self {
            ref_id: "A".to_string(),
            expr: String::new(),
            datasource_uid: None,
            legend_format: None,
            interval: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Panel {
    pub id: u32,
    pub title: String,
    pub panel_type: PanelType,
    pub datasource_uid: Option<String>,
    pub targets: Vec<Target>,
    pub description: Option<String>,
    pub alert: Option<AlertRule>,
    pub options: serde_json::Value,
    pub grid_pos: GridPos,
}

impl Panel {
    pub fn new(id: u32, title: impl Into<String>, panel_type: PanelType) -> Self {
        Self {
            id,
            title: title.into(),
            panel_type,
            datasource_uid: None,
            targets: vec![],
            description: None,
            alert: None,
            options: serde_json::Value::Object(Default::default()),
            grid_pos: GridPos::default(),
        }
    }
}

// ─── Row ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    pub id: u32,
    pub title: String,
    pub collapsed: bool,
    pub panels: Vec<Panel>,
    pub repeat: Option<String>,
}

// ─── Variables ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VariableType {
    Query,
    Custom,
    Interval,
    Textbox,
    Datasource,
    Constant,
    AdHoc,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VariableRefresh {
    #[default]
    Never,
    OnDashboardLoad,
    OnTimeRangeChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VariableHide {
    #[default]
    DontHide,
    HideLabel,
    HideVariable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableOption {
    pub text: String,
    pub value: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    pub label: Option<String>,
    pub var_type: VariableType,
    pub query: Option<String>,
    pub options: Vec<VariableOption>,
    pub current: Option<VariableOption>,
    pub multi: bool,
    pub include_all: bool,
    pub refresh: VariableRefresh,
    pub hide: VariableHide,
    pub description: Option<String>,
}

impl Variable {
    pub fn current_value(&self) -> Option<&str> {
        self.current.as_ref().map(|o| o.value.as_str())
    }
}

// ─── Annotations ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationType {
    #[default]
    Manual,
    Query,
}

/// Panel-level annotation query definition (embedded in dashboard).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationQuery {
    pub name: String,
    pub datasource_uid: String,
    pub enable: bool,
    pub color: String,
    pub query: Option<String>,
    pub query_type: AnnotationType,
}

/// A stored annotation event (time-series mark).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub id: u64,
    pub dashboard_uid: String,
    pub panel_id: Option<u32>,
    pub time: DateTime<Utc>,
    pub time_end: Option<DateTime<Utc>>,
    pub tags: Vec<String>,
    pub text: String,
    pub annotation_type: AnnotationType,
}

// ─── Alerting ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    #[default]
    Ok,
    Alerting,
    NoData,
    Pending,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NoDataState {
    #[default]
    NoData,
    Alerting,
    Ok,
    KeepState,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecErrState {
    #[default]
    Alerting,
    KeepState,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvaluator {
    /// "gt", "lt", "eq", "within_range", "outside_range", "no_value"
    pub evaluator_type: String,
    pub params: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlertReducer {
    /// "avg", "min", "max", "sum", "count", "last"
    pub reducer_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertQuery {
    /// [refId, from, to] e.g. ["A", "5m", "now"]
    pub params: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlertOperator {
    /// "and" or "or"
    pub op_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertCondition {
    pub ref_id: String,
    pub evaluator: AlertEvaluator,
    pub operator: AlertOperator,
    pub reducer: AlertReducer,
    pub query: AlertQuery,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlertRule {
    pub id: u64,
    pub name: String,
    pub message: String,
    pub frequency: String,
    pub for_duration: String,
    pub conditions: Vec<AlertCondition>,
    /// Notification channel UIDs.
    pub notifications: Vec<String>,
    pub state: AlertState,
    pub no_data_state: NoDataState,
    pub exec_err_state: ExecErrState,
}

// ─── Alert Notification Channels ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationChannelType {
    Webhook,
    Email,
    Slack,
    PagerDuty,
    Opsgenie,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NotificationSettings {
    pub url: Option<String>,
    pub addresses: Option<String>,
    pub token: Option<String>,
    pub channel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertNotificationChannel {
    pub id: u32,
    pub uid: String,
    pub name: String,
    pub channel_type: NotificationChannelType,
    pub settings: NotificationSettings,
    pub is_default: bool,
    pub send_reminder: bool,
    pub frequency: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─── DataSource ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceType {
    Prometheus,
    Loki,
    Jaeger,
    Unknown,
}

impl Default for DataSourceType {
    fn default() -> Self { Self::Prometheus }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceAccess {
    #[default]
    Proxy,
    Direct,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSource {
    pub id: u32,
    pub uid: String,
    pub name: String,
    pub datasource_type: DataSourceType,
    pub url: String,
    pub access: DataSourceAccess,
    pub is_default: bool,
    pub basic_auth: bool,
    pub json_data: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─── Folder ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: u32,
    pub uid: String,
    pub title: String,
    pub url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─── Dashboard ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dashboard {
    pub id: u32,
    pub uid: String,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub time: TimeRange,
    pub refresh: String,
    pub schema_version: u32,
    pub version: u32,
    pub panels: Vec<Panel>,
    pub rows: Vec<Row>,
    pub variables: Vec<Variable>,
    pub annotation_queries: Vec<AnnotationQuery>,
    pub folder_uid: Option<String>,
    pub is_starred: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: String,
    pub updated_by: String,
}

impl Dashboard {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: 0,
            uid: Uuid::new_v4().to_string(),
            title: title.into(),
            description: String::new(),
            tags: vec![],
            time: TimeRange::default(),
            refresh: String::new(),
            schema_version: 38,
            version: 1,
            panels: vec![],
            rows: vec![],
            variables: vec![],
            annotation_queries: vec![],
            folder_uid: None,
            is_starred: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: String::new(),
            updated_by: String::new(),
        }
    }

    pub fn slug(&self) -> String {
        self.title.to_lowercase().replace(' ', "-")
    }

    pub fn url(&self) -> String {
        format!("/d/{}/{}", self.uid, self.slug())
    }
}

// ─── Snapshot ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: u32,
    pub key: String,
    pub delete_key: String,
    pub name: String,
    pub dashboard: serde_json::Value,
    pub expires: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub external: bool,
    pub external_url: Option<String>,
}

impl Snapshot {
    pub fn is_expired(&self) -> bool {
        if let Some(exp) = self.expires {
            Utc::now() > exp
        } else {
            false
        }
    }
}

// ─── Playlist ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistItemType {
    DashboardByTag,
    DashboardById,
    DashboardByUid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistItem {
    pub item_type: PlaylistItemType,
    pub value: String,
    pub order: u32,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub interval: String,
    pub items: Vec<PlaylistItem>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─── Permissions ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLevel {
    View,
    Edit,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardPermission {
    pub dashboard_uid: Option<String>,
    pub folder_uid: Option<String>,
    pub user_id: Option<u32>,
    pub team_id: Option<u32>,
    pub role: Option<String>,
    pub permission: PermissionLevel,
}

// ─── API Request/Response Types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpsertDashboardRequest {
    pub dashboard: serde_json::Value,
    pub folder_uid: Option<String>,
    pub overwrite: Option<bool>,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UpsertDashboardResponse {
    pub id: u32,
    pub uid: String,
    pub url: String,
    pub status: String,
    pub version: u32,
    pub slug: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct SearchQuery {
    pub query: Option<String>,
    pub tag: Option<String>,
    #[serde(rename = "type")]
    pub item_type: Option<String>,
    pub folder_uid: Option<String>,
    pub starred: Option<bool>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub id: u32,
    pub uid: String,
    pub title: String,
    pub url: String,
    pub item_type: String,
    pub tags: Vec<String>,
    pub is_starred: bool,
    pub folder_uid: Option<String>,
    pub folder_title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFolderRequest {
    pub uid: Option<String>,
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFolderRequest {
    pub title: String,
    pub version: Option<u32>,
    pub overwrite: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDataSourceRequest {
    pub name: String,
    pub datasource_type: DataSourceType,
    pub url: String,
    pub access: Option<DataSourceAccess>,
    pub is_default: Option<bool>,
    pub json_data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAlertChannelRequest {
    pub name: String,
    pub channel_type: NotificationChannelType,
    pub settings: Option<NotificationSettings>,
    pub is_default: Option<bool>,
    pub frequency: Option<String>,
    pub send_reminder: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSnapshotRequest {
    pub dashboard: serde_json::Value,
    pub name: Option<String>,
    pub expires: Option<i64>,
    pub external: Option<bool>,
    pub key: Option<String>,
    pub delete_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateSnapshotResponse {
    pub key: String,
    pub delete_key: String,
    pub url: String,
    pub delete_url: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePlaylistRequest {
    pub name: String,
    pub interval: String,
    pub items: Vec<PlaylistItem>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAnnotationRequest {
    pub dashboard_uid: String,
    pub panel_id: Option<u32>,
    pub time: Option<DateTime<Utc>>,
    pub time_end: Option<DateTime<Utc>>,
    pub tags: Option<Vec<String>>,
    pub text: String,
}
