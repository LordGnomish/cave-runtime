# CAVE Platform Runbook §03 — cave-ctl CLI and MCP Server

**Document Version:** 1.0
**Last Updated:** 2026-03-08
**Status:** Authoritative
**Related ADRs:** ADR-076, ADR-092, ADR-102, ADR-130

---

## 3.1 Overview

`cave-ctl` is the CAVE Platform's primary command-line interface and control plane tool. It operates simultaneously as two distinct but integrated components: a human-operated CLI for direct platform and tenant management, and an MCP (Model Context Protocol) Server that enables AI assistants—including Claude, LibreChat, Backstage AI, and APOL—to execute platform operations in a controlled, audited manner.

Unlike traditional Infrastructure-as-Code tools that focus narrowly on resource provisioning, `cave-ctl` provides a unified control surface across the entire CAVE stack: cloud provisioning, cluster lifecycle management, tenant operations, resource composition via Crossplane, identity and access management, financial operations, compliance and governance, incident response, and security emergency procedures. It is designed to be equally comfortable in a CI/CD pipeline, an interactive terminal session, or within an AI agent's function-calling loop.

The philosophy underlying `cave-ctl` is that platform operations should be **transparent, auditable, RBAC-enforced, and accessible to both humans and intelligent agents** without sacrificing security, consistency, or accountability. Every `cave-ctl` invocation is logged to the CAVE Ledger, traced through the audit system, and subject to role-based access control enforcement—whether initiated by a human operator at a terminal or by an AI system calling the MCP Server interface.

### Key Characteristics

- **Unified control plane:** Single tool for profile, stack, resource, tenant, identity, and governance operations
- **Multi-cloud ready:** Native support for Hetzner (primary) and Azure (secondary) with provider-agnostic abstractions
- **RBAC-enforced at privilege ceiling:** AI systems inherit the calling user's permissions; no escalation possible
- **Fully audited:** All operations logged to Ledger with immutable hash chain verification
- **MCP-native:** Designed to be called by AI systems and assistants through the MCP protocol
- **Developer-friendly:** Intuitive command structure with sensible defaults and clear output formatting

---

## 3.2 ADR Rationale and Decision Drivers

### ADR-076: Unified CLI Design

**Context:** Early CAVE deployments faced operational fragmentation. Platform teams initially relied on a combination of Terraform CLI, kubectl, cloud provider CLIs, and custom scripts. This created several problems: inconsistent mental models, fragmented audit trails, difficulty enforcing RBAC uniformly, and steep learning curve for new operators.

**Decision:** Build a single, intentional CLI (`cave-ctl`) that serves as the primary human and programmatic interface to CAVE operations, with abstractions that hide provider-specific complexity while remaining transparent when needed.

**Alternatives Considered and Rejected:**

1. **Terraform CLI only:** Would require extensive custom Terraform provider development for CAVE-specific concepts (tenants, stacks, classifications). Doesn't expose governance operations. Poor natural language interface for AI agents.
2. **kubectl wrappers:** Suitable for cluster operations but insufficient for cloud provisioning, tenant management, financial reporting, or governance. Would require multiple tools anyway.
3. **Backstage GUI only:** Visual tools are excellent for discovery but inadequate for automation, CI/CD integration, and emergency response where latency matters. Doesn't support full RBAC flexibility.
4. **Custom API + client library:** Requires all consumers to implement their own CLIs and MCP servers. Creates more surface area for security vulnerabilities and inconsistent implementations.

**Why cave-ctl won:** It is a single point of consistent enforcement, audit, and RBAC. It integrates naturally with both human workflow and AI agent invocation. It scales to govern edge cases like emergency procedures that don't fit neatly into IaC paradigms. Most importantly, it establishes CAVE as an intentional platform, not a collection of tools.

### ADR-092: MCP Server Integration

**Context:** As CAVE matured, the platform team recognized that future platform operation would involve increasingly AI-assisted workflows. Rather than build separate integrations for each AI system, the team decided to standardize on Anthropic's Model Context Protocol—a mechanism by which AI systems can safely call external tools with consistent input/output contracts.

**Decision:** Expose `cave-ctl` as an MCP Server so that any MCP-compatible AI assistant (Claude, LibreChat, Backstage AI, APOL) can invoke platform operations with the same security, audit, and RBAC guarantees as human operators.

**Privilege Ceiling Mechanism:** The MCP Server enforces a critical security property: an AI assistant calling `cave-ctl` MCP operations can never perform actions that the calling user could not perform themselves. This is the **privilege ceiling**—the maximum set of permissions an MCP operation can receive is the union of the authenticated user's RBAC roles. An AI system cannot escalate privileges, request `sudo`, or bypass governance controls.

**Allowlist/Denylist Mechanism:** Organizations can configure which `cave-ctl` commands are permitted to be invoked through the MCP interface (allowlist) and which are prohibited regardless (denylist). For example, a sensitive operation like `cave-ctl mesh permissive --node <n>` (which disables mutual TLS, used only in emergencies) might be allowlisted only for on-call SREs but explicitly denylisted for all other users. Backstage policies can further restrict AI agent invocation based on incident context.

---

## 3.3 Tool Comparison Matrix

The following matrix evaluates `cave-ctl` against competing approaches across key platform engineering dimensions:

| Dimension | cave-ctl | Terraform CLI | kubectl wrapper | Backstage GUI only | Pulumi API |
|-----------|----------|---------------|-----------------|-------------------|-----------|
| **Human usability** | Excellent (verb-noun syntax, sensible defaults) | Good (but complex for non-IaC operators) | Poor (requires kubectl expertise) | Excellent (visual, low floor) | Good (Python/Node, steep learning) |
| **Automation/CI-CD** | Excellent (exit codes, structured JSON output, idempotent) | Excellent (mature ecosystem) | Moderate (shell parsing fragile) | Poor (not designed for automation) | Excellent (programmatic, but needs SDK) |
| **AI integration (MCP)** | Native first-class support | No standard integration | No standard integration | Not designed for AI calling | No standard integration |
| **RBAC enforcement** | Strong (privilege ceiling, per-command control) | External (via Terraform Cloud/Enterprise) | Weak (falls through to kubectl RBAC) | Good (policy engine, but GUI-only) | Moderate (SDK-level, not universal) |
| **Audit trail** | Complete (Ledger + hash verification) | Workable (Terraform Cloud) | Poor (scattered logs) | Good (event log, but opaque) | Workable (SDK-level) |
| **Extensibility** | Plugins, custom XR definitions | Providers (large ecosystem) | Limited (shell scripts) | Plugin system | Package management system |
| **Learning curve** | Gentle (intuitive verbs, help system) | Steep (HCL, state management concepts) | Moderate (Kubernetes knowledge required) | Very gentle (self-service UI) | Steep (programming language) |
| **Multi-cloud abstraction** | First-class (Hetzner + Azure + future providers) | Mature (multiple providers) | Poor (cluster-specific) | Moderate (depends on plugins) | Mature (multiple providers) |
| **Governance operations** | Comprehensive (compliance export, policy override, resurrection drill, etc.) | Limited (mainly provisioning) | None | Limited (policy engine only) | None |
| **Emergency operations** | Strong (mesh permissive, force-sync, fallback controls) | Not suitable | Not suitable | Limited (not latency-optimized) | Not suitable |

**Conclusion:** `cave-ctl` is the only tool in the matrix that is simultaneously excellent at human operation, AI integration via MCP, RBAC enforcement with privilege ceiling semantics, and comprehensive governance. It is purpose-built for CAVE; the alternatives are general-purpose tools that require adaptation.

---

## 3.4 24-Month Roadmap Analysis

### MCP Protocol Evolution

Anthropic's MCP specification is rapidly maturing. The CAVE team tracks several anticipated enhancements that will inform `cave-ctl` evolution:

