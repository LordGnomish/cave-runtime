use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use serde_json::json;

mod client;
use client::ApiClient;

// ── Root CLI ──────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "cave",
    about = "CAVE Runtime CLI — terminal access to all platform modules",
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
    /// Kafka management
    Kafka {
        #[command(subcommand)]
        cmd: KafkaCmd,
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

        // ── Kafka ─────────────────────────────────────────────────────────────
        Commands::Kafka { cmd } => match cmd {
            KafkaCmd::Topics => c.get("/api/kafka/topics").await,
            KafkaCmd::Consumers => c.get("/api/kafka/consumers").await,
            KafkaCmd::Schemas => c.get("/api/kafka/schemas").await,
            KafkaCmd::Connectors => c.get("/api/kafka/connectors").await,
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
