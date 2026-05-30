// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use serde_json::json;

// 2026-05-15 polish — `client` now lives in the lib (so library-side
// modules can reference `crate::client::ApiClient` without breaking
// the lib build). Re-import here so the bin keeps the same `ApiClient`
// + `Format` types as before.
use cavectl::client::ApiClient;

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
    #[arg(
        long,
        global = true,
        default_value = "http://localhost:3000",
        env = "CAVE_SERVER"
    )]
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

// 2026-05-15 polish — `Format` consolidated in the lib so the bin
// and library targets agree on a single type. Re-export here so the
// existing `--format` clap arg keeps working without callsite churn.
use cavectl::client::Format;

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
    /// Container / package registry (Pulp replacement) — legacy alias of `artifacts pulp`.
    Registry {
        #[command(subcommand)]
        cmd: RegistryCmd,
    },
    /// Artifact platform (Harbor + Pulp + Nexus + Cosign consolidated).
    Artifacts {
        #[command(subcommand)]
        cmd: ArtifactsCmd,
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
    /// RDBMS operator (CloudNativePG cluster lifecycle: list/failover/scale/backup)
    Rdbms {
        #[command(subcommand)]
        cmd: RdbmsCmd,
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
    /// Event-driven autoscaler (KEDA replacement)
    Keda {
        #[command(subcommand)]
        cmd: KedaCmd,
    },
    /// HashiCorp Vault parity — secret backends + audit
    Vault {
        #[command(subcommand)]
        cmd: VaultCmd,
    },
    /// Istio Ambient service-mesh — sidecar-less L4/L7
    Mesh {
        #[command(subcommand)]
        cmd: MeshCmd,
    },
    /// Kamaji tenant control plane operator
    Kamaji {
        #[command(subcommand)]
        cmd: KamajiCmd,
    },
    /// Backstage permission framework — RBAC for portal views
    Permission {
        #[command(subcommand)]
        cmd: PermissionCmd,
    },
    /// Charter compliance + audit
    Compliance {
        #[command(subcommand)]
        cmd: ComplianceCmd,
    },
    /// Cluster lifecycle (Kamaji-backed kube clusters)
    Cluster {
        #[command(subcommand)]
        cmd: ClusterCmd,
    },
    /// kube-proxy iptables/IPVS dataplane introspection
    KubeProxy {
        #[command(subcommand)]
        cmd: KubeProxyCmd,
    },
    /// Tracing pipeline (Tempo/Jaeger parity)
    Tracing {
        #[command(subcommand)]
        cmd: TracingCmd,
    },
    /// cave-auth CLI parity
    Auth {
        #[command(subcommand)]
        cmd: AuthCmd,
    },
    /// cave-container-scan CLI parity
    ContainerScan {
        #[command(subcommand)]
        cmd: ContainerScanCmd,
    },
    /// cave-dashboard CLI parity
    Dashboard {
        #[command(subcommand)]
        cmd: DashboardCmd,
    },
    /// cave-deploy CLI parity
    Deploy {
        #[command(subcommand)]
        cmd: DeployCmd,
    },
    /// cave-dns CLI parity
    Dns {
        #[command(subcommand)]
        cmd: DnsCmd,
    },
    /// cave-erp CLI parity
    Erp {
        #[command(subcommand)]
        cmd: ErpCmd,
    },
    /// cave-ha CLI parity
    Ha {
        #[command(subcommand)]
        cmd: HaCmd,
    },
    /// cave-knative CLI parity
    Knative {
        #[command(subcommand)]
        cmd: KnativeServiceCmd,
    },
    /// cave-llm-gateway CLI parity
    LlmGateway {
        #[command(subcommand)]
        cmd: LlmGwCmd,
    },
    /// cave-logs CLI parity
    Logs {
        #[command(subcommand)]
        cmd: LogsCmd,
    },
    /// cave-metrics CLI parity
    Metrics {
        #[command(subcommand)]
        cmd: MetricsCmd,
    },
    /// cave-pipelines CLI parity
    Pipelines {
        #[command(subcommand)]
        cmd: PipelinesCmd,
    },
    /// cave-rdbms (engine) CLI parity
    RdbmsEngine {
        #[command(subcommand)]
        cmd: RdbmsEngineCmd,
    },
    /// cave-rollouts CLI parity
    Rollouts {
        #[command(subcommand)]
        cmd: RolloutsCmd,
    },
    /// cave-security CLI parity
    Security {
        #[command(subcommand)]
        cmd: SecurityCmd,
    },
    /// cave-store CLI parity
    Store {
        #[command(subcommand)]
        cmd: StoreCmd,
    },
    /// cave-trace CLI parity
    Trace {
        #[command(subcommand)]
        cmd: TraceCmd,
    },
    /// cave-tracker CLI parity
    Tracker {
        #[command(subcommand)]
        cmd: TrackerCmd,
    },
    /// cave-upstream CLI parity
    Upstream {
        #[command(subcommand)]
        cmd: UpstreamCmd,
    },
    /// cave-admission CLI parity
    Admission {
        #[command(subcommand)]
        cmd: AdmissionCmd,
    },
    /// cave-cdc CLI parity
    Cdc {
        #[command(subcommand)]
        cmd: CdcCmd,
    },
    /// cave-certs CLI parity
    Certs {
        #[command(subcommand)]
        cmd: CertsCmd,
    },
    /// cave-crm CLI parity
    Crm {
        #[command(subcommand)]
        cmd: CrmCmd,
    },
    /// cave-crossplane CLI parity
    Crossplane {
        #[command(subcommand)]
        cmd: CrossplaneCmd,
    },
    /// cave-gitops-config CLI parity
    GitopsConfig {
        #[command(subcommand)]
        cmd: GitopsCmd,
    },
    /// cave-karpenter CLI parity
    Karpenter {
        #[command(subcommand)]
        cmd: KarpenterCmd,
    },
    /// cave-kubevirt CLI parity
    Kubevirt {
        #[command(subcommand)]
        cmd: KubevirtCmd,
    },
    /// cave-ledger CLI parity
    Ledger {
        #[command(subcommand)]
        cmd: LedgerCmd,
    },
    /// cave-oncall CLI parity
    Oncall {
        #[command(subcommand)]
        cmd: OncallCmd,
    },
    /// cave-search CLI parity
    Search {
        #[command(subcommand)]
        cmd: SearchCmd,
    },
    /// cave-spark-operator CLI parity
    Spark {
        #[command(subcommand)]
        cmd: SparkCmd,
    },
    /// cave-jupyter CLI parity
    Jupyter {
        #[command(subcommand)]
        cmd: JupyterCmd,
    },
    /// cave-mlflow CLI parity
    Mlflow {
        #[command(subcommand)]
        cmd: MlflowCmd,
    },
    /// cave-flux CLI parity
    Flux {
        #[command(subcommand)]
        cmd: FluxCmd,
    },

    // ── eksik-sweep 2026-05-24: SPIRE identity + CIS bench wiring ────────────
    /// cave-identity (SPIRE) CLI parity — workload SPIFFE IDs, trust bundle, federation.
    Identity {
        #[command(subcommand)]
        cmd: IdentityCmd,
    },
    /// cave-bench (kube-bench + kubescape) CLI parity — CIS/NSA/MITRE scans (in-process).
    Bench {
        #[command(subcommand)]
        cmd: BenchCmd,
    },
    /// cave-falco (runtime security) CLI parity — rule pack parse + observability (in-process).
    Falco {
        #[command(subcommand)]
        cmd: FalcoCmd,
    },
}

// ── Per-module subcommand enums ───────────────────────────────────────────────

#[derive(Subcommand)]
enum AuthCmd {
    Status,
    Sessions,
    Users,
    /// List authentication realms (Keycloak `realms` resource).
    Realms,
    /// List OIDC client registrations.
    Clients,
    /// Audit-log of authentication events.
    Events,
    /// SAML 2.0 metadata download (`<md:EntityDescriptor>`).
    SamlMetadata,
    /// Verify an inbound SAML AuthnRequest by its `ID`.
    SamlVerifyRequest,
    /// Show the c14n-canonicalized form of an in-flight document.
    SamlC14n,
    // ── LDAP federation (Keycloak federation/ldap parity) ────────────────────
    /// Bind against the LDAP federation provider and report resultCode.
    LdapTestConnection,
    /// Run a full user-federation sync (LDAP → cave user model).
    LdapSyncUsers,
    /// Run a group + memberOf sync.
    LdapSyncGroups,
    // ── Kerberos / SPNEGO (Keycloak federation/kerberos parity) ──────────────
    /// Parse the configured keytab file; dump principal + enctype + vno.
    KerberosValidateKeytab,
    /// Drive the SPNEGO 401-challenge / Negotiate handshake.
    KerberosTestSpnego,
}

#[derive(Subcommand)]
enum ContainerScanCmd {
    List,
    Get,
    Scan,
    /// Per-image CVE table (Trivy "Vulnerabilities" tab).
    Vulnerabilities,
    /// Deduplicated image roster (Trivy "Images" tab).
    Images,
    /// Admission-gate rules derived from scan output.
    Policies,
    /// Chronological scan log.
    History,
    /// Per-severity roll-up report.
    Reports,
}

#[derive(Subcommand)]
enum DashboardCmd {
    List,
    Get,
    Import,
    /// Evaluate a server-side expression (Grafana __expr__): reduce / resample
    /// / math / threshold / classic condition over upstream results.
    Expr {
        /// Full ExprEvalRequest JSON: {"vars":{...},"command":{...}}.
        #[arg(long)]
        request: String,
    },
    /// List a nested folder's ancestors (root-first) + full path.
    FolderParents {
        /// Folder UID.
        uid: String,
    },
    /// List a nested folder's direct children.
    FolderChildren {
        /// Folder UID.
        uid: String,
    },
    /// Move a nested folder to a new parent (depth + circular validated).
    FolderMove {
        /// Folder UID to move.
        uid: String,
        /// New parent folder UID (omit for root).
        #[arg(long)]
        parent: Option<String>,
    },
    /// Evaluate an RBAC action+scope check against a permissions map.
    AccessEval {
        /// Full AccessEvalRequest JSON: {"permissions":{...},"action":"...","scopes":[...]}.
        #[arg(long)]
        request: String,
    },
}

#[derive(Subcommand)]
enum DeployCmd {
    /// Application CRUD — list / get / diff / history / refresh / delete (ArgoCD parity).
    App {
        #[command(subcommand)]
        cmd: DeployAppCmd,
    },
    /// Trigger a sync operation for an application.
    Sync {
        /// Application name.
        name: String,
        /// Optional target revision (commit SHA, tag, branch).
        #[arg(long)]
        revision: Option<String>,
        /// Render the plan without applying.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Delete resources that are no longer in git.
        #[arg(long, default_value_t = false)]
        prune: bool,
        /// Override conflicts on apply.
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    /// Roll an application back to a previous revision-history entry.
    Rollback {
        /// Application name.
        name: String,
        /// Revision-history id (see `cavectl deploy app history <name>`).
        #[arg(long)]
        history_id: u64,
        /// Delete pruned resources after rollback.
        #[arg(long, default_value_t = false)]
        prune: bool,
        /// Render the plan without applying.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
    /// Module health probe.
    Health,
    /// AppProject management — list / get / delete.
    Project {
        #[command(subcommand)]
        cmd: DeployProjectCmd,
    },
}

#[derive(Subcommand)]
enum DeployAppCmd {
    /// List all applications.
    List,
    /// Get a single application by name.
    Get {
        /// Application name.
        name: String,
    },
    /// Inspect a single application's pending diff.
    Diff {
        /// Application name.
        name: String,
    },
    /// Inspect the revision history of an application.
    History {
        /// Application name.
        name: String,
    },
    /// Trigger a refresh (re-pull manifests + recompute drift).
    Refresh {
        /// Application name.
        name: String,
    },
    /// Delete an application.
    Delete {
        /// Application name.
        name: String,
    },
}

#[derive(Subcommand)]
enum DeployProjectCmd {
    /// List all AppProjects.
    List,
    /// Get a single AppProject by name.
    Get {
        /// Project name.
        name: String,
    },
    /// Delete an AppProject.
    Delete {
        /// Project name.
        name: String,
    },
}

#[derive(Subcommand)]
enum DnsCmd {
    Zones,
    Records,
    Query,
}

#[derive(Subcommand)]
enum ErpCmd {
    Invoices,
    Customers,
    Ledger,
    /// Per-customer stock view (ERPNext "Stock" tab).
    Inventory,
    /// AR waterfall by status.
    Accounting,
    /// Relationship-manager directory.
    Hr,
    /// Per-customer project roll-up.
    Projects,
}

#[derive(Subcommand)]
enum HaCmd {
    Status,
    Failovers,
    Trigger,
}

#[derive(Subcommand)]
enum KnativeServiceCmd {
    Services,
    Revisions,
    Routes,
}

#[derive(Subcommand)]
enum LlmGwCmd {
    Routes,
    Usage,
    Limits,
}

#[derive(Subcommand)]
enum LogsCmd {
    Streams,
    Query,
    Sinks,
}

#[derive(Subcommand)]
enum MetricsCmd {
    Series,
    Query,
    Scrapers,
}

#[derive(Subcommand)]
enum PipelinesCmd {
    List,
    Runs,
    Trigger,
}

#[derive(Subcommand)]
enum RdbmsEngineCmd {
    Query,
    Stats,
    Schemas,
}

#[derive(Subcommand)]
enum RolloutsCmd {
    Status,
    Promote,
    Abort,
}

#[derive(Subcommand)]
enum SecurityCmd {
    Events,
    Policies,
    Audit,
}

#[derive(Subcommand)]
enum StoreCmd {
    Buckets,
    Objects,
    Policies,
}

#[derive(Subcommand)]
enum TraceCmd {
    Services,
    TraceId,
    Search,
}

#[derive(Subcommand)]
enum TrackerCmd {
    Issues,
    Create,
    Transition,
}

#[derive(Subcommand)]
enum UpstreamCmd {
    List,
    Check,
    Bump,
}

#[derive(Subcommand)]
enum AdmissionCmd {
    Decisions,
    Policies,
    Audit,
}
#[derive(Subcommand)]
enum CdcCmd {
    Pipelines,
    Lag,
    Snapshot,
}
#[derive(Subcommand)]
enum CertsCmd {
    List,
    Issue,
    Renew,
}
#[derive(Subcommand)]
enum CrmCmd {
    Accounts,
    Contacts,
    Opportunities,
    /// Per-account next-touch list.
    Activities,
    /// Lifecycle workflows.
    Workflows,
    /// Per-plan revenue roll-up.
    Reports,
}

#[derive(Subcommand)]
enum SparkCmd {
    /// Cluster-wide application list (Spark Operator).
    Applications,
    /// Scheduled application definitions.
    Scheduled,
    /// Completed-application history.
    History,
    /// Namespaces submitting Spark jobs.
    Namespaces,
    /// Per-application status + metrics.
    Status,
}

#[derive(Subcommand)]
enum JupyterCmd {
    /// Notebook server list.
    Servers,
    /// Active kernel processes.
    Kernels,
    /// Notebook documents per user.
    Notebooks,
    /// Available kernel environments / images.
    Environments,
    /// Open user sessions.
    Sessions,
}

#[derive(Subcommand)]
enum MlflowCmd {
    /// MLflow experiments.
    Experiments,
    /// Individual experiment runs.
    Runs,
    /// Registered ML models.
    Models,
    /// Model versions in the registry.
    Versions,
    /// Model-serving deployments.
    Deployments,
}

#[derive(Subcommand)]
enum FluxCmd {
    /// HelmRelease CRs reconciled by Flux.
    HelmReleases,
    /// Kustomization CRs reconciled by Flux.
    Kustomizations,
    /// Source CRs (GitRepository / HelmRepository / OCIRepository).
    Sources,
    /// Image-automation CRs.
    Images,
    /// Notification provider + alert CRs.
    Notifications,
}

// ── eksik-sweep 2026-05-24: SPIRE identity + CIS bench subcommands ─────────────

#[derive(Subcommand)]
enum IdentityCmd {
    /// List registration entries (SPIRE `/api/identity/entries`).
    Entries,
    /// List attested agents.
    Agents,
    /// Fetch own trust bundle (JWKS doc).
    Bundle,
    /// List federation relationships.
    Federation,
    /// OIDC JWKS keys (for JWT-SVID verifiers).
    OidcKeys,
}

#[derive(Subcommand)]
enum BenchCmd {
    /// List built-in profiles (CIS / NSA / MITRE).
    Profiles,
    /// List checks within a framework (cis | nsa | mitre).
    Checks {
        #[arg(long)]
        framework: String,
    },
    /// Run a scan profile against a host (in-process).
    Scan {
        #[arg(long)]
        profile: String,
        #[arg(long)]
        host: String,
        /// markdown | json | sarif
        #[arg(long, default_value = "markdown")]
        format: String,
    },
    /// Print scheduled scans.
    Schedules,
    /// Print observability dashboards + alert rules.
    Observability,
}

#[derive(Subcommand)]
enum FalcoCmd {
    /// Parse a Falco rule pack YAML and print rule/macro/list counts.
    RulesParse {
        #[arg(long)]
        path: String,
    },
    /// List built-in rules shipped by cave-falco (none — packs loaded at runtime).
    RulesListBuiltin,
    /// Print observability dashboards + alert YAML.
    Observability,
    /// Print cave-falco upstream version.
    Version,
}
#[derive(Subcommand)]
enum CrossplaneCmd {
    Claims,
    Compositions,
    Providers,
}
#[derive(Subcommand)]
enum GitopsCmd {
    Apps,
    Sync,
    Diff,
}
#[derive(Subcommand)]
enum KarpenterCmd {
    Nodepools,
    Nodeclaims,
    Drift,
}
#[derive(Subcommand)]
enum KubevirtCmd {
    Vms,
    Vmis,
    Migrate,
}
#[derive(Subcommand)]
enum LedgerCmd {
    Entries,
    Verify,
    Export,
}
#[derive(Subcommand)]
enum OncallCmd {
    Shifts,
    Rotations,
    Incidents,
}
#[derive(Subcommand)]
enum SearchCmd {
    Indexes,
    Query,
    Reindex,
}

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
    /// Start a new code scan (SonarQube-style SAST)
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
    /// Scan a container image (Trivy-style)
    Image {
        /// Image reference (e.g. alpine:3.20) or local tarball path
        target: String,
        /// Report format: table | json | sarif | cyclonedx | spdx
        #[arg(long = "report-format", default_value = "table")]
        report_format: String,
        /// Minimum severity to surface: CRITICAL/HIGH/MEDIUM/LOW/INFO
        #[arg(long, default_value = "MEDIUM")]
        severity: String,
    },
    /// Scan a local filesystem path for installed packages
    Fs {
        /// Path to scan
        path: std::path::PathBuf,
        #[arg(long = "report-format", default_value = "table")]
        report_format: String,
    },
    /// Scan IaC config (Terraform / Kubernetes / Dockerfile / Helm / CloudFormation)
    Config {
        /// Path to a config file or directory
        path: std::path::PathBuf,
        #[arg(long = "report-format", default_value = "table")]
        report_format: String,
    },
    /// Scan for committed secrets / credentials
    Secret {
        /// Path to scan
        path: std::path::PathBuf,
        #[arg(long = "report-format", default_value = "table")]
        report_format: String,
    },
    /// License scan and copyleft summary
    License {
        /// Path to scan
        path: std::path::PathBuf,
        #[arg(long = "report-format", default_value = "table")]
        report_format: String,
    },
    /// Generate an SBOM (CycloneDX or SPDX)
    Sbom {
        /// Target (image ref, fs path, or tarball)
        target: String,
        /// SBOM format: cyclonedx | spdx
        #[arg(long = "report-format", default_value = "cyclonedx")]
        report_format: String,
    },
}

#[derive(Subcommand)]
enum VulnsCmd {
    /// Trigger a legacy vulnerability scan (kept for backwards compat).
    Scan {
        /// Target (image, repo, or path)
        #[arg(long)]
        target: String,
    },
    /// List vulnerabilities (legacy endpoint).
    List,
    /// Get vulnerability detail (legacy endpoint).
    Detail {
        /// Vulnerability ID
        id: String,
    },
    /// Finding triage (DefectDojo-parity).
    Finding {
        #[command(subcommand)]
        cmd: VulnsFindingCmd,
    },
    /// Engagement management.
    Engagement {
        #[command(subcommand)]
        cmd: VulnsEngagementCmd,
    },
    /// Product / ProductType browser.
    Product {
        #[command(subcommand)]
        cmd: VulnsProductCmd,
    },
    /// Import a native scan output (Bandit/Trivy/ZAP/Semgrep/SARIF/Snyk/Nuclei).
    ImportScan {
        /// DefectDojo scan_type — e.g. "Bandit Scan", "SARIF", "Trivy Scan".
        #[arg(long)]
        scan_type: String,
        /// Path to the scan output file.
        #[arg(long)]
        file: String,
        /// Override dedup algorithm (legacy / hash_code / unique_id_from_tool /
        /// unique_id_from_tool_or_hash_code). Default: hash_code.
        #[arg(long)]
        dedup: Option<String>,
    },
    /// RiskAcceptance workflow.
    RiskAccept {
        #[command(subcommand)]
        cmd: VulnsRiskAcceptCmd,
    },
    /// SLA configuration + rollup.
    Sla {
        #[command(subcommand)]
        cmd: VulnsSlaCmd,
    },
    /// Executive report.
    Report {
        #[command(subcommand)]
        cmd: VulnsReportCmd,
    },
    /// List registered scan parsers.
    ScanTypes,
    /// Health.
    Health,
}

#[derive(Subcommand)]
enum VulnsFindingCmd {
    /// List findings (paginated).
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// Get a finding by id.
    Get { id: String },
    /// Create a finding from inline JSON (`@-` for stdin).
    Create {
        #[arg(long)]
        json: String,
    },
}

#[derive(Subcommand)]
enum VulnsEngagementCmd {
    List,
    Create {
        #[arg(long)]
        json: String,
    },
}

#[derive(Subcommand)]
enum VulnsProductCmd {
    List,
    Create {
        #[arg(long)]
        json: String,
    },
    /// ProductType helpers.
    Types {
        #[command(subcommand)]
        cmd: VulnsProductTypeCmd,
    },
}

#[derive(Subcommand)]
enum VulnsProductTypeCmd {
    List,
    Create {
        #[arg(long)]
        json: String,
    },
}

#[derive(Subcommand)]
enum VulnsRiskAcceptCmd {
    List,
    Create {
        #[arg(long)]
        json: String,
    },
}

#[derive(Subcommand)]
enum VulnsSlaCmd {
    /// Rollup + config.
    Rollup,
}

#[derive(Subcommand)]
enum VulnsReportCmd {
    /// Executive summary as JSON.
    Executive,
    /// Executive summary as HTML (saved to file).
    ExecutiveHtml {
        #[arg(long, default_value = "vulns-executive.html")]
        out: String,
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
    /// List SBOMs (legacy)
    List,
    /// Get SBOM detail (legacy)
    Detail {
        /// SBOM ID
        id: String,
    },
    /// Upload (ingest) a BOM file (auto-detect CycloneDX/SPDX)
    Ingest {
        /// Path to BOM file (JSON or XML)
        #[arg(long)]
        file: String,
        /// Existing project UUID; omitted = create new
        #[arg(long)]
        project_uuid: Option<String>,
    },
    /// Component sub-commands (Dependency-Track parity)
    #[command(subcommand)]
    Component(SbomComponentCmd),
    /// Project sub-commands
    #[command(subcommand)]
    Project(SbomProjectCmd),
    /// Vulnerability sub-commands
    #[command(subcommand)]
    Vuln(SbomVulnCmd),
    /// Policy sub-commands
    #[command(subcommand)]
    Policy(SbomPolicyCmd),
    /// Portfolio metrics
    Portfolio,
}

#[derive(Subcommand)]
enum SbomComponentCmd {
    /// List components (paginated)
    List {
        #[arg(long, default_value_t = 1)]
        page: usize,
        #[arg(long, default_value_t = 50)]
        page_size: usize,
    },
    /// Get component detail by UUID
    Get { uuid: String },
}

#[derive(Subcommand)]
enum SbomProjectCmd {
    /// List projects
    List {
        #[arg(long, default_value_t = 1)]
        page: usize,
        #[arg(long, default_value_t = 50)]
        page_size: usize,
    },
    /// Get project detail by UUID
    Get { uuid: String },
    /// Create a project
    Create {
        #[arg(long)]
        name: String,
        #[arg(long = "ver", id = "proj_ver")]
        version: Option<String>,
    },
}

#[derive(Subcommand)]
enum SbomVulnCmd {
    /// List vulnerabilities
    List {
        #[arg(long, default_value_t = 1)]
        page: usize,
        #[arg(long, default_value_t = 50)]
        page_size: usize,
    },
    /// Get vulnerability by ID (CVE / GHSA / OSV)
    Get { id: String },
    /// Transition the analysis state
    Analyze {
        id: String,
        /// One of NOT_SET / EXPLOITABLE / IN_TRIAGE / RESOLVED / FALSE_POSITIVE / NOT_AFFECTED
        #[arg(long)]
        state: String,
    },
}

#[derive(Subcommand)]
enum SbomPolicyCmd {
    /// List policies
    List,
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

// ── Artifact platform: Harbor + Pulp + Nexus + Cosign ────────────────────────

#[derive(Subcommand)]
enum ArtifactsCmd {
    /// Roll-up health for the consolidated artifact platform.
    Health,
    /// Harbor sub-tree (container registry + projects + scanning).
    Harbor {
        #[command(subcommand)]
        cmd: HarborCmd,
    },
    /// Pulp sub-tree (multi-format repos: RPM/Deb/PyPI/etc).
    Pulp {
        #[command(subcommand)]
        cmd: PulpCmd,
    },
    /// Nexus sub-tree (universal repository: hosted/proxy/group + raw).
    Nexus {
        #[command(subcommand)]
        cmd: NexusCmd,
    },
    /// Cosign sub-tree (supply-chain signatures: ECDSA-P256 + ML-DSA-65 hybrid).
    Cosign {
        #[command(subcommand)]
        cmd: CosignCmd,
    },
}

#[derive(Subcommand)]
enum HarborCmd {
    /// List Harbor projects.
    Projects,
    /// Create a Harbor project.
    ProjectCreate {
        #[arg(long)]
        name: String,
    },
    /// Push an OCI image (registry/foo:tag).
    Push {
        #[arg(long)]
        image: String,
    },
    /// Pull an OCI image.
    Pull {
        #[arg(long)]
        image: String,
    },
    /// Trigger a vulnerability scan for a digest.
    Scan {
        #[arg(long)]
        digest: String,
    },
}

#[derive(Subcommand)]
enum PulpCmd {
    /// Sync content from a remote into the named repository.
    Sync {
        #[arg(long)]
        repository: String,
        #[arg(long)]
        remote: String,
    },
    /// Publish a repository version.
    Publish {
        #[arg(long)]
        repository: String,
    },
    /// Distribute a publication under a base path.
    Distribute {
        #[arg(long)]
        publication: String,
        #[arg(long)]
        base_path: String,
    },
    /// List content units in a repository.
    Content {
        #[arg(long)]
        repository: String,
    },
}

#[derive(Subcommand)]
enum NexusCmd {
    /// List Nexus repositories.
    Repos,
    /// Create a hosted Nexus repository.
    RepoCreate {
        #[arg(long)]
        name: String,
        /// One of: raw, maven2, npm, docker, pypi, nuget, helm, apt, yum.
        #[arg(long = "repo-format", default_value = "raw")]
        format_kind: String,
    },
    /// Upload a raw asset to a hosted repository.
    Upload {
        #[arg(long)]
        repository: String,
        #[arg(long)]
        path: String,
        #[arg(long)]
        file: String,
    },
    /// Download a raw asset from a repository.
    Download {
        #[arg(long)]
        repository: String,
        #[arg(long)]
        path: String,
    },
    /// List assets in a repository.
    Assets {
        #[arg(long)]
        repository: String,
    },
}

#[derive(Subcommand)]
enum CosignCmd {
    /// Generate a fresh keypair (ecdsa-p256 | ml-dsa-65).
    Keypair {
        #[arg(long, default_value = "ecdsa-p256")]
        alg: String,
    },
    /// Sign an image manifest digest with a known key.
    Sign {
        #[arg(long)]
        key_id: String,
        #[arg(long)]
        reference: String,
        #[arg(long)]
        digest: String,
    },
    /// Verify a stored signature against a key.
    Verify {
        #[arg(long)]
        key_id: String,
        #[arg(long)]
        digest: String,
    },
    /// List signatures attached to a digest.
    Signatures {
        #[arg(long)]
        digest: String,
    },
    /// Show signing/verification counters (PQC vs classic split).
    Counters,
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
enum VaultCmd {
    /// List secret backends (kv, pki, transit, ...)
    Backends,
    /// List secret paths under a backend
    List { backend: String },
    /// Read metadata (no secret value) for a path
    Meta { backend: String, path: String },
    /// Show recent audit entries
    Audit {
        #[arg(long, default_value = "100")]
        limit: u32,
    },
    /// Seal/unseal status
    Status,
}

#[derive(Subcommand)]
enum MeshCmd {
    /// List AuthorizationPolicy resources
    Policies,
    /// Inspect one AuthorizationPolicy
    Get { name: String },
    /// Show recent flow log entries
    Flows {
        #[arg(long, default_value = "100")]
        limit: u32,
    },
    /// Show waypoint proxy status
    Waypoints,
    /// Show mTLS peer identity stats
    Peers,
}

#[derive(Subcommand)]
enum KamajiCmd {
    /// List TenantControlPlanes
    #[command(name = "tcp-list")]
    TcpList,
    /// Inspect a single TCP (replicas, version, endpoint)
    #[command(name = "tcp-get")]
    TcpGet { name: String },
    /// Scale a TCP to N replicas
    #[command(name = "tcp-scale")]
    TcpScale {
        name: String,
        #[arg(long)]
        replicas: u32,
    },
    /// Show TCP version upgrade plan
    #[command(name = "tcp-upgrade-plan")]
    TcpUpgradePlan {
        name: String,
        #[arg(long)]
        target_version: String,
    },
}

#[derive(Subcommand)]
enum PermissionCmd {
    /// List subjects (users + groups + service accounts)
    Subjects,
    /// List role assignments
    Assignments,
    /// Check a single (principal, permission) decision
    Check {
        principal: String,
        permission: String,
    },
    /// List supported permission names
    Catalog,
}

#[derive(Subcommand)]
enum ComplianceCmd {
    /// Per-crate compliance snapshot
    Snapshot,
    /// Trigger a refresh of the snapshot cache (no-op if no cache)
    Refresh,
    /// Aggregate score + grade letter
    Score,
}

#[derive(Subcommand)]
enum ClusterCmd {
    /// List managed kube clusters
    List,
    /// Inspect a single cluster (version, nodes, control-plane health)
    Get { name: String },
    /// Show node list for a cluster
    Nodes { name: String },
    /// Trigger an upgrade plan
    Upgrade {
        name: String,
        #[arg(long)]
        target_version: String,
    },
}

#[derive(Subcommand)]
enum KubeProxyCmd {
    /// Show dataplane mode (iptables / ipvs / nftables)
    Mode,
    /// List service → backend mappings on this node
    Services,
    /// Show sync statistics
    SyncStats,
    /// Show recent error events
    Errors,
}

#[derive(Subcommand)]
enum TracingCmd {
    /// Show trace ingest rate + drop counters
    Stats,
    /// List recent service-graph nodes
    Services,
    /// Look up a single trace by id
    Trace { trace_id: String },
    /// Show retention policy + storage backend status
    Retention,
}

#[derive(Subcommand)]
enum KedaCmd {
    /// List ScaledObjects in the active tenant
    #[command(name = "scaledobjects")]
    ScaledObjects,
    /// Inspect one ScaledObject (target ref, replica bounds, triggers)
    #[command(name = "get")]
    Get {
        /// ScaledObject name
        name: String,
    },
    /// List the active scaler triggers across all ScaledObjects
    Scalers,
    /// Show metric values for a single ScaledObject's scalers
    Metrics {
        /// ScaledObject name
        name: String,
    },
    /// Show the recent scale-event log
    History {
        /// Maximum events to return
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Pause auto-scaling (sets autoscaling.keda.sh/paused on the ScaledObject)
    Pause {
        /// ScaledObject name
        name: String,
    },
    /// Resume auto-scaling
    Resume {
        /// ScaledObject name
        name: String,
    },
    /// List TriggerAuthentication resources in the active tenant
    #[command(name = "triggerauth-list")]
    TriggerAuthList,
    /// Inspect a single TriggerAuthentication (kind, secrets, env mappings)
    #[command(name = "triggerauth-get")]
    TriggerAuthGet {
        /// TriggerAuthentication name
        name: String,
    },
    /// List ScaledJobs in the active tenant
    #[command(name = "scaledjob-list")]
    ScaledJobList,
    /// Inspect a single ScaledJob (parallelism, completions, last run)
    #[command(name = "scaledjob-get")]
    ScaledJobGet {
        /// ScaledJob name
        name: String,
    },
    /// Show per-scaler activity / latency stats for one ScaledObject
    #[command(name = "scaler-stats")]
    ScalerStats {
        /// ScaledObject name
        name: String,
    },

    // ── 2026-05-12: new admin-portal-backed surface ────────────────────
    /// Pretty-print a ScaledObject's full CRD detail (admin portal-backed)
    #[command(name = "scaledobject-describe")]
    ScaledObjectDescribe {
        /// Namespace
        #[arg(long)]
        namespace: String,
        /// ScaledObject name
        name: String,
    },
    /// Apply a ScaledObject from a YAML file
    #[command(name = "scaledobject-apply")]
    ScaledObjectApply {
        /// Path to a ScaledObject YAML manifest
        #[arg(short, long)]
        file: String,
    },
    /// Delete a ScaledObject by namespace/name
    #[command(name = "scaledobject-delete")]
    ScaledObjectDelete {
        #[arg(long)]
        namespace: String,
        name: String,
    },
    /// Full ScaledJob detail by namespace/name
    #[command(name = "scaledjob-describe")]
    ScaledJobDescribe {
        #[arg(long)]
        namespace: String,
        name: String,
    },
    /// Per-scaler-kind catalog entry (docs URL, metadata keys, example YAML)
    #[command(name = "scaler-detail")]
    ScalerDetail {
        /// Scaler trigger type, e.g. `kafka`, `aws-sqs-queue`
        kind: String,
    },
    /// Tenant-wide per-scaler Prometheus stats (events/min, errors/min, p50/p99)
    #[command(name = "scaler-metrics")]
    ScalerMetrics,
}

#[derive(Subcommand)]
enum RdbmsCmd {
    /// List managed Postgres clusters in the active tenant
    #[command(name = "cluster-list")]
    ClusterList,
    /// Inspect a single cluster (instances, primary, replication state, lag)
    #[command(name = "cluster-get")]
    ClusterGet {
        /// Cluster name
        name: String,
    },
    /// Detailed status incl. backups, lag, primary pod, version
    #[command(name = "cluster-describe")]
    ClusterDescribe {
        /// Cluster name
        name: String,
    },
    /// Trigger a manual failover (promotes a replica to primary)
    #[command(name = "cluster-failover")]
    ClusterFailover {
        /// Cluster name
        name: String,
    },
    /// Scale a cluster to N instances (≥1)
    #[command(name = "cluster-scale")]
    ClusterScale {
        /// Cluster name
        name: String,
        /// Desired instance count
        #[arg(long)]
        instances: u32,
    },
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
    /// KRaft mode — controller quorum + metadata log
    Kraft {
        #[command(subcommand)]
        cmd: KraftCmd,
    },
    /// Kafka Connect — workers, connectors, tasks
    Connect {
        #[command(subcommand)]
        cmd: ConnectCmd,
    },
}

#[derive(Subcommand)]
enum ConnectCmd {
    /// Worker roster + per-worker status
    Worker {
        #[command(subcommand)]
        cmd: ConnectWorkerCmd,
    },
    /// Connector CRUD + lifecycle
    Connector {
        #[command(subcommand)]
        cmd: ConnectConnectorCmd,
    },
    /// Task list + status + restart
    Task {
        #[command(subcommand)]
        cmd: ConnectTaskCmd,
    },
}

#[derive(Subcommand)]
enum ConnectWorkerCmd {
    /// List workers in the Connect cluster
    List,
    /// Show one worker's owned-connector + owned-task counts
    Status {
        /// Worker id (e.g. "worker-1")
        id: String,
    },
}

#[derive(Subcommand)]
enum ConnectConnectorCmd {
    /// List connectors in the cluster
    List,
    /// Get connector info + tasks
    Get { name: String },
    /// Get connector source offsets
    Offsets { name: String },
    /// Create a connector. Each `--config k=v` is a connector property.
    Create {
        /// Connector name
        name: String,
        /// Repeatable `k=v` connector property (`tasks.max=2`,
        /// `connector.class=...`, `topics=orders`).
        #[arg(long = "config", value_name = "k=v")]
        configs: Vec<String>,
    },
    /// Delete a connector + drop its task state.
    Delete { name: String },
    /// Pause a connector — its tasks stop ticking.
    Pause { name: String },
    /// Resume a paused connector.
    Resume { name: String },
    /// Restart a connector + every task.
    Restart { name: String },
}

#[derive(Subcommand)]
enum ConnectTaskCmd {
    /// List tasks for one connector
    List { connector: String },
    /// Show one task's state + failure_trace
    Status { connector: String, task: u32 },
    /// Restart one task (clears failure_trace)
    Restart { connector: String, task: u32 },
}

#[derive(Subcommand)]
enum KraftCmd {
    /// Describe the current quorum (leader, voters, high-watermark).
    DescribeQuorum,
    /// List voters in the controller quorum.
    Voters,
    /// Report the current leader id + epoch.
    Leader,
    /// Show the metadata log high-water mark + live key count.
    Log,
    /// Inspect the materialised cluster-metadata snapshot.
    Snapshot,
    /// Show the set of MetadataRecord types the controller supports.
    Records,
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
    /// Fetch the consolidated portal audit roll-up — five grades
    /// (Structural, Upstream Parity, Honest Parity, Behavioral
    /// Parity, Accessibility) plus crate count + total stub count.
    /// Backed by `/admin/_audit.json`. PlatformAdmin only.
    ///
    /// `--tenant` defaults to the `CAVE_TENANT` env var or `default`.
    Audit {
        /// Tenant id to query as. Falls back to `CAVE_TENANT` env
        /// var, then the literal string `default` if neither is set.
        #[arg(long, env = "CAVE_TENANT")]
        tenant: Option<String>,
    },
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

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Read JSON either inline (the arg itself) or from stdin (when `@-`).
fn read_inline_or_stdin(arg: &str) -> anyhow::Result<String> {
    if arg == "@-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        Ok(s)
    } else if let Some(path) = arg.strip_prefix('@') {
        std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("read {path}: {e}"))
    } else {
        Ok(arg.to_owned())
    }
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

/// `streams connect ...` dispatch. Mirrors the upstream Kafka
/// Connect REST API surface (`/connectors/*`,
/// `/connectors/{name}/tasks/*`) plus the cave-runtime
/// extension routes (`/api/streams/connect/workers`).
async fn dispatch_connect(c: &ApiClient, cmd: ConnectCmd) -> Result<()> {
    match cmd {
        ConnectCmd::Worker { cmd } => match cmd {
            ConnectWorkerCmd::List => c.get("/api/streams/connect/workers").await,
            ConnectWorkerCmd::Status { id } => {
                c.get(&format!("/api/streams/connect/workers/{id}")).await
            }
        },
        ConnectCmd::Connector { cmd } => match cmd {
            ConnectConnectorCmd::List => c.get("/connectors").await,
            ConnectConnectorCmd::Get { name } => c.get(&format!("/connectors/{name}")).await,
            ConnectConnectorCmd::Offsets { name } => {
                c.get(&format!("/connectors/{name}/offsets")).await
            }
            ConnectConnectorCmd::Create { name, configs } => {
                let mut cfg_map = serde_json::Map::new();
                for kv in &configs {
                    if let Some((k, v)) = kv.split_once('=') {
                        cfg_map.insert(k.trim().to_string(), json!(v.trim()));
                    }
                }
                c.post("/connectors", json!({ "name": name, "config": cfg_map }))
                    .await
            }
            ConnectConnectorCmd::Delete { name } => c.delete(&format!("/connectors/{name}")).await,
            ConnectConnectorCmd::Pause { name } => {
                c.put_bytes(&format!("/connectors/{name}/pause"), Vec::new())
                    .await
            }
            ConnectConnectorCmd::Resume { name } => {
                c.put_bytes(&format!("/connectors/{name}/resume"), Vec::new())
                    .await
            }
            ConnectConnectorCmd::Restart { name } => {
                c.post(&format!("/connectors/{name}/restart"), json!({}))
                    .await
            }
        },
        ConnectCmd::Task { cmd } => match cmd {
            ConnectTaskCmd::List { connector } => {
                c.get(&format!("/connectors/{connector}/tasks")).await
            }
            ConnectTaskCmd::Status { connector, task } => {
                c.get(&format!("/connectors/{connector}/tasks/{task}/status"))
                    .await
            }
            ConnectTaskCmd::Restart { connector, task } => {
                c.post(
                    &format!("/connectors/{connector}/tasks/{task}/restart"),
                    json!({}),
                )
                .await
            }
        },
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
            ScanCmd::Image { target, report_format, severity } => {
                c.post(
                    "/api/scan/image",
                    json!({ "target": target, "format": report_format, "severity": severity }),
                )
                .await
            }
            ScanCmd::Fs { path, report_format } => {
                c.post(
                    "/api/scan/fs",
                    json!({ "path": path.to_string_lossy(), "format": report_format }),
                )
                .await
            }
            ScanCmd::Config { path, report_format } => {
                c.post(
                    "/api/scan/config",
                    json!({ "path": path.to_string_lossy(), "format": report_format }),
                )
                .await
            }
            ScanCmd::Secret { path, report_format } => {
                c.post(
                    "/api/scan/secret",
                    json!({ "path": path.to_string_lossy(), "format": report_format }),
                )
                .await
            }
            ScanCmd::License { path, report_format } => {
                c.post(
                    "/api/scan/license",
                    json!({ "path": path.to_string_lossy(), "format": report_format }),
                )
                .await
            }
            ScanCmd::Sbom { target, report_format } => {
                c.post(
                    "/api/scan/sbom",
                    json!({ "target": target, "format": report_format }),
                )
                .await
            }
        },

        // ── Vulns ─────────────────────────────────────────────────────────────
        Commands::Vulns { cmd } => match cmd {
            VulnsCmd::Scan { target } => {
                c.post("/api/vulns/scan", json!({ "target": target })).await
            }
            VulnsCmd::List => c.get("/api/vulns").await,
            VulnsCmd::Detail { id } => c.get(&format!("/api/vulns/{id}")).await,
            VulnsCmd::Health => c.get("/api/vulns/health").await,
            VulnsCmd::ScanTypes => c.get("/api/vulns/scan-types").await,
            VulnsCmd::Finding { cmd } => match cmd {
                VulnsFindingCmd::List { limit, offset } => {
                    c.get(&format!("/api/vulns/findings?limit={limit}&offset={offset}")).await
                }
                VulnsFindingCmd::Get { id } => c.get(&format!("/api/vulns/findings/{id}")).await,
                VulnsFindingCmd::Create { json } => {
                    let body: serde_json::Value = serde_json::from_str(&read_inline_or_stdin(&json)?)?;
                    c.post("/api/vulns/findings", body).await
                }
            },
            VulnsCmd::Engagement { cmd } => match cmd {
                VulnsEngagementCmd::List => c.get("/api/vulns/engagements").await,
                VulnsEngagementCmd::Create { json } => {
                    let body: serde_json::Value = serde_json::from_str(&read_inline_or_stdin(&json)?)?;
                    c.post("/api/vulns/engagements", body).await
                }
            },
            VulnsCmd::Product { cmd } => match cmd {
                VulnsProductCmd::List => c.get("/api/vulns/products").await,
                VulnsProductCmd::Create { json } => {
                    let body: serde_json::Value = serde_json::from_str(&read_inline_or_stdin(&json)?)?;
                    c.post("/api/vulns/products", body).await
                }
                VulnsProductCmd::Types { cmd } => match cmd {
                    VulnsProductTypeCmd::List => c.get("/api/vulns/product-types").await,
                    VulnsProductTypeCmd::Create { json } => {
                        let body: serde_json::Value = serde_json::from_str(&read_inline_or_stdin(&json)?)?;
                        c.post("/api/vulns/product-types", body).await
                    }
                }
            },
            VulnsCmd::ImportScan { scan_type, file, dedup } => {
                let content = std::fs::read_to_string(&file)
                    .map_err(|e| anyhow::anyhow!("read {file}: {e}"))?;
                let mut body = serde_json::json!({"scan_type": scan_type, "content": content});
                if let Some(d) = dedup { body["dedup"] = serde_json::Value::String(d); }
                c.post("/api/vulns/import-scan", body).await
            }
            VulnsCmd::RiskAccept { cmd } => match cmd {
                VulnsRiskAcceptCmd::List => c.get("/api/vulns/risk-acceptances").await,
                VulnsRiskAcceptCmd::Create { json } => {
                    let body: serde_json::Value = serde_json::from_str(&read_inline_or_stdin(&json)?)?;
                    c.post("/api/vulns/risk-acceptances", body).await
                }
            },
            VulnsCmd::Sla { cmd } => match cmd {
                VulnsSlaCmd::Rollup => c.get("/api/vulns/sla").await,
            },
            VulnsCmd::Report { cmd } => match cmd {
                VulnsReportCmd::Executive => c.get("/api/vulns/reports/executive").await,
                VulnsReportCmd::ExecutiveHtml { out: _ } => {
                    c.get("/api/vulns/reports/executive.html").await
                }
            },
        },

        // ── SBOM ──────────────────────────────────────────────────────────────
        Commands::Sbom { cmd } => match cmd {
            SbomCmd::Generate { project, version } => {
                c.post("/api/sbom", json!({ "project": project, "version": version })).await
            }
            SbomCmd::List => c.get("/api/sbom").await,
            SbomCmd::Detail { id } => c.get(&format!("/api/sbom/{id}")).await,
            SbomCmd::Ingest { file, project_uuid } => {
                use base64::Engine;
                let bytes = std::fs::read(&file)
                    .map_err(|e| anyhow::anyhow!("read {}: {}", file, e))?;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                c.post(
                    "/api/v1/bom",
                    json!({ "project_uuid": project_uuid, "bom_b64": b64 }),
                )
                .await
            }
            SbomCmd::Component(sub) => match sub {
                SbomComponentCmd::List { page, page_size } => {
                    c.get(&format!("/api/v1/component?page={page}&page_size={page_size}")).await
                }
                SbomComponentCmd::Get { uuid } => c.get(&format!("/api/v1/component/{uuid}")).await,
            },
            SbomCmd::Project(sub) => match sub {
                SbomProjectCmd::List { page, page_size } => {
                    c.get(&format!("/api/v1/project?page={page}&page_size={page_size}")).await
                }
                SbomProjectCmd::Get { uuid } => c.get(&format!("/api/v1/project/{uuid}")).await,
                SbomProjectCmd::Create { name, version } => {
                    c.post("/api/v1/project", json!({ "name": name, "version": version })).await
                }
            },
            SbomCmd::Vuln(sub) => match sub {
                SbomVulnCmd::List { page, page_size } => {
                    c.get(&format!("/api/v1/vulnerability?page={page}&page_size={page_size}")).await
                }
                SbomVulnCmd::Get { id } => c.get(&format!("/api/v1/vulnerability/{id}")).await,
                SbomVulnCmd::Analyze { id, state } => {
                    c.post(
                        &format!("/api/v1/vulnerability/{id}/analysis"),
                        json!({ "state": state }),
                    )
                    .await
                }
            },
            SbomCmd::Policy(sub) => match sub {
                SbomPolicyCmd::List => c.get("/api/v1/policy").await,
            },
            SbomCmd::Portfolio => c.get("/api/v1/metrics/portfolio").await,
        },

        // ── Registry (legacy alias of `artifacts pulp`) ──────────────────────
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

        // ── Artifacts (Harbor + Pulp + Nexus + Cosign consolidated) ──────────
        Commands::Artifacts { cmd } => match cmd {
            ArtifactsCmd::Health => c.get("/api/artifacts/health").await,
            ArtifactsCmd::Harbor { cmd } => match cmd {
                HarborCmd::Projects => c.get("/api/v2.0/projects").await,
                HarborCmd::ProjectCreate { name } => {
                    c.post("/api/v2.0/projects", json!({ "project_name": name })).await
                }
                HarborCmd::Push { image } => {
                    c.post("/api/registry/push", json!({ "image": image })).await
                }
                HarborCmd::Pull { image } => {
                    c.post("/api/registry/pull", json!({ "image": image })).await
                }
                HarborCmd::Scan { digest } => {
                    c.post("/api/v2.0/scanners/scan", json!({ "digest": digest })).await
                }
            },
            ArtifactsCmd::Pulp { cmd } => match cmd {
                PulpCmd::Sync { repository, remote } => {
                    c.post(
                        "/api/artifacts/repositories/sync",
                        json!({ "repository": repository, "remote": remote }),
                    )
                    .await
                }
                PulpCmd::Publish { repository } => {
                    c.post(
                        "/api/artifacts/publications",
                        json!({ "repository": repository }),
                    )
                    .await
                }
                PulpCmd::Distribute { publication, base_path } => {
                    c.post(
                        "/api/artifacts/distributions",
                        json!({ "publication": publication, "base_path": base_path }),
                    )
                    .await
                }
                PulpCmd::Content { repository } => {
                    c.get(&format!("/api/artifacts/content?repository={repository}"))
                        .await
                }
            },
            ArtifactsCmd::Nexus { cmd } => match cmd {
                NexusCmd::Repos => c.get("/api/nexus/v1/repositories").await,
                NexusCmd::RepoCreate { name, format_kind } => {
                    c.post(
                        "/api/nexus/v1/repositories",
                        json!({
                            "name": name,
                            "format": format_kind,
                            "type": "hosted",
                            "write_policy": "allow",
                        }),
                    )
                    .await
                }
                NexusCmd::Upload { repository, path, file } => {
                    let bytes = std::fs::read(&file)
                        .map_err(|e| anyhow::anyhow!("read {file}: {e}"))?;
                    c.put_bytes(
                        &format!("/api/nexus/repository/{repository}/{path}"),
                        bytes,
                    )
                    .await
                }
                NexusCmd::Download { repository, path } => {
                    c.get(&format!("/api/nexus/repository/{repository}/{path}")).await
                }
                NexusCmd::Assets { repository } => {
                    c.get(&format!("/api/nexus/v1/assets?repository={repository}")).await
                }
            },
            ArtifactsCmd::Cosign { cmd } => match cmd {
                CosignCmd::Keypair { alg } => {
                    c.post("/api/cosign/v1/keypair", json!({ "alg": alg })).await
                }
                CosignCmd::Sign { key_id, reference, digest } => {
                    c.post(
                        "/api/cosign/v1/sign",
                        json!({
                            "key_id": key_id,
                            "reference": reference,
                            "digest": digest,
                        }),
                    )
                    .await
                }
                CosignCmd::Verify { key_id: _, digest } => {
                    // Convenience verify: pull the most recent stored signature
                    // for the digest from the server-side index, server handles
                    // the per-key cross-check internally.
                    c.get(&format!("/api/cosign/v1/signatures/{digest}")).await
                }
                CosignCmd::Signatures { digest } => {
                    c.get(&format!("/api/cosign/v1/signatures/{digest}")).await
                }
                CosignCmd::Counters => c.get("/api/cosign/v1/counters").await,
            },
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

        // ── Rdbms (rdbms-operator: cluster lifecycle) ─────────────────────────
        Commands::Rdbms { cmd } => match cmd {
            RdbmsCmd::ClusterList => c.get("/api/rdbms-operator/clusters").await,
            RdbmsCmd::ClusterGet { name } => {
                c.get(&format!(
                    "/api/rdbms-operator/clusters/{}",
                    urlencode(&name)
                ))
                .await
            }
            RdbmsCmd::ClusterDescribe { name } => {
                c.get(&format!(
                    "/api/rdbms-operator/clusters/{}/describe",
                    urlencode(&name)
                ))
                .await
            }
            RdbmsCmd::ClusterFailover { name } => {
                c.post(
                    &format!(
                        "/api/rdbms-operator/clusters/{}/failover",
                        urlencode(&name)
                    ),
                    json!({}),
                )
                .await
            }
            RdbmsCmd::ClusterScale { name, instances } => {
                c.post(
                    &format!(
                        "/api/rdbms-operator/clusters/{}/scale",
                        urlencode(&name)
                    ),
                    json!({ "instances": instances }),
                )
                .await
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
            StreamsCmd::Kraft { cmd } => match cmd {
                KraftCmd::DescribeQuorum => c.get("/api/streams/kraft/describe-quorum").await,
                KraftCmd::Voters         => c.get("/api/streams/kraft/voters").await,
                KraftCmd::Leader         => c.get("/api/streams/kraft/leader").await,
                KraftCmd::Log            => c.get("/api/streams/kraft/log").await,
                KraftCmd::Snapshot       => c.get("/api/streams/kraft/snapshot").await,
                KraftCmd::Records        => c.get("/api/streams/kraft/records").await,
            },
            StreamsCmd::Connect { cmd } => dispatch_connect(&c, cmd).await,
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
            PortalCmd::Audit { tenant } => {
                let t = tenant.unwrap_or_else(|| "default".to_string());
                c.get(&format!("/admin/_audit.json?tenant_id={t}")).await
            }
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
            // /api/local-llm/ — surface for compliance audit.
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
        },

        // ── keda (event-driven autoscaler) ────────────────────────────────────
        // ── Vault ─────────────────────────────────────────────────────────────
        Commands::Vault { cmd } => match cmd {
            VaultCmd::Backends => c.get("/api/vault/backends").await,
            VaultCmd::List { backend } => {
                c.get(&format!("/api/vault/backends/{}/paths", urlencode(&backend))).await
            }
            VaultCmd::Meta { backend, path } => {
                c.get(&format!(
                    "/api/vault/backends/{}/paths/{}",
                    urlencode(&backend),
                    urlencode(&path)
                ))
                .await
            }
            VaultCmd::Audit { limit } => c.get(&format!("/api/vault/audit?limit={limit}")).await,
            VaultCmd::Status => c.get("/api/vault/status").await,
        },

        // ── Mesh ──────────────────────────────────────────────────────────────
        Commands::Mesh { cmd } => match cmd {
            MeshCmd::Policies => c.get("/api/mesh/authz").await,
            MeshCmd::Get { name } => c.get(&format!("/api/mesh/authz/{}", urlencode(&name))).await,
            MeshCmd::Flows { limit } => c.get(&format!("/api/mesh/flows?limit={limit}")).await,
            MeshCmd::Waypoints => c.get("/api/mesh/waypoints").await,
            MeshCmd::Peers => c.get("/api/mesh/peers").await,
        },

        // ── Kamaji ────────────────────────────────────────────────────────────
        Commands::Kamaji { cmd } => match cmd {
            KamajiCmd::TcpList => c.get("/api/kamaji/tcps").await,
            KamajiCmd::TcpGet { name } => {
                c.get(&format!("/api/kamaji/tcps/{}", urlencode(&name))).await
            }
            KamajiCmd::TcpScale { name, replicas } => {
                c.post(
                    &format!("/api/kamaji/tcps/{}/scale", urlencode(&name)),
                    json!({ "replicas": replicas }),
                )
                .await
            }
            KamajiCmd::TcpUpgradePlan { name, target_version } => {
                c.post(
                    &format!("/api/kamaji/tcps/{}/upgrade-plan", urlencode(&name)),
                    json!({ "target_version": target_version }),
                )
                .await
            }
        },

        // ── Permission ────────────────────────────────────────────────────────
        Commands::Permission { cmd } => match cmd {
            PermissionCmd::Subjects => c.get("/api/permission/subjects").await,
            PermissionCmd::Assignments => c.get("/api/permission/assignments").await,
            PermissionCmd::Check { principal, permission } => {
                c.post(
                    "/api/permission/check",
                    json!({ "principal": principal, "permission": permission }),
                )
                .await
            }
            PermissionCmd::Catalog => c.get("/api/permission/catalog").await,
        },

        // ── Compliance ────────────────────────────────────────────────────────
        Commands::Compliance { cmd } => match cmd {
            ComplianceCmd::Snapshot => c.get("/api/compliance/snapshot").await,
            ComplianceCmd::Refresh => c.post("/api/compliance/refresh", json!({})).await,
            ComplianceCmd::Score => c.get("/api/compliance/score").await,
        },

        // ── Cluster ───────────────────────────────────────────────────────────
        Commands::Cluster { cmd } => match cmd {
            ClusterCmd::List => c.get("/api/cluster/clusters").await,
            ClusterCmd::Get { name } => {
                c.get(&format!("/api/cluster/clusters/{}", urlencode(&name))).await
            }
            ClusterCmd::Nodes { name } => {
                c.get(&format!("/api/cluster/clusters/{}/nodes", urlencode(&name))).await
            }
            ClusterCmd::Upgrade { name, target_version } => {
                c.post(
                    &format!("/api/cluster/clusters/{}/upgrade", urlencode(&name)),
                    json!({ "target_version": target_version }),
                )
                .await
            }
        },

        // ── KubeProxy ─────────────────────────────────────────────────────────
        Commands::KubeProxy { cmd } => match cmd {
            KubeProxyCmd::Mode => c.get("/api/kube-proxy/mode").await,
            KubeProxyCmd::Services => c.get("/api/kube-proxy/services").await,
            KubeProxyCmd::SyncStats => c.get("/api/kube-proxy/sync-stats").await,
            KubeProxyCmd::Errors => c.get("/api/kube-proxy/errors").await,
        },

        // ── Tracing ───────────────────────────────────────────────────────────
        Commands::Tracing { cmd } => match cmd {
            TracingCmd::Stats => c.get("/api/tracing/stats").await,
            TracingCmd::Services => c.get("/api/tracing/services").await,
            TracingCmd::Trace { trace_id } => {
                c.get(&format!("/api/tracing/traces/{}", urlencode(&trace_id))).await
            }
            TracingCmd::Retention => c.get("/api/tracing/retention").await,
        },

        // ── Tier1 cavectl batch (2026-05-11): 19 crates ────────────────────────
        Commands::Auth { cmd } => match cmd {
            AuthCmd::Status            => c.get("/api/auth/status").await,
            AuthCmd::Sessions          => c.get("/api/auth/sessions").await,
            AuthCmd::Users             => c.get("/api/auth/users").await,
            AuthCmd::Realms            => c.get("/api/auth/realms").await,
            AuthCmd::Clients           => c.get("/api/auth/clients").await,
            AuthCmd::Events            => c.get("/api/auth/events").await,
            AuthCmd::SamlMetadata      => c.get("/api/auth/saml/metadata").await,
            AuthCmd::SamlVerifyRequest => c.get("/api/auth/saml/verify").await,
            AuthCmd::SamlC14n          => c.get("/api/auth/saml/c14n").await,
            // LDAP federation
            AuthCmd::LdapTestConnection => c.get(cavectl::auth::ldap::PATH_TEST_CONNECTION).await,
            AuthCmd::LdapSyncUsers      => c.get(cavectl::auth::ldap::PATH_SYNC_USERS).await,
            AuthCmd::LdapSyncGroups     => c.get(cavectl::auth::ldap::PATH_SYNC_GROUPS).await,
            // Kerberos / SPNEGO
            AuthCmd::KerberosValidateKeytab => c.get(cavectl::auth::kerberos::PATH_VALIDATE_KEYTAB).await,
            AuthCmd::KerberosTestSpnego     => c.get(cavectl::auth::kerberos::PATH_TEST_SPNEGO).await,
        },
        Commands::ContainerScan { cmd } => match cmd {
            ContainerScanCmd::List            => c.get("/api/container-scan/list").await,
            ContainerScanCmd::Get             => c.get("/api/container-scan/get").await,
            ContainerScanCmd::Scan            => c.get("/api/container-scan/scan").await,
            ContainerScanCmd::Vulnerabilities => c.get("/api/container-scan/vulnerabilities").await,
            ContainerScanCmd::Images          => c.get("/api/container-scan/images").await,
            ContainerScanCmd::Policies        => c.get("/api/container-scan/policies").await,
            ContainerScanCmd::History         => c.get("/api/container-scan/history").await,
            ContainerScanCmd::Reports         => c.get("/api/container-scan/reports").await,
        },
        Commands::Dashboard { cmd } => match cmd {
            DashboardCmd::List => c.get("/api/dashboard/list").await,
            DashboardCmd::Get => c.get("/api/dashboard/get").await,
            DashboardCmd::Import => c.get("/api/dashboard/import").await,
            DashboardCmd::Expr { request } => {
                let body: serde_json::Value = serde_json::from_str(&request)?;
                c.post("/api/ds/query/expr", body).await
            }
            DashboardCmd::FolderParents { uid } => {
                c.get(&format!("/api/folders/{uid}/parents")).await
            }
            DashboardCmd::FolderChildren { uid } => {
                c.get(&format!("/api/folders/{uid}/children")).await
            }
            DashboardCmd::FolderMove { uid, parent } => {
                c.post(
                    &format!("/api/folders/{uid}/move"),
                    json!({ "parentUid": parent }),
                )
                .await
            }
            DashboardCmd::AccessEval { request } => {
                let body: serde_json::Value = serde_json::from_str(&request)?;
                c.post("/api/access-control/eval", body).await
            }
        },
        Commands::Deploy { cmd } => match cmd {
            DeployCmd::App { cmd } => match cmd {
                DeployAppCmd::List => c.get("/api/deploy/apps").await,
                DeployAppCmd::Get { name } => {
                    c.get(&format!("/api/deploy/apps/{name}")).await
                }
                DeployAppCmd::Diff { name } => {
                    c.get(&format!("/api/deploy/apps/{name}/diff")).await
                }
                DeployAppCmd::History { name } => {
                    c.get(&format!("/api/deploy/apps/{name}/history")).await
                }
                DeployAppCmd::Refresh { name } => {
                    c.post(&format!("/api/deploy/apps/{name}/refresh"), json!({}))
                        .await
                }
                DeployAppCmd::Delete { name } => {
                    c.delete(&format!("/api/deploy/apps/{name}")).await
                }
            },
            DeployCmd::Sync {
                name,
                revision,
                dry_run,
                prune,
                force,
            } => {
                let mut body = json!({
                    "dry_run": dry_run,
                    "prune": prune,
                    "force": force,
                });
                if let Some(r) = revision {
                    body["revision"] = json!(r);
                }
                c.post(&format!("/api/deploy/apps/{name}/sync"), body).await
            }
            DeployCmd::Rollback {
                name,
                history_id,
                prune,
                dry_run,
            } => {
                let body = json!({
                    "history_id": history_id,
                    "prune": prune,
                    "dry_run": dry_run,
                });
                c.post(&format!("/api/deploy/apps/{name}/rollback"), body).await
            }
            DeployCmd::Health => c.get("/api/deploy/health").await,
            DeployCmd::Project { cmd } => match cmd {
                DeployProjectCmd::List => c.get("/api/deploy/projects").await,
                DeployProjectCmd::Get { name } => {
                    c.get(&format!("/api/deploy/projects/{name}")).await
                }
                DeployProjectCmd::Delete { name } => {
                    c.delete(&format!("/api/deploy/projects/{name}")).await
                }
            },
        },
        Commands::Dns { cmd } => match cmd {
            DnsCmd::Zones => c.get("/api/dns/zones").await,
            DnsCmd::Records => c.get("/api/dns/records").await,
            DnsCmd::Query => c.get("/api/dns/query").await,
        },
        Commands::Erp { cmd } => match cmd {
            ErpCmd::Invoices   => c.get("/api/erp/invoices").await,
            ErpCmd::Customers  => c.get("/api/erp/customers").await,
            ErpCmd::Ledger     => c.get("/api/erp/ledger").await,
            ErpCmd::Inventory  => c.get("/api/erp/inventory").await,
            ErpCmd::Accounting => c.get("/api/erp/accounting").await,
            ErpCmd::Hr         => c.get("/api/erp/hr").await,
            ErpCmd::Projects   => c.get("/api/erp/projects").await,
        },
        Commands::Ha { cmd } => match cmd {
            HaCmd::Status => c.get("/api/ha/status").await,
            HaCmd::Failovers => c.get("/api/ha/failovers").await,
            HaCmd::Trigger => c.post("/api/ha/trigger", json!({})).await,
        },
        Commands::Knative { cmd } => match cmd {
            KnativeServiceCmd::Services => c.get("/api/knative/services").await,
            KnativeServiceCmd::Revisions => c.get("/api/knative/revisions").await,
            KnativeServiceCmd::Routes => c.get("/api/knative/routes").await,
        },
        Commands::LlmGateway { cmd } => match cmd {
            LlmGwCmd::Routes => c.get("/api/llm-gateway/routes").await,
            LlmGwCmd::Usage => c.get("/api/llm-gateway/usage").await,
            LlmGwCmd::Limits => c.get("/api/llm-gateway/limits").await,
        },
        Commands::Logs { cmd } => match cmd {
            LogsCmd::Streams => c.get("/api/logs/streams").await,
            LogsCmd::Query => c.get("/api/logs/query").await,
            LogsCmd::Sinks => c.get("/api/logs/sinks").await,
        },
        Commands::Metrics { cmd } => match cmd {
            MetricsCmd::Series => c.get("/api/metrics/series").await,
            MetricsCmd::Query => c.get("/api/metrics/query").await,
            MetricsCmd::Scrapers => c.get("/api/metrics/scrapers").await,
        },
        Commands::Pipelines { cmd } => match cmd {
            PipelinesCmd::List => c.get("/api/pipelines/list").await,
            PipelinesCmd::Runs => c.get("/api/pipelines/runs").await,
            PipelinesCmd::Trigger => c.post("/api/pipelines/trigger", json!({})).await,
        },
        Commands::RdbmsEngine { cmd } => match cmd {
            RdbmsEngineCmd::Query => c.get("/api/rdbms/query").await,
            RdbmsEngineCmd::Stats => c.get("/api/rdbms/stats").await,
            RdbmsEngineCmd::Schemas => c.get("/api/rdbms/schemas").await,
        },
        Commands::Rollouts { cmd } => match cmd {
            RolloutsCmd::Status => c.get("/api/rollouts/status").await,
            RolloutsCmd::Promote => c.post("/api/rollouts/promote", json!({})).await,
            RolloutsCmd::Abort => c.post("/api/rollouts/abort", json!({})).await,
        },
        Commands::Security { cmd } => match cmd {
            SecurityCmd::Events => c.get("/api/security/events").await,
            SecurityCmd::Policies => c.get("/api/security/policies").await,
            SecurityCmd::Audit => c.get("/api/security/audit").await,
        },
        Commands::Store { cmd } => match cmd {
            StoreCmd::Buckets => c.get("/api/store/buckets").await,
            StoreCmd::Objects => c.get("/api/store/objects").await,
            StoreCmd::Policies => c.get("/api/store/policies").await,
        },
        Commands::Trace { cmd } => match cmd {
            TraceCmd::Services => c.get("/api/trace/services").await,
            TraceCmd::TraceId => c.get("/api/trace/trace-id").await,
            TraceCmd::Search => c.get("/api/trace/search").await,
        },
        Commands::Tracker { cmd } => match cmd {
            TrackerCmd::Issues => c.get("/api/tracker/issues").await,
            TrackerCmd::Create => c.post("/api/tracker/create", json!({})).await,
            TrackerCmd::Transition => c.post("/api/tracker/transition", json!({})).await,
        },
        Commands::Upstream { cmd } => match cmd {
            UpstreamCmd::List => c.get("/api/upstream/list").await,
            UpstreamCmd::Check => c.post("/api/upstream/check", json!({})).await,
            UpstreamCmd::Bump => c.post("/api/upstream/bump", json!({})).await,
        },
        Commands::Admission { cmd } => match cmd {
            AdmissionCmd::Decisions => c.get("/api/admission/decisions").await,
            AdmissionCmd::Policies => c.get("/api/admission/policies").await,
            AdmissionCmd::Audit => c.get("/api/admission/audit").await,
        },
        Commands::Cdc { cmd } => match cmd {
            CdcCmd::Pipelines => c.get("/api/cdc/pipelines").await,
            CdcCmd::Lag => c.get("/api/cdc/lag").await,
            CdcCmd::Snapshot => c.get("/api/cdc/snapshot").await,
        },
        Commands::Certs { cmd } => match cmd {
            CertsCmd::List => c.get("/api/certs/list").await,
            CertsCmd::Issue => c.get("/api/certs/issue").await,
            CertsCmd::Renew => c.get("/api/certs/renew").await,
        },
        Commands::Crm { cmd } => match cmd {
            CrmCmd::Accounts      => c.get("/api/crm/accounts").await,
            CrmCmd::Contacts      => c.get("/api/crm/contacts").await,
            CrmCmd::Opportunities => c.get("/api/crm/opportunities").await,
            CrmCmd::Activities    => c.get("/api/crm/activities").await,
            CrmCmd::Workflows     => c.get("/api/crm/workflows").await,
            CrmCmd::Reports       => c.get("/api/crm/reports").await,
        },
        Commands::Crossplane { cmd } => match cmd {
            CrossplaneCmd::Claims => c.get("/api/crossplane/claims").await,
            CrossplaneCmd::Compositions => c.get("/api/crossplane/compositions").await,
            CrossplaneCmd::Providers => c.get("/api/crossplane/providers").await,
        },
        Commands::GitopsConfig { cmd } => match cmd {
            GitopsCmd::Apps => c.get("/api/gitops-config/apps").await,
            GitopsCmd::Sync => c.get("/api/gitops-config/sync").await,
            GitopsCmd::Diff => c.get("/api/gitops-config/diff").await,
        },
        Commands::Karpenter { cmd } => match cmd {
            KarpenterCmd::Nodepools => c.get("/api/karpenter/nodepools").await,
            KarpenterCmd::Nodeclaims => c.get("/api/karpenter/nodeclaims").await,
            KarpenterCmd::Drift => c.get("/api/karpenter/drift").await,
        },
        Commands::Kubevirt { cmd } => match cmd {
            KubevirtCmd::Vms => c.get("/api/kubevirt/vms").await,
            KubevirtCmd::Vmis => c.get("/api/kubevirt/vmis").await,
            KubevirtCmd::Migrate => c.get("/api/kubevirt/migrate").await,
        },
        Commands::Ledger { cmd } => match cmd {
            LedgerCmd::Entries => c.get("/api/ledger/entries").await,
            LedgerCmd::Verify => c.get("/api/ledger/verify").await,
            LedgerCmd::Export => c.get("/api/ledger/export").await,
        },
        Commands::Oncall { cmd } => match cmd {
            OncallCmd::Shifts => c.get("/api/oncall/shifts").await,
            OncallCmd::Rotations => c.get("/api/oncall/rotations").await,
            OncallCmd::Incidents => c.get("/api/oncall/incidents").await,
        },
        Commands::Search { cmd } => match cmd {
            SearchCmd::Indexes => c.get("/api/search/indexes").await,
            SearchCmd::Query => c.get("/api/search/query").await,
            SearchCmd::Reindex => c.get("/api/search/reindex").await,
        },

        // ── Batch 4 (2026-05-13): new subcommand groups ────────────────────────
        Commands::Spark { cmd } => match cmd {
            SparkCmd::Applications => c.get("/api/spark/applications").await,
            SparkCmd::Scheduled    => c.get("/api/spark/scheduled").await,
            SparkCmd::History      => c.get("/api/spark/history").await,
            SparkCmd::Namespaces   => c.get("/api/spark/namespaces").await,
            SparkCmd::Status       => c.get("/api/spark/status").await,
        },
        Commands::Jupyter { cmd } => match cmd {
            JupyterCmd::Servers      => c.get("/api/jupyter/servers").await,
            JupyterCmd::Kernels      => c.get("/api/jupyter/kernels").await,
            JupyterCmd::Notebooks    => c.get("/api/jupyter/notebooks").await,
            JupyterCmd::Environments => c.get("/api/jupyter/environments").await,
            JupyterCmd::Sessions     => c.get("/api/jupyter/sessions").await,
        },
        Commands::Mlflow { cmd } => match cmd {
            MlflowCmd::Experiments => c.get("/api/mlflow/experiments").await,
            MlflowCmd::Runs        => c.get("/api/mlflow/runs").await,
            MlflowCmd::Models      => c.get("/api/mlflow/models").await,
            MlflowCmd::Versions    => c.get("/api/mlflow/versions").await,
            MlflowCmd::Deployments => c.get("/api/mlflow/deployments").await,
        },
        Commands::Flux { cmd } => match cmd {
            FluxCmd::HelmReleases   => c.get("/api/flux/helmreleases").await,
            FluxCmd::Kustomizations => c.get("/api/flux/kustomizations").await,
            FluxCmd::Sources        => c.get("/api/flux/sources").await,
            FluxCmd::Images         => c.get("/api/flux/images").await,
            FluxCmd::Notifications  => c.get("/api/flux/notifications").await,
        },

        // ── eksik-sweep 2026-05-24: SPIRE identity (HTTP) + CIS bench (in-process)
        Commands::Identity { cmd } => match cmd {
            IdentityCmd::Entries    => c.get("/api/identity/entries").await,
            IdentityCmd::Agents     => c.get("/api/identity/agents").await,
            IdentityCmd::Bundle     => c.get("/api/identity/bundle").await,
            IdentityCmd::Federation => c.get("/api/identity/federation").await,
            IdentityCmd::OidcKeys   => c.get("/api/identity/oidc/keys").await,
        },
        Commands::Bench { cmd } => {
            use cave_bench::cli::{BenchSubcommand, dispatch as bench_dispatch};
            use cave_bench::report::Format as BenchFormat;
            let sub = match cmd {
                BenchCmd::Profiles => BenchSubcommand::Profiles,
                BenchCmd::Checks { framework } => BenchSubcommand::Checks { framework },
                BenchCmd::Scan { profile, host, format } => {
                    let fmt = match format.as_str() {
                        "json" => BenchFormat::Json,
                        "sarif" => BenchFormat::Sarif,
                        _ => BenchFormat::Markdown,
                    };
                    BenchSubcommand::Scan { profile_id: profile, host, format: fmt }
                }
                BenchCmd::Schedules => BenchSubcommand::Schedules,
                BenchCmd::Observability => BenchSubcommand::Observability,
            };
            let out = bench_dispatch(sub).map_err(|e| anyhow::anyhow!("cave-bench: {e}"))?;
            print!("{out}");
            Ok(())
        }
        Commands::Falco { cmd } => {
            use cave_falco::cli::{FalcoSubcommand, dispatch as falco_dispatch};
            let sub = match cmd {
                FalcoCmd::RulesParse { path } => FalcoSubcommand::RulesParse { path },
                FalcoCmd::RulesListBuiltin => FalcoSubcommand::RulesListBuiltin,
                FalcoCmd::Observability => FalcoSubcommand::Observability,
                FalcoCmd::Version => FalcoSubcommand::Version,
            };
            let out = falco_dispatch(sub).map_err(|e| anyhow::anyhow!("cave-falco: {e}"))?;
            print!("{out}");
            Ok(())
        }

        Commands::Keda { cmd } => match cmd {
            KedaCmd::ScaledObjects => c.get("/api/keda/scaledobjects").await,
            KedaCmd::Get { name } => {
                c.get(&format!("/api/keda/scaledobjects/{}", urlencode(&name))).await
            }
            KedaCmd::Scalers => c.get("/api/keda/scalers").await,
            KedaCmd::Metrics { name } => {
                c.get(&format!(
                    "/api/keda/scaledobjects/{}/metrics",
                    urlencode(&name)
                ))
                .await
            }
            KedaCmd::History { limit } => {
                c.get(&format!("/api/keda/events?limit={limit}")).await
            }
            KedaCmd::Pause { name } => {
                c.post(
                    &format!("/api/keda/scaledobjects/{}/pause", urlencode(&name)),
                    json!({}),
                )
                .await
            }
            KedaCmd::Resume { name } => {
                c.post(
                    &format!("/api/keda/scaledobjects/{}/resume", urlencode(&name)),
                    json!({}),
                )
                .await
            }
            KedaCmd::TriggerAuthList => c.get("/api/keda/triggerauth").await,
            KedaCmd::TriggerAuthGet { name } => {
                c.get(&format!("/api/keda/triggerauth/{}", urlencode(&name))).await
            }
            KedaCmd::ScaledJobList => c.get("/api/keda/scaledjobs").await,
            KedaCmd::ScaledJobGet { name } => {
                c.get(&format!("/api/keda/scaledjobs/{}", urlencode(&name))).await
            }
            KedaCmd::ScalerStats { name } => {
                c.get(&format!(
                    "/api/keda/scaledobjects/{}/scaler-stats",
                    urlencode(&name)
                ))
                .await
            }
            // ── 2026-05-12: admin-portal-backed paths ──────────────────
            KedaCmd::ScaledObjectDescribe { namespace, name } => {
                c.get(&format!(
                    "/admin/keda/scaledobjects/{}/{}",
                    urlencode(&namespace),
                    urlencode(&name)
                ))
                .await
            }
            KedaCmd::ScaledObjectApply { file } => {
                let body = tokio::fs::read_to_string(&file).await.map_err(|e| {
                    anyhow::anyhow!("Failed to read {file}: {e}")
                })?;
                c.post("/api/keda/scaledobjects", json!({ "yaml": body })).await
            }
            KedaCmd::ScaledObjectDelete { namespace, name } => {
                c.post(
                    &format!(
                        "/admin/keda/scaledobjects/{}/{}/delete",
                        urlencode(&namespace),
                        urlencode(&name)
                    ),
                    json!({}),
                )
                .await
            }
            KedaCmd::ScaledJobDescribe { namespace, name } => {
                c.get(&format!(
                    "/admin/keda/scaledjobs/{}/{}",
                    urlencode(&namespace),
                    urlencode(&name)
                ))
                .await
            }
            KedaCmd::ScalerDetail { kind } => {
                c.get(&format!("/admin/keda/scalers/{}", urlencode(&kind))).await
            }
            KedaCmd::ScalerMetrics => c.get("/admin/keda/metrics").await,
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
    use base64::{Engine, engine::general_purpose::STANDARD};
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
        "pvc" | "persistentvolumeclaim" | "persistentvolumeclaims" => {
            Some("persistentvolumeclaims")
        }
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

#[cfg(test)]
mod artifacts_parse_tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap_or_else(|e| panic!("parse {args:?}: {e}"))
    }

    #[test]
    fn artifacts_health_parses() {
        let cli = parse(&["cavectl", "artifacts", "health"]);
        assert!(matches!(
            cli.command,
            Commands::Artifacts {
                cmd: ArtifactsCmd::Health
            }
        ));
    }

    #[test]
    fn artifacts_harbor_projects_parses() {
        let cli = parse(&["cavectl", "artifacts", "harbor", "projects"]);
        match cli.command {
            Commands::Artifacts {
                cmd:
                    ArtifactsCmd::Harbor {
                        cmd: HarborCmd::Projects,
                    },
            } => {}
            other => panic!(
                "wrong variant: {other:?}",
                other = std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn artifacts_harbor_project_create_requires_name() {
        let cli = parse(&[
            "cavectl",
            "artifacts",
            "harbor",
            "project-create",
            "--name",
            "alpha",
        ]);
        match cli.command {
            Commands::Artifacts {
                cmd:
                    ArtifactsCmd::Harbor {
                        cmd: HarborCmd::ProjectCreate { name },
                    },
            } => {
                assert_eq!(name, "alpha");
            }
            _ => panic!("wrong variant"),
        }
        // Missing --name must be a parse error.
        assert!(
            Cli::try_parse_from(&["cavectl", "artifacts", "harbor", "project-create"]).is_err()
        );
    }

    #[test]
    fn artifacts_harbor_scan_takes_digest() {
        let cli = parse(&[
            "cavectl",
            "artifacts",
            "harbor",
            "scan",
            "--digest",
            "sha256:abcd",
        ]);
        match cli.command {
            Commands::Artifacts {
                cmd:
                    ArtifactsCmd::Harbor {
                        cmd: HarborCmd::Scan { digest },
                    },
            } => {
                assert_eq!(digest, "sha256:abcd");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn artifacts_pulp_sync_requires_repository_and_remote() {
        let cli = parse(&[
            "cavectl",
            "artifacts",
            "pulp",
            "sync",
            "--repository",
            "r",
            "--remote",
            "u",
        ]);
        match cli.command {
            Commands::Artifacts {
                cmd:
                    ArtifactsCmd::Pulp {
                        cmd: PulpCmd::Sync { repository, remote },
                    },
            } => {
                assert_eq!(repository, "r");
                assert_eq!(remote, "u");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn artifacts_pulp_distribute_takes_base_path() {
        let cli = parse(&[
            "cavectl",
            "artifacts",
            "pulp",
            "distribute",
            "--publication",
            "p",
            "--base-path",
            "/x/y",
        ]);
        match cli.command {
            Commands::Artifacts {
                cmd:
                    ArtifactsCmd::Pulp {
                        cmd:
                            PulpCmd::Distribute {
                                publication,
                                base_path,
                            },
                    },
            } => {
                assert_eq!(publication, "p");
                assert_eq!(base_path, "/x/y");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn artifacts_nexus_repo_create_defaults_to_raw() {
        let cli = parse(&[
            "cavectl",
            "artifacts",
            "nexus",
            "repo-create",
            "--name",
            "rel",
        ]);
        match cli.command {
            Commands::Artifacts {
                cmd:
                    ArtifactsCmd::Nexus {
                        cmd: NexusCmd::RepoCreate { name, format_kind },
                    },
            } => {
                assert_eq!(name, "rel");
                assert_eq!(format_kind, "raw");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn artifacts_nexus_repo_create_accepts_other_formats() {
        let cli = parse(&[
            "cavectl",
            "artifacts",
            "nexus",
            "repo-create",
            "--name",
            "mvn",
            "--repo-format",
            "maven2",
        ]);
        match cli.command {
            Commands::Artifacts {
                cmd:
                    ArtifactsCmd::Nexus {
                        cmd: NexusCmd::RepoCreate { format_kind, .. },
                    },
            } => {
                assert_eq!(format_kind, "maven2");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn artifacts_nexus_upload_collects_three_args() {
        let cli = parse(&[
            "cavectl",
            "artifacts",
            "nexus",
            "upload",
            "--repository",
            "r",
            "--path",
            "dir/f.bin",
            "--file",
            "/tmp/f.bin",
        ]);
        match cli.command {
            Commands::Artifacts {
                cmd:
                    ArtifactsCmd::Nexus {
                        cmd:
                            NexusCmd::Upload {
                                repository,
                                path,
                                file,
                            },
                    },
            } => {
                assert_eq!(repository, "r");
                assert_eq!(path, "dir/f.bin");
                assert_eq!(file, "/tmp/f.bin");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn artifacts_cosign_keypair_defaults_to_ecdsa() {
        let cli = parse(&["cavectl", "artifacts", "cosign", "keypair"]);
        match cli.command {
            Commands::Artifacts {
                cmd:
                    ArtifactsCmd::Cosign {
                        cmd: CosignCmd::Keypair { alg },
                    },
            } => {
                assert_eq!(alg, "ecdsa-p256");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn artifacts_cosign_keypair_accepts_pqc_alg() {
        let cli = parse(&[
            "cavectl",
            "artifacts",
            "cosign",
            "keypair",
            "--alg",
            "ml-dsa-65",
        ]);
        match cli.command {
            Commands::Artifacts {
                cmd:
                    ArtifactsCmd::Cosign {
                        cmd: CosignCmd::Keypair { alg },
                    },
            } => {
                assert_eq!(alg, "ml-dsa-65");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn artifacts_cosign_sign_collects_key_ref_digest() {
        let cli = parse(&[
            "cavectl",
            "artifacts",
            "cosign",
            "sign",
            "--key-id",
            "k1",
            "--reference",
            "registry/img:tag",
            "--digest",
            "sha256:abc",
        ]);
        match cli.command {
            Commands::Artifacts {
                cmd:
                    ArtifactsCmd::Cosign {
                        cmd:
                            CosignCmd::Sign {
                                key_id,
                                reference,
                                digest,
                            },
                    },
            } => {
                assert_eq!(key_id, "k1");
                assert_eq!(reference, "registry/img:tag");
                assert_eq!(digest, "sha256:abc");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn artifacts_cosign_counters_parses() {
        let cli = parse(&["cavectl", "artifacts", "cosign", "counters"]);
        assert!(matches!(
            cli.command,
            Commands::Artifacts {
                cmd: ArtifactsCmd::Cosign {
                    cmd: CosignCmd::Counters
                }
            }
        ));
    }

    #[test]
    fn registry_subcommand_still_works_for_back_compat() {
        let cli = parse(&["cavectl", "registry", "list"]);
        assert!(matches!(
            cli.command,
            Commands::Registry {
                cmd: RegistryCmd::List
            }
        ));
    }
}

#[cfg(test)]
mod batch4_parse_tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap_or_else(|e| panic!("parse {args:?}: {e}"))
    }

    // ── Expanded auth subcommands ─────────────────────────────────────────────
    #[test]
    fn auth_realms_parses() {
        let cli = parse(&["cavectl", "auth", "realms"]);
        assert!(matches!(
            cli.command,
            Commands::Auth {
                cmd: AuthCmd::Realms
            }
        ));
    }

    #[test]
    fn auth_clients_parses() {
        let cli = parse(&["cavectl", "auth", "clients"]);
        assert!(matches!(
            cli.command,
            Commands::Auth {
                cmd: AuthCmd::Clients
            }
        ));
    }

    #[test]
    fn auth_events_parses() {
        let cli = parse(&["cavectl", "auth", "events"]);
        assert!(matches!(
            cli.command,
            Commands::Auth {
                cmd: AuthCmd::Events
            }
        ));
    }

    #[test]
    fn auth_saml_metadata_parses() {
        let cli = parse(&["cavectl", "auth", "saml-metadata"]);
        assert!(matches!(
            cli.command,
            Commands::Auth {
                cmd: AuthCmd::SamlMetadata
            }
        ));
    }

    #[test]
    fn auth_saml_verify_request_parses() {
        let cli = parse(&["cavectl", "auth", "saml-verify-request"]);
        assert!(matches!(
            cli.command,
            Commands::Auth {
                cmd: AuthCmd::SamlVerifyRequest
            }
        ));
    }

    #[test]
    fn auth_saml_c14n_parses() {
        let cli = parse(&["cavectl", "auth", "saml-c14n"]);
        assert!(matches!(
            cli.command,
            Commands::Auth {
                cmd: AuthCmd::SamlC14n
            }
        ));
    }

    // ── LDAP federation + Kerberos/SPNEGO sub-commands ───────────────────────
    #[test]
    fn auth_ldap_test_connection_parses() {
        let cli = parse(&["cavectl", "auth", "ldap-test-connection"]);
        assert!(matches!(
            cli.command,
            Commands::Auth {
                cmd: AuthCmd::LdapTestConnection
            }
        ));
    }

    #[test]
    fn auth_ldap_sync_users_parses() {
        let cli = parse(&["cavectl", "auth", "ldap-sync-users"]);
        assert!(matches!(
            cli.command,
            Commands::Auth {
                cmd: AuthCmd::LdapSyncUsers
            }
        ));
    }

    #[test]
    fn auth_ldap_sync_groups_parses() {
        let cli = parse(&["cavectl", "auth", "ldap-sync-groups"]);
        assert!(matches!(
            cli.command,
            Commands::Auth {
                cmd: AuthCmd::LdapSyncGroups
            }
        ));
    }

    #[test]
    fn auth_kerberos_validate_keytab_parses() {
        let cli = parse(&["cavectl", "auth", "kerberos-validate-keytab"]);
        assert!(matches!(
            cli.command,
            Commands::Auth {
                cmd: AuthCmd::KerberosValidateKeytab
            }
        ));
    }

    #[test]
    fn auth_kerberos_test_spnego_parses() {
        let cli = parse(&["cavectl", "auth", "kerberos-test-spnego"]);
        assert!(matches!(
            cli.command,
            Commands::Auth {
                cmd: AuthCmd::KerberosTestSpnego
            }
        ));
    }

    // ── Expanded container-scan subcommands ───────────────────────────────────
    #[test]
    fn container_scan_vulnerabilities_parses() {
        let cli = parse(&["cavectl", "container-scan", "vulnerabilities"]);
        assert!(matches!(
            cli.command,
            Commands::ContainerScan {
                cmd: ContainerScanCmd::Vulnerabilities
            }
        ));
    }

    #[test]
    fn container_scan_images_parses() {
        let cli = parse(&["cavectl", "container-scan", "images"]);
        assert!(matches!(
            cli.command,
            Commands::ContainerScan {
                cmd: ContainerScanCmd::Images
            }
        ));
    }

    #[test]
    fn container_scan_policies_parses() {
        let cli = parse(&["cavectl", "container-scan", "policies"]);
        assert!(matches!(
            cli.command,
            Commands::ContainerScan {
                cmd: ContainerScanCmd::Policies
            }
        ));
    }

    #[test]
    fn container_scan_history_parses() {
        let cli = parse(&["cavectl", "container-scan", "history"]);
        assert!(matches!(
            cli.command,
            Commands::ContainerScan {
                cmd: ContainerScanCmd::History
            }
        ));
    }

    #[test]
    fn container_scan_reports_parses() {
        let cli = parse(&["cavectl", "container-scan", "reports"]);
        assert!(matches!(
            cli.command,
            Commands::ContainerScan {
                cmd: ContainerScanCmd::Reports
            }
        ));
    }

    // ── Expanded erp subcommands ──────────────────────────────────────────────
    #[test]
    fn erp_inventory_parses() {
        let cli = parse(&["cavectl", "erp", "inventory"]);
        assert!(matches!(
            cli.command,
            Commands::Erp {
                cmd: ErpCmd::Inventory
            }
        ));
    }

    #[test]
    fn erp_accounting_parses() {
        let cli = parse(&["cavectl", "erp", "accounting"]);
        assert!(matches!(
            cli.command,
            Commands::Erp {
                cmd: ErpCmd::Accounting
            }
        ));
    }

    #[test]
    fn erp_hr_parses() {
        let cli = parse(&["cavectl", "erp", "hr"]);
        assert!(matches!(cli.command, Commands::Erp { cmd: ErpCmd::Hr }));
    }

    #[test]
    fn erp_projects_parses() {
        let cli = parse(&["cavectl", "erp", "projects"]);
        assert!(matches!(
            cli.command,
            Commands::Erp {
                cmd: ErpCmd::Projects
            }
        ));
    }

    // ── Expanded crm subcommands ──────────────────────────────────────────────
    #[test]
    fn crm_activities_parses() {
        let cli = parse(&["cavectl", "crm", "activities"]);
        assert!(matches!(
            cli.command,
            Commands::Crm {
                cmd: CrmCmd::Activities
            }
        ));
    }

    #[test]
    fn crm_workflows_parses() {
        let cli = parse(&["cavectl", "crm", "workflows"]);
        assert!(matches!(
            cli.command,
            Commands::Crm {
                cmd: CrmCmd::Workflows
            }
        ));
    }

    #[test]
    fn crm_reports_parses() {
        let cli = parse(&["cavectl", "crm", "reports"]);
        assert!(matches!(
            cli.command,
            Commands::Crm {
                cmd: CrmCmd::Reports
            }
        ));
    }

    // ── streams kraft subcommands ─────────────────────────────────────────────
    #[test]
    fn streams_kraft_describe_quorum_parses() {
        let cli = parse(&["cavectl", "streams", "kraft", "describe-quorum"]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Kraft {
                        cmd: KraftCmd::DescribeQuorum,
                    },
            } => {}
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_kraft_voters_parses() {
        let cli = parse(&["cavectl", "streams", "kraft", "voters"]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Kraft {
                        cmd: KraftCmd::Voters,
                    },
            } => {}
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_kraft_leader_parses() {
        let cli = parse(&["cavectl", "streams", "kraft", "leader"]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Kraft {
                        cmd: KraftCmd::Leader,
                    },
            } => {}
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_kraft_snapshot_parses() {
        let cli = parse(&["cavectl", "streams", "kraft", "snapshot"]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Kraft {
                        cmd: KraftCmd::Snapshot,
                    },
            } => {}
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    // ── streams connect subcommands ───────────────────────────────────────────
    #[test]
    fn streams_connect_worker_list_parses() {
        let cli = parse(&["cavectl", "streams", "connect", "worker", "list"]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Connect {
                        cmd:
                            ConnectCmd::Worker {
                                cmd: ConnectWorkerCmd::List,
                            },
                    },
            } => {}
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_connect_worker_status_parses() {
        let cli = parse(&[
            "cavectl", "streams", "connect", "worker", "status", "worker-1",
        ]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Connect {
                        cmd:
                            ConnectCmd::Worker {
                                cmd: ConnectWorkerCmd::Status { id },
                            },
                    },
            } => assert_eq!(id, "worker-1"),
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_connect_connector_list_parses() {
        let cli = parse(&["cavectl", "streams", "connect", "connector", "list"]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Connect {
                        cmd:
                            ConnectCmd::Connector {
                                cmd: ConnectConnectorCmd::List,
                            },
                    },
            } => {}
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_connect_connector_get_parses() {
        let cli = parse(&["cavectl", "streams", "connect", "connector", "get", "jdbc"]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Connect {
                        cmd:
                            ConnectCmd::Connector {
                                cmd: ConnectConnectorCmd::Get { name },
                            },
                    },
            } => assert_eq!(name, "jdbc"),
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_connect_connector_offsets_parses() {
        let cli = parse(&[
            "cavectl",
            "streams",
            "connect",
            "connector",
            "offsets",
            "jdbc",
        ]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Connect {
                        cmd:
                            ConnectCmd::Connector {
                                cmd: ConnectConnectorCmd::Offsets { name },
                            },
                    },
            } => assert_eq!(name, "jdbc"),
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_connect_connector_create_with_multiple_configs_parses() {
        let cli = parse(&[
            "cavectl",
            "streams",
            "connect",
            "connector",
            "create",
            "jdbc",
            "--config",
            "connector.class=...JdbcSource",
            "--config",
            "tasks.max=2",
            "--config",
            "topics=orders,refunds",
        ]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Connect {
                        cmd:
                            ConnectCmd::Connector {
                                cmd: ConnectConnectorCmd::Create { name, configs },
                            },
                    },
            } => {
                assert_eq!(name, "jdbc");
                assert_eq!(configs.len(), 3);
                assert!(configs.iter().any(|c| c == "topics=orders,refunds"));
            }
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_connect_connector_delete_parses() {
        let cli = parse(&[
            "cavectl",
            "streams",
            "connect",
            "connector",
            "delete",
            "jdbc",
        ]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Connect {
                        cmd:
                            ConnectCmd::Connector {
                                cmd: ConnectConnectorCmd::Delete { name },
                            },
                    },
            } => assert_eq!(name, "jdbc"),
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_connect_connector_pause_parses() {
        let cli = parse(&[
            "cavectl",
            "streams",
            "connect",
            "connector",
            "pause",
            "jdbc",
        ]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Connect {
                        cmd:
                            ConnectCmd::Connector {
                                cmd: ConnectConnectorCmd::Pause { name },
                            },
                    },
            } => assert_eq!(name, "jdbc"),
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_connect_connector_resume_parses() {
        let cli = parse(&[
            "cavectl",
            "streams",
            "connect",
            "connector",
            "resume",
            "jdbc",
        ]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Connect {
                        cmd:
                            ConnectCmd::Connector {
                                cmd: ConnectConnectorCmd::Resume { name },
                            },
                    },
            } => assert_eq!(name, "jdbc"),
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_connect_connector_restart_parses() {
        let cli = parse(&[
            "cavectl",
            "streams",
            "connect",
            "connector",
            "restart",
            "jdbc",
        ]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Connect {
                        cmd:
                            ConnectCmd::Connector {
                                cmd: ConnectConnectorCmd::Restart { name },
                            },
                    },
            } => assert_eq!(name, "jdbc"),
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_connect_task_list_parses() {
        let cli = parse(&["cavectl", "streams", "connect", "task", "list", "jdbc"]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Connect {
                        cmd:
                            ConnectCmd::Task {
                                cmd: ConnectTaskCmd::List { connector },
                            },
                    },
            } => assert_eq!(connector, "jdbc"),
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_connect_task_status_parses() {
        let cli = parse(&[
            "cavectl", "streams", "connect", "task", "status", "jdbc", "0",
        ]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Connect {
                        cmd:
                            ConnectCmd::Task {
                                cmd: ConnectTaskCmd::Status { connector, task },
                            },
                    },
            } => {
                assert_eq!(connector, "jdbc");
                assert_eq!(task, 0);
            }
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_connect_task_restart_parses() {
        let cli = parse(&[
            "cavectl", "streams", "connect", "task", "restart", "jdbc", "1",
        ]);
        match cli.command {
            Commands::Streams {
                cmd:
                    StreamsCmd::Connect {
                        cmd:
                            ConnectCmd::Task {
                                cmd: ConnectTaskCmd::Restart { connector, task },
                            },
                    },
            } => {
                assert_eq!(connector, "jdbc");
                assert_eq!(task, 1);
            }
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn streams_connect_global_format_flag_applies() {
        let cli = parse(&[
            "cavectl",
            "--format",
            "json",
            "streams",
            "connect",
            "connector",
            "list",
        ]);
        assert!(matches!(cli.format, Format::Json));
    }

    #[test]
    fn streams_connect_yaml_format_applies() {
        let cli = parse(&[
            "cavectl", "--format", "yaml", "streams", "connect", "worker", "list",
        ]);
        assert!(matches!(cli.format, Format::Yaml));
    }

    #[test]
    fn streams_connect_table_default_format() {
        let cli = parse(&["cavectl", "streams", "connect", "task", "list", "jdbc"]);
        assert!(matches!(cli.format, Format::Table));
    }

    // ── New top-level groups ──────────────────────────────────────────────────
    #[test]
    fn spark_applications_parses() {
        let cli = parse(&["cavectl", "spark", "applications"]);
        assert!(matches!(
            cli.command,
            Commands::Spark {
                cmd: SparkCmd::Applications
            }
        ));
    }

    #[test]
    fn spark_scheduled_parses() {
        let cli = parse(&["cavectl", "spark", "scheduled"]);
        assert!(matches!(
            cli.command,
            Commands::Spark {
                cmd: SparkCmd::Scheduled
            }
        ));
    }

    #[test]
    fn jupyter_servers_parses() {
        let cli = parse(&["cavectl", "jupyter", "servers"]);
        assert!(matches!(
            cli.command,
            Commands::Jupyter {
                cmd: JupyterCmd::Servers
            }
        ));
    }

    #[test]
    fn jupyter_kernels_parses() {
        let cli = parse(&["cavectl", "jupyter", "kernels"]);
        assert!(matches!(
            cli.command,
            Commands::Jupyter {
                cmd: JupyterCmd::Kernels
            }
        ));
    }

    #[test]
    fn mlflow_experiments_parses() {
        let cli = parse(&["cavectl", "mlflow", "experiments"]);
        assert!(matches!(
            cli.command,
            Commands::Mlflow {
                cmd: MlflowCmd::Experiments
            }
        ));
    }

    #[test]
    fn mlflow_models_parses() {
        let cli = parse(&["cavectl", "mlflow", "models"]);
        assert!(matches!(
            cli.command,
            Commands::Mlflow {
                cmd: MlflowCmd::Models
            }
        ));
    }

    #[test]
    fn flux_helmreleases_parses() {
        let cli = parse(&["cavectl", "flux", "helm-releases"]);
        assert!(matches!(
            cli.command,
            Commands::Flux {
                cmd: FluxCmd::HelmReleases
            }
        ));
    }

    #[test]
    fn flux_kustomizations_parses() {
        let cli = parse(&["cavectl", "flux", "kustomizations"]);
        assert!(matches!(
            cli.command,
            Commands::Flux {
                cmd: FluxCmd::Kustomizations
            }
        ));
    }

    #[test]
    fn flux_sources_parses() {
        let cli = parse(&["cavectl", "flux", "sources"]);
        assert!(matches!(
            cli.command,
            Commands::Flux {
                cmd: FluxCmd::Sources
            }
        ));
    }

    // ── 2026-05-15 polish — `cavectl portal audit` ────────────────

    #[test]
    fn portal_audit_parses_without_tenant_flag() {
        let cli = parse(&["cavectl", "portal", "audit"]);
        match cli.command {
            Commands::Portal {
                cmd: PortalCmd::Audit { tenant },
            } => {
                // env-default may be present; only assert variant.
                let _ = tenant;
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn portal_audit_accepts_explicit_tenant_flag() {
        let cli = parse(&["cavectl", "portal", "audit", "--tenant", "acme"]);
        match cli.command {
            Commands::Portal {
                cmd: PortalCmd::Audit { tenant },
            } => {
                assert_eq!(tenant.as_deref(), Some("acme"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn portal_status_still_parses_after_audit_was_added() {
        let cli = parse(&["cavectl", "portal", "status"]);
        assert!(matches!(
            cli.command,
            Commands::Portal {
                cmd: PortalCmd::Status
            }
        ));
    }

    // ── 2026-05-15 Trivy scan subcommands ────────────────────────

    #[test]
    fn scan_image_parses_with_defaults() {
        let cli = parse(&["cavectl", "scan", "image", "alpine:3.20"]);
        match cli.command {
            Commands::Scan {
                cmd:
                    ScanCmd::Image {
                        target,
                        report_format,
                        severity,
                    },
            } => {
                assert_eq!(target, "alpine:3.20");
                assert_eq!(report_format, "table");
                assert_eq!(severity, "MEDIUM");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn scan_image_accepts_sarif_format() {
        let cli = parse(&[
            "cavectl",
            "scan",
            "image",
            "alpine:3.20",
            "--report-format",
            "sarif",
        ]);
        match cli.command {
            Commands::Scan {
                cmd: ScanCmd::Image { report_format, .. },
            } => {
                assert_eq!(report_format, "sarif")
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn scan_image_accepts_severity_override() {
        let cli = parse(&["cavectl", "scan", "image", "img", "--severity", "CRITICAL"]);
        match cli.command {
            Commands::Scan {
                cmd: ScanCmd::Image { severity, .. },
            } => {
                assert_eq!(severity, "CRITICAL")
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn scan_fs_parses_path() {
        let cli = parse(&["cavectl", "scan", "fs", "/tmp/proj"]);
        match cli.command {
            Commands::Scan {
                cmd:
                    ScanCmd::Fs {
                        path,
                        report_format,
                    },
            } => {
                assert_eq!(path.to_string_lossy(), "/tmp/proj");
                assert_eq!(report_format, "table");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn scan_config_parses() {
        let cli = parse(&["cavectl", "scan", "config", "infra/"]);
        assert!(matches!(
            cli.command,
            Commands::Scan {
                cmd: ScanCmd::Config { .. }
            }
        ));
    }

    #[test]
    fn scan_secret_parses() {
        let cli = parse(&["cavectl", "scan", "secret", "."]);
        assert!(matches!(
            cli.command,
            Commands::Scan {
                cmd: ScanCmd::Secret { .. }
            }
        ));
    }

    #[test]
    fn scan_license_parses() {
        let cli = parse(&["cavectl", "scan", "license", "."]);
        assert!(matches!(
            cli.command,
            Commands::Scan {
                cmd: ScanCmd::License { .. }
            }
        ));
    }

    #[test]
    fn scan_sbom_cyclonedx_default() {
        let cli = parse(&["cavectl", "scan", "sbom", "alpine:3.20"]);
        match cli.command {
            Commands::Scan {
                cmd: ScanCmd::Sbom { report_format, .. },
            } => {
                assert_eq!(report_format, "cyclonedx")
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn scan_sbom_spdx_override() {
        let cli = parse(&[
            "cavectl",
            "scan",
            "sbom",
            "alpine:3.20",
            "--report-format",
            "spdx",
        ]);
        match cli.command {
            Commands::Scan {
                cmd: ScanCmd::Sbom { report_format, .. },
            } => {
                assert_eq!(report_format, "spdx")
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn scan_start_still_parses_after_trivy_added() {
        let cli = parse(&["cavectl", "scan", "start", "--repo", "github.com/x/y"]);
        assert!(matches!(
            cli.command,
            Commands::Scan {
                cmd: ScanCmd::Start { .. }
            }
        ));
    }

    #[test]
    fn scan_list_still_parses_after_trivy_added() {
        let cli = parse(&["cavectl", "scan", "list"]);
        assert!(matches!(cli.command, Commands::Scan { cmd: ScanCmd::List }));
    }

    #[test]
    fn scan_results_still_parses_after_trivy_added() {
        let cli = parse(&["cavectl", "scan", "results", "abc-1"]);
        assert!(matches!(
            cli.command,
            Commands::Scan {
                cmd: ScanCmd::Results { .. }
            }
        ));
    }
}

#[cfg(test)]
mod sbom_parse_tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap_or_else(|e| panic!("parse {args:?}: {e}"))
    }

    #[test]
    fn sbom_ingest_parses_with_file_arg() {
        let cli = parse(&["cavectl", "sbom", "ingest", "--file", "bom.json"]);
        match cli.command {
            Commands::Sbom {
                cmd: SbomCmd::Ingest { file, project_uuid },
            } => {
                assert_eq!(file, "bom.json");
                assert!(project_uuid.is_none());
            }
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn sbom_ingest_accepts_project_uuid_override() {
        let cli = parse(&[
            "cavectl",
            "sbom",
            "ingest",
            "--file",
            "bom.json",
            "--project-uuid",
            "uu-123",
        ]);
        match cli.command {
            Commands::Sbom {
                cmd: SbomCmd::Ingest { project_uuid, .. },
            } => {
                assert_eq!(project_uuid.as_deref(), Some("uu-123"));
            }
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn sbom_component_list_default_pagination() {
        let cli = parse(&["cavectl", "sbom", "component", "list"]);
        match cli.command {
            Commands::Sbom {
                cmd: SbomCmd::Component(SbomComponentCmd::List { page, page_size }),
            } => {
                assert_eq!(page, 1);
                assert_eq!(page_size, 50);
            }
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn sbom_component_get_takes_uuid() {
        let cli = parse(&["cavectl", "sbom", "component", "get", "uu-abc"]);
        match cli.command {
            Commands::Sbom {
                cmd: SbomCmd::Component(SbomComponentCmd::Get { uuid }),
            } => {
                assert_eq!(uuid, "uu-abc");
            }
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn sbom_project_create_requires_name() {
        let cli = parse(&[
            "cavectl", "sbom", "project", "create", "--name", "p", "--ver", "1.0",
        ]);
        match cli.command {
            Commands::Sbom {
                cmd: SbomCmd::Project(SbomProjectCmd::Create { name, version }),
            } => {
                assert_eq!(name, "p");
                assert_eq!(version.as_deref(), Some("1.0"));
            }
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn sbom_project_list_paginates() {
        let cli = parse(&[
            "cavectl",
            "sbom",
            "project",
            "list",
            "--page",
            "3",
            "--page-size",
            "25",
        ]);
        match cli.command {
            Commands::Sbom {
                cmd: SbomCmd::Project(SbomProjectCmd::List { page, page_size }),
            } => {
                assert_eq!(page, 3);
                assert_eq!(page_size, 25);
            }
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn sbom_vuln_get_takes_cve_id() {
        let cli = parse(&["cavectl", "sbom", "vuln", "get", "CVE-2024-12345"]);
        match cli.command {
            Commands::Sbom {
                cmd: SbomCmd::Vuln(SbomVulnCmd::Get { id }),
            } => {
                assert_eq!(id, "CVE-2024-12345");
            }
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn sbom_vuln_analyze_requires_state() {
        let cli = parse(&[
            "cavectl", "sbom", "vuln", "analyze", "CVE-1", "--state", "RESOLVED",
        ]);
        match cli.command {
            Commands::Sbom {
                cmd: SbomCmd::Vuln(SbomVulnCmd::Analyze { id, state }),
            } => {
                assert_eq!(id, "CVE-1");
                assert_eq!(state, "RESOLVED");
            }
            other => panic!("wrong variant: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn sbom_policy_list_parses() {
        let cli = parse(&["cavectl", "sbom", "policy", "list"]);
        assert!(matches!(
            cli.command,
            Commands::Sbom {
                cmd: SbomCmd::Policy(SbomPolicyCmd::List)
            }
        ));
    }

    #[test]
    fn sbom_portfolio_parses() {
        let cli = parse(&["cavectl", "sbom", "portfolio"]);
        assert!(matches!(
            cli.command,
            Commands::Sbom {
                cmd: SbomCmd::Portfolio
            }
        ));
    }

    #[test]
    fn sbom_legacy_list_still_parses() {
        let cli = parse(&["cavectl", "sbom", "list"]);
        assert!(matches!(cli.command, Commands::Sbom { cmd: SbomCmd::List }));
    }
}

#[cfg(test)]
mod vulns_parse_tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap_or_else(|e| panic!("parse {args:?}: {e}"))
    }

    #[test]
    fn legacy_scan_target_parses() {
        let cli = parse(&["cavectl", "vulns", "scan", "--target", "alpine:3"]);
        match cli.command {
            Commands::Vulns {
                cmd: VulnsCmd::Scan { target },
            } => assert_eq!(target, "alpine:3"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn vulns_health_parses() {
        let cli = parse(&["cavectl", "vulns", "health"]);
        assert!(matches!(
            cli.command,
            Commands::Vulns {
                cmd: VulnsCmd::Health
            }
        ));
    }

    #[test]
    fn vulns_scan_types_parses() {
        let cli = parse(&["cavectl", "vulns", "scan-types"]);
        assert!(matches!(
            cli.command,
            Commands::Vulns {
                cmd: VulnsCmd::ScanTypes
            }
        ));
    }

    #[test]
    fn vulns_finding_list_with_pagination_parses() {
        let cli = parse(&[
            "cavectl", "vulns", "finding", "list", "--limit", "50", "--offset", "10",
        ]);
        match cli.command {
            Commands::Vulns {
                cmd:
                    VulnsCmd::Finding {
                        cmd: VulnsFindingCmd::List { limit, offset },
                    },
            } => {
                assert_eq!(limit, 50);
                assert_eq!(offset, 10);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn vulns_finding_list_uses_defaults() {
        let cli = parse(&["cavectl", "vulns", "finding", "list"]);
        match cli.command {
            Commands::Vulns {
                cmd:
                    VulnsCmd::Finding {
                        cmd: VulnsFindingCmd::List { limit, offset },
                    },
            } => {
                assert_eq!(limit, 100);
                assert_eq!(offset, 0);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn vulns_finding_get_requires_id() {
        let cli = parse(&["cavectl", "vulns", "finding", "get", "abc-123"]);
        match cli.command {
            Commands::Vulns {
                cmd:
                    VulnsCmd::Finding {
                        cmd: VulnsFindingCmd::Get { id },
                    },
            } => {
                assert_eq!(id, "abc-123");
            }
            _ => panic!("wrong variant"),
        }
        assert!(Cli::try_parse_from(&["cavectl", "vulns", "finding", "get"]).is_err());
    }

    #[test]
    fn vulns_finding_create_with_json() {
        let cli = parse(&["cavectl", "vulns", "finding", "create", "--json", "{}"]);
        match cli.command {
            Commands::Vulns {
                cmd:
                    VulnsCmd::Finding {
                        cmd: VulnsFindingCmd::Create { json },
                    },
            } => {
                assert_eq!(json, "{}");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn vulns_import_scan_requires_scan_type_and_file() {
        let cli = parse(&[
            "cavectl",
            "vulns",
            "import-scan",
            "--scan-type",
            "SARIF",
            "--file",
            "out.sarif",
        ]);
        match cli.command {
            Commands::Vulns {
                cmd:
                    VulnsCmd::ImportScan {
                        scan_type,
                        file,
                        dedup,
                    },
            } => {
                assert_eq!(scan_type, "SARIF");
                assert_eq!(file, "out.sarif");
                assert!(dedup.is_none());
            }
            _ => panic!("wrong variant"),
        }
        assert!(
            Cli::try_parse_from(&["cavectl", "vulns", "import-scan", "--scan-type", "SARIF"])
                .is_err()
        );
    }

    #[test]
    fn vulns_import_scan_accepts_dedup_override() {
        let cli = parse(&[
            "cavectl",
            "vulns",
            "import-scan",
            "--scan-type",
            "Bandit Scan",
            "--file",
            "b.json",
            "--dedup",
            "legacy",
        ]);
        match cli.command {
            Commands::Vulns {
                cmd: VulnsCmd::ImportScan { dedup, .. },
            } => assert_eq!(dedup, Some("legacy".into())),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn vulns_engagement_list_parses() {
        let cli = parse(&["cavectl", "vulns", "engagement", "list"]);
        assert!(matches!(
            cli.command,
            Commands::Vulns {
                cmd: VulnsCmd::Engagement {
                    cmd: VulnsEngagementCmd::List
                }
            }
        ));
    }

    #[test]
    fn vulns_product_list_parses() {
        let cli = parse(&["cavectl", "vulns", "product", "list"]);
        assert!(matches!(
            cli.command,
            Commands::Vulns {
                cmd: VulnsCmd::Product {
                    cmd: VulnsProductCmd::List
                }
            }
        ));
    }

    #[test]
    fn vulns_product_types_list_parses() {
        let cli = parse(&["cavectl", "vulns", "product", "types", "list"]);
        match cli.command {
            Commands::Vulns {
                cmd:
                    VulnsCmd::Product {
                        cmd:
                            VulnsProductCmd::Types {
                                cmd: VulnsProductTypeCmd::List,
                            },
                    },
            } => {}
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn vulns_risk_accept_list_parses() {
        let cli = parse(&["cavectl", "vulns", "risk-accept", "list"]);
        assert!(matches!(
            cli.command,
            Commands::Vulns {
                cmd: VulnsCmd::RiskAccept {
                    cmd: VulnsRiskAcceptCmd::List
                }
            }
        ));
    }

    #[test]
    fn vulns_sla_rollup_parses() {
        let cli = parse(&["cavectl", "vulns", "sla", "rollup"]);
        assert!(matches!(
            cli.command,
            Commands::Vulns {
                cmd: VulnsCmd::Sla {
                    cmd: VulnsSlaCmd::Rollup
                }
            }
        ));
    }

    #[test]
    fn vulns_report_executive_parses() {
        let cli = parse(&["cavectl", "vulns", "report", "executive"]);
        assert!(matches!(
            cli.command,
            Commands::Vulns {
                cmd: VulnsCmd::Report {
                    cmd: VulnsReportCmd::Executive
                }
            }
        ));
    }

    #[test]
    fn vulns_report_executive_html_default_out() {
        let cli = parse(&["cavectl", "vulns", "report", "executive-html"]);
        match cli.command {
            Commands::Vulns {
                cmd:
                    VulnsCmd::Report {
                        cmd: VulnsReportCmd::ExecutiveHtml { out },
                    },
            } => {
                assert_eq!(out, "vulns-executive.html");
            }
            _ => panic!("wrong variant"),
        }
    }
}