1. **Streaming responses (H2 2026):** MCP tools currently return atomic responses. Support for streaming will allow real-time output from long-running operations (e.g., `cave-ctl stack deploy` showing live progress) to flow directly to the AI system, improving feedback quality.

2. **Transactional batching (H1 2026):** Multiple related operations (e.g., create database, configure network, set budget) could be submitted as a logical transaction, ensuring atomic semantics even across multiple `cave-ctl` invocations.

3. **Capability negotiation (H2 2026):** MCP servers will declare capabilities (e.g., "this server supports Azure but not Hetzner for PAM operations"). Allows AI systems to gracefully degrade when certain features are unavailable.

4. **Formal privilege expression (H2 2026):** Rather than relying on privilege ceiling at execution time, MCP servers will be able to declare upfront what permissions an operation requires, allowing the AI system to pre-filter recommendations and improve transparency.

### CLI Tooling Trends

Industry trends in platform engineering are converging on several principles that `cave-ctl` embodies:

- **Verb-noun command structure:** Moving away from monolithic commands toward composable, intuitive syntax (e.g., `cave-ctl tenant create` vs. `cave-ctl tenant_create`).
- **AI-friendly output formats:** JSON, YAML, and structured tables that are equally parseable by machines and readable by humans.
- **Declarative workflows in imperative shells:** Allowing users to express intent (e.g., "ensure this database exists with this configuration") rather than state changes.
- **Policy-as-code integration:** RBAC, governance, and compliance rules expressed in version-controlled code and enforced uniformly across all access paths.

### AI-Driven Operations Roadmap

The CAVE team anticipates that within 24 months, 40–60% of routine platform operations will be driven by AI agents rather than human operators. This implies:

- **Natural language self-service:** Developers and operators will describe what they want in English; APOL and Backstage AI translate to `cave-ctl` operations.
- **Proactive AI:** Rather than reactive CLI calls, AI systems will continuously monitor platform state, recommend optimizations, and offer autonomous execution of low-risk operations.
- **Hyperscaler AI integration:** Cloud providers are investing heavily in AI-native operations. `cave-ctl` must remain the canonical CAVE interface while coordinating with Azure AI and Hetzner native AI tools.

---

## 3.5 Command Reference and Operational Guide

This section provides the complete `cave-ctl` command reference with detailed explanations of when, why, and how to use each command. Commands are organized by operational domain.

### 3.5.1 Profile Management

Profiles are named configurations representing isolated CAVE deployments. A developer might have `dev`, `staging`, and `prod` profiles, each pointing to a different cloud account and Hetzner/Azure organization.

#### `cave-ctl create [dev|staging|prod] [hetzner|azure] [--config <path>]`

**Purpose:** Initialize a new CAVE profile in a fresh cloud environment.

**When to use:** During initial platform setup, environment expansion (e.g., adding a new staging environment in a different region), or disaster recovery bootstrap.

**What it triggers:**
1. Cloud account prerequisites validation (Hetzner project or Azure subscription configured).
2. Creation of root cloud resources: VPC/vNet, managed Kubernetes cluster, managed database (PostgreSQL), managed object store (S3/Blob), load balancer.
3. CAVE control plane bootstrap: installation of Crossplane, Backstage database, Ledger, APOL, observability stack.
4. Writing profile configuration to `~/.cave/profiles/<profile-name>.yaml`.
5. Registration of profile in local context file.

**Example:**
```bash
cave-ctl create staging hetzner --config /tmp/staging-config.yaml
```

**What happens internally:** The create command is idempotent and works by applying a sequence of Crossplane compositions. If the profile already exists, it verifies consistency rather than failing. Profile creation typically takes 15–25 minutes depending on cloud provider and region.

#### `cave-ctl local [up|down|status]`

**Purpose:** Manage local development environment (Docker Compose or Colima + Kind on developer machines).

**When to use:**
- `local up`: Before starting local platform development; sets up a complete CAVE stack in Docker with reduced resource footprint.
- `local down`: When shutting down to free resources.
- `local status`: Checking if local CAVE is running and reporting port mappings.

**What it triggers:**
- `local up`: Launches Docker Compose stack with PostgreSQL, Redis, Kubernetes (Kind), observability (Prometheus, Loki), and stubbed cloud provider APIs.
- Mounts local source trees as volumes so code changes immediately propagate to running services.
- Initializes local Ledger in SQLite.
- Outputs a summary of exposed ports and API endpoints.

**Example:**
```bash
cave-ctl local up --memory 8gb --cpus 4
cave-ctl local status
cave-ctl local down
```

**Why this matters:** Local development enables rapid iteration without incurring cloud costs or multi-minute deploy cycles. Developers can test entire workflows (stack deployment, tenant provisioning, compliance export) in under 60 seconds.

#### `cave-ctl profile [list|switch|show] [--format json|table]`

**Purpose:** List, switch, and inspect profiles.

**When to use:**
- `profile list`: Before any operation, to understand available profiles and current selection.
- `profile switch <n>`: To change the active profile (e.g., after finishing work in staging, switch to prod).
- `profile show`: To inspect detailed configuration and status of current profile.

**Example:**
```bash
cave-ctl profile list
# Output:
#  Profile          Cloud      Region      Status    Org
#  dev              hetzner    nbg1        healthy   acme-dev
#  staging          hetzner    fsn1        healthy   acme-staging
#* prod             azure      East US     healthy   acme-prod

cave-ctl profile switch staging
cave-ctl profile show --format json
```

**What it outputs:** For `list`, a table showing all profiles with their cloud provider, region, health status, and owning organization. For `show`, detailed JSON including cluster addresses, database endpoints, MCP server URLs, and RBAC context.

#### `cave-ctl platform [promote] [from] [to] [--verify]`

**Purpose:** Promote a platform configuration (including all tenants, policies, and settings) from one profile to another.

**When to use:** Advancing through the promotion chain: `dev` → `staging` → `prod`. Typically part of a CI/CD pipeline after running integration tests in staging.

**What it does:**
1. Verifies that the source profile's Ledger state is in "promoted" condition (all governance checks passed).
2. Exports the complete platform state from source profile: tenant configurations, RBAC policies, compliance overrides, budget allocations, incident response runbooks.
3. Imports and applies that state to the destination profile.
4. Runs reconciliation to ensure consistency across both profiles.
5. Logs promotion to Ledger as an immutable event.

**Example:**
```bash
cave-ctl platform promote staging prod --verify
```

**Why this pattern:** Staging and production should be identical except for scale and external integrations. Platform promotion ensures they diverge only intentionally. The `--verify` flag enforces that staging has passed all compliance checks before allowing promotion.

### 3.5.2 Stack Management

Stacks are layered collections of platform components. CAVE provides six primary stacks, plus custom stacks for specialized needs.

#### `cave-ctl stack [deploy|status|rollback|destroy] [core|data|ai|auth|cicd|dataplatform|serverless|<custom>]`

**Purpose:** Deploy and manage platform stacks.

**Stack breakdown:**

| Stack | Components | Typical Deploy Time | Use Case |
|-------|-----------|-------------------|----------|
| **core** | Kubernetes cluster, networking, storage (required) | 20–30 min | Base layer; always deployed first |
| **data** | PostgreSQL, Redis, Elasticsearch via Crossplane | 5–10 min | Stateful data services |
| **ai** | Ollama, vLLM, GPU infrastructure | 15–20 min | On-platform ML inference |
| **auth** | OIDC provider, identity management, Teleport | 10–15 min | Identity and PAM (ADR-130) |
| **cicd** | GitOps (ArgoCD or Flux), webhook handlers | 8–12 min | Continuous deployment |
| **dataplatform** | Apache Kafka, Spark, data lake services | 25–40 min | Event streaming and batch processing |
| **serverless** | Knative, OpenFaaS, or Firecracker isolate | 12–18 min | Ephemeral function execution |

**Dependencies:** Stacks have implicit ordering. The core stack must deploy successfully before any other stack. Data platform typically depends on core and data. Most other stacks can deploy in parallel once core is ready.

