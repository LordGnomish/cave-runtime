use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use serde_json::json;

mod client;
use client::ApiClient;

// ── Root CLI ──────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "cavectl",
    about = "CAVE Runtime CLI — single binary, native + compatibility surfaces (ADR-RUNTIME-CLI-CONSOLIDATION-001)",
    version,
    propagate_version = true
)]
struct Cli {
    /// Runtime API server URL
    #[arg(long, global = true, default_value = "http://localhost:3000", env = "CAVE_SERVER")]
    server: String,

    /// Output format
    #[arg(long, global = true, default_value = "table", value_enum)]
    format: Format,

    /// Bearer authentication token
    #[arg(long, global = true, env = "CAVE_TOKEN")]
    token: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum Format {
    /// ASCII table (default)
    Table,
    /// Pretty-printed JSON
    Json,
    /// YAML
    Yaml,
}

// ── Top-level subcommands ─────────────────────────────────────────────────────

#[derive(Subcommand)]
enum Commands {
    /// Feature flag management (Unleash replacement)
    Flags {
        #[command(subcommand)]
        cmd: FlagsCmd,
    },
    /// Secret detection and management (TruffleHog + Gitleaks replacement)
    Secrets {
        #[command(subcommand)]
        cmd: SecretsCmd,
    },
    /// Code scanning (SonarQube replacement)
    Scan {
        #[command(subcommand)]
        cmd: ScanCmd,
    },
    /// Vulnerability management (DefectDojo replacement)
    Vulns {
        #[command(subcommand)]
        cmd: VulnsCmd,
    },
    /// SBOM & dependency tracking (DependencyTrack replacement)
    Sbom {
        #[command(subcommand)]
        cmd: SbomCmd,
    },
    /// Container / package registry (Pulp replacement)
    Registry {
        #[command(subcommand)]
        cmd: RegistryCmd,
    },
    /// API gateway management
    Gateway {
        #[command(subcommand)]
        cmd: GatewayCmd,
    },
    /// PostgreSQL management
    Pg {
        #[command(subcommand)]
        cmd: PgCmd,
    },
    /// Document database (MongoDB-compatible) management
    Docdb {
        #[command(subcommand)]
        cmd: DocdbCmd,
    },
    /// Cache (Redis/Valkey-compatible) management
    Cache {
        #[command(subcommand)]
        cmd: CacheCmd,
    },
    /// Lakehouse (Iceberg + DataFusion) management
    Lakehouse {
        #[command(subcommand)]
        cmd: LakehouseCmd,
    },
    /// Kafka management (legacy alias of `streams kafka`)
    Kafka {
        #[command(subcommand)]
        cmd: KafkaCmd,
    },
    /// Streaming platform — Kafka + Pulsar parity (cave-streams)
    Streams {
        #[command(subcommand)]
        cmd: StreamsCmd,
    },
    /// Infrastructure as code (Terraform/Pulumi replacement)
    Infra {
        #[command(subcommand)]
        cmd: InfraCmd,
    },
    /// Alerting (Alertmanager replacement)
    Alerts {
        #[command(subcommand)]
        cmd: AlertsCmd,
    },
    /// Incident management (Grafana OnCall replacement)
    Incidents {
        #[command(subcommand)]
        cmd: IncidentsCmd,
    },
    /// SLO tracking
    Slo {
        #[command(subcommand)]
        cmd: SloCmd,
    },
    /// Synthetic uptime monitoring (Uptime Kuma replacement)
    Uptime {
        #[command(subcommand)]
        cmd: UptimeCmd,
    },
    /// FinOps cost management (OpenCost replacement)
    Cost {
        #[command(subcommand)]
        cmd: CostCmd,
    },
    /// Team chat (LibreChat replacement)
    Chat {
        #[command(subcommand)]
        cmd: ChatCmd,
    },
    /// Workflow automation (n8n replacement)
    Workflows {
        #[command(subcommand)]
        cmd: WorkflowsCmd,
    },
    /// Chaos engineering (Chaos Mesh replacement)
    Chaos {
        #[command(subcommand)]
        cmd: ChaosCmd,
    },
    /// Policy engine (OPA + OPAL replacement)
    Policy {
        #[command(subcommand)]
        cmd: PolicyCmd,
    },
    /// Dynamic security testing (OWASP ZAP replacement)
    Dast {
        #[command(subcommand)]
        cmd: DastCmd,
    },
    /// Privileged access management (Teleport CE replacement)
    Pam {
        #[command(subcommand)]
        cmd: PamCmd,
    },
    /// PII detection and redaction (Presidio replacement)
    Pii {
        #[command(subcommand)]
        cmd: PiiCmd,
    },
    /// Backup and restore (Velero replacement)
    Backup {
        #[command(subcommand)]
        cmd: BackupCmd,
    },
    /// Digital forensics (Tetragon consumer)
    Forensics {
        #[command(subcommand)]
        cmd: ForensicsCmd,
    },
    /// Performance profiling (Pyroscope replacement)
    Profiler {
        #[command(subcommand)]
        cmd: ProfilerCmd,
    },
    /// Dev metrics (DevLake integration)
    Devlake {
        #[command(subcommand)]
        cmd: DevlakeCmd,
    },
    /// AI observability (Langfuse replacement)
    #[command(name = "ai-obs")]
    AiObs {
        #[command(subcommand)]
        cmd: AiObsCmd,
    },
    /// Developer portal info (Backstage replacement)
    Portal {
        #[command(subcommand)]
        cmd: PortalCmd,
    },
    /// Project scaffolding (Backstage scaffolder replacement)
    Scaffold {
        #[command(subcommand)]
        cmd: ScaffoldCmd,
    },
    /// Artifact signing (Sigstore replacement)
    Sign {
        #[command(subcommand)]
        cmd: SignCmd,
    },
    /// Overall system status
    Status {
        #[command(subcommand)]
        cmd: StatusCmd,
    },
    /// Upstream parity tracking
    Parity {
        #[command(subcommand)]
        cmd: ParityCmd,
    },
    /// Local LLM draft-generation daemon (Ollama / Qwen 2.5 Coder)
    #[command(name = "local-llm")]
    LocalLlm {
        #[command(subcommand)]
        cmd: LocalLlmCmd,
    },
    /// Distributed key-value store (etcd v3 replacement; etcdctl parity)
    Etcd {
        #[command(subcommand)]
        cmd: EtcdCmd,
    },
    /// Container Runtime Interface (containerd/crun replacement; crictl parity)
    Cri {
        #[command(subcommand)]
        cmd: CriCmd,
    },
    /// Kubernetes API server (kube-apiserver replacement; kubectl parity)
    Apiserver {
        #[command(subcommand)]
        cmd: ApiserverCmd,
    },
    /// Node agent (kubelet replacement)
    Kubelet {
        #[command(subcommand)]
        cmd: KubeletCmd,
    },
    /// Pod scheduler (kube-scheduler replacement)
    Scheduler {
        #[command(subcommand)]
        cmd: SchedulerCmd,
    },
    /// Pod networking & service routing (kube-proxy/CNI replacement)
    Net {
        #[command(subcommand)]
        cmd: NetCmd,
    },
    /// Workload controllers (kube-controller-manager replacement)
    #[command(name = "controller-manager")]
    ControllerManager {
        #[command(subcommand)]
        cmd: ControllerManagerCmd,
    },
    /// Cloud-provider controllers (cloud-controller-manager replacement)
    #[command(name = "cloud-controller-manager")]
    CloudControllerManager {
        #[command(subcommand)]
        cmd: CloudControllerManagerCmd,
    },
}

// ── Per-module subcommand enums ───────────────────────────────────────────────

#[derive(Subcommand)]
enum FlagsCmd {
    /// List all feature flags
    List,
    /// Create a new feature flag
    Create {
        /// Flag display name
        #[arg(long)]
        name: String,
        /// Flag key (slug)
        #[arg(long)]
        key: String,
        /// Enable the flag immediately
        #[arg(long, default_value = "false")]
        enabled: bool,
    },
    /// Toggle a feature flag on/off
    Toggle {
        /// Flag key
        key: String,
    },
    /// Delete a feature flag
    Delete {
        /// Flag key
        key: String,
    },
}

#[derive(Subcommand)]
enum SecretsCmd {
    /// List registered secrets
    List,
    /// Scan content for leaked secrets
    Scan {
        /// Inline content to scan
        #[arg(long, conflicts_with = "file")]
        content: Option<String>,
        /// Path to file to scan
        #[arg(long)]
        file: Option<String>,
    },
    /// Register a new secret
    Add {
        /// Secret name
        #[arg(long)]
        name: String,
        /// Secret value
        #[arg(long)]
        value: String,
        /// Secret type (e.g. api_key, db_password, token)
        #[arg(long, default_value = "generic")]
        kind: String,
    },
    /// Rotate a secret
    Rotate {
        /// Secret name
        name: String,
    },
}

#[derive(Subcommand)]
enum ScanCmd {
    /// Start a new code scan
    Start {
        /// Repository URL or local path
        #[arg(long)]
        repo: String,
        /// Branch to scan
        #[arg(long, default_value = "main")]
        branch: String,
    },
    /// List code scan jobs
    List,
    /// Get scan results
    Results {
        /// Scan job ID
        id: String,
    },
}

#[derive(Subcommand)]
enum VulnsCmd {
    /// Trigger a vulnerability scan
    Scan {
        /// Target (image, repo, or path)
        #[arg(long)]
        target: String,
    },
    /// List vulnerabilities
    List,
    /// Get vulnerability detail
    Detail {
        /// Vulnerability ID
        id: String,
    },
}

#[derive(Subcommand)]
enum SbomCmd {
    /// Generate an SBOM for a project
    Generate {
        /// Project name or path
        #[arg(long)]
        project: String,
        /// Version / tag
        #[arg(long)]
        version: Option<String>,
    },
    /// List SBOMs
    List,
    /// Get SBOM detail
    Detail {
        /// SBOM ID
        id: String,
    },
}

#[derive(Subcommand)]
enum RegistryCmd {
    /// List images / packages
    List,
    /// Push an image or package
    Push {
        /// Image reference (e.g. myimage:latest)
        #[arg(long)]
        image: String,
    },
    /// Pull an image or package
    Pull {
        /// Image reference
        #[arg(long)]
        image: String,
    },
    /// Run garbage collection
    Gc,
}

#[derive(Subcommand)]
enum GatewayCmd {
    /// List configured routes
    Routes,
    /// List upstream services
    Services,
    /// List installed plugins
    Plugins,
    /// Show gateway traffic stats
    Stats,
    /// Gravitee API / plan / application / subscription management
    Gravitee {
        #[command(subcommand)]
        cmd: GraviteeCmd,
    },
}

#[derive(Subcommand)]
enum GraviteeCmd {
    /// List Gravitee APIs
    Apis,
    /// List Gravitee plans
    Plans,
    /// List Gravitee applications
    Applications,
    /// List Gravitee subscriptions
    Subscriptions,
    /// List Portal-visible (Public + Published) APIs
    Portal,
}

#[derive(Subcommand)]
enum PgCmd {
    /// List databases
    Databases,
    /// List pending and applied migrations
    Migrations,
    /// Show connection pool stats
    Pools,
    /// Show slow and active queries
    Queries,
    /// Trigger a database backup
    Backup {
        /// Database name
        #[arg(long)]
        database: Option<String>,
    },
}

#[derive(Subcommand)]
enum DocdbCmd {
    /// Health check (mongosh `db.adminCommand({ping:1})` parity)
    Health,
    /// List databases (mongosh `show dbs` parity)
    Databases,
    /// List collections in a database (mongosh `show collections` parity)
    Collections {
        /// Database name
        #[arg(long)]
        db: String,
    },
    /// Show engine-wide statistics (mongosh `db.serverStatus()` subset)
    Stats,
    /// Show collection statistics (mongosh `db.<col>.stats()`)
    CollStats {
        #[arg(long)]
        db: String,
        #[arg(long)]
        collection: String,
    },
    /// Run a find query (mongosh `db.<col>.find(filter)`)
    Find {
        #[arg(long)]
        db: String,
        #[arg(long)]
        collection: String,
        /// JSON filter document
        #[arg(long, default_value = "{}")]
        filter: String,
    },
    /// List indexes on a collection (mongosh `db.<col>.getIndexes()`)
    Indexes {
        #[arg(long)]
        db: String,
        #[arg(long)]
        collection: String,
    },
    /// Show wire-protocol server info (mongosh `db.runCommand({hello:1})`)
    Info,
}

#[derive(Subcommand)]
enum CacheCmd {
    /// Health check (redis-cli PING parity)
    Health,
    /// List keys matching a pattern (redis-cli KEYS pattern)
    Keys {
        /// Glob pattern (default `*`)
        #[arg(long, default_value = "*")]
        pattern: String,
    },
    /// Get a value (redis-cli GET key)
    Get {
        /// Key
        key: String,
    },
    /// Set a value (redis-cli SET key value [EX seconds])
    Set {
        /// Key
        key: String,
        /// Value (JSON-encoded)
        value: String,
        /// Optional TTL in seconds
        #[arg(long)]
        ttl: Option<u64>,
    },
    /// Delete a key (redis-cli DEL key)
    Del {
        /// Key
        key: String,
    },
    /// Show server stats (redis-cli INFO subset)
    Stats,
    /// List active pub/sub channels (redis-cli PUBSUB CHANNELS)
    Pubsub,
    /// Publish a message (redis-cli PUBLISH channel message)
    Publish {
        /// Channel name
        channel: String,
        /// Message body
        message: String,
    },
}

#[derive(Subcommand)]
enum LakehouseCmd {
    /// Health check (Iceberg REST `GET /v1/config` + DataFusion ping parity)
    Health,
    /// List catalogs (Iceberg `GET /v1/catalogs`)
    Catalogs,
    /// List namespaces in a catalog (Iceberg `GET /v1/{prefix}/namespaces`)
    Namespaces {
        #[arg(long)]
        catalog: String,
    },
    /// List tables in a namespace
    Tables {
        #[arg(long)]
        catalog: String,
        #[arg(long)]
        namespace: String,
    },
    /// Show table metadata (current schema, partition spec, current snapshot)
    Describe {
        #[arg(long)]
        catalog: String,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        table: String,
    },
    /// List snapshots (time-travel candidates)
    Snapshots {
        #[arg(long)]
        catalog: String,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        table: String,
    },
    /// Run a DataFusion SQL query
    Query {
        /// SQL string; use single quotes around the whole arg
        sql: String,
        /// Logical-plan-only — do not execute
        #[arg(long)]
        explain: bool,
    },
    /// Trigger compaction (small-file consolidation)
    Compact {
        #[arg(long)]
        catalog: String,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        table: String,
    },
    /// Expire snapshots older than `--older-than-days` (Iceberg `expire_snapshots`)
    ExpireSnapshots {
        #[arg(long)]
        catalog: String,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        table: String,
        #[arg(long, default_value = "7")]
        older_than_days: u32,
    },
}

#[derive(Subcommand)]
enum KafkaCmd {
    /// List topics
    Topics,
    /// List consumer groups
    Consumers,
    /// List registered schemas
    Schemas,
    /// List connectors
    Connectors,
}

#[derive(Subcommand)]
enum StreamsCmd {
    /// Combined liveness/health summary (Kafka + Pulsar)
    Health,
    /// Kafka subset — topics, consumer groups, schemas, connectors
    Kafka {
        #[command(subcommand)]
        cmd: KafkaCmd,
    },
    /// Pulsar subset — tenants, namespaces, topics, subscriptions
    Pulsar {
        #[command(subcommand)]
        cmd: PulsarCmd,
    },
}

#[derive(Subcommand)]
enum PulsarCmd {
    /// List tenants (pulsar-admin tenants list)
    Tenants,
    /// Create a tenant
    CreateTenant {
        /// Tenant name
        name: String,
    },
    /// Delete a tenant (cascades to namespaces and topics)
    DeleteTenant {
        /// Tenant name
        name: String,
    },
    /// List namespaces in a tenant
    Namespaces {
        /// Tenant name
        tenant: String,
    },
    /// Create a namespace
    CreateNamespace {
        /// Tenant name
        tenant: String,
        /// Namespace name
        namespace: String,
    },
    /// Delete a namespace
    DeleteNamespace {
        /// Tenant name
        tenant: String,
        /// Namespace name
        namespace: String,
    },
    /// Set retention on a namespace
    SetRetention {
        /// Tenant name
        tenant: String,
        /// Namespace name
        namespace: String,
        /// Retention in minutes
        #[arg(long)]
        minutes: u64,
        /// Retention size limit in MiB
        #[arg(long)]
        size_mb: u64,
    },
    /// List topics in a namespace
    Topics {
        /// Tenant name
        tenant: String,
        /// Namespace name
        namespace: String,
    },
    /// Create a topic
    CreateTopic {
        /// Tenant name
        tenant: String,
        /// Namespace name
        namespace: String,
        /// Topic name
        topic: String,
        /// "persistent" (default) or "non-persistent"
        #[arg(long)]
        domain: Option<String>,
        /// Number of partitions (0 = non-partitioned, default)
        #[arg(long)]
        partitions: Option<u32>,
    },
    /// Delete a topic
    DeleteTopic {
        /// Tenant name
        tenant: String,
        /// Namespace name
        namespace: String,
        /// Topic name
        topic: String,
    },
    /// Show topic statistics
    TopicStats {
        /// Tenant name
        tenant: String,
        /// Namespace name
        namespace: String,
        /// Topic name
        topic: String,
    },
    /// List subscriptions on a topic
    Subscriptions {
        /// Tenant name
        tenant: String,
        /// Namespace name
        namespace: String,
        /// Topic name
        topic: String,
    },
    /// Create a subscription
    CreateSubscription {
        /// Tenant name
        tenant: String,
        /// Namespace name
        namespace: String,
        /// Topic name
        topic: String,
        /// Subscription name
        subscription: String,
        /// Subscription type: Exclusive | Shared | Failover | KeyShared
        #[arg(long, default_value = "Exclusive")]
        sub_type: String,
        /// Initial position: earliest | latest
        #[arg(long, default_value = "earliest")]
        initial_position: String,
    },
    /// Delete a subscription
    DeleteSubscription {
        /// Tenant name
        tenant: String,
        /// Namespace name
        namespace: String,
        /// Topic name
        topic: String,
        /// Subscription name
        subscription: String,
    },
    /// Skip all backlog on a subscription
    SkipAll {
        /// Tenant name
        tenant: String,
        /// Namespace name
        namespace: String,
        /// Topic name
        topic: String,
        /// Subscription name
        subscription: String,
    },
    /// Reset the subscription cursor
    ResetCursor {
        /// Tenant name
        tenant: String,
        /// Namespace name
        namespace: String,
        /// Topic name
        topic: String,
        /// Subscription name
        subscription: String,
        /// Position: "earliest", "latest", or numeric offset
        #[arg(long, default_value = "earliest")]
        position: String,
    },
}

#[derive(Subcommand)]
enum InfraCmd {
    /// Show desired infrastructure intent
    Intent,
    /// Preview planned infrastructure changes
    Plan {
        /// Stack name
        #[arg(long)]
        stack: Option<String>,
    },
    /// Apply infrastructure changes
    Apply {
        /// Stack name
        #[arg(long)]
        stack: Option<String>,
    },
    /// Destroy infrastructure
    Destroy {
        /// Stack name
        #[arg(long)]
        stack: Option<String>,
    },
    /// Show current infrastructure state
    State {
        /// Stack name
        #[arg(long)]
        stack: Option<String>,
    },
    /// Detect infrastructure drift
    Drift,
}

#[derive(Subcommand)]
enum AlertsCmd {
    /// List alert rules
    List,
    /// Create an alert rule
    Create {
        /// Alert name
        #[arg(long)]
        name: String,
        /// PromQL expression
        #[arg(long)]
        expr: String,
        /// Severity (critical, warning, info)
        #[arg(long, default_value = "warning")]
        severity: String,
    },
    /// Silence an alert
    Silence {
        /// Alert ID
        id: String,
        /// Silence duration (e.g. 2h, 30m)
        #[arg(long, default_value = "1h")]
        duration: String,
    },
    /// List alert routes / receivers
    Routes,
}

#[derive(Subcommand)]
enum IncidentsCmd {
    /// List incidents
    List,
    /// Create an incident
    Create {
        /// Incident title
        #[arg(long)]
        title: String,
        /// Severity (p1, p2, p3, p4)
        #[arg(long, default_value = "p3")]
        severity: String,
    },
    /// Acknowledge an incident
    Ack {
        /// Incident ID
        id: String,
    },
    /// Resolve an incident
    Resolve {
        /// Incident ID
        id: String,
    },
    /// View incident timeline
    Timeline {
        /// Incident ID
        id: String,
    },
}

#[derive(Subcommand)]
enum SloCmd {
    /// List SLOs
    List,
    /// Create an SLO
    Create {
        /// SLO name
        #[arg(long)]
        name: String,
        /// Target percentage (e.g. 99.9)
        #[arg(long)]
        target: f64,
        /// Rolling window (e.g. 30d)
        #[arg(long, default_value = "30d")]
        window: String,
    },
    /// Show error budget for an SLO
    Budget {
        /// SLO ID
        id: String,
    },
    /// Show SLO compliance report
    Compliance,
}

#[derive(Subcommand)]
enum UptimeCmd {
    /// List uptime probes
    Probes,
    /// Show current status of all probes
    Status,
    /// Show uptime statistics
    Stats,
}

#[derive(Subcommand)]
enum CostCmd {
    /// Cost summary by namespace / team
    Summary,
    /// Cost breakdown by resource type
    Breakdown {
        /// Time period (e.g. 7d, 30d)
        #[arg(long, default_value = "30d")]
        period: String,
    },
    /// Cost forecast
    Forecast,
}

#[derive(Subcommand)]
enum ChatCmd {
    /// List chat channels
    Channels,
    /// Send a message to a channel
    Send {
        /// Channel name
        #[arg(long)]
        channel: String,
        /// Message text
        #[arg(long)]
        message: String,
    },
    /// List threads in a channel
    Threads {
        /// Channel name
        #[arg(long)]
        channel: String,
    },
}

#[derive(Subcommand)]
enum WorkflowsCmd {
    /// List workflows
    List,
    /// Create a new workflow
    Create {
        /// Workflow name
        #[arg(long)]
        name: String,
        /// Trigger type (webhook, schedule, manual)
        #[arg(long, default_value = "manual")]
        trigger: String,
    },
    /// Trigger a workflow run
    Run {
        /// Workflow ID
        id: String,
    },
    /// Get workflow run status
    Status {
        /// Workflow ID
        id: String,
    },
}

#[derive(Subcommand)]
enum ChaosCmd {
    /// List chaos experiments
    Experiments,
    /// List chaos experiment templates
    Templates,
    /// Run a chaos experiment
    Run {
        /// Template name or ID
        #[arg(long)]
        template: String,
        /// Target namespace
        #[arg(long)]
        namespace: Option<String>,
    },
}

#[derive(Subcommand)]
enum PolicyCmd {
    /// List policies
    List,
    /// Evaluate input JSON against policies
    Evaluate {
        /// JSON input data
        #[arg(long)]
        input: String,
        /// Policy package to evaluate
        #[arg(long)]
        package: Option<String>,
    },
    /// View policy audit log
    Audit,
}

#[derive(Subcommand)]
enum DastCmd {
    /// List DAST scans
    Scans,
    /// Start a new DAST scan
    Start {
        /// Target URL
        #[arg(long)]
        target: String,
        /// Scan profile (passive, active, full)
        #[arg(long, default_value = "passive")]
        profile: String,
    },
    /// List DAST findings
    Findings {
        /// Filter by scan ID
        #[arg(long)]
        scan_id: Option<String>,
    },
}

#[derive(Subcommand)]
enum PamCmd {
    /// List active privileged sessions
    Sessions,
    /// Check out credentials for a resource
    Checkout {
        /// Resource name (e.g. prod-db, k8s-admin)
        #[arg(long)]
        resource: String,
        /// Reason for access
        #[arg(long)]
        reason: Option<String>,
    },
    /// View PAM audit log
    Audit,
}

#[derive(Subcommand)]
enum PiiCmd {
    /// Scan content for PII
    Scan {
        /// Inline content to scan
        #[arg(long, conflicts_with = "file")]
        content: Option<String>,
        /// Path to file to scan
        #[arg(long)]
        file: Option<String>,
    },
    /// List PII findings
    Findings,
    /// Redact PII from content
    Redact {
        /// Content to redact
        #[arg(long)]
        content: String,
    },
}

#[derive(Subcommand)]
enum BackupCmd {
    /// List backups
    List,
    /// Create a new backup
    Create {
        /// Kubernetes namespace to back up
        #[arg(long)]
        namespace: Option<String>,
        /// Backup schedule (cron expression)
        #[arg(long)]
        schedule: Option<String>,
    },
    /// Restore from a backup
    Restore {
        /// Backup ID
        id: String,
    },
    /// List backup policies
    Policies,
}

#[derive(Subcommand)]
enum ForensicsCmd {
    /// Collect forensic data from a pod
    Collect {
        /// Pod name
        #[arg(long)]
        pod: String,
        /// Kubernetes namespace
        #[arg(long, default_value = "default")]
        namespace: String,
    },
    /// Analyze a collected forensic artifact
    Analyze {
        /// Artifact ID
        id: String,
    },
    /// View event timeline for an artifact
    Timeline {
        /// Artifact ID
        id: String,
    },
}

#[derive(Subcommand)]
enum ProfilerCmd {
    /// List profiling profiles
    Profiles,
    /// Get flamegraph for a profile
    Flamegraph {
        /// Profile ID
        id: String,
    },
    /// Show top CPU / memory consumers
    Top,
}

#[derive(Subcommand)]
enum DevlakeCmd {
    /// Show engineering metrics
    Metrics,
    /// Show DORA metrics
    Dora,
    /// Show team activity
    Activity,
}

#[derive(Subcommand)]
enum AiObsCmd {
    /// List LLM traces
    Traces,
    /// Show LLM cost breakdown
    Costs,
    /// List tracked models
    Models,
}

#[derive(Subcommand)]
enum PortalCmd {
    /// Show developer portal status
    Status,
}

#[derive(Subcommand)]
enum ParityCmd {
    /// Show parity report for all modules (from portal cache)
    All,
    /// Show parity report for a specific module
    Show {
        /// Module name (etcd, cri, apiserver)
        module: String,
    },
    /// Generate a skeleton parity.manifest.toml for an upstream project
    Init {
        /// Upstream GitHub org/repo (e.g. etcd-io/etcd)
        #[arg(long)]
        upstream: String,
        /// Upstream version tag (e.g. v3.5.13)
        #[arg(long, default_value = "latest")]
        version: String,
        /// Local module name (defaults to repo name)
        #[arg(long)]
        name: Option<String>,
        /// Output path (defaults to ./parity.manifest.toml)
        #[arg(long, default_value = "parity.manifest.toml")]
        output: String,
    },
}

#[derive(Subcommand)]
enum LocalLlmCmd {
    /// Show local LLM daemon/service status
    Status,
    /// Daemon lifecycle management
    Daemon {
        #[command(subcommand)]
        cmd: DaemonSubCmd,
    },
    /// Show queue JSON summary
    Queue,
}

#[derive(Subcommand)]
enum DaemonSubCmd {
    /// Start the daemon in the foreground (runs cave-local-llm-daemon start)
    Start,
    /// Signal a running daemon to stop gracefully
    Stop,
    /// Print the daemon stop-signal file path and whether it exists
    Status,
}

#[derive(Subcommand)]
enum ScaffoldCmd {
    /// Create a new project from a template
    Create {
        /// Template name
        #[arg(long)]
        template: String,
        /// New project name
        #[arg(long)]
        name: String,
        /// Output directory
        #[arg(long, default_value = ".")]
        output_dir: String,
    },
    /// List available scaffold templates
    Templates,
}

#[derive(Subcommand)]
enum SignCmd {
    /// Sign an artifact
    Sign {
        /// Artifact path or reference
        #[arg(long)]
        artifact: String,
        /// Signing key ID
        #[arg(long)]
        key_id: Option<String>,
    },
    /// Verify an artifact signature
    Verify {
        /// Artifact path or reference
        #[arg(long)]
        artifact: String,
        /// Expected signer identity
        #[arg(long)]
        identity: Option<String>,
    },
}

#[derive(Subcommand)]
enum StatusCmd {
    /// List all CAVE services and their health
    Services,
    /// Show overall runtime health
    Health,
}

// ── etcd (etcdctl parity) ─────────────────────────────────────────────────────

#[derive(Subcommand)]
enum EtcdCmd {
    /// Get a key (or a key range with --prefix)
    Get {
        /// Key to fetch
        key: String,
        /// Treat <key> as a prefix and return all matching keys
        #[arg(long)]
        prefix: bool,
    },
    /// Put a value at <key>
    Put {
        /// Key
        key: String,
        /// Value
        value: String,
        /// Attach to existing lease ID (optional)
        #[arg(long)]
        lease: Option<i64>,
    },
    /// Delete a key (or a range with --prefix)
    Del {
        /// Key to delete
        key: String,
        /// Delete all keys matching the prefix
        #[arg(long)]
        prefix: bool,
    },
    /// Compact key history at the given revision
    Compact {
        /// Revision to compact at
        revision: i64,
    },
    /// Watch for changes on a key (registers a watch and returns its id)
    Watch {
        /// Key to watch
        key: String,
        /// Watch a prefix range
        #[arg(long)]
        prefix: bool,
    },
    /// Lease management
    Lease {
        #[command(subcommand)]
        cmd: EtcdLeaseCmd,
    },
    /// Cluster member management
    Member {
        #[command(subcommand)]
        cmd: EtcdMemberCmd,
    },
    /// Trigger a backend snapshot
    Snapshot,
    /// Defragment the backend store
    Defrag,
    /// Show backend status (raft, db_size, leader, etc.)
    Status,
    /// Show etcd version
    Version,
    /// Show parity vs upstream etcd
    Parity,
}

#[derive(Subcommand)]
enum EtcdLeaseCmd {
    /// Grant a new lease with the given TTL (seconds)
    Grant {
        /// TTL in seconds
        #[arg(long)]
        ttl: i64,
    },
    /// Revoke a lease by ID
    Revoke {
        /// Lease ID
        id: i64,
    },
    /// Refresh lease TTL (keepalive single-shot)
    Keepalive {
        /// Lease ID
        id: i64,
    },
    /// Show remaining TTL for a lease
    Ttl {
        /// Lease ID
        id: i64,
    },
    /// List all active leases
    List,
}

#[derive(Subcommand)]
enum EtcdMemberCmd {
    /// List cluster members
    List,
    /// Add a new member with the given peer URL
    Add {
        /// Peer URL (e.g. http://10.0.0.1:2380)
        #[arg(long)]
        peer_url: String,
    },
    /// Remove a member by ID
    Remove {
        /// Member ID
        id: u64,
    },
}

// ── CRI (crictl parity) ───────────────────────────────────────────────────────

#[derive(Subcommand)]
enum CriCmd {
    /// List containers (crictl ps)
    Ps {
        /// Show all containers, including stopped ones
        #[arg(long, short = 'a')]
        all: bool,
    },
    /// Inspect a container by ID (crictl inspect)
    Inspect {
        /// Container ID
        id: String,
    },
    /// Start a container (crictl start)
    Start {
        /// Container ID
        id: String,
    },
    /// Stop a container (crictl stop)
    Stop {
        /// Container ID
        id: String,
    },
    /// Kill a container (crictl rm -f)
    Kill {
        /// Container ID
        id: String,
    },
    /// Remove a container (crictl rm)
    Rm {
        /// Container ID
        id: String,
    },
    /// Show container logs (crictl logs)
    Logs {
        /// Container ID
        id: String,
    },
    /// Show container stats (crictl stats)
    Stats {
        /// Container ID; omit for node-wide stats
        id: Option<String>,
    },
    /// List images (crictl images)
    Images,
    /// Pull an image from a registry (crictl pull)
    Pull {
        /// Image reference (e.g. docker.io/library/nginx:latest)
        image: String,
    },
    /// Remove an image (crictl rmi)
    Rmi {
        /// Image reference
        image: String,
    },
    /// List pod sandboxes (crictl pods)
    Pods,
    /// Remove a pod sandbox
    Rmp {
        /// Sandbox ID
        id: String,
    },
    /// Show CRI runtime status (crictl info)
    Info,
    /// Show CRI runtime version (crictl version)
    Version,
    /// Show parity vs upstream containerd CRI
    Parity,
}

// ── apiserver (kubectl parity) ────────────────────────────────────────────────

#[derive(Subcommand)]
enum ApiserverCmd {
    /// Get a resource or list resources (kubectl get)
    Get {
        /// Resource type: pods, nodes, namespaces, deployments, services, configmaps, secrets, ingresses, jobs, cronjobs, daemonsets, statefulsets, replicasets, pvcs, pvs, events, endpoints
        resource: String,
        /// Resource name (omit to list)
        name: Option<String>,
        /// Namespace
        #[arg(long, short = 'n')]
        namespace: Option<String>,
    },
    /// Delete a resource (kubectl delete)
    Delete {
        /// Resource type
        resource: String,
        /// Resource name
        name: String,
        /// Namespace
        #[arg(long, short = 'n')]
        namespace: Option<String>,
    },
    /// Create a namespace (kubectl create namespace)
    CreateNamespace {
        /// Namespace name
        name: String,
    },
    /// List API groups (kubectl api-versions)
    ApiVersions,
    /// List core v1 API resources (kubectl api-resources --api-group="")
    ApiResources,
    /// Show API server version (kubectl version --short --client=false)
    Version,
    /// Liveness probe (/healthz)
    Healthz,
    /// Readiness probe (/readyz)
    Readyz,
    /// Show parity vs upstream kube-apiserver
    Parity,
}

// ── kubelet ───────────────────────────────────────────────────────────────────

#[derive(Subcommand)]
enum KubeletCmd {
    /// Show node status (capacity, allocatable, conditions)
    Status,
    /// List pods assigned to this kubelet
    Pods,
    /// Assign a pod to the kubelet (admission)
    Assign {
        /// Pod UID
        #[arg(long)]
        uid: String,
        /// Pod name
        #[arg(long)]
        name: String,
        /// Namespace
        #[arg(long, default_value = "default")]
        namespace: String,
    },
    /// Start a pod by UID
    Start {
        /// Pod UID
        uid: String,
    },
    /// Stop a pod by UID
    Stop {
        /// Pod UID
        uid: String,
    },
    /// Remove a pod by UID
    Remove {
        /// Pod UID
        uid: String,
    },
    /// Liveness probe (/api/kubelet/health)
    Health,
}

// ── scheduler (kube-scheduler parity) ─────────────────────────────────────────

#[derive(Subcommand)]
enum SchedulerCmd {
    /// List nodes registered with the scheduler
    Nodes,
    /// Show a single node's state (capacity, allocatable, taints)
    Node {
        /// Node name
        name: String,
    },
    /// Register a new node
    Register {
        /// Node name
        #[arg(long)]
        name: String,
        /// CPU capacity (millicores)
        #[arg(long, default_value_t = 4000)]
        cpu_milli: u64,
        /// Memory capacity (bytes)
        #[arg(long, default_value_t = 8_000_000_000)]
        memory_bytes: u64,
    },
    /// Unregister a node
    Unregister {
        /// Node name
        name: String,
    },
    /// Cordon a node (mark unschedulable)
    Cordon {
        /// Node name
        name: String,
    },
    /// Uncordon a node (mark schedulable)
    Uncordon {
        /// Node name
        name: String,
    },
    /// Schedule a pod (returns chosen node)
    Schedule {
        /// Pod UID
        #[arg(long)]
        uid: String,
        /// Pod name
        #[arg(long)]
        name: String,
        /// Namespace
        #[arg(long, default_value = "default")]
        namespace: String,
        /// CPU request (millicores)
        #[arg(long, default_value_t = 100)]
        cpu_milli: u64,
        /// Memory request (bytes)
        #[arg(long, default_value_t = 128_000_000)]
        memory_bytes: u64,
    },
    /// Liveness probe (/api/scheduler/health)
    Health,
}

// ── net (kube-proxy / CNI parity) ─────────────────────────────────────────────

#[derive(Subcommand)]
enum NetCmd {
    /// List allocated pod IPs
    Pods,
    /// Allocate a pod IP from the cluster CIDR
    Alloc {
        /// Namespace
        #[arg(long)]
        namespace: String,
        /// Pod name
        #[arg(long)]
        name: String,
    },
    /// Release a pod IP
    Release {
        /// Namespace
        namespace: String,
        /// Pod name
        name: String,
    },
    /// List ClusterIP services
    Services,
    /// Register a ClusterIP service
    RegisterService {
        /// Namespace
        #[arg(long)]
        namespace: String,
        /// Service name
        #[arg(long)]
        name: String,
        /// Service port
        #[arg(long)]
        port: u16,
        /// Target port on backing pods
        #[arg(long)]
        target_port: u16,
    },
    /// Remove a service
    RemoveService {
        /// Namespace
        namespace: String,
        /// Service name
        name: String,
    },
    /// List NetworkPolicies
    Policies,
    /// Apply a default-deny ingress NetworkPolicy
    DenyIngress {
        /// Namespace
        #[arg(long)]
        namespace: String,
        /// Policy name
        #[arg(long)]
        name: String,
    },
    /// Remove a NetworkPolicy
    RemovePolicy {
        /// Namespace
        namespace: String,
        /// Policy name
        name: String,
    },
    /// List recent network flows
    Flows,
    /// Check whether traffic from src to dst is allowed by policy
    Check {
        /// Source pod (namespace/name)
        #[arg(long)]
        src: String,
        /// Destination pod (namespace/name)
        #[arg(long)]
        dst: String,
        /// Destination port
        #[arg(long)]
        port: u16,
    },
    /// Liveness probe (/api/net/health)
    Health,
}

// ── controller-manager (kube-controller-manager parity) ──────────────────────

#[derive(Subcommand)]
enum ControllerManagerCmd {
    /// Show the current leader (LeaseLock holder identity).
    GetLeader,
    /// List all controller loops compiled into the manager binary.
    ListControllers,
    /// Show controller-manager status (active controllers, version).
    Status,
    /// Print the upstream parity report.
    Parity,
    /// Liveness probe (/api/portal/controller-manager/health).
    Health,
    /// Inspect per-controller workqueue depth + retries. Pass `--controller`
    /// to drill into one controller, omit for the per-controller summary.
    QueuesInspect {
        /// Controller name (e.g. `deployment`, `replicaset`, `hpa`).
        #[arg(long)]
        controller: Option<String>,
    },
    /// Tail the bounded reflector→workqueue event ring (Add/Update/Delete).
    EventsTail,
}

// ── cloud-controller-manager parity ──────────────────────────────────────────

#[derive(Subcommand)]
enum CloudControllerManagerCmd {
    /// List cloud-provider-specific controllers (node, service, route, ...).
    ListCloudControllers,
    /// Show cloud-controller-manager status (active controllers, providers).
    Status,
    /// Print the upstream parity report.
    Parity,
    /// Liveness probe (/api/portal/cloud-controller-manager/health).
    Health,
    /// Cloud LoadBalancer inventory (lifecycle phase + ingress IP per service).
    LoadBalancers,
    /// Cloud instance state per node (Running / Shutdown / Terminated / NotFound / Unreachable).
    Instances {
        /// Drill into a single node by name. Omit for the full inventory.
        #[arg(long)]
        node: Option<String>,
    },
    /// Cloud route-table sync state (desired vs current, blackhole list).
    Routes,
    /// Sync-status summary (counts per controller + last error).
    SyncStatus,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Minimal RFC-3986 unreserved-character percent-encoder for path/query segments.
fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("{}: {:#}", "error".red().bold(), e);
        std::process::exit(1);
    }
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

async fn dispatch_kafka(c: &ApiClient, cmd: KafkaCmd) -> Result<()> {
    match cmd {
        KafkaCmd::Topics => c.get("/api/streams/topics").await,
        KafkaCmd::Consumers => c.get("/api/streams/groups").await,
        KafkaCmd::Schemas => c.get("/subjects").await,
        KafkaCmd::Connectors => c.get("/connectors").await,
    }
}

async fn run(cli: Cli) -> Result<()> {
    let c = ApiClient::new(cli.server, cli.token, cli.format);

    match cli.command {
        // ── Flags ─────────────────────────────────────────────────────────────
        Commands::Flags { cmd } => match cmd {
            FlagsCmd::List => c.get("/api/flags").await,
            FlagsCmd::Create { name, key, enabled } => {
                c.post("/api/flags", json!({ "name": name, "key": key, "enabled": enabled }))
                    .await
            }
            FlagsCmd::Toggle { key } => {
                c.post(&format!("/api/flags/{key}/toggle"), json!({})).await
            }
            FlagsCmd::Delete { key } => c.delete(&format!("/api/flags/{key}")).await,
        },

        // ── Secrets ───────────────────────────────────────────────────────────
        Commands::Secrets { cmd } => match cmd {
            SecretsCmd::List => c.get("/api/secrets").await,
            SecretsCmd::Scan { content, file } => {
                let text = resolve_content(content, file).await?;
                c.post("/api/secrets/scan", json!({ "content": text })).await
            }
            SecretsCmd::Add { name, value, kind } => {
                c.post("/api/secrets", json!({ "name": name, "value": value, "kind": kind }))
                    .await
            }
            SecretsCmd::Rotate { name } => {
                c.post(&format!("/api/secrets/{name}/rotate"), json!({})).await
            }
        },

        // ── Scan ──────────────────────────────────────────────────────────────
        Commands::Scan { cmd } => match cmd {
            ScanCmd::Start { repo, branch } => {
                c.post("/api/scan", json!({ "repo": repo, "branch": branch })).await
            }
            ScanCmd::List => c.get("/api/scan").await,
            ScanCmd::Results { id } => c.get(&format!("/api/scan/{id}/results")).await,
        },

        // ── Vulns ─────────────────────────────────────────────────────────────
        Commands::Vulns { cmd } => match cmd {
            VulnsCmd::Scan { target } => {
                c.post("/api/vulns/scan", json!({ "target": target })).await
            }
            VulnsCmd::List => c.get("/api/vulns").await,
            VulnsCmd::Detail { id } => c.get(&format!("/api/vulns/{id}")).await,
        },

        // ── SBOM ──────────────────────────────────────────────────────────────
        Commands::Sbom { cmd } => match cmd {
            SbomCmd::Generate { project, version } => {
                c.post("/api/sbom", json!({ "project": project, "version": version })).await
            }
            SbomCmd::List => c.get("/api/sbom").await,
            SbomCmd::Detail { id } => c.get(&format!("/api/sbom/{id}")).await,
        },

        // ── Registry ──────────────────────────────────────────────────────────
        Commands::Registry { cmd } => match cmd {
            RegistryCmd::List => c.get("/api/registry").await,
            RegistryCmd::Push { image } => {
                c.post("/api/registry/push", json!({ "image": image })).await
            }
            RegistryCmd::Pull { image } => {
                c.post("/api/registry/pull", json!({ "image": image })).await
            }
            RegistryCmd::Gc => c.post("/api/registry/gc", json!({})).await,
        },

        // ── Gateway ───────────────────────────────────────────────────────────
        Commands::Gateway { cmd } => match cmd {
            GatewayCmd::Routes => c.get("/api/gateway/routes").await,
            GatewayCmd::Services => c.get("/api/gateway/services").await,
            GatewayCmd::Plugins => c.get("/api/gateway/plugins").await,
            GatewayCmd::Stats => c.get("/api/gateway/stats").await,
            GatewayCmd::Gravitee { cmd } => match cmd {
                GraviteeCmd::Apis => c.get("/api/gateway/gravitee/apis").await,
                GraviteeCmd::Plans => c.get("/api/gateway/gravitee/plans").await,
                GraviteeCmd::Applications => c.get("/api/gateway/gravitee/applications").await,
                GraviteeCmd::Subscriptions => c.get("/api/gateway/gravitee/subscriptions").await,
                GraviteeCmd::Portal => c.get("/api/gateway/gravitee/portal/apis").await,
            },
        },

        // ── Pg ────────────────────────────────────────────────────────────────
        Commands::Pg { cmd } => match cmd {
            PgCmd::Databases => c.get("/api/pg/databases").await,
            PgCmd::Migrations => c.get("/api/pg/migrations").await,
            PgCmd::Pools => c.get("/api/pg/pools").await,
            PgCmd::Queries => c.get("/api/pg/queries").await,
            PgCmd::Backup { database } => {
                c.post("/api/pg/backup", json!({ "database": database })).await
            }
        },

        // ── Docdb ─────────────────────────────────────────────────────────────
        Commands::Docdb { cmd } => match cmd {
            DocdbCmd::Health => c.get("/api/docdb/health").await,
            DocdbCmd::Databases => c.get("/api/docdb/databases").await,
            DocdbCmd::Collections { db } => {
                c.get(&format!("/api/docdb/databases/{db}/collections")).await
            }
            DocdbCmd::Stats => c.get("/api/docdb/stats").await,
            DocdbCmd::CollStats { db, collection } => {
                c.get(&format!(
                    "/api/docdb/databases/{db}/collections/{collection}/stats"
                ))
                .await
            }
            DocdbCmd::Find { db, collection, filter } => {
                let filter_json: serde_json::Value =
                    serde_json::from_str(&filter).unwrap_or(json!({}));
                c.post(
                    &format!("/api/docdb/databases/{db}/collections/{collection}/find"),
                    json!({ "filter": filter_json }),
                )
                .await
            }
            DocdbCmd::Indexes { db, collection } => {
                c.get(&format!(
                    "/api/docdb/databases/{db}/collections/{collection}/indexes"
                ))
                .await
            }
            DocdbCmd::Info => c.get("/api/docdb/server/info").await,
        },

        // ── Cache ─────────────────────────────────────────────────────────────
        Commands::Cache { cmd } => match cmd {
            CacheCmd::Health => c.get("/api/cache/health").await,
            CacheCmd::Keys { pattern } => {
                c.get(&format!("/api/cache/keys?pattern={}", urlencode(&pattern))).await
            }
            CacheCmd::Get { key } => {
                c.get(&format!("/api/cache/keys/{}", urlencode(&key))).await
            }
            CacheCmd::Set { key, value, ttl } => {
                let parsed: serde_json::Value =
                    serde_json::from_str(&value).unwrap_or_else(|_| json!(value));
                c.post(
                    "/api/cache/keys",
                    json!({ "key": key, "value": parsed, "ttl": ttl }),
                )
                .await
            }
            CacheCmd::Del { key } => {
                c.delete(&format!("/api/cache/keys/{}", urlencode(&key))).await
            }
            CacheCmd::Stats => c.get("/api/cache/stats").await,
            CacheCmd::Pubsub => c.get("/api/cache/pubsub").await,
            CacheCmd::Publish { channel, message } => {
                c.post(
                    "/api/cache/pubsub/publish",
                    json!({ "channel": channel, "message": message }),
                )
                .await
            }
        },

        // ── Lakehouse (Iceberg + DataFusion) ──────────────────────────────────
        Commands::Lakehouse { cmd } => match cmd {
            LakehouseCmd::Health => c.get("/api/lakehouse/health").await,
            LakehouseCmd::Catalogs => c.get("/api/lakehouse/catalogs").await,
            LakehouseCmd::Namespaces { catalog } => {
                c.get(&format!(
                    "/api/lakehouse/catalogs/{}/namespaces",
                    urlencode(&catalog)
                ))
                .await
            }
            LakehouseCmd::Tables { catalog, namespace } => {
                c.get(&format!(
                    "/api/lakehouse/catalogs/{}/namespaces/{}/tables",
                    urlencode(&catalog),
                    urlencode(&namespace)
                ))
                .await
            }
            LakehouseCmd::Describe {
                catalog,
                namespace,
                table,
            } => {
                c.get(&format!(
                    "/api/lakehouse/catalogs/{}/namespaces/{}/tables/{}",
                    urlencode(&catalog),
                    urlencode(&namespace),
                    urlencode(&table)
                ))
                .await
            }
            LakehouseCmd::Snapshots {
                catalog,
                namespace,
                table,
            } => {
                c.get(&format!(
                    "/api/lakehouse/catalogs/{}/namespaces/{}/tables/{}/snapshots",
                    urlencode(&catalog),
                    urlencode(&namespace),
                    urlencode(&table)
                ))
                .await
            }
            LakehouseCmd::Query { sql, explain } => {
                c.post(
                    "/api/lakehouse/query",
                    json!({ "sql": sql, "explain": explain }),
                )
                .await
            }
            LakehouseCmd::Compact {
                catalog,
                namespace,
                table,
            } => {
                c.post(
                    &format!(
                        "/api/lakehouse/catalogs/{}/namespaces/{}/tables/{}/compact",
                        urlencode(&catalog),
                        urlencode(&namespace),
                        urlencode(&table)
                    ),
                    json!({}),
                )
                .await
            }
            LakehouseCmd::ExpireSnapshots {
                catalog,
                namespace,
                table,
                older_than_days,
            } => {
                c.post(
                    &format!(
                        "/api/lakehouse/catalogs/{}/namespaces/{}/tables/{}/expire-snapshots",
                        urlencode(&catalog),
                        urlencode(&namespace),
                        urlencode(&table)
                    ),
                    json!({ "older_than_days": older_than_days }),
                )
                .await
            }
        },

        // ── Kafka (legacy alias of `streams kafka`) ───────────────────────────
        Commands::Kafka { cmd } => dispatch_kafka(&c, cmd).await,

        // ── Streams (Kafka + Pulsar) ──────────────────────────────────────────
        Commands::Streams { cmd } => match cmd {
            StreamsCmd::Health => c.get("/api/streams/health").await,
            StreamsCmd::Kafka { cmd } => dispatch_kafka(&c, cmd).await,
            StreamsCmd::Pulsar { cmd } => match cmd {
                PulsarCmd::Tenants => c.get("/api/streams/pulsar/tenants").await,
                PulsarCmd::CreateTenant { name } => {
                    c.post("/api/streams/pulsar/tenants", json!({ "name": name })).await
                }
                PulsarCmd::DeleteTenant { name } => {
                    c.delete(&format!("/api/streams/pulsar/tenants/{name}")).await
                }
                PulsarCmd::Namespaces { tenant } => {
                    c.get(&format!("/api/streams/pulsar/namespaces/{tenant}")).await
                }
                PulsarCmd::CreateNamespace { tenant, namespace } => {
                    c.post(
                        &format!("/api/streams/pulsar/namespaces/{tenant}"),
                        json!({ "namespace": namespace }),
                    ).await
                }
                PulsarCmd::DeleteNamespace { tenant, namespace } => {
                    c.delete(&format!("/api/streams/pulsar/namespaces/{tenant}/{namespace}")).await
                }
                PulsarCmd::SetRetention { tenant, namespace, minutes, size_mb } => {
                    c.post(
                        &format!("/api/streams/pulsar/namespaces/{tenant}/{namespace}/retention"),
                        json!({ "retentionTimeInMinutes": minutes, "retentionSizeInMB": size_mb }),
                    ).await
                }
                PulsarCmd::Topics { tenant, namespace } => {
                    c.get(&format!("/api/streams/pulsar/topics/{tenant}/{namespace}")).await
                }
                PulsarCmd::CreateTopic { tenant, namespace, topic, domain, partitions } => {
                    let mut q = vec![];
                    if let Some(d) = domain { q.push(format!("domain={d}")); }
                    if let Some(p) = partitions { q.push(format!("partitions={p}")); }
                    let qs = if q.is_empty() { String::new() } else { format!("?{}", q.join("&")) };
                    c.post(
                        &format!("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}{qs}"),
                        json!({}),
                    ).await
                }
                PulsarCmd::DeleteTopic { tenant, namespace, topic } => {
                    c.delete(&format!("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}")).await
                }
                PulsarCmd::TopicStats { tenant, namespace, topic } => {
                    c.get(&format!("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}/stats")).await
                }
                PulsarCmd::Subscriptions { tenant, namespace, topic } => {
                    c.get(&format!("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}/subscriptions")).await
                }
                PulsarCmd::CreateSubscription { tenant, namespace, topic, subscription, sub_type, initial_position } => {
                    c.post(
                        &format!("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}/subscription/{subscription}"),
                        json!({ "sub_type": sub_type, "initial_position": initial_position }),
                    ).await
                }
                PulsarCmd::DeleteSubscription { tenant, namespace, topic, subscription } => {
                    c.delete(&format!("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}/subscription/{subscription}")).await
                }
                PulsarCmd::SkipAll { tenant, namespace, topic, subscription } => {
                    c.post(
                        &format!("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}/subscription/{subscription}/skipAll"),
                        json!({}),
                    ).await
                }
                PulsarCmd::ResetCursor { tenant, namespace, topic, subscription, position } => {
                    c.post(
                        &format!("/api/streams/pulsar/topics/{tenant}/{namespace}/{topic}/subscription/{subscription}/resetCursor"),
                        json!({ "position": position }),
                    ).await
                }
            },
        },

        // ── Infra ─────────────────────────────────────────────────────────────
        Commands::Infra { cmd } => match cmd {
            InfraCmd::Intent => c.get("/api/infra/intent").await,
            InfraCmd::Plan { stack } => {
                c.post("/api/infra/plan", json!({ "stack": stack })).await
            }
            InfraCmd::Apply { stack } => {
                c.post("/api/infra/apply", json!({ "stack": stack })).await
            }
            InfraCmd::Destroy { stack } => {
                c.post("/api/infra/destroy", json!({ "stack": stack })).await
            }
            InfraCmd::State { stack } => match stack {
                Some(s) => c.get(&format!("/api/infra/state/{s}")).await,
                None => c.get("/api/infra/state").await,
            },
            InfraCmd::Drift => c.get("/api/infra/drift").await,
        },

        // ── Alerts ────────────────────────────────────────────────────────────
        Commands::Alerts { cmd } => match cmd {
            AlertsCmd::List => c.get("/api/alerts").await,
            AlertsCmd::Create { name, expr, severity } => {
                c.post(
                    "/api/alerts",
                    json!({ "name": name, "expr": expr, "severity": severity }),
                )
                .await
            }
            AlertsCmd::Silence { id, duration } => {
                c.post(
                    &format!("/api/alerts/{id}/silence"),
                    json!({ "duration": duration }),
                )
                .await
            }
            AlertsCmd::Routes => c.get("/api/alerts/routes").await,
        },

        // ── Incidents ─────────────────────────────────────────────────────────
        Commands::Incidents { cmd } => match cmd {
            IncidentsCmd::List => c.get("/api/incidents").await,
            IncidentsCmd::Create { title, severity } => {
                c.post("/api/incidents", json!({ "title": title, "severity": severity })).await
            }
            IncidentsCmd::Ack { id } => {
                c.post(&format!("/api/incidents/{id}/ack"), json!({})).await
            }
            IncidentsCmd::Resolve { id } => {
                c.post(&format!("/api/incidents/{id}/resolve"), json!({})).await
            }
            IncidentsCmd::Timeline { id } => {
                c.get(&format!("/api/incidents/{id}/timeline")).await
            }
        },

        // ── SLO ───────────────────────────────────────────────────────────────
        Commands::Slo { cmd } => match cmd {
            SloCmd::List => c.get("/api/slo").await,
            SloCmd::Create { name, target, window } => {
                c.post("/api/slo", json!({ "name": name, "target": target, "window": window }))
                    .await
            }
            SloCmd::Budget { id } => c.get(&format!("/api/slo/{id}/budget")).await,
            SloCmd::Compliance => c.get("/api/slo/compliance").await,
        },

        // ── Uptime ────────────────────────────────────────────────────────────
        Commands::Uptime { cmd } => match cmd {
            UptimeCmd::Probes => c.get("/api/uptime/probes").await,
            UptimeCmd::Status => c.get("/api/uptime/status").await,
            UptimeCmd::Stats => c.get("/api/uptime/stats").await,
        },

        // ── Cost ──────────────────────────────────────────────────────────────
        Commands::Cost { cmd } => match cmd {
            CostCmd::Summary => c.get("/api/cost/summary").await,
            CostCmd::Breakdown { period } => {
                c.get(&format!("/api/cost/breakdown?period={period}")).await
            }
            CostCmd::Forecast => c.get("/api/cost/forecast").await,
        },

        // ── Chat ──────────────────────────────────────────────────────────────
        Commands::Chat { cmd } => match cmd {
            ChatCmd::Channels => c.get("/api/chat/channels").await,
            ChatCmd::Send { channel, message } => {
                c.post("/api/chat/messages", json!({ "channel": channel, "message": message }))
                    .await
            }
            ChatCmd::Threads { channel } => c.get(&format!("/api/chat/threads/{channel}")).await,
        },

        // ── Workflows ─────────────────────────────────────────────────────────
        Commands::Workflows { cmd } => match cmd {
            WorkflowsCmd::List => c.get("/api/workflows").await,
            WorkflowsCmd::Create { name, trigger } => {
                c.post("/api/workflows", json!({ "name": name, "trigger": trigger })).await
            }
            WorkflowsCmd::Run { id } => {
                c.post(&format!("/api/workflows/{id}/run"), json!({})).await
            }
            WorkflowsCmd::Status { id } => c.get(&format!("/api/workflows/{id}/status")).await,
        },

        // ── Chaos ─────────────────────────────────────────────────────────────
        Commands::Chaos { cmd } => match cmd {
            ChaosCmd::Experiments => c.get("/api/chaos/experiments").await,
            ChaosCmd::Templates => c.get("/api/chaos/templates").await,
            ChaosCmd::Run { template, namespace } => {
                c.post(
                    "/api/chaos/run",
                    json!({ "template": template, "namespace": namespace }),
                )
                .await
            }
        },

        // ── Policy ────────────────────────────────────────────────────────────
        Commands::Policy { cmd } => match cmd {
            PolicyCmd::List => c.get("/api/policy").await,
            PolicyCmd::Evaluate { input, package } => {
                let input_val: serde_json::Value = serde_json::from_str(&input)
                    .unwrap_or(serde_json::Value::String(input));
                c.post(
                    "/api/policy/evaluate",
                    json!({ "input": input_val, "package": package }),
                )
                .await
            }
            PolicyCmd::Audit => c.get("/api/policy/audit").await,
        },

        // ── DAST ──────────────────────────────────────────────────────────────
        Commands::Dast { cmd } => match cmd {
            DastCmd::Scans => c.get("/api/dast/scans").await,
            DastCmd::Start { target, profile } => {
                c.post("/api/dast/scans", json!({ "target": target, "profile": profile })).await
            }
            DastCmd::Findings { scan_id } => match scan_id {
                Some(id) => c.get(&format!("/api/dast/findings?scan_id={id}")).await,
                None => c.get("/api/dast/findings").await,
            },
        },

        // ── PAM ───────────────────────────────────────────────────────────────
        Commands::Pam { cmd } => match cmd {
            PamCmd::Sessions => c.get("/api/pam/sessions").await,
            PamCmd::Checkout { resource, reason } => {
                c.post("/api/pam/checkout", json!({ "resource": resource, "reason": reason }))
                    .await
            }
            PamCmd::Audit => c.get("/api/pam/audit").await,
        },

        // ── PII ───────────────────────────────────────────────────────────────
        Commands::Pii { cmd } => match cmd {
            PiiCmd::Scan { content, file } => {
                let text = resolve_content(content, file).await?;
                c.post("/api/pii/scan", json!({ "content": text })).await
            }
            PiiCmd::Findings => c.get("/api/pii/findings").await,
            PiiCmd::Redact { content } => {
                c.post("/api/pii/redact", json!({ "content": content })).await
            }
        },

        // ── Backup ────────────────────────────────────────────────────────────
        Commands::Backup { cmd } => match cmd {
            BackupCmd::List => c.get("/api/backup").await,
            BackupCmd::Create { namespace, schedule } => {
                c.post("/api/backup", json!({ "namespace": namespace, "schedule": schedule }))
                    .await
            }
            BackupCmd::Restore { id } => {
                c.post(&format!("/api/backup/{id}/restore"), json!({})).await
            }
            BackupCmd::Policies => c.get("/api/backup/policies").await,
        },

        // ── Forensics ─────────────────────────────────────────────────────────
        Commands::Forensics { cmd } => match cmd {
            ForensicsCmd::Collect { pod, namespace } => {
                c.post("/api/forensics/collect", json!({ "pod": pod, "namespace": namespace }))
                    .await
            }
            ForensicsCmd::Analyze { id } => {
                c.post("/api/forensics/analyze", json!({ "id": id })).await
            }
            ForensicsCmd::Timeline { id } => {
                c.get(&format!("/api/forensics/timeline/{id}")).await
            }
        },

        // ── Profiler ──────────────────────────────────────────────────────────
        Commands::Profiler { cmd } => match cmd {
            ProfilerCmd::Profiles => c.get("/api/profiler/profiles").await,
            ProfilerCmd::Flamegraph { id } => {
                c.get(&format!("/api/profiler/flamegraph/{id}")).await
            }
            ProfilerCmd::Top => c.get("/api/profiler/top").await,
        },

        // ── Devlake ───────────────────────────────────────────────────────────
        Commands::Devlake { cmd } => match cmd {
            DevlakeCmd::Metrics => c.get("/api/devlake/metrics").await,
            DevlakeCmd::Dora => c.get("/api/devlake/dora").await,
            DevlakeCmd::Activity => c.get("/api/devlake/activity").await,
        },

        // ── AI Obs ────────────────────────────────────────────────────────────
        Commands::AiObs { cmd } => match cmd {
            AiObsCmd::Traces => c.get("/api/ai-obs/traces").await,
            AiObsCmd::Costs => c.get("/api/ai-obs/costs").await,
            AiObsCmd::Models => c.get("/api/ai-obs/models").await,
        },

        // ── Portal ────────────────────────────────────────────────────────────
        Commands::Portal { cmd } => match cmd {
            PortalCmd::Status => c.get("/api/portal/status").await,
        },

        // ── Parity ────────────────────────────────────────────────────────────
        Commands::Parity { cmd } => match cmd {
            ParityCmd::All => c.get("/api/portal/parity").await,
            ParityCmd::Show { module } => {
                // Try module-direct endpoint first, fall back to portal cache
                let direct = match module.as_str() {
                    "etcd"      => Some("/api/etcd/parity"),
                    "cri"       => Some("/api/cri/parity"),
                    "apiserver" => Some("/api/apiserver/parity"),
                    _           => None,
                };
                if let Some(path) = direct {
                    c.get(path).await
                } else {
                    c.get(&format!("/api/portal/parity/{module}")).await
                }
            }
            ParityCmd::Init { upstream, version, name, output } => {
                let parts: Vec<&str> = upstream.splitn(2, '/').collect();
                let (org, repo) = if parts.len() == 2 {
                    (parts[0], parts[1])
                } else {
                    eprintln!("Error: --upstream must be in org/repo format");
                    return Ok(());
                };
                let module_name = name.unwrap_or_else(|| repo.to_string());
                let manifest = format!(
r#"# parity.manifest.toml — {module_name}
# Upstream: {upstream}  https://github.com/{upstream}
# Generated by: cave parity init --upstream {upstream} --version {version}

[upstream]
org     = "{org}"
repo    = "{repo}"
version = "{version}"

[module]
name        = "{module_name}"
description = ""
source_root = "src"

# ── File mappings ────────────────────────────────────────────────────────────
# [[files]]
# upstream = "path/to/upstream.go"
# local    = "src/local.rs"

# ── Function mappings ────────────────────────────────────────────────────────
# [[functions]]
# upstream_name = "UpstreamFunc"
# local_name    = "local_fn"
# file          = "src/routes.rs"

# ── Test mappings ────────────────────────────────────────────────────────────
# [[tests]]
# upstream_test = "TestUpstream"
# local_test    = "test_local"

# ── Surface mappings ─────────────────────────────────────────────────────────
# [[surfaces]]
# kind          = "http"
# upstream_path = "/api/upstream"
# local_path    = "/api/local"
"#,
                    module_name = module_name,
                    upstream = upstream,
                    org = org,
                    repo = repo,
                    version = version,
                );
                match std::fs::write(&output, &manifest) {
                    Ok(_) => {
                        println!("Generated {output}");
                        println!("Edit the file to fill in your mappings, then run:");
                        println!("  cave parity show {module_name}");
                    }
                    Err(e) => eprintln!("Error writing {output}: {e}"),
                }
                Ok(())
            }
        },

        // ── Scaffold ──────────────────────────────────────────────────────────
        Commands::Scaffold { cmd } => match cmd {
            ScaffoldCmd::Create {
                template,
                name,
                output_dir,
            } => {
                c.post(
                    "/api/scaffold",
                    json!({ "template": template, "name": name, "output_dir": output_dir }),
                )
                .await
            }
            ScaffoldCmd::Templates => c.get("/api/scaffold/templates").await,
        },

        // ── Sign ──────────────────────────────────────────────────────────────
        Commands::Sign { cmd } => match cmd {
            SignCmd::Sign { artifact, key_id } => {
                c.post("/api/sign/sign", json!({ "artifact": artifact, "key_id": key_id })).await
            }
            SignCmd::Verify { artifact, identity } => {
                c.post(
                    "/api/sign/verify",
                    json!({ "artifact": artifact, "identity": identity }),
                )
                .await
            }
        },

        // ── Status ────────────────────────────────────────────────────────────
        Commands::Status { cmd } => match cmd {
            StatusCmd::Services => c.get("/api/status/services").await,
            StatusCmd::Health => c.get("/health").await,
        },

        // ── Local LLM ─────────────────────────────────────────────────────────
        Commands::LocalLlm { cmd } => match cmd {
            LocalLlmCmd::Status => {
                println!("cave-local-llm: Phase 3 scheduler daemon active");
                println!("  draft:  cave-local-llm run --crate <name>");
                println!("  daemon: cave local-llm daemon start|stop|status");
                println!("  queue:  cave local-llm queue");
                println!("  docs:   docs/local-llm/README.md");
                Ok(())
            }
            LocalLlmCmd::Daemon { cmd } => {
                let signal_path = std::env::current_dir()
                    .unwrap_or_default()
                    .join(".cave-daemon.stop");
                match cmd {
                    DaemonSubCmd::Start => {
                        println!("Starting cave-local-llm-daemon…  (Ctrl-C or `cave local-llm daemon stop` to halt)");
                        let mut child = std::process::Command::new("cave-local-llm-daemon")
                            .arg("start")
                            .spawn()
                            .map_err(|e| anyhow::anyhow!("failed to spawn daemon: {e}"))?;
                        child.wait().map_err(|e| anyhow::anyhow!("{e}"))?;
                        Ok(())
                    }
                    DaemonSubCmd::Stop => {
                        std::fs::write(&signal_path, b"stop")
                            .map_err(|e| anyhow::anyhow!("write stop signal: {e}"))?;
                        println!("stop signal written → {}", signal_path.display());
                        Ok(())
                    }
                    DaemonSubCmd::Status => {
                        if signal_path.exists() {
                            println!("status: stop-signal present — daemon will stop at next tick");
                        } else {
                            println!("status: no stop-signal — daemon is running (or not started)");
                        }
                        println!("  signal: {}", signal_path.display());
                        Ok(())
                    }
                }
            }
            LocalLlmCmd::Queue => {
                let queue_path = std::env::current_dir()
                    .unwrap_or_default()
                    .join("docs")
                    .join("BUILD-PLAN-TIER1.yaml");
                if !queue_path.exists() {
                    println!("Queue not initialised — run `cave local-llm daemon start` first.");
                    return Ok(());
                }
                let raw = std::fs::read_to_string(&queue_path)
                    .map_err(|e| anyhow::anyhow!("read queue: {e}"))?;
                let value: serde_json::Value = serde_yaml::from_str(&raw)
                    .map_err(|e| anyhow::anyhow!("parse queue YAML: {e}"))?;
                let items = value
                    .get("items")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let mut summary = serde_json::json!({ "total": items.len() });
                for status in ["pending", "in_progress", "done", "stuck"] {
                    let n = items
                        .iter()
                        .filter(|i| i.get("status").and_then(|s| s.as_str()) == Some(status))
                        .count();
                    summary[status] = serde_json::json!(n);
                }
                println!("{}", serde_json::to_string_pretty(&summary).unwrap());
                Ok(())
            }
        },

        // ── etcd ──────────────────────────────────────────────────────────────
        Commands::Etcd { cmd } => match cmd {
            EtcdCmd::Get { key, prefix } => {
                let mut body = json!({ "key": b64encode(key.as_bytes()) });
                if prefix {
                    body["range_end"] = json!(b64encode(&prefix_range_end(key.as_bytes())));
                }
                c.post("/api/etcd/v3/kv/range", body).await
            }
            EtcdCmd::Put { key, value, lease } => {
                let mut body = json!({
                    "key": b64encode(key.as_bytes()),
                    "value": b64encode(value.as_bytes()),
                });
                if let Some(id) = lease {
                    body["lease"] = json!(id);
                }
                c.post("/api/etcd/v3/kv/put", body).await
            }
            EtcdCmd::Del { key, prefix } => {
                let mut body = json!({ "key": b64encode(key.as_bytes()) });
                if prefix {
                    body["range_end"] = json!(b64encode(&prefix_range_end(key.as_bytes())));
                }
                c.post("/api/etcd/v3/kv/deleterange", body).await
            }
            EtcdCmd::Compact { revision } => {
                c.post("/api/etcd/v3/kv/compaction", json!({ "revision": revision })).await
            }
            EtcdCmd::Watch { key, prefix } => {
                let mut body = json!({ "key": b64encode(key.as_bytes()) });
                if prefix {
                    body["range_end"] = json!(b64encode(&prefix_range_end(key.as_bytes())));
                }
                c.post("/api/etcd/v3/watch", body).await
            }
            EtcdCmd::Lease { cmd } => match cmd {
                EtcdLeaseCmd::Grant { ttl } => {
                    c.post("/api/etcd/v3/lease/grant", json!({ "TTL": ttl })).await
                }
                EtcdLeaseCmd::Revoke { id } => {
                    c.post("/api/etcd/v3/lease/revoke", json!({ "ID": id })).await
                }
                EtcdLeaseCmd::Keepalive { id } => {
                    c.post("/api/etcd/v3/lease/keepalive", json!({ "ID": id })).await
                }
                EtcdLeaseCmd::Ttl { id } => {
                    c.post("/api/etcd/v3/lease/timetolive", json!({ "ID": id })).await
                }
                EtcdLeaseCmd::List => c.get("/api/etcd/v3/lease/leases").await,
            },
            EtcdCmd::Member { cmd } => match cmd {
                EtcdMemberCmd::List => {
                    c.post("/api/etcd/v3/cluster/member/list", json!({})).await
                }
                EtcdMemberCmd::Add { peer_url } => {
                    c.post(
                        "/api/etcd/v3/cluster/member/add",
                        json!({ "peerURLs": [peer_url] }),
                    )
                    .await
                }
                EtcdMemberCmd::Remove { id } => {
                    c.post(
                        "/api/etcd/v3/cluster/member/remove",
                        json!({ "ID": id }),
                    )
                    .await
                }
            },
            EtcdCmd::Snapshot => {
                c.post("/api/etcd/v3/maintenance/snapshot", json!({})).await
            }
            EtcdCmd::Defrag => {
                c.post("/api/etcd/v3/maintenance/defragment", json!({})).await
            }
            EtcdCmd::Status => c.get("/api/etcd/status").await,
            EtcdCmd::Version => c.get("/api/etcd/v3/version").await,
            EtcdCmd::Parity => c.get("/api/etcd/parity").await,
        },

        // ── CRI ───────────────────────────────────────────────────────────────
        Commands::Cri { cmd } => match cmd {
            CriCmd::Ps { all } => {
                let path = if all { "/api/cri/containers?all=true" } else { "/api/cri/containers" };
                c.get(path).await
            }
            CriCmd::Inspect { id } => c.get(&format!("/api/cri/containers/{id}")).await,
            CriCmd::Start { id } => {
                c.post(&format!("/api/cri/containers/{id}/start"), json!({})).await
            }
            CriCmd::Stop { id } => {
                c.post(&format!("/api/cri/containers/{id}/stop"), json!({})).await
            }
            CriCmd::Kill { id } => {
                c.post(&format!("/api/cri/containers/{id}/kill"), json!({})).await
            }
            CriCmd::Rm { id } => c.delete(&format!("/api/cri/containers/{id}")).await,
            CriCmd::Logs { id } => c.get(&format!("/api/cri/containers/{id}/logs")).await,
            CriCmd::Stats { id } => match id {
                Some(id) => c.get(&format!("/api/cri/containers/{id}/stats")).await,
                None => c.get("/api/cri/stats").await,
            },
            CriCmd::Images => c.get("/api/cri/images").await,
            CriCmd::Pull { image } => {
                c.post("/api/cri/images/pull", json!({ "image": image })).await
            }
            CriCmd::Rmi { image } => c.delete(&format!("/api/cri/images/{image}")).await,
            CriCmd::Pods => c.get("/api/cri/sandboxes").await,
            CriCmd::Rmp { id } => c.delete(&format!("/api/cri/sandboxes/{id}")).await,
            CriCmd::Info => c.get("/api/cri/status").await,
            CriCmd::Version => c.get("/api/cri/version").await,
            CriCmd::Parity => c.get("/api/cri/parity").await,
        },

        // ── apiserver ─────────────────────────────────────────────────────────
        Commands::Apiserver { cmd } => match cmd {
            ApiserverCmd::Get { resource, name, namespace } => {
                let path = apiserver_resource_path(&resource, name.as_deref(), namespace.as_deref())?;
                c.get(&path).await
            }
            ApiserverCmd::Delete { resource, name, namespace } => {
                let path = apiserver_resource_path(&resource, Some(&name), namespace.as_deref())?;
                c.delete(&path).await
            }
            ApiserverCmd::CreateNamespace { name } => {
                c.post(
                    "/api/v1/namespaces",
                    json!({
                        "apiVersion": "v1",
                        "kind": "Namespace",
                        "metadata": { "name": name }
                    }),
                )
                .await
            }
            ApiserverCmd::ApiVersions => c.get("/apis").await,
            ApiserverCmd::ApiResources => c.get("/api/v1").await,
            ApiserverCmd::Version => c.get("/version").await,
            ApiserverCmd::Healthz => c.get("/healthz").await,
            ApiserverCmd::Readyz => c.get("/readyz").await,
            ApiserverCmd::Parity => c.get("/api/apiserver/parity").await,
        },

        // ── kubelet ───────────────────────────────────────────────────────────
        Commands::Kubelet { cmd } => match cmd {
            KubeletCmd::Status => c.get("/api/kubelet/status").await,
            KubeletCmd::Pods => c.get("/api/kubelet/pods").await,
            KubeletCmd::Assign { uid, name, namespace } => {
                c.post(
                    "/api/kubelet/pods",
                    json!({
                        "uid": uid,
                        "name": name,
                        "namespace": namespace,
                    }),
                )
                .await
            }
            KubeletCmd::Start { uid } => {
                c.post(&format!("/api/kubelet/pods/{uid}/start"), json!({})).await
            }
            KubeletCmd::Stop { uid } => {
                c.post(&format!("/api/kubelet/pods/{uid}/stop"), json!({})).await
            }
            KubeletCmd::Remove { uid } => {
                c.delete(&format!("/api/kubelet/pods/{uid}")).await
            }
            KubeletCmd::Health => c.get("/api/kubelet/health").await,
        },

        // ── scheduler ─────────────────────────────────────────────────────────
        Commands::Scheduler { cmd } => match cmd {
            SchedulerCmd::Nodes => c.get("/api/scheduler/nodes").await,
            SchedulerCmd::Node { name } => c.get(&format!("/api/scheduler/nodes/{name}")).await,
            SchedulerCmd::Register { name, cpu_milli, memory_bytes } => {
                c.post(
                    "/api/scheduler/nodes",
                    json!({
                        "name": name,
                        "cpu_milli": cpu_milli,
                        "memory_bytes": memory_bytes,
                    }),
                )
                .await
            }
            SchedulerCmd::Unregister { name } => {
                c.delete(&format!("/api/scheduler/nodes/{name}")).await
            }
            SchedulerCmd::Cordon { name } => {
                c.post(&format!("/api/scheduler/nodes/{name}/cordon"), json!({})).await
            }
            SchedulerCmd::Uncordon { name } => {
                c.post(&format!("/api/scheduler/nodes/{name}/uncordon"), json!({})).await
            }
            SchedulerCmd::Schedule { uid, name, namespace, cpu_milli, memory_bytes } => {
                c.post(
                    "/api/scheduler/schedule",
                    json!({
                        "uid": uid,
                        "name": name,
                        "namespace": namespace,
                        "cpu_milli": cpu_milli,
                        "memory_bytes": memory_bytes,
                    }),
                )
                .await
            }
            SchedulerCmd::Health => c.get("/api/scheduler/health").await,
        },

        // ── net ───────────────────────────────────────────────────────────────
        Commands::Net { cmd } => match cmd {
            NetCmd::Pods => c.get("/api/net/pods").await,
            NetCmd::Alloc { namespace, name } => {
                c.post(
                    "/api/net/pods",
                    json!({ "namespace": namespace, "name": name }),
                )
                .await
            }
            NetCmd::Release { namespace, name } => {
                c.delete(&format!("/api/net/pods/{namespace}/{name}")).await
            }
            NetCmd::Services => c.get("/api/net/services").await,
            NetCmd::RegisterService { namespace, name, port, target_port } => {
                c.post(
                    "/api/net/services",
                    json!({
                        "namespace": namespace,
                        "name": name,
                        "port": port,
                        "target_port": target_port,
                    }),
                )
                .await
            }
            NetCmd::RemoveService { namespace, name } => {
                c.delete(&format!("/api/net/services/{namespace}/{name}")).await
            }
            NetCmd::Policies => c.get("/api/net/policies").await,
            NetCmd::DenyIngress { namespace, name } => {
                c.post(
                    "/api/net/policies",
                    json!({
                        "namespace": namespace,
                        "name": name,
                        "kind": "deny_ingress",
                    }),
                )
                .await
            }
            NetCmd::RemovePolicy { namespace, name } => {
                c.delete(&format!("/api/net/policies/{namespace}/{name}")).await
            }
            NetCmd::Flows => c.get("/api/net/flows").await,
            NetCmd::Check { src, dst, port } => {
                c.post(
                    "/api/net/check",
                    json!({ "src": src, "dst": dst, "port": port }),
                )
                .await
            }
            NetCmd::Health => c.get("/api/net/health").await,
        },

        // ── controller-manager ────────────────────────────────────────────────
        Commands::ControllerManager { cmd } => match cmd {
            ControllerManagerCmd::GetLeader => c.get("/api/controller-manager/leader").await,
            ControllerManagerCmd::ListControllers => c.get("/api/controller-manager/controllers").await,
            ControllerManagerCmd::Status => c.get("/api/controller-manager/status").await,
            ControllerManagerCmd::Parity => c.get("/api/controller-manager/parity").await,
            ControllerManagerCmd::Health => c.get("/api/portal/controller-manager/health").await,
            ControllerManagerCmd::QueuesInspect { controller } => match controller {
                Some(name) => c.get(&format!("/api/portal/cm/queues/{name}")).await,
                None => c.get("/api/portal/cm/queues").await,
            },
            ControllerManagerCmd::EventsTail => c.get("/api/portal/cm/events").await,
        },

        // ── cloud-controller-manager ──────────────────────────────────────────
        Commands::CloudControllerManager { cmd } => match cmd {
            CloudControllerManagerCmd::ListCloudControllers => {
                c.get("/api/cloud-controller-manager/cloud-controllers").await
            }
            CloudControllerManagerCmd::Status => c.get("/api/cloud-controller-manager/status").await,
            CloudControllerManagerCmd::Parity => c.get("/api/cloud-controller-manager/parity").await,
            CloudControllerManagerCmd::Health => {
                c.get("/api/portal/cloud-controller-manager/health").await
            }
            CloudControllerManagerCmd::LoadBalancers => c.get("/api/portal/ccm/loadbalancers").await,
            CloudControllerManagerCmd::Instances { node } => match node {
                Some(name) => c.get(&format!("/api/portal/ccm/instances/{name}")).await,
                None => c.get("/api/portal/ccm/instances").await,
            },
            CloudControllerManagerCmd::Routes => c.get("/api/portal/ccm/routes").await,
            CloudControllerManagerCmd::SyncStatus => c.get("/api/portal/ccm").await,
        },
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Resolve scan content from an inline string or a file path.
async fn resolve_content(content: Option<String>, file: Option<String>) -> Result<String> {
    match (content, file) {
        (Some(c), _) => Ok(c),
        (_, Some(f)) => tokio::fs::read_to_string(&f)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read {f}: {e}")),
        (None, None) => anyhow::bail!("Provide --content <text> or --file <path>"),
    }
}

/// Base64-encode bytes using the standard alphabet (etcd v3 wire format).
fn b64encode(bytes: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine};
    STANDARD.encode(bytes)
}

/// Compute the etcd v3 prefix range_end: increment the last byte of the prefix.
/// If the prefix ends in 0xff bytes those are stripped; an empty result means
/// "to the end of keyspace".
fn prefix_range_end(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    while let Some(&last) = end.last() {
        if last < 0xff {
            *end.last_mut().unwrap() = last + 1;
            return end;
        }
        end.pop();
    }
    // All-0xff prefix → from-prefix-to-end-of-keyspace, encoded as a single \0.
    vec![0]
}

/// Map a kubectl-style resource short name + namespace + optional name into an
/// apiserver REST path. Returns Err for unsupported resources (rather than
/// silently mis-routing).
fn apiserver_resource_path(
    resource: &str,
    name: Option<&str>,
    namespace: Option<&str>,
) -> Result<String> {
    // Cluster-scoped core/v1.
    let cluster_core = match resource {
        "ns" | "namespace" | "namespaces" => Some("namespaces"),
        "no" | "node" | "nodes" => Some("nodes"),
        "pv" | "persistentvolume" | "persistentvolumes" => Some("persistentvolumes"),
        _ => None,
    };
    if let Some(plural) = cluster_core {
        return Ok(match name {
            Some(n) => format!("/api/v1/{plural}/{n}"),
            None => format!("/api/v1/{plural}"),
        });
    }

    // Namespaced core/v1.
    let ns_core = match resource {
        "po" | "pod" | "pods" => Some("pods"),
        "svc" | "service" | "services" => Some("services"),
        "cm" | "configmap" | "configmaps" => Some("configmaps"),
        "secret" | "secrets" => Some("secrets"),
        "sa" | "serviceaccount" | "serviceaccounts" => Some("serviceaccounts"),
        "ev" | "event" | "events" => Some("events"),
        "ep" | "endpoint" | "endpoints" => Some("endpoints"),
        "pvc" | "persistentvolumeclaim" | "persistentvolumeclaims" => Some("persistentvolumeclaims"),
        "quota" | "resourcequota" | "resourcequotas" => Some("resourcequotas"),
        "limits" | "limitrange" | "limitranges" => Some("limitranges"),
        _ => None,
    };
    if let Some(plural) = ns_core {
        let ns = namespace.unwrap_or("default");
        return Ok(match name {
            Some(n) => format!("/api/v1/namespaces/{ns}/{plural}/{n}"),
            None => format!("/api/v1/namespaces/{ns}/{plural}"),
        });
    }

    // Namespaced apps/v1.
    let ns_apps = match resource {
        "deploy" | "deployment" | "deployments" => Some("deployments"),
        "sts" | "statefulset" | "statefulsets" => Some("statefulsets"),
        "ds" | "daemonset" | "daemonsets" => Some("daemonsets"),
        "rs" | "replicaset" | "replicasets" => Some("replicasets"),
        _ => None,
    };
    if let Some(plural) = ns_apps {
        let ns = namespace.unwrap_or("default");
        return Ok(match name {
            Some(n) => format!("/apis/apps/v1/namespaces/{ns}/{plural}/{n}"),
            None => format!("/apis/apps/v1/namespaces/{ns}/{plural}"),
        });
    }

    // Namespaced batch/v1.
    let ns_batch = match resource {
        "job" | "jobs" => Some("jobs"),
        "cj" | "cronjob" | "cronjobs" => Some("cronjobs"),
        _ => None,
    };
    if let Some(plural) = ns_batch {
        let ns = namespace.unwrap_or("default");
        return Ok(match name {
            Some(n) => format!("/apis/batch/v1/namespaces/{ns}/{plural}/{n}"),
            None => format!("/apis/batch/v1/namespaces/{ns}/{plural}"),
        });
    }

    // Namespaced networking.k8s.io/v1.
    if matches!(resource, "ing" | "ingress" | "ingresses") {
        let ns = namespace.unwrap_or("default");
        return Ok(match name {
            Some(n) => format!("/apis/networking.k8s.io/v1/namespaces/{ns}/ingresses/{n}"),
            None => format!("/apis/networking.k8s.io/v1/namespaces/{ns}/ingresses"),
        });
    }

    anyhow::bail!(
        "unsupported resource '{resource}' — supported: pods, nodes, namespaces, services, \
         configmaps, secrets, deployments, statefulsets, daemonsets, replicasets, jobs, \
         cronjobs, ingresses, persistentvolumes, persistentvolumeclaims, serviceaccounts, \
         events, endpoints, resourcequotas, limitranges (plus common short names)"
    )
}

#[cfg(test)]
mod helpers_tests {
    use super::*;

    #[test]
    fn b64encode_matches_standard_alphabet() {
        assert_eq!(b64encode(b"foo"), "Zm9v");
        assert_eq!(b64encode(b""), "");
        assert_eq!(b64encode(&[0xff, 0xff]), "//8=");
    }

    #[test]
    fn prefix_range_end_increments_last_byte() {
        assert_eq!(prefix_range_end(b"foo"), b"fop");
        assert_eq!(prefix_range_end(b"a"), b"b");
    }

    #[test]
    fn prefix_range_end_strips_trailing_ff_bytes() {
        // "a\xff" → "b" (drop trailing 0xff, increment 'a').
        assert_eq!(prefix_range_end(&[b'a', 0xff]), b"b");
    }

    #[test]
    fn prefix_range_end_all_ff_uses_zero_sentinel() {
        // All 0xff → "from prefix to end of keyspace" sentinel.
        assert_eq!(prefix_range_end(&[0xff, 0xff]), &[0u8]);
    }

    #[test]
    fn apiserver_path_namespaced_pods_default_ns() {
        assert_eq!(
            apiserver_resource_path("pods", None, None).unwrap(),
            "/api/v1/namespaces/default/pods"
        );
    }

    #[test]
    fn apiserver_path_pods_named_with_ns() {
        assert_eq!(
            apiserver_resource_path("po", Some("nginx"), Some("kube-system")).unwrap(),
            "/api/v1/namespaces/kube-system/pods/nginx"
        );
    }

    #[test]
    fn apiserver_path_cluster_scoped_nodes() {
        assert_eq!(
            apiserver_resource_path("nodes", None, None).unwrap(),
            "/api/v1/nodes"
        );
        assert_eq!(
            apiserver_resource_path("no", Some("worker-1"), None).unwrap(),
            "/api/v1/nodes/worker-1"
        );
    }

    #[test]
    fn apiserver_path_apps_v1_deployments() {
        assert_eq!(
            apiserver_resource_path("deploy", Some("api"), Some("prod")).unwrap(),
            "/apis/apps/v1/namespaces/prod/deployments/api"
        );
    }

    #[test]
    fn apiserver_path_batch_v1_cronjobs() {
        assert_eq!(
            apiserver_resource_path("cj", None, Some("etl")).unwrap(),
            "/apis/batch/v1/namespaces/etl/cronjobs"
        );
    }

    #[test]
    fn apiserver_path_networking_ingress() {
        assert_eq!(
            apiserver_resource_path("ingress", Some("web"), Some("edge")).unwrap(),
            "/apis/networking.k8s.io/v1/namespaces/edge/ingresses/web"
        );
    }

    #[test]
    fn apiserver_path_unknown_resource_errors() {
        let err = apiserver_resource_path("widgets", None, None).unwrap_err();
        assert!(err.to_string().contains("unsupported resource"));
    }
}