#### `cave-ctl stack deploy [--profile <p>] [--force] [--dry-run]`

**Example:**
```bash
# Deploy only the core stack
cave-ctl stack deploy core --profile staging

# Deploy multiple stacks in order
cave-ctl stack deploy core data ci/cd --profile prod

# Dry run to see what would be deployed
cave-ctl stack deploy dataplatform --dry-run --profile staging
```

**What happens:** Each stack deploy applies a set of Helm charts and Crossplane compositions. The tool verifies prerequisites (e.g., sufficient cluster capacity, required secrets present), applies manifests to the Kubernetes cluster, waits for readiness probes, and logs the deployment to the Ledger. If any deployment fails, it reports the error and offers automatic rollback.

#### `cave-ctl stack status [--profile <p>] [--watch]`

**Purpose:** Check the deployment status and health of stacks.

**Example:**
```bash
cave-ctl stack status --watch
```

**Output:** A live table showing each stack, its deployment status (Pending, In Progress, Ready, Degraded, Failed), replica counts, and last update timestamp. The `--watch` flag continuously refreshes, useful during deployments to monitor progress.

#### `cave-ctl stack rollback [--to-revision <n>]`

**Purpose:** Revert a stack to a previous revision if a deployment introduced issues.

**When to use:** After deploying a stack update that causes problems (e.g., a configuration change breaks connectivity). Rollback is fast and preserves data.

**Example:**
```bash
cave-ctl stack rollback cicd --to-revision 3
```

### 3.5.3 Resource Management via Crossplane XRs

CAVE uses Crossplane Composite Resources (XRs) to provide cloud-agnostic abstractions for databases, storage, caches, and message buses. Developers compose infrastructure declaratively without learning cloud-specific APIs.

#### `cave-ctl xr [create|list|describe|delete|update] [db|bucket|cache|messagebus|search|vectordb]`

**Purpose:** Create and manage composite resources.

**Key parameters:**
- `--name <n>`: Logical name (becomes DNS label and resource identifier).
- `--size [small|medium|large|xlarge]`: Determines instance type, disk size, replica count. Maps to cloud provider equivalents (e.g., small = Hetzner CPX11 or Azure B1s).
- `--env [dev|staging|prod]`: Influences retention policies, backup frequency, monitoring sensitivity.
- `--classification [public|internal|restricted|secret]`: Mandatory (ADR-102). Controls encryption, network isolation, audit logging, who can access the resource, and what data can be stored.

#### Resource Types

**Database (PostgreSQL):**
```bash
cave-ctl xr create db --name analytics-db --size large --env prod \
  --classification restricted --retention 30d
```

Creates a managed PostgreSQL instance with automated backups, point-in-time recovery, and read replicas. Connection string automatically provisioned and stored in Kubernetes secret.

**Object Store (S3/Blob):**
```bash
cave-ctl xr create bucket --name tenant-data --size medium --env staging \
  --classification internal --versioning enabled
```

Creates an object store bucket with optional versioning, access logging, and lifecycle policies based on classification.

**Cache (Redis):**
```bash
cave-ctl xr create cache --name session-cache --size small --env prod \
  --classification internal
```

Managed Redis instance with automatic failover, persistence options, and monitoring.

**Message Bus (Kafka):**
```bash
cave-ctl xr create messagebus --name events --size large --env prod \
  --classification internal --partitions 12 --replication-factor 3
```

Kafka cluster with topic auto-creation, schema registry, and consumer group management.

**Search (Elasticsearch):**
```bash
cave-ctl xr create search --name logs-index --size medium --env prod \
  --classification restricted --snapshot-repo s3://backups
```

Elasticsearch cluster with ILM policies, security plugins, and cross-cluster replication.

**Vector Database (Weaviate/Milvus):**
```bash
cave-ctl xr create vectordb --name embeddings --size large --env prod \
  --classification restricted --model all-minilm-l6-v2
```

Vector database optimized for LLM embeddings, with automatic indexing and similarity search.

#### `cave-ctl xr list [--env <e>] [--classification <c>] [--format json|table]`

**Purpose:** Discover and audit resources.

**Example:**
```bash
cave-ctl xr list --env prod --classification restricted --format json
```

Outputs all restricted-classification resources in prod, useful for compliance audits and capacity planning.

#### Classification System (ADR-102)

The classification parameter is mandatory and enforces organization-wide data governance:

- **public:** Unclassified data, no encryption required, accessible to anyone in the organization.
- **internal:** Business data, encrypted at rest, accessible within organization only.
- **restricted:** Sensitive business data (financial, customer PII, proprietary algorithms), encrypted at rest and in transit, audit logged, network isolated.
- **secret:** Highly sensitive (cryptographic keys, authentication credentials, regulatory data), encrypted at rest and in transit with key rotation, access audit-logged with justification, available only to specific identities.

Classification drives infrastructure decisions: a `secret` database is automatically placed in a private subnet, encrypted with a managed HSM key, and configured with minimal privilege (no public IP, no direct internet access). A `public` resource might be globally accessible without encryption.

### 3.5.4 Tenant Management

Tenants are isolated workloads in CAVE, either customer deployments (SaaS use case) or internal product teams (platform-as-a-service use case). Each tenant has its own namespace, budget, and resource quotas.

#### `cave-ctl tenant [create|list|delete|promote|demote|status] [--tier soft|hard|dedicated]`

**Purpose:** Manage tenant lifecycle.

#### `cave-ctl tenant create [<name>] [--tier soft|hard|dedicated] [--provider hetzner|azure] [--quota-cpu <cores>] [--quota-memory <gib>] [--quota-storage <gib>]`

**Tier explanation:**
- **soft:** Multi-tenant Kubernetes namespace, shared observability, best-effort scheduling. Lowest cost, suitable for non-critical workloads. No SLA.
- **hard:** Dedicated Kubernetes node pool, isolated observability, reserved CPU and memory. Medium cost, suitable for production workloads. 99.5% uptime SLA.
- **dedicated:** Dedicated Kubernetes cluster and cloud infrastructure. Highest cost, highest isolation, suitable for highly regulated or high-scale workloads. Custom SLA negotiable.

**Example:**
```bash
cave-ctl tenant create acme-prod --tier hard --provider hetzner \
  --quota-cpu 32 --quota-memory 128 --quota-storage 1000
```

Creates a new hard-tier tenant named `acme-prod` with dedicated node pool, 32 CPU cores, 128 GiB memory, and 1 TiB storage quota. Tenant is initialized with a namespace, network policies, RBAC rules, budget tracking, and observability.

#### `cave-ctl tenant delete [<name>] [--retention 30d] [--purge]`

**Purpose:** Deactivate and clean up a tenant.

**Retention policy:** By default, deleted tenants are retained in a recoverable state for 30 days. The Ledger preserves all historical data indefinitely. The `--purge` flag immediately destroys the tenant (irreversible).

**Use case:** Customer churn, internal consolidation, cost control.

#### `cave-ctl tenant promote [<name>] [from] [to]`

**Purpose:** Move tenant from one tier to another or between cloud providers.

**Example:**
```bash
cave-ctl tenant promote acme-prod soft hard
cave-ctl tenant promote legacy-app hetzner azure
```

Promotion automatically migrates data, updates network policies, provisions new infrastructure, and verifies consistency before and after. Non-disruptive for most workloads.

#### `cave-ctl tenant budget [set|report|forecast]`

**Purpose:** Set and monitor tenant spending.

**Example:**
```bash
cave-ctl tenant budget set acme-prod --monthly 10000 --currency usd
cave-ctl tenant budget report acme-prod --month 2026-03
cave-ctl tenant budget forecast acme-prod --horizon 12m
```

- `set`: Establishes a monthly budget cap. When spending exceeds 80% of budget, the system issues warnings; at 100%, non-critical workloads are throttled.
- `report`: Shows current month spending, breakdown by resource type, and trend analysis.
- `forecast`: Predicts end-of-month spending based on current burn rate, useful for proactive cost control.

#### `cave-ctl tenant [egress|network] [status|quarantine|restore]`

**Purpose:** Control tenant network egress and emergency response.

**Use case:** If a tenant has compromised credentials or is exhibiting abnormal outbound traffic (data exfiltration, botnet), the operator can instantly sever its internet connectivity without deleting the tenant. Data remains intact.

**Example:**
```bash
cave-ctl tenant network quarantine acme-prod --reason "suspected compromise"
cave-ctl tenant network status acme-prod
cave-ctl tenant network restore acme-prod
```

Quarantine is logged to Ledger with timestamp and reason, triggering incident escalation and alerting.

### 3.5.5 Privileged Access Management (PAM)

CAVE implements PAM via Teleport (Hetzner) and CyberArk (Azure), transparently managed by `cave-ctl`. This enforces zero-trust access to databases, Kubernetes, and web services.

#### `cave-ctl pam sessions [list|connect|terminate] [k8s|db|web]`

**Purpose:** Initiate and monitor PAM sessions.

#### `cave-ctl pam sessions connect k8s --tenant <name> --pod <pod-name>`

**What happens:** The user's identity is verified against RBAC policies. If authorized, a Teleport (Hetzner) or CyberArk (Azure) session is initiated, provisioning a short-lived certificate or credential. The CLI establishes a secure tunnel and opens an interactive shell in the Kubernetes pod. All keystrokes and output are recorded to the Ledger for forensic audit.

**Example:**
```bash
cave-ctl pam sessions connect k8s --tenant acme-prod --pod web-server-abc123
# Opens interactive shell in pod with session recorded
```

#### `cave-ctl pam sessions connect db --tenant <name> --database <name>`

**Purpose:** Open a PAM session to a managed database.

**What happens:** A temporary database user is created with a random password, granted to the authenticated user, and valid only for the duration of the session (default 1 hour). The CLI opens a psql/mysql connection through the secure tunnel. Upon disconnection, the temporary user is revoked.

**Example:**
```bash
cave-ctl pam sessions connect db --tenant acme-prod --database analytics-db
# Opens psql connection with temporary credentials, auto-revoked on disconnect
```

#### `cave-ctl pam sessions replay [<session-id>]`

**Purpose:** Audit a past PAM session by replaying recorded output and input.

**Use case:** Security investigation, compliance verification, incident post-mortem.

#### `cave-ctl pam request [create|approve|deny|list]`

**Purpose:** Request privileged access outside normal RBAC rules.

**Use case:** Emergency access, temporary developer access to production for debugging.

**Example:**
```bash
cave-ctl pam request create --target db:prod-db --justification "debugging replication lag" \
  --duration 2h --request-id INC-12345
# Sends request to on-call approver; awaits approval
```

#### Teleport Integration (Hetzner)

On Hetzner, PAM is backed by Teleport, a modern zero-trust access platform. `cave-ctl` abstracts Teleport's complexity: users don't interact with Teleport directly; they use intuitive `cave-ctl pam` commands.

**Architecture:** Teleport proxy runs in CAVE core stack. Users authenticate via SSO (Okta, Azure AD, GitHub), receive short-lived certificates, and use them to access infrastructure. All access is recorded to Teleport audit log, also synced to CAVE Ledger.

**MCP integration:** When an AI system invokes `cave-ctl pam` operations through the MCP Server, it receives a temporary credential valid only for the specific resource and operation. The AI's actions are recorded under the calling user's identity, preserving accountability.

#### CyberArk Integration (Azure)

On Azure, PAM is backed by CyberArk Conjur, a secrets and identity management platform. Similar security properties to Teleport but implemented via API rather than certificate-based.

---

## 3.5.6 Operations and Health

#### `cave-ctl doctor [--profile <p>] [--deep]`

**Purpose:** Comprehensive health check of a CAVE deployment.

**What it validates:**
- Cloud account credentials and quotas.
- Kubernetes cluster health (node status, API server availability, etcd quorum).
- Core CAVE components (Ledger, APOL, Backstage, MCP servers).
- Network connectivity between tiers.
- Storage system health and free capacity.
- Certificate validity and expiration warnings.
- Database replication lag.
- Backup completeness and recovery testing.
- Observability pipeline (Prometheus, Loki, Jaeger).

**Example:**
```bash
cave-ctl doctor --profile prod --deep
```

The `--deep` flag includes expensive checks like attempting to recover from a backup snapshot and validating disaster recovery procedures.

**Output:** Colored summary showing green (healthy), yellow (warning), or red (critical) status for each component. Typically completes in 2–5 minutes.

#### `cave-ctl finops [report|pnl|usage|forecast] [--tenant <n>] [--period month|quarter|year]`

**Purpose:** Financial operations and cost analysis.

#### `cave-ctl finops report [--period 2026-02] [--format json|csv|html]`

**Example:**
```bash
cave-ctl finops report --period 2026-02 --format html > february_cost_report.html
```

Generates a comprehensive cost breakdown by tenant, cloud provider, resource type, and service. Compares against budget, identifies cost drivers, and recommends optimizations.

#### `cave-ctl finops pnl --tenant acme-prod [--period 2026-q1]`

**Purpose:** Profit-and-loss analysis for a SaaS tenant (revenue minus COGS).

**Use case:** Calculating whether a customer's consumption is profitable, informing pricing decisions and capacity planning.

#### `cave-ctl incident [create|list|resolve|postmortem]`

**Purpose:** Incident lifecycle management integrated with CAVE operations.

#### `cave-ctl incident create --title "Database replication lag" --severity critical --context prod`

**What happens:** Creates an incident in CAVE's incident system, which:
1. Pages the on-call engineer.
2. Opens a war room in Slack/Teams with incident timeline and escalation procedures.
3. Initiates automated mitigation (e.g., reducing tenant load, scaling databases).
4. Logs all subsequent `cave-ctl` operations to the incident timeline.

#### `cave-ctl chaos [status|pause|resume|inject]`

**Purpose:** Control Chaos Engineering experiments (automated resilience testing).

**What it does:** CAVE uses Chaos Toolkit or similar to continuously inject failures (pod crashes, network latency, disk fills) and measure system resilience. The `chaos` commands let operators manage these experiments.

**Example:**
```bash
cave-ctl chaos status
# Shows: Experiment "latency-injection" running (8% packet delay on zone-b)

cave-ctl chaos pause --experiment latency-injection
# Pauses experiment if investigating a customer issue

cave-ctl chaos resume --experiment latency-injection
# Resumes after investigation
```

#### `cave-ctl reflex [list|history|dry-run|pause|resume]`

**Purpose:** Control Reflex Engine, the autonomous self-healing system (ADR-125).

**Example:**
```bash
cave-ctl reflex list
# Shows: 3 active reflexes (auto-scaling, certificate rotation, pod eviction)

cave-ctl reflex dry-run --reflex auto-scaling
# Simulates auto-scaling decision without applying it

cave-ctl reflex history --reflex certificate-rotation
# Shows past certificate rotations with timestamps and results
```

---

## 3.5.7 Governance and Compliance

#### `cave-ctl ledger [list|verify|export] [--since <timestamp>] [--format json|csv]`

**Purpose:** Query and audit the CAVE Ledger, the immutable append-only log of all platform decisions.

**Architecture:** Every significant operation (resource creation, RBAC change, policy override, emergency action) is recorded to the Ledger as a structured event, cryptographically signed and chained (each event includes a hash of the previous event). This creates an audit trail that cannot be tampered with retroactively.

#### `cave-ctl ledger list --since 2026-02-01 --format json`

**Output:** JSON stream of Ledger events, including timestamp, actor (user/AI), action, resource, result, and verification hash.

#### `cave-ctl ledger verify [<hash>]`

**Purpose:** Cryptographically verify that a Ledger entry has not been tampered with.

**Example:**
```bash
cave-ctl ledger verify abcd1234ef5678
# Output: ✓ Entry verified. Chain intact from event 5678 through current head.
```

#### `cave-ctl compliance [export|status|schedule] [--framework soc2|iso27001|nis2|gdpr]`

**Purpose:** Generate compliance evidence and reports.

**Framework explanations:**
- **SOC 2:** Security, availability, processing integrity, confidentiality, privacy controls. Required for B2B SaaS.
- **ISO 27001:** Information security management system. European and regulated organizations.
- **NIS2:** Network and Information Systems Directive (EU cybersecurity regulation). Network operators and critical infrastructure.
- **GDPR:** General Data Protection Regulation. Organizations processing EU resident data.

#### `cave-ctl compliance export --framework soc2 --format pdf`

**What it generates:** A PDF report including:
- Ledger export (all access events from past 12 months).
- Encryption key rotation audit.
- Backup and disaster recovery test results.
- RBAC policy review.
- Incident response runbook.
- Network diagram and security controls documentation.

**Use case:** Annual SOC 2 audit, customer due diligence, regulatory reporting.

#### `cave-ctl resurrection drill [--profile <p>] [--dry-run]`

**Purpose:** Test disaster recovery by simulating complete platform loss and recovery.

**What it does:**
1. Creates a snapshot of current Ledger state.
2. Deploys a new CAVE profile in a different region or cloud account.
3. Restores all tenants, configurations, and data from backup.
4. Runs integration tests to verify functionality.
5. Reports RTO (Recovery Time Objective) and RPO (Recovery Point Objective).
6. Cleans up the test deployment (unless `--persist` is set).

**Example:**
```bash
cave-ctl resurrection drill --profile prod --dry-run
# Simulates recovery without creating real resources
```

**Why this matters:** Many organizations practice disaster recovery drills but don't automate them. CAVE embeds this as a CLI command, making it cheap to run frequently (weekly or monthly), catching problems before they become critical.

#### `cave-ctl apol [status|override|fallback|constitution]`

**Purpose:** Manage APOL (Autonomous Policy Language), the declarative policy system (ADR-128).

**APOL context:** APOL policies define organization-wide governance rules in YAML. Examples:
- "Databases must be encrypted at rest."
- "Production resources must have redundancy."
- "Restricted-classification resources must be in private subnets."

#### `cave-ctl apol status [--profile <p>]`

**Output:** Which policies are active, how many resources are in compliance, and violations detected.

```bash
cave-ctl apol status --profile prod
# Policy: "databases-encrypted-at-rest" (3/3 compliant)
# Policy: "network-isolation-restricted" (2/3 compliant) — VIOLATION: vectordb-prod not isolated
# Policies overall: 47/50 compliant (94%)
```

#### `cave-ctl apol override [<policy-name>] [--resource <r>] [--justification <j>] [--expiration <date>]`

**Purpose:** Temporarily waive a policy for a specific resource.

**Use case:** Emergency access during incident (e.g., disabling encryption for fast debugging) or rapid development (e.g., temporarily allowing public database access for integration tests).

**Example:**
```bash
cave-ctl apol override network-isolation-restricted --resource vectordb-prod \
  --justification "debugging replication" --expiration 2026-03-10
```

Creates a time-limited policy override. After the expiration date, the policy is automatically re-enforced. The override is logged to the Ledger with full context.

#### `cave-ctl apol fallback [--enable|--disable]`

**Purpose:** Emergency control for policy enforcement.

**Scenario:** During a major incident, policies might be preventing necessary emergency actions (e.g., a policy forbids mesh permissive, but an emergency network isolation is needed). The `fallback` command enables a "break glass" mode where policies are advisory (logged) but not enforced.

**This is dangerous and requires:**
1. Explicit user confirmation.
2. Automatic expiration (default 1 hour).
3. Immediate audit logging and alerting.

#### `cave-ctl apol constitution diff [--between-profiles dev prod]`

**Purpose:** Compare governance policies across environments.

**Use case:** Ensuring dev and prod have consistent security policies. Detects drift.

#### `cave-ctl upgrade [check|attest|schedule]`

**Purpose:** Manage CAVE platform upgrades.

#### `cave-ctl upgrade check [--profile <p>]`

**Output:** Latest CAVE version, current version, breaking changes, new features, and estimated upgrade time.

#### `cave-ctl upgrade attest [--profile <p>]`

**Purpose:** Pre-upgrade validation. Runs a suite of tests to ensure the environment is ready for upgrade.

**What it checks:**
- All components are healthy.
- No pending policy violations (can be overridden if needed).
- Sufficient free capacity for upgrade workloads.
- Backups are current.
- No active incidents or chaos experiments.

#### `cave-ctl roadmap [scan|report] [--profile <p>] [--months <n>]`

**Purpose:** Forecast future infrastructure needs based on growth trends.

**Example:**
```bash
cave-ctl roadmap scan --profile prod --months 24
```

Analyzes historical resource usage, growth rate, planned tenant onboarding, and generates a 24-month forecast. Useful for capacity planning and budget forecasting.

#### `cave-ctl identity [dormant|recertify|jit|drift]`

**Purpose:** Manage identity and access lifecycle (ADR-130).

#### `cave-ctl identity dormant --days 90`

**What it does:** Lists all CAVE users who haven't accessed the platform in 90 days. Useful for periodic access review.

#### `cave-ctl identity recertify [--manager-ids user1,user2]`

**Purpose:** Trigger access recertification. Managers review their team's access and certify it's appropriate.

#### `cave-ctl identity jit [grant|revoke] --user <u> --role <r> --duration <d> --justification <j>`

**Purpose:** Just-in-time access provisioning. Grant temporary elevated access for a specific task, automatically revoked after duration.

**Example:**
```bash
cave-ctl identity jit grant --user alice --role tenant-admin:prod --duration 4h \
  --justification "migrating prod database"
```

Alice receives `tenant-admin` permissions on the prod profile for 4 hours. After 4 hours, permissions are automatically revoked. Useful for granting temporary admin access during incidents or migrations.

#### `cave-ctl identity drift`

**Purpose:** Detect identity inconsistencies between CAVE and external identity providers (Okta, Azure AD).

**Example:** A user is deprovisioned from Okta but still has CAVE permissions. The `drift` command detects this and recommends remediation.

#### `cave-ctl classify [scan|remediate|report] [--tenant <n>]`

**Purpose:** Audit and enforce data classification (ADR-102).

#### `cave-ctl classify scan --tenant acme-prod --output report.json`

**What it does:**
1. Scans all resources (databases, buckets, caches, etc.) for classification metadata.
2. Uses heuristics to infer classification (e.g., if database name contains "secret" or is connected to restricted-classification application, it's likely secret-classified).
3. Reports discrepancies (e.g., unclassified resources, misclassified resources).

#### `cave-ctl classify remediate --tenant acme-prod --infer`

**Purpose:** Automatically fix classification issues based on heuristics.

**Requires explicit confirmation:** System shows inferred classifications and waits for approval before applying.

#### `cave-ctl entropy [report|tenant] [--profile <p>] [--threshold <n>]`

**Purpose:** Detect chaos and anomalies in platform state.

**Entropy concept:** A measure of disorder in the platform. High entropy might indicate configuration drift, unmanaged resources, or forgotten test deployments. Low entropy indicates tight control but possibly over-regulation.

**Example:**
```bash
cave-ctl entropy report --profile prod --threshold 0.3
```

Reports resources or configurations that are anomalous (differ significantly from the declared desired state).

#### `cave-ctl forensics [query|timeline|investigate]`

**Purpose:** Deep security investigation and incident response.

#### `cave-ctl forensics query --pod <n> --since 2026-03-05T12:00Z`

**What it returns:** Complete logs, network flows, filesystem access, and process execution for a pod from the specified time range. Useful for investigating suspected compromises.

#### `cave-ctl forensics timeline --tenant <n> --event-type network-egress`

**Purpose:** Reconstruct a timeline of specific event types for a tenant.

**Example:** If a customer reports suspicious data exfiltration, this command shows all outbound network connections from that tenant's resources, when they occurred, and what data might have been transferred.

#### `cave-ctl network [test|policy|audit] [--tenant <n>]`

**Purpose:** Network connectivity and security testing.

#### `cave-ctl network test --tenant acme-prod`

**What it does:**
1. Tests connectivity between tenant resources (pods to database, pods to caches, etc.).
2. Tests internet connectivity and DNS resolution.
3. Tests network policy enforcement (verifies that blocked connections fail as expected).
4. Reports on network latency and packet loss.

#### `cave-ctl mesh [permissive|strict] [--node <n>] [--duration <d>]`

**Purpose:** Emergency network control. Mutually TLS (mTLS) enforcement in service mesh.

#### `cave-ctl mesh permissive --node zone-b --duration 15m`

**What it does:** Disables mTLS between services in zone-b for 15 minutes. This is a break-glass for emergencies where service-to-service authentication is causing operational issues.

**Critical notes:**
1. This severely reduces security. Used only in emergencies.
2. Automatically expires after the specified duration.
3. Logged to Ledger with high visibility (sends alerts).
4. Requires explicit user confirmation.

#### `cave-ctl gitops [force-sync|rollback|status] [--app <a>]`

**Purpose:** Emergency Git-Ops controls.

#### `cave-ctl gitops force-sync --app payments-service`

**What it does:** Forces ArgoCD (or Flux) to immediately sync the specified application from Git, overriding any timing delays or sync policies. Used when a critical fix is deployed to Git but GitOps hasn't picked it up yet.

**Example scenario:** Customer reports a critical bug. Fix is committed and merged. But ArgoCD is configured to sync every 5 minutes. Force-sync deploys immediately.

#### `cave-ctl observability [fallback|test|failover]`

**Purpose:** Emergency observability controls.

#### `cave-ctl observability fallback --enable`

**What it does:** If the primary observability pipeline (Prometheus, Loki) becomes unhealthy, enable fallback to a simpler, more resilient observation system (sidecar collectors, local queuing).

#### `cave-ctl portability [drill|export] [--tenant <n>]`

**Purpose:** Data portability and multi-cloud mobility.

#### `cave-ctl portability drill --tenant acme-prod --target azure`

**What it does:** Simulates exporting all of tenant's data (databases, object stores, configurations) in a vendor-agnostic format and importing to a different cloud provider (in this case, Azure). Tests the complete data portability workflow without actually migrating.

**Why it matters:** CAVE's multi-cloud design assumes tenants might move between providers. Regular drills verify this works in practice.

#### `cave-ctl migrate [tenant] [--from hetzner|azure] [--to hetzner|azure]`

**Purpose:** Migrate a tenant and all its data between cloud providers.

**Example:**
```bash
cave-ctl migrate tenant acme-prod --from hetzner --to azure
```

Non-disruptive migration (if possible, uses blue-green deployment; otherwise, coordinated failover). Typically takes 30–120 minutes depending on data size. Can be done with low/zero customer impact if timed correctly.

---

## 3.6 MCP Server Integration

### Architecture and Design

`cave-ctl` exposes an MCP Server interface—a standardized protocol by which external applications (AI assistants, CI/CD systems, orchestration platforms) can invoke platform operations. This enables natural language interfaces, autonomous agent workflows, and seamless integration with existing tools.

**Key properties:**

1. **Privilege Ceiling:** An MCP client can only invoke operations that the authenticated user can perform. If Alice has the `tenant-admin:staging` role, she can invoke `cave-ctl tenant create` for staging but not for prod. An AI system calling MCP as Alice cannot exceed Alice's permissions.

2. **Complete Auditability:** Every MCP invocation is logged to the Ledger with the caller's identity, parameters, and result. This preserves accountability even when operations are initiated by AI.

3. **Allowlist/Denylist Control:** Organizations can configure which `cave-ctl` commands are permitted to be invoked through the MCP interface. For example, an organization might allowlist `cave-ctl xr create` and `cave-ctl tenant status` but denylist `cave-ctl apol override` and `cave-ctl mesh permissive` (reserved for humans in emergencies).

4. **Tool Definitions:** The MCP Server declares all available operations and their parameters (input schema) to clients. AI systems use this schema to generate prompts, validate parameters, and handle errors gracefully.

### How AI Systems Call cave-ctl via MCP

**Scenario: Developer uses Backstage AI to create a database.**

1. Developer opens Backstage self-service interface and says (in natural language): "I need a PostgreSQL database called analytics-db for my production environment, with restricted classification."

2. Backstage AI parses this request and calls the `cave-ctl` MCP Server with the parameters:
   ```json
   {
     "command": "xr create db",
     "parameters": {
       "name": "analytics-db",
       "size": "large",
       "env": "prod",
       "classification": "restricted"
     }
   }
   ```

3. The MCP Server verifies:
   - The developer is authenticated.
   - The developer has the `tenant-admin:prod` role (or equivalent).
   - The `xr create` command is on the allowlist.
   - The request is not denied by organization policies (e.g., no budget overrun).

4. If all checks pass, the MCP Server executes the underlying `cave-ctl` command and returns:
   ```json
   {
     "status": "success",
     "resource_id": "db-analytics-db-prod",
     "connection_string": "postgresql://...",
     "message": "Database created successfully"
   }
   ```

5. Backstage AI presents this result to the developer: "Your database is ready at postgresql://..."

6. The operation is logged to the Ledger with the developer's identity.

### cave-ctl as MCP Client

`cave-ctl` also consumes other MCP servers, coordinating across multiple platforms:

- **Azure (via Terraform MCP):** When deploying CAVE to Azure, `cave-ctl` calls Terraform MCP to provision cloud resources (vNets, AKS clusters, managed databases).
- **GitHub (via GitHub MCP):** When creating CI/CD pipelines, `cave-ctl` calls GitHub MCP to create repositories, workflows, and secrets.
- **Confluent (via Confluent MCP):** When provisioning Kafka clusters, `cave-ctl` calls Confluent MCP for topic management and schema registry.
- **Databricks (via Databricks MCP):** When setting up data platforms, `cave-ctl` calls Databricks MCP for workspace provisioning and job scheduling.
- **Hetzner (GitHub only):** Hetzner does not expose MCP servers; CAVE uses Hetzner's native APIs directly (via custom Crossplane providers).

This model allows `cave-ctl` to seamlessly orchestrate infrastructure across multiple clouds and platforms without duplicating logic.

### Teleport MCP Integration (ADR-130, Hetzner PAM)

On Hetzner, `cave-ctl pam` operations are transparently proxied through Teleport's MCP server. This allows AI systems to request PAM sessions with the same security guarantees as human operators:

1. AI system requests a PAM session: `cave-ctl pam sessions connect k8s --pod web-server-abc123`.
2. `cave-ctl` calls Teleport MCP Server to verify AI identity and request a session.
3. Teleport generates a short-lived certificate valid only for the requested resource.
4. The certificate is returned to the AI system (or to the human on whose behalf the AI is acting).
5. The AI system uses the certificate to access the resource.
6. All activity is recorded to Teleport audit log and synced to CAVE Ledger.

This ensures zero-trust access even for AI-driven operations.

### Backstage AI and Natural Language Self-Service

CAVE integrates with Backstage AI, a platform for AI-assisted developer self-service. The architecture:

1. **LiteLLM Proxy:** Routes AI requests to Claude (or other LLMs) for natural language processing.
2. **cave-ctl MCP Server:** Defines available operations and their schemas.
3. **Policy Engine:** Enforces governance rules and RBAC before executing operations.

**Flow:**
```
Developer: "Create a Redis cache for session storage in staging"
           ↓
Backstage AI (LiteLLM) → Claude (processes natural language)
           ↓
Claude invokes: cave-ctl xr create cache --name session-cache --env staging --size medium
           ↓
Policy Engine: Verify permissions, budget, classification (missing, prompt user)
           ↓
cave-ctl executes: Creates Redis cluster
           ↓
Backstage: Shows developer "Your cache is ready at redis://..."
```

---

## 3.7 Use Cases and Developer Scenarios

### Scenario 1: Developer Creates a New Database

**Actor:** Alice, a backend engineer.

**Goal:** Set up a PostgreSQL database for a new microservice in staging.

**Flow:**

1. Alice opens a terminal and runs:
   ```bash
   cave-ctl profile switch staging
   ```

2. Alice creates the database:
   ```bash
   cave-ctl xr create db --name user-service-db --size medium --env staging \
     --classification internal
   ```

3. `cave-ctl` validates:
   - Alice has the `developer:staging` role ✓
   - The database name is unique ✓
   - Medium size is within quota ✓
   - Internal classification is appropriate for microservice data ✓

4. Database is provisioned in ~5 minutes. `cave-ctl` outputs:
   ```
   ✓ Database created: user-service-db
   ✓ Connection string: postgresql://user-service-db.internal:5432/main
   ✓ Secret stored in Kubernetes: user-service-db-credentials
   ✓ Backups enabled (daily, 30-day retention)
   ```

5. Alice updates her microservice Helm values to reference the secret. On next deploy, the application connects to the database.

**What happened under the hood:**
- Crossplane created a PostgreSQL XR, which mapped to a managed RDS instance (Azure) or Cloud SQL (Hetzner native).
- CAVE provisioned network policies to allow the microservice pod to connect to the database.
- Automated backups were configured.
- The operation was logged to the Ledger.

### Scenario 2: AI SRE Scales a Deployment

**Actor:** APOL (autonomous policy operation language), responding to a performance alert.

**Trigger:** Prometheus detects that a service is CPU-bound; response latency is elevated.

**Flow:**

1. APOL detects the alert and reasons: "The service needs more CPU. I'll scale it horizontally."

2. APOL invokes (via MCP):
   ```
   cave-ctl stack status --pod payment-service --format json
   ```
   Returns: Current replicas = 3, requested CPU = 1 core/replica, current utilization = 85%.

3. APOL invokes:
   ```
   cave-ctl kubernetes scale deployment payment-service --replicas 5 --tenant prod
   ```

4. Permission check: APOL is running as the `auto-scaler:prod` service account, which has the `scaler` role. ✓

5. Kubernetes scales the payment service from 3 to 5 replicas. New pods start in ~30 seconds.

6. APOL monitors metrics and confirms: Response latency drops from 450ms to 180ms. ✓

7. APOL logs the action to the Ledger: "Scaled payment-service 3→5 (CPU alert INC-456)".

8. On-call engineer receives a notification: "APOL auto-scaled payment-service in response to CPU alert."

**Why this matters:** This operation, initiated by an AI system, is fully audited, respects the user's RBAC, and doesn't require human intervention. The engineer can override if needed, but routine scaling is now autonomous and safe.

### Scenario 3: Platform Admin Runs Resurrection Drill

**Actor:** Charlie, platform engineering lead.

**Goal:** Test disaster recovery procedures to ensure CAVE can recover from complete failure.

**Flow:**

1. Charlie schedules a resurrection drill for Sunday at 2 AM (low-traffic time):
   ```bash
   cave-ctl resurrection drill --profile prod --schedule "0 2 * * 0"
   ```

2. Every Sunday at 2 AM, the drill automatically runs:
   - Snapshots current Ledger state.
   - Provisions a new CAVE profile in a different Azure region.
   - Restores all tenants and data from backups.
   - Runs integration tests (create a tenant, deploy an app, run queries).
   - Reports RTO (estimated recovery time) and RPO (data loss window).
   - Tears down the test deployment (to avoid extra costs).
   - Emails Charlie a summary: "Drill succeeded. RTO = 45 minutes, RPO = 5 minutes."

3. If the drill fails, Charlie is immediately alerted and can investigate.

**Why this matters:** By automating disaster recovery testing, CAVE ensures the team's disaster recovery procedures actually work, rather than discovering gaps during a real disaster.

### Scenario 4: Security Audit and Compliance Export

**Actor:** Diana, security officer.

**Goal:** Prepare SOC 2 audit evidence for an annual audit.

**Flow:**

1. Diana runs:
   ```bash
   cave-ctl compliance export --framework soc2 --format pdf --period 2025-01-01:2026-01-01
   ```

2. `cave-ctl` generates a comprehensive PDF including:
   - Complete Ledger export (all access events, policy changes, incidents).
   - Encryption key rotation audit (proves keys are rotated regularly).
   - Backup recovery test results (from resurrection drills).
   - RBAC policy review.
   - Network diagram and security controls.
   - Incident response runbook.

3. Diana submits the PDF to the auditor. The auditor verifies:
   - Ledger entries are cryptographically signed (tamper-proof).
   - All sensitive operations required multiple approval steps.
   - Access is consistently logged and reviewed.

4. Audit passes. ✓

**Why this matters:** Rather than cobbling together evidence from multiple systems, CAVE provides a single, authoritative, cryptographically verified audit trail that satisfies SOC 2 and other compliance frameworks.

### Scenario 5: Emergency—Force-Syncing ArgoCD During Incident

**Actor:** Evan, on-call engineer. **Scenario:** Incident INC-789. A critical bug fix has been merged to Git and is ready to deploy, but ArgoCD's sync period is 5 minutes and we can't wait.

**Flow:**

1. Evan runs:
   ```bash
   cave-ctl gitops force-sync --app payments-service
   ```

2. `cave-ctl` verifies:
   - Evan is on the `oncall-engineer` role ✓
   - `gitops force-sync` is allowlisted for on-call engineers ✓
   - There's an active incident (INC-789) in progress ✓

3. ArgoCD immediately syncs payments-service. Fix is deployed in ~20 seconds.

4. Evan monitors the service and confirms: "Error rate is back to normal."

5. Evan resolves the incident: `cave-ctl incident resolve INC-789`.

6. `cave-ctl` logs to Ledger: "force-sync by evan for incident INC-789" and includes timestamps and result.

**Why this matters:** In emergencies, every second counts. Evan didn't need to SSH into the ArgoCD server or manually trigger a sync; a simple `cave-ctl` command did it, with full auditability.

### Scenario 6: FinOps—Checking Tenant P&L

**Actor:** Frank, finance ops engineer.

**Goal:** Determine if a customer's consumption is profitable.

**Flow:**

1. Frank asks Backstage AI: "What's the P&L for acme-prod for February?"

2. Backstage AI invokes:
   ```
   cave-ctl finops pnl --tenant acme-prod --period 2026-02
   ```

3. `cave-ctl` retrieves:
   - Acme's revenue (from billing system): $50,000
   - COGS (cloud + platform costs): $8,500
   - Gross margin: $41,500 (83%)

4. Backstage AI responds: "Acme is highly profitable. Cost trend is stable."

5. Frank uses this data to optimize pricing or identify opportunities to reduce COGS.

**Why this matters:** FinOps is critical for SaaS profitability. By exposing this data through `cave-ctl`, it's accessible to both humans and AI systems, enabling data-driven decisions.

### Scenario 7: Shadow IT Detection

**Actor:** Grace, platform lead.

**Goal:** Find unmanaged infrastructure that wasn't provisioned through `cave-ctl`.

**Flow:**

1. Grace runs:
   ```bash
   cave-ctl doctor --profile prod --deep
   ```

2. `doctor` scans the entire Kubernetes cluster and cloud account, comparing actual resources against the declared desired state (stored in Ledger).

3. Grace discovers:
   - A Kubernetes StatefulSet that's not tracked in `cave-ctl` (unknown origin).
   - An Azure VM running a cron job (should be in Kubernetes).
   - A Hetzner bare-metal server with unknown purpose.

4. Grace investigates and finds the resources were created by developers as temporary workarounds and never cleaned up.

5. Grace works with teams to migrate workloads to proper CAVE stacks and deletes the shadow IT.

**Why this matters:** Unmanaged infrastructure is a security and compliance risk. By regularly running `doctor`, Grace prevents shadow IT from accumulating.

---

## 3.8 Operations: Installation, Configuration, Authentication

### Installation

`cave-ctl` is distributed via multiple channels:

- **Homebrew (macOS/Linux):** `brew install cave-ctl`
- **Docker:** `docker run cave-ctl:latest <command>`
- **Binary releases:** Download from releases.cavePlatform.io
- **Source:** `git clone https://github.com/cave-platform/cave-ctl.git && make install`

### Configuration

Configuration is stored in `~/.cave/config.yaml`:

```yaml
current-profile: prod
default-output: table
mcp-server-enabled: true
mcp-server-port: 9876
audit-log: ~/.cave/audit.log
```

Profiles are stored individually in `~/.cave/profiles/<profile-name>.yaml`:

```yaml
name: prod
cloud-provider: azure
region: East US
cluster-api: https://cave-prod.eastus.azmk8s.io
backup-retention: 30d
```

### Authentication

`cave-ctl` uses multi-factor authentication:

1. **Primary:** OIDC (OpenID Connect) via Okta, Azure AD, or GitHub. User logs in once, receives a token, and CLI uses it.
2. **Secondary (for sensitive operations):** Additional MFA (TOTP or hardware key).
3. **Service accounts:** CI/CD systems authenticate via Kubernetes service accounts (for deployment in prod) or API keys (for external systems).

**Login flow:**

```bash
cave-ctl login
# Launches browser to OIDC provider
# User logs in, authorizes
# Token written to ~/.cave/auth.token (encrypted with local key)
```

### Plugin and Extension Model

`cave-ctl` supports plugins written in Go or any language that can be invoked via subprocess:

```bash
# Install a custom plugin
cave-ctl plugin install https://github.com/example/cave-plugin-custom

# Plugin is downloaded and registered
# New commands are available
cave-ctl custom <command>
```

Plugins have the same RBAC enforcement as built-in commands. They're useful for organization-specific operations (e.g., internal billing systems, custom approval workflows).

---

## 3.9 Troubleshooting

### Issue 1: "Permission denied" when running `cave-ctl`

**Diagnosis:**
```bash
cave-ctl doctor --diagnose
# Output: RBAC role missing for this operation
```

**Resolution:** Ensure your user account has the appropriate role. Ask a platform admin to grant the role:
```bash
cave-ctl identity jit grant --user alice --role tenant-admin:staging
```

### Issue 2: `cave-ctl` command hangs or times out

**Diagnosis:** Check network connectivity and service health:
```bash
cave-ctl doctor --profile <p>
# If core CAVE components are unhealthy, likely cause is identified
```

**Resolution:** If Kubernetes API is unresponsive, try:
```bash
cave-ctl kubernetes api-server restart --node <node-name>
```

### Issue 3: Database creation fails with "quota exceeded"

**Diagnosis:**
```bash
cave-ctl tenant status acme-prod
# Shows current quota usage vs. limits
```

**Resolution:** Either reduce quota usage (delete underutilized resources) or request quota increase:
```bash
cave-ctl tenant quota set acme-prod --quota-storage 2000 --reason "dataset growth"
```

### Issue 4: Ledger verification fails

**Diagnosis:**
```bash
cave-ctl ledger verify <entry-hash>
# Output: ✗ Hash mismatch. Chain integrity compromised.
```

**Resolution:** This indicates the Ledger was tampered with. Investigate immediately:
```bash
cave-ctl forensics investigate --ledger-chain-break
```

### Issue 5: MCP server not responding

**Diagnosis:**
```bash
cave-ctl mcp server status
```

**Resolution:** Restart the MCP server:
```bash
cave-ctl mcp server restart
```

### Issue 6: Compliance export is slow

**Diagnosis:** Exporting a large amount of Ledger data can take minutes.

**Resolution:** Filter by time period:
```bash
cave-ctl compliance export --framework soc2 --since 2026-02-01 --until 2026-02-28
```

### Issue 7: Resurrection drill fails

**Diagnosis:**
```bash
cave-ctl resurrection drill --profile prod --persist  # Keep the test deployment
# Investigate the test deployment to see what went wrong
```

**Resolution:** Check backup integrity and cloud quotas. Retry after addressing the issue.

### Issue 8: PAM session denied

**Diagnosis:**
```bash
cave-ctl pam sessions connect db --tenant acme-prod --database prod-db
# Output: Access denied. Missing PAM approval.
```

**Resolution:** Request PAM access:
```bash
cave-ctl pam request create --target db:prod-db --justification "debugging query performance"
```

Wait for approval from on-call approver.

### Issue 9: Policy violation preventing deployment

**Diagnosis:**
```bash
cave-ctl apol status
# Shows which policies are violated
```

**Resolution:** Either fix the resource to comply with the policy, or (temporarily) override:
```bash
cave-ctl apol override network-isolation --resource <r> --duration 1h
```

### Issue 10: Tenant network quarantine is preventing work

**Diagnosis:**
```bash
cave-ctl tenant network status acme-prod
# Output: quarantined (reason: "suspected compromise")
```

**Resolution:** If the incident is resolved:
```bash
cave-ctl tenant network restore acme-prod
```

### Issue 11: Budget exceeded

**Diagnosis:**
```bash
cave-ctl tenant budget report acme-prod
# Shows 120% of monthly budget spent
```

**Resolution:** Reduce resource consumption or increase budget:
```bash
cave-ctl tenant budget set acme-prod --monthly 15000
```

### Issue 12: Chaos experiment interfering with debugging

**Diagnosis:**
```bash
cave-ctl chaos status
# Shows active chaos experiments
```

**Resolution:** Pause the experiment:
```bash
cave-ctl chaos pause --experiment <name>
```

---

## 3.10 Compliance Mapping

| Compliance Framework | Relevant cave-ctl Commands | Evidence |
|---------------------|---------------------------|----------|
| **SOC 2** | `compliance export soc2`, `ledger export`, `pam sessions replay`, `identity recertify` | Ledger hash chain, access logs, policy override justifications |
| **ISO 27001** | `apol status`, `classify scan`, `identity drift`, `forensics timeline` | Policy compliance report, encryption audit, access control audit |
| **NIS2** | `resurrection drill`, `network test`, `entropy report`, `incident list` | Incident response capability, network resilience, system monitoring |
| **GDPR** | `portability drill`, `ledger verify`, `classify scan --classification secret`, `tenant delete` | Data portability evidence, consent audit, data deletion confirmation |

---

## 3.11 Related ADRs

- **ADR-076:** CLI Design — Rationale for unified CLI architecture
- **ADR-092:** MCP Server Integration — Security and privilege ceiling model
- **ADR-102:** Data Classification — Resource classification system
- **ADR-125:** Reflex Engine — Autonomous self-healing system
- **ADR-128:** APOL (Autonomous Policy Language) — Declarative policy framework
- **ADR-130:** PAM and Identity — Zero-trust access and Teleport integration

---

## 3.12 Related Runbook Sections

- **§00 Executive Summary:** Overview of CAVE as a platform
- **§00.1 Prerequisites and Account Setup:** Initial cloud account configuration
- **§01 Architecture and Principles:** Design principles underlying cave-ctl and MCP integration
- **§10 Data Platform (PostgreSQL):** Details on managed database provisioning via `cave-ctl xr create db`
- **§04 (planned) Observability:** Integration with Prometheus, Loki, and observability fallback controls

---

**Document generated:** 2026-03-08
**Status:** Authoritative runbook section
**Next review:** 2026-06-08
