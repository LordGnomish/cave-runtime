# CAVE Platform Runbook §00 — Executive Summary

**CAVE (Cloud-Agnostic Virtualized Environment)** is a sovereign, open-source Internal Developer Platform (IDP) designed to eliminate vendor lock-in while delivering enterprise-grade developer experience across multiple infrastructure targets. This document provides the executive overview of the platform's architecture, principles, components, and operational model.

---

## 0.1 Overview

### What CAVE Is

CAVE is a professional-grade Internal Developer Platform that serves a dual purpose: a fully sovereign, self-hosted platform for organizations prioritizing infrastructure control and auditability, and simultaneously a reference architecture and SaaS backbone for commercialization. It is designed around the principle that **developers should never be blocked by infrastructure**, yet infrastructure decisions should never be hidden from the business.

The platform operates under a core philosophy: **self-hosted where possible, open-source everywhere, zero vendor lock-in, fully GitOps-driven, and fully auditable**. Every infrastructure decision is captured in Architecture Decision Records (ADRs), every security control is policy-as-code, and every deployment is reproducible from Git.

### Deployment Targets & Domains

CAVE operates across **7 deployment profiles**:

- **Hetzner infrastructure**: `dev`, `staging`, `prod` (self-hosted, bare metal)
- **Azure infrastructure**: `dev`, `staging`, `prod` (managed services where beneficial)
- **Local development**: Single command—`cave-ctl local up`—spins a fully functional local replica

**Primary domains**:
- `caveplatform.dev`: Platform governance and runbook repository
- `*.cave.caveplatform.dev`: Platform services (Backstage, ArgoCD, Kong API Gateway, observability)
- `*.api.caveplatform.dev`: Tenant workload APIs and data plane

### The Sovereign + SaaS Duality

On **Hetzner**, every component is self-hosted: Kubernetes (Talos Linux), databases (CloudNativePG), object storage (MinIO), message queues (Strimzi/Kafka), identity (Keycloak), and more. This provides maximum control, auditability, and exit optionality.

On **Azure**, specific components are replaced with managed equivalents (AKS instead of Talos, Azure Database for PostgreSQL instead of CloudNativePG, ADLS Gen2 instead of MinIO), but the **developer and operator experience remains identical**. The same Backstage UI, same ArgoCD GitOps workflows, same `cave-ctl` CLI commands, and same observability dashboards work across both environments. This allows CAVE to serve as both a sovereign platform for direct deployments and a standardized backbone for managed SaaS offerings.

### Dual Software Development Lifecycles (SDLC)

CAVE operates two independent SDLCs that do not block each other:

1. **Platform SDLC**: Infrastructure code flows through `dev → staging → prod` profiles, controlled by platform engineers. New components, upgrades, and policy changes follow strict promotion gates and testing.

2. **Tenant SDLC**: Each tenant operates its own `dev → staging → prod` environments within the same cluster or separate vcluster instances. Tenant deployments are independent and do not require platform coordination.

This decoupling is fundamental: a platform release in progress does not delay tenant deployments, and a tenant incident does not impose platform-level change freezes.

---

## 0.2 Architecture Principles Summary

CAVE is built on **18 foundational principles** that shape every decision, component selection, and operational procedure. These principles form the constitutional guardrails of the platform.

### 1. Single Pane of Glass: Backstage Only

**Principle**: All developer self-service, platform visibility, and operational dashboards route through Backstage. No parallel UIs.

**Rationale**: Cognitive load reduction. Developers and operators should never ask "where do I check this?" There is one answer: Backstage. This includes service catalog, deployment pipelines, infrastructure cost visibility, on-call schedules, security posture, and compliance status. Custom integrations pull data from specialized systems (ArgoCD for sync status, Prometheus for metrics, Sovereign Ledger for audit events) but present unified views through Backstage plugins.

**References**: ADR-011 (Backstage as Single Pane of Glass), ADR-025 (Plugin Architecture for Extensibility)

### 2. Three Control Planes with Clear Separation of Concern

**Principle**: Three distinct control planes serve different purposes and authority models.

- **Backstage**: Self-service UI for developers. Declarative intent models (Service Definitions, Resource Claims, Deployment Requests). Primary control surface for 99% of operations.
- **cave-ctl**: CLI + MCP Server for advanced operations, scripting, and integration with external systems. Supports automation, CI/CD pipelines, and infrastructure-as-code workflows.
- **Emergency CLI**: Direct `kubectl` access to the cluster root, reserved for incident response only (e.g., cluster is down and Backstage cannot reach the API). Requires multi-sig approval and hardware key access.

These are ordered by trust boundary. Developers use Backstage. Automation uses `cave-ctl`. Only human SREs with physical presence can use Emergency CLI.

**Rationale**: This creates a clear escalation path and reduces the risk of automation-induced cascading failures. A broken Backstage plugin does not prevent `cave-ctl` automation. A `cave-ctl` runaway loop can still be stopped by Emergency CLI. This hierarchy mirrors incident response protocols in mature organizations.

### 3. Profile-Driven Deployment

**Principle**: The same source code, same Helm charts, same Kustomize bases, and same configuration patterns deploy across all 7 profiles with profile-specific overlays only.

**Rationale**: This eliminates configuration drift and "works on my profile" problems. A feature tested on local and dev profiles is verifiably deployable to prod. Configuration differences are minimal and explicit (e.g., replica counts, resource requests, log levels).

### 4. Two-Layer Provisioning: OpenTofu (Day 0) + Crossplane v2 (Day 1+)

**Principle**: Infrastructure provisioning is split into two phases, each with its own abstraction and tooling.

- **Day 0 (Bootstrapping)**: OpenTofu provisions the initial cloud infrastructure (VPCs, subnets, managed Kubernetes clusters, initial RBAC, domain records). This is a one-time operation per profile and is stored in version control.
- **Day 1+ (Ongoing)**: Crossplane Custom Resources (XRs) in the Kubernetes cluster define and manage all workload infrastructure (databases, caches, object storage buckets, message queues, credentials, users). Developers declare intent via XRs; Crossplane provisions and maintains the resources until deletion or drift correction occurs.

ArgoCD orchestrates both layers: it deploys the OpenTofu plan and then deploys Crossplane XRs. The result is **deterministic, auditable, reproducible infrastructure as code**.

**Rationale**: This split acknowledges different operational realities. Day 0 infrastructure (network, auth, cluster itself) is fragile and rarely changes; OpenTofu's state management and plan/apply model is ideal. Day 1 infrastructure (databases, users, permissions) is fluid and tenant-driven; Crossplane's declarative, continuous reconciliation model is ideal. Neither layer blocks the other: Day 0 is promoted through deployment profiles; Day 1 XRs can be created, updated, or deleted within a profile at any time.

**References**: ADR-067 (Crossplane as Abstraction Layer), ADR-119 (Crossplane v2 Migration Strategy)

### 5. Crossplane-First Abstraction: Namespaced XRs & Multi-Region Auto-Placement

**Principle**: Workload infrastructure is declared via namespaced Crossplane Custom Resources (XRs). Composition Functions encode provisioning logic and multi-region placement strategies.

**Multi-Region Auto-Placement (MRAP)**: XRs can specify region preferences, redundancy requirements, or cost targets. Composition Functions automatically select the optimal infrastructure provider and region based on real-time cost, latency, and capacity metrics. For example, a database XR with `redundancy: "none"` on dev profile might provision to the cheapest single-region option, while prod automatically gets multi-AZ replication and read replicas.

**Namespaced XRs**: Every Crossplane XR lives in the same namespace as the tenant or workload that claims it. This provides soft multi-tenancy isolation: namespace-level RBAC controls who can create, read, update, or delete infrastructure. Hard isolation is available via vcluster or Dedicated (maximum) isolation modes.

**Rationale**: Namespaced XRs distribute infrastructure responsibility to teams without requiring a separate infrastructure provisioning request loop. A developer creates a database XR in their namespace; Crossplane reconciles it; within minutes, a production-grade PostgreSQL instance exists with automated backups, high availability, and encrypted credentials delivered via External Secrets Operator. No ticket queue. No handoff. This transforms infrastructure from "something ops provides" to "something developers define."

**References**: ADR-067, ADR-119 (Composition Functions and v2 namespace model), ADR-124 (XR Federation Pattern)

### 6. GitOps Everything: ArgoCD v3.3+, OCI Registry, Server-Side Apply

**Principle**: All infrastructure and application state flows through Git or OCI registries, and the cluster applies changes in a declarative, auditable manner.

ArgoCD v3.3+ continuously reconciles cluster state to Git and OCI registry sources:
- **Git sources**: Infrastructure code (OpenTofu), Kubernetes manifests, Kustomize overlays, Helm values, policy-as-code rules.
- **OCI registry sources** (Harbor): Container images with signed provenance, configuration packages, policy bundles, and attestations.

**Server-side apply** ensures that ArgoCD applies manifests to the cluster using Kubernetes' native server-side apply strategy, which provides conflict detection and better merge semantics than client-side apply.

**Rationale**: Git becomes the source of truth for infrastructure. Every change is auditable via Git history, every rollback is a Git revert, and every deployment is deterministic and reproducible. OCI registries handle binary artifacts (images, policies, configurations) with cryptographic signatures for provenance verification.

**References**: ADR-026 (GitOps Architecture), ADR-120 (Server-Side Apply for Deterministic Reconciliation)

### 7. Security by Default: Zero-Trust, Signed Images, SBOM, SLSA Level 3, Runtime Enforcement

**Principle**: Security is not an option or afterthought—it is embedded in every layer of the platform.

- **Zero-trust network**: Cilium provides network policy enforcement by default. All traffic is denied unless explicitly allowed by policy. Istio ambient mesh adds mTLS and distributed authorization.
- **Container image supply chain**: All images must be signed (Cosign/Sigstore). Provenance and attestations are verified at admission time. SBOM (Software Bill of Materials) is generated for every image and stored in the OCI registry.
- **SLSA Level 3**: Build pipelines include trusted builder attestations. Every artifact is traceable to its source code and build environment.
- **Runtime enforcement**: Tetragon (eBPF-based runtime enforcement) blocks syscalls that violate security policy. Seccomp profiles are automatically applied. Pod Security Standards are enforced.
- **Policy-as-code**: OPA Gatekeeper validates every admission request (pods, XRs, secrets, network policies, etc.). Policies are declarative, versionable, and instantly updatable.

**Rationale**: Zero-trust is the only model that scales. A vendor or tenant cannot be "mostly trusted." Either they are authorized to perform an action or they are not. This requires comprehensive enforcement at network, admission, and runtime layers. By making these controls default and mandatory, CAVE removes the burden of "security awareness" from developers—they simply cannot violate policy, intentionally or accidentally.

**References**: ADR-077 (Zero-Trust Architecture), ADR-101 (Supply Chain Security), ADR-105 (Runtime Enforcement with Tetragon), ADR-106 (Signed Images and Provenance)

### 8. Multi-Tenant Isolation: Soft, Hard, and Maximum Options

**Principle**: Isolation is a spectrum, not binary. Different tenants have different isolation requirements and resource budgets.

- **Soft isolation**: Shared cluster, dedicated namespaces. Namespace RBAC controls who can create resources. Network policies isolate traffic. Compute quotas limit resource consumption. Suitable for internal teams with high trust and shared incident response (e.g., microservices from the same company).
- **Hard isolation**: Dedicated vcluster per tenant. Each vcluster has its own Kubernetes API server, etcd, and control plane, but shares host cluster nodes. Higher isolation, higher resource cost. Suitable for external customers or high-risk internal workloads.
- **Maximum isolation**: Dedicated cluster per tenant (Hetzner or Azure). Complete infrastructure separation. Suitable for compliance-critical workloads or dedicated SaaS customers.

**Rationale**: Forcing all tenants into one isolation model is either wasteful (paying for maximum isolation when soft isolation suffices) or risky (forcing shared infrastructure when hard isolation is required). CAVE allows operators to select isolation per tenant based on their compliance, trust, and cost posture.

**References**: ADR-012 (Multi-Tenancy Model), ADR-084 (vcluster Provisioning Automation)

### 9. Self-Hosted AI with Governance: Ollama, LiteLLM, Classification-Aware Routing

**Principle**: Large language models are critical infrastructure. They should be self-hosted, not outsourced to untrusted third parties. However, self-hosting must include governance and cost controls.

- **Ollama**: Self-hosted LLM inference engine. Runs on GPU nodes in the cluster. Models are pulled from huggingface.co or local registries.
- **Azure OpenAI**: Available on Azure profile for compliance reasons (data residency, Azure AD integration, audit logs in Azure). Costs are tracked per-tenant.
- **LiteLLM gateway**: Sits in front of Ollama and Azure OpenAI. Routes requests based on:
  - **Model classification**: Sensitive prompts (PII, code, security decisions) route to stronger, slower models. Casual prompts (documentation, summaries) route to faster, cheaper models.
  - **Tenant budget**: If a tenant is near budget limits, requests are rate-limited or queued.
  - **Prompt security**: Malicious prompts (injection attempts, jailbreaks) are detected and logged in Sovereign Ledger before reaching the model.
  - **Cost optimization**: LiteLLM batches requests and selects the cheapest provider that meets quality requirements.

**Rationale**: AI is a core capability of modern platforms, not a luxury. However, AI models are black boxes with known failure modes (hallucinations, prompt injection, training data leakage). Self-hosting provides control. LiteLLM governance ensures that AI enhances platform reliability without introducing new risk vectors. Multi-model routing trades latency for cost and security—developers get fast local models for everyday tasks and higher-quality models for high-stakes decisions.

**References**: ADR-009 (Self-Hosted LLM Infrastructure), ADR-013 (LiteLLM Governance and Routing), ADR-103 (Prompt Security & Injection Detection), ADR-111 (Cost-Aware AI Routing)

### 10. Full Observability: Same Dashboards Across All Profiles

**Principle**: The observability stack (metrics, logs, traces, events) is identical across dev, staging, and prod profiles on both Hetzner and Azure. No surprises at production time.

- **Prometheus**: Metrics collection (infrastructure, applications, custom metrics).
- **Grafana**: Unified dashboarding. Dashboards reference the same queries and thresholds regardless of profile.
- **Loki**: Structured log aggregation with strong multi-tenancy support.
- **Tempo**: Distributed tracing with trace correlation to metrics and logs.
- **Thanos**: Long-term metric storage and cross-cluster federation for multi-region observability.

Every dashboard available on dev is instantly available on prod. SLOs and alert rules are defined once and promoted through profiles, reducing the gap between dev testing and production reality.

**Rationale**: Observability is often an afterthought, leading to production incidents that could not have been reproduced or diagnosed in dev. By enforcing identical observability stacks, CAVE ensures that the signal-to-noise ratio, alert sensitivity, and debugging tools are calibrated once for all profiles.

**References**: ADR-029 (Observability Architecture), ADR-109 (Cross-Profile Observability Consistency)

### 11. Policy-as-Code: OPA Gatekeeper + OPAL for Real-Time Policy Updates

**Principle**: Compliance, security, and organizational policies are written in code (Rego language), deployed via GitOps, and enforced in real time via admission control.

- **OPA Gatekeeper**: Admission controller that evaluates every create/update/delete request against a policy bundle. Policies are compiled into Rego, a declarative query language.
- **OPAL**: Policy Administration Point. Manages policy distribution, versioning, and real-time updates. When a policy is updated in Git, OPAL pushes the change to all OPA Gatekeeper instances without requiring a reboot or redeployment.
- **Tenant-specific policies**: Policies can include tenant metadata (budget limits, data classification, region restrictions) injected by OPAL. When a developer creates a database XR, OPA checks: "Is this tenant in the APAC region? Yes. Is this database type allowed in APAC? No. Denied."

**Rationale**: Traditional compliance frameworks rely on humans reading documentation and following procedures. Policy-as-code makes compliance automatic and verifiable. A developer cannot accidentally violate a policy because the policy is evaluated before the resource is created. Policies are versioned, auditable, and can be tested like any other code.

**References**: ADR-030 (Policy-as-Code with OPA Gatekeeper), ADR-089 (OPAL for Real-Time Policy), ADR-131 (Tenant Metadata Injection into Policies)

### 12. SLO-Driven FinOps: Kill Switch by Workload Criticality, Per-Tenant Unit Economics

**Principle**: Infrastructure costs are not separate from operational reliability. SLOs and budgets are tightly coupled.

- **Kill switch by workload criticality**: When a tenant exceeds their monthly budget, non-critical workloads are suspended (scaled to zero, but their state is preserved). Critical workloads (payment processing, data pipelines) continue. This avoids the all-or-nothing choice between "let them run and pay massive overage fees" or "hard-terminate everything."
- **Per-tenant unit economics**: Every tenant has a cost visibility dashboard showing CPU, memory, storage, and network costs broken down by service. This visibility drives optimization: teams see immediately if their change doubled the cost.
- **SLO-budget translation**: An SLO of "99.9% availability" translates to a budget of ~43 minutes of downtime per month. Infrastructure costs are allocated to achieve this target: high-availability targets get multi-AZ deployment and read replicas; lower-availability targets get single-region, cheaper options.

**Rationale**: Cost control without insight is impossible. By coupling visibility (dashboards) with enforcement (kill switch), CAVE ensures that cost discipline is automatic, not manual. Teams optimize because they can see the impact of their choices.

**References**: ADR-096 (SLO-Driven Infrastructure), ADR-110 (Per-Tenant Cost Visibility), ADR-126 (Cost-Driven Resource Allocation)

### 13. Immutable Infrastructure: Talos Linux on All Hetzner Clusters

**Principle**: Hetzner nodes run Talos Linux, a minimal, immutable operating system designed for Kubernetes. No SSH access. No package manager. No manual configuration drift.

**Rationale**: Traditional Linux nodes are mutable—admins SSH in, edit files, install packages, and create state that is not in version control. Talos is designed to be cattle, not pets: if a node is misconfigured, it is destroyed and replaced. Configuration is declarative (MachineConfig YAML) and GitOps-driven. This eliminates an entire class of production incidents ("node was working yesterday, but we forgot why") and makes cluster state fully auditable.

**References**: ADR-098 (Immutable Infrastructure with Talos Linux)

### 14. Sovereign Auditability: Sovereign Ledger (WORM + Sigstore)

**Principle**: Every material action taken by the platform (policy violation, access grant, resource creation, admission denial, AI prompt, break-glass action) is recorded in an immutable, cryptographically signed ledger.

**Sovereign Ledger architecture**:
- **WORM (Write-Once Read-Many) storage**: Events are appended to MinIO (or ADLS on Azure) with object retention policies. Once written, events cannot be modified or deleted.
- **Sigstore integration**: Events are signed with keys managed by Sigstore. The signature chain is verifiable, even if the ledger itself is compromised.
- **Schema**: Each event includes actor, action, resource, timestamp, decision (allowed/denied), reasoning (policy that was checked), and cryptographic hash of the previous event (blockchain-style chain of custody).

**Rationale**: Compliance auditors ask, "What happened on 2025-03-06 at 14:23 UTC?" The Sovereign Ledger provides a tamper-proof answer with full context and reasoning. This is essential for SOC2, HIPAA, PCI-DSS, and regulatory audits.

**References**: ADR-093 (Sovereign Ledger Design), ADR-106 (Cryptographic Signature Integration), ADR-090 (Audit Trail Requirements)

### 15. Autonomous Operations (APOL): 4 AI Roles, 0 FTE Ops Target, Constitutional Guardian

**Principle**: The platform should operate with minimal human intervention. AI agents are assigned distinct roles, each with specific responsibilities and constraints.

**4 AI SRE Roles**:
1. **Diagnostician**: Analyzes metrics, logs, and traces when alerts fire. Composes a hypothesis of root cause.
2. **Remediation Agent**: Executes automated fixes based on Reflex Engine rules (see principle 16). Files incidents if remediation fails or is not allowed.
3. **Analyst**: Writes post-incident reviews, updates runbooks, proposes policy changes.
4. **Guardian**: Constitutional enforcer. Vetoes any action that violates platform constitutional values (e.g., attempting to delete the Sovereign Ledger, breaking out of multi-tenancy isolation).

**0 FTE ops target**: The goal is that the platform requires minimal human on-call burden. Humans handle escalations and constitutional decisions. Routine troubleshooting, remediation, and runbook updates are AI-driven.

**Constitutional Guardian**: One AI agent has the role of "constitutional guardian" and 1+ humans also hold this role. Decisions that violate constitutional values require multi-sig approval (2-of-3 guardian signatures, at least one human).

**Rationale**: Modern platforms generate too much data for humans to analyze in real time. AI is better at pattern recognition, correlation, and rapid iteration. By giving AI clear roles and explicit constraints, CAVE achieves high reliability without hero culture or on-call burnout.

**References**: ADR-112 (AI SRE Roles & Responsibilities), ADR-125 (Constitutional Guardian Pattern), ADR-128 (Conflict Resolution in Autonomous Operations)

### 16. Two-Tier Automated Remediation: Crossplane Operations + Reflex Engine

**Principle**: Remediation is two-tiered based on complexity and risk.

- **Tier 1 (Crossplane Operations)**: Simple, idempotent actions managed by Crossplane XRs. Examples:
  - Rotate database credentials (secret update).
  - Scale a read replica due to CPU saturation.
  - Restart a pod with OOMKilled status.
  - Trigger a scheduled database backup.

  These are defined as CronOperations or OperationSets in Crossplane and are safe to retry.

- **Tier 2 (Reflex Engine)**: Complex, stateful remediation with decision trees and human approval gates. Examples:
  - Detect a cascading failure in a data pipeline and trigger a full replication restart.
  - Identify a memory leak in a tenant workload and suggest code changes.
  - Detect anomalous traffic patterns and initiate egress quarantine.

  Reflex Engine rules include pre-conditions, success criteria, rollback strategies, and escalation paths. If a Reflex action is risky (e.g., deleting data), it requires human approval before execution.

**Rationale**: Automation that is too conservative wastes reliability gains. Automation that is too aggressive risks data loss or security breaches. Two-tier remediation balances safety and responsiveness.

**References**: ADR-095 (Reflex Engine Architecture), ADR-119 (Crossplane Operations Integration)

### 17. Exit Strategy Built-In: Every Azure Service Has Hetzner Equivalent

**Principle**: CAVE is designed for portability. At any time, a customer can decide to move from Azure to Hetzner (or vice versa) without losing capability or requiring revalidation.

Every managed Azure service used in CAVE has a Hetzner equivalent in the self-hosted stack:
- AKS ↔ Talos Linux + Kubernetes
- Azure Database for PostgreSQL ↔ CloudNativePG
- ADLS Gen2 ↔ MinIO
- Confluent Cloud ↔ Strimzi/Kafka
- Azure Redis ↔ Valkey
- Azure AI Search ↔ OpenSearch or Qdrant
- Azure Key Vault ↔ OpenBao

**Exit drill**: Annually, the platform team runs a full rehearsal: spin up a Hetzner cluster, migrate all data, validate all functionality. This ensures that the exit strategy is not theoretical—it is tested and operational.

**Rationale**: Vendor lock-in is a form of risk. If Azure changes pricing, SLA, or terms, CAVE customers can migrate to Hetzner without reengineering. This optionality is powerful in commercial negotiations and provides peace of mind for sovereign customers.

**References**: ADR-066 (Azure ↔ Hetzner Equivalence), ADR-029 (Exit Strategy Validation)

### 18. ADRs for Every Decision, Constitutional Artifacts with 2-of-3 Multi-Sig

**Principle**: Architecture decisions are not tribal knowledge—they are recorded in Architecture Decision Records (ADRs). Constitutional artifacts (policies, RBAC roles, SLOs) are protected by cryptographic multi-sig.

- **ADRs**: Every architectural decision has a numbered ADR explaining the context, options considered, the chosen option, and rationale. ADRs are versioned in Git.
- **Constitutional artifacts**: Core policies, roles, and SLOs that define the platform's identity are signed with hardware keys. Changes require 2-of-3 guardian multi-sig (e.g., Platform Lead, Security Lead, one human guardian). Signatures are verified before applying changes.

**Rationale**: This creates an auditable decision trail and protects the platform from accidental or malicious subversion of core values.

### Complexity Budget & Survivability Invariant

**Complexity budget**: The platform has a fixed complexity budget of ~2 weeks of engineer time per month. No new component is added without removing an equivalent or more complex component. This prevents unbounded growth and forces thoughtful prioritization.

**Survivability invariant**: Only Kong (the API Gateway) failure causes external traffic interruption. All other single-component failures degrade functionality (no self-service UI, no GitOps) but do not drop traffic. This is tested monthly via chaos engineering.

---

## 0.3 Component Map

CAVE consists of approximately **73 components** across three categories: infrastructure providers (managed/self-hosted), platform services (control plane), and observability/operational tools. This section maps all components, noting which are provider-specific and which provide Cross-plane XRs.

### Infrastructure Provider-Specific Components (14 Total)

| Component | Hetzner | Azure | Has XR? | Notes |
|-----------|---------|-------|---------|-------|
| **Kubernetes** | Talos Linux + k3s/kubeadm | AKS | No | OpenTofu provisions both |
| **PostgreSQL** | CloudNativePG | Azure Database for PostgreSQL Flexible Server | Yes | ADR-067: Crossplane XR abstracts both |
| **Object Storage** | MinIO HA | ADLS Gen2 | Yes | S3-compatible interface on both |
| **Kafka/Messaging** | Strimzi (Kafka in Kubernetes) | Confluent Cloud | Yes | Event-driven architecture; ADR-067 |
| **Cache** | Valkey (Redis compatible) | Azure Cache for Redis | Yes | Sub-millisecond latency across profiles |
| **Full-Text Search** | OpenSearch | Azure AI Search | Yes | Tenant search indexes; ADR-114 |
| **Vector Search** | Qdrant | Azure AI Search (vector store) | Yes | RAG workloads, semantic search; ADR-114 |
| **Secrets** | OpenBao + ESO | Azure Key Vault + ESO | No | Both integrated via External Secrets Operator |
| **Identity** | Keycloak (OIDC/SAML) | Okta + Entra ID | No | Federation supported on both |
| **PAM** | Teleport CE | CyberArk Privilege Cloud | No | Break-glass sessions recorded; ADR-130 |
| **Data Platform** | Spark Operator + JupyterHub | Databricks | No | ML training, batch analytics |
| **MLOps** | MLflow standalone | MLflow (Databricks integration) | No | Experiment tracking, model registry |
| **LLM** | Ollama (self-hosted) | Azure OpenAI or Ollama | No | Routed via LiteLLM gateway; ADR-009 |
| **Tenant Git** | Gitea HA | GitHub Enterprise | No | Source of truth for tenant code |

### Platform Services (Self-Hosted on All Profiles)

| Component | Purpose | Category |
|-----------|---------|----------|
| **ArgoCD v3.3+** | GitOps orchestration (Day 0 + Day 1) | Control Plane |
| **Backstage** | Developer self-service portal | Control Plane |
| **cave-ctl** | CLI + MCP Server for automation | Control Plane |
| **Kong API Gateway** | Ingress, rate limiting, request routing | Data Plane |
| **Cilium** | CNI, network policy, DDoS protection | Network |
| **Istio ambient** | mTLS, distributed authorization, VirtualService | Mesh |
| **Harbor** | OCI registry for images, policies, configs | Registry |
| **Tetragon** | eBPF runtime enforcement | Security |
| **OPA Gatekeeper** | Admission control, policy validation | Security |
| **OPAL** | Policy distribution and real-time updates | Security |
| **Sigstore** | Image signing, provenance verification | Security |
| **Prometheus** | Metrics collection | Observability |
| **Grafana** | Dashboarding and alerting | Observability |
| **Loki** | Log aggregation | Observability |
| **Tempo** | Distributed tracing | Observability |
| **Thanos** | Long-term metric storage, federation | Observability |
| **Grafana OnCall** | Incident routing and escalation | Observability |
| **Sovereign Ledger** | Immutable audit trail (WORM + Sigstore) | Audit |
| **Argo Rollouts** | Canary/blue-green deployments | Deployment |
| **Argo Workflows** | DAG-based orchestration | Workflows |
| **Unleash** | Feature flag service | Operations |
| **Renovate** | Dependency updates with Pluto checks | Operations |
| **Chaos Mesh** | Chaos engineering (kill Kong, nodes, etc.) | Testing |
| **DefectDojo** + **DTrack** | SAST/DAST findings, SBOMs | Security |
| **DevLake** | DORA metrics collection | Observability |
| **Uptime Kuma** | Health checks, status page | Observability |
| **n8n** | Low-code workflow automation | Integrations |
| **Pulp** | Package repository management | Registry |
| **vcluster** | Virtual Kubernetes clusters for tenants | Isolation |
| **OpenCost** | Infrastructure cost allocation | FinOps |
| **k6** | Load testing and synthetic monitoring | Testing |
| **Cilium Hubble** | Network observability | Observability |
| **LibreChat** | Multi-model chat interface | AI |
| **Langfuse** | LLM observability and cost tracking | AI |
| **Teleport** | PAM, privileged session recording | Security |
| **Velero** | Cluster backup and disaster recovery | Backup |
| **LiteLLM** | LLM routing, cost optimization, prompt security | AI |
| **KEDA** | Event-driven autoscaling (core for Reflex) | Autoscaling |

**Total**: ~73 components (including all sub-components like Prometheus Operator, Grafana plugins, etc.).

### Phase 4 Extensions (Not in Initial Rollout)

- **Knative + KEDA**: Serverless workload support (functions, event-driven services)
- **Multi-region active-active**: Data replication, conflict resolution, and failover orchestration across regions

---

## 0.4 Phased Rollout

CAVE deployment follows a structured 4-phase rollout to manage complexity and derisk each stage.

### Phase 1: Core Platform (Week 1–4)

**Objective**: Establish the Kubernetes foundation, control planes, and identity.

**Components**:
- Kubernetes cluster (Talos Linux on Hetzner, AKS on Azure)
- Crossplane (Day 1 infrastructure provisioning)
- Backstage (Developer portal, static plugin set)
- ArgoCD (GitOps orchestration of Day 0 and Day 1)
- Kong (API Gateway, ingress)
- Cilium (CNI, network policy)
- Istio ambient (mTLS, zero-trust)
- Identity layer (Keycloak on Hetzner, Okta/Entra on Azure)
- Secrets (OpenBao on Hetzner, Key Vault on Azure)
- Observability (Prometheus, Grafana, Loki, Tempo)
- Harbor (OCI registry)
- OPA Gatekeeper (basic admission policies)

**Success criteria**: Developers can log into Backstage, trigger a CI pipeline, and observe metrics for their workload.

### Phase 2: Data Platform & AI (Week 5–10)

**Objective**: Enable stateful workloads and AI-driven capabilities.

**Components**:
- PostgreSQL (CloudNativePG or Azure PG)
- Kafka (Strimzi or Confluent Cloud)
- MinIO (or ADLS Gen2)
- Valkey (or Azure Redis)
- OpenSearch (or Azure AI Search)
- Qdrant (vector search)
- Ollama + LiteLLM (self-hosted LLM + routing)
- MLflow (experiment tracking)
- Spark Operator + JupyterHub (data workloads)
- Archival storage (cold tier for Thanos, backups)

**Success criteria**: Developers can declare a database XR and use it within their application. First ML training pipeline runs.

### Phase 3: Advanced Operations & Security (Week 11–18)

**Objective**: Implement security hardening, observability refinement, and autonomous operations.

**Components**:
- Tetragon (runtime enforcement)
- OPAL (real-time policy updates)
- Sigstore (image signing, SBOMs)
- DefectDojo + DTrack (vulnerability tracking)
- Sovereign Ledger (immutable audit)
- Grafana OnCall (incident routing)
- Chaos Mesh (reliability testing)
- Argo Rollouts (progressive deployments)
- Argo Workflows (complex orchestration)
- APOL (4 AI SRE roles)
- Reflex Engine (automated remediation)
- PAM (Teleport CE or CyberArk)
- Velero (backup/DR)
- DevLake (DORA metrics)

**Success criteria**: No human intervention required for pod restarts, database failovers, or policy updates. Kubernetes cluster can be destroyed and restored within 4 hours.

### Phase 4: Extensions & Multi-Region (Week 19+)

**Objective**: Advanced scaling and serverless workloads.

**Components**:
- Knative (serverless functions, event sources)
- Multi-region active-active (Thanos federations, data replication)
- Chaos Mesh advanced scenarios (AZ failure, network partition)

---

## 0.5 Platform SLOs

Service Level Objectives are the "north star" metrics that define platform success. These are tested monthly and tracked in Grafana dashboards.

### Velocity SLOs

| SLO | Target | Profile | Notes |
|-----|--------|---------|-------|
| Time-to-Hello-World | <5 min | All | New tenant, scaffold to first workload |
| CI Pipeline (p95) | <15 min | All | From git push to artifact in registry |
| XR Provisioning | <3 min | All | Database, cache, storage XR creation to ready |
| ArgoCD sync | <30 sec | All | From Git commit to cluster reconciliation |

### Reliability SLOs

| SLO | Target | Profile | Notes |
|-----|--------|---------|-------|
| Pod recovery | <1 min | All | OOMKilled, Evicted, or CrashLoopBackOff → Running |
| Node failure | <5 min | All | Node cordoned, pods rescheduled |
| AZ loss | <30 min | All | Full availability zone down → traffic rerouted |
| Cluster resurrection (full loss) | <4 hours | All | From cluster deletion to production-ready |
| Observability stack recovery | <30 min | All | Prometheus/Grafana/Loki down → restored |

### Audit & Compliance SLOs

| SLO | Target | Notes |
|-----|--------|-------|
| Audit log latency | <5 sec | Event → Sovereign Ledger storage |
| Policy update propagation | <10 sec | Policy change in Git → all nodes enforcing |
| Compliance report generation | <1 hour | Annual SOC2 audit evidence export |

### Platform SLA by Infrastructure Provider

| Metric | Hetzner Dedicated Cluster | Azure Managed | Shared Multi-Tenant Hetzner |
|--------|---------------------------|---------------|-----------------------------|
| Availability | 99.95% | 99.95% | 99.5% (best effort) |
| RTO (Recovery Time Objective) | 4 hours | 2 hours | 4 hours |
| RPO (Recovery Point Objective) | 15 minutes | 5 minutes | 1 hour |
| Failover automation | Tier 2 (Reflex) | Tier 2 (Reflex) | Tier 1 (Crossplane) |

### Estimated Profile Costs (Monthly, 50 Workload Tenants)

| Profile | Infrastructure | Approx. Cost | CapEx |
|---------|----------------|--------------|-------|
| Hetzner dev | 2x Hetzner servers (32 vCPU, 256 GB RAM) + storage | ~$800 | $0 |
| Hetzner staging | 3x Hetzner servers | ~$1,200 | $0 |
| Hetzner prod | 5x Hetzner servers + redundancy | ~$2,000 | $0 |
| Azure dev | AKS + managed services | ~$1,200 | $0 |
| Azure staging | AKS + managed services | ~$2,000 | $0 |
| Azure prod | AKS + high-availability | ~$3,500 | $0 |
| Local (dev laptop) | Laptop resources (k3s + Kind VClusters) | $0 | $0 |

**Note**: Costs are illustrative and vary by region, workload type, and data transfer.

---

## 0.6 Developer Scenarios Overview

CAVE is designed to handle 30+ distinct developer and operator workflows. Each scenario is tested and documented in runbook sections (§N). This section provides a complete reference table.

### Scenario Reference Table

| # | Scenario | Description | Primary Runbook Sections | Key Components |
|---|----------|-------------|--------------------------|-----------------|
| 1 | Pre-commit secret leak | Pre-commit hook detects and blocks credential in code | §7 (Supply Chain), §8 (Secret Rotation) | Gitleaks, pre-commit framework |
| 2 | Developer creates database | Dev declares PostgreSQL XR in their namespace; Crossplane provisions within 3 min | §23 (Infrastructure as Code) | Crossplane, CloudNativePG, External Secrets |
| 3 | Tenant promotes via canary | Argo Rollouts progressively shifts traffic (10% → 50% → 100%) with auto-rollback on metric deviation | §20 (Deployment Pipelines), §22 (GitOps) | Argo Rollouts, Prometheus, Grafana |
| 4 | Crossplane drift correction | Resource modified outside of Kubernetes (e.g., via Azure portal); Crossplane detects and reapplies declared state | §24 (Operational Excellence) | Crossplane Compositions, drift detection |
| 5 | Unsigned image rejected | Pod with unsigned image hits OPA Gatekeeper; admission denied with audit log | §6 (Security), §7 (Supply Chain) | OPA Gatekeeper, Cosign, Sigstore |
| 6 | Database credential rotation | ESO detects secret rotation; new credentials injected into pod without restart | §6 (Security), §8 (Secret Rotation) | External Secrets Operator, Crossplane |
| 7 | Tenant offboarding | Tenant marked for deletion; 30-day retention policy; data exported to customer; cluster state deleted on day 31 | §21 (Multi-Tenancy) | Velero, RBAC, namespace-level policies |
| 8 | Control plane down | Backstage and ArgoCD offline; cluster heals itself via Reflex Engine; Emergency CLI available | §36 (Disaster Recovery) | Prometheus alert → Reflex Engine, Emergency CLI |
| 9 | AI attempts namespace delete | APOL Diagnostician suggests namespace deletion; Guardian AI votes no; decision logged in Sovereign Ledger | §40 (APOL Orchestration) | OPA Gatekeeper, Sovereign Ledger, Constitutional Guardian |
| 10 | SOC2 auditor request | Auditor requests evidence of access, changes, and audit logs for a date range; Backstage generates compliant export in 10 min | §42 (Compliance & Audit) | Sovereign Ledger, Prometheus, Grafana |
| 11 | PostgreSQL IOPS saturated | Metrics alert fires; Reflex detects saturation; auto-scales storage and IOPS; resolves in <2 min | §27.2 (Automated Remediation) | Prometheus, Reflex Engine, Crossplane |
| 12 | Kong fails at 3 AM | Kong pod OOMKilled; OnCall escalates; Diagnostician analyzes; issue root-caused; fixed in <12 sec | §26 (Chaos & Testing), §27.2 (Remediation) | Tetragon, Grafana OnCall, Chaos Mesh |
| 13 | Renovate upgrade blocked | Renovate proposes dependency upgrade; Pluto (deprecated API checker) scans; blocks if API is deprecated; Backstage shows reason | §27.1 (Deployment Safety) | Renovate, Pluto, Backstage |
| 14 | Tenant at 155% budget | Tenant exceeds $10k/month limit; non-critical workloads scaled to zero; critical workloads continue; Backstage notifies team | §30 (FinOps & Cost Control) | OpenCost, Reflex Engine, kill switch |
| 15 | LLM prompt injection blocked | LiteLLM detects prompt injection attempt; blocks request; logs to Sovereign Ledger with evidence | §14 (AI Services), §40 (APOL) | LiteLLM, OPA Gatekeeper, Sovereign Ledger |
| 16 | Dormant admin auto-disabled | Admin account unused for 95 days; Identity provider auto-disables; notification sent; re-activation requires manager approval | §9 (RBAC & Access) | Keycloak, CronJob, audit log |
| 17 | JIT admin granted | Developer requests 4-hour database access; dual approval required; session recorded; TTL enforced; access revoked at 4h | §9 (RBAC & Access) | Teleport PAM, Keycloak, Grafana OnCall |
| 18 | Annual exit drill | Hetzner → Azure migration drilled; tenant data synced; validation confirmed; disaster recovery playbook validated | §29 (Exit & Portability) | Velero, pg_dump, object storage migration |
| 19 | Shadow IT discovered | `cave-ctl doctor` scan finds EC2 instance not in Crossplane; alerts security; analyst flags as compliance violation | §39 (Platform Diagnostics) | cave-ctl doctor, AWS CLI, Asset inventory |
| 20 | TB-scale MinIO→ADLS | Tenant requests cloud migration; Reflex Engine orchestrates parallel copy; verifies checksums; cutover with zero downtime | §29 (Exit & Portability) | MinIO, ADLS, rclone, Reflex Engine |
| 21 | Egress spike detected | Unusual traffic spike detected; automatic egress quarantine enabled; Safe-Exit FQDNs preserved; human review triggered | §30 (Cost Control), §5 (Network) | Cilium, Prometheus, egress policies |
| 22 | APOL unavailable | All 4 AI agents unreachable; fallback mode activated; Tier 1 Crossplane Operations continue; manual escalation enabled | §40 (APOL Orchestration) | Backup operators, fallback policies |
| 23 | OnCall escalation | P1 alert not acknowledged within 5 min; Grafana OnCall escalates to on-call manager; incident page auto-created | §17 (Observability), §27 (Incident Response) | Grafana OnCall, PagerDuty integration |
| 24 | Network policy after upgrade | Cilium version upgraded; existing network policies broken; test suite detects; automatic rollback via ArgoCD | §5 (Network & Security), §27.1 (Deployment Safety) | Cilium, policy-as-code tests, ArgoCD |
| 25 | DB XR without labels | Dev creates PostgreSQL XR missing required `classification` label; OPA rejects; error message explains requirement | §23 (Infrastructure), §6 (Security) | Crossplane, OPA Gatekeeper, OPAL |
| 26 | Scheduled DB maintenance | CronOperation triggers weekly checkpoint, analyze, and VACUUM on all tenant databases; no downtime | §23 (Infrastructure), §27.2 (Remediation) | Crossplane CronOperation, CloudNativePG |
| 27 | ArgoCD syncs from Harbor | Deployment source is OCI registry artifact; provenance verified; attestation checked; sync proceeds only if valid | §22 (GitOps), §6 (Security) | ArgoCD, Harbor, Sigstore, SLSA |
| 28 | Backstage plugin addition | Platform team adds Grafana plugin via declarative YAML; Backstage reloaded via GitOps; developers see it immediately | §19 (Developer Portal) | Backstage, declarative plugin config, ArgoCD |
| 29 | MRAP blocks Azure MR | Developer on dev profile attempts to use expensive multi-region database; MRAP Composition Function selects cheaper single-region on dev | §23 (Infrastructure), §12 (FinOps) | Crossplane MRAP, Composition Functions |
| 30 | Tenant at 125% budget | Tenant at $12.5k on $10k limit; batch workloads suspended; payment processing continues; finance alerted | §30 (FinOps & Cost Control) | OpenCost, Reflex Engine, kill switch |
| 31 | cave-ctl upgrade check | User runs `cave-ctl upgrade check`; tool identifies breaking changes in upstream dependencies; proposes safe upgrade path | §27.1 (Deployment Safety) | cave-ctl, dependency graph, semantic versioning |
| 32 | Egress quarantine with Safe-Exit | Egress quarantine blocks unknown FQDNs; Safe-Exit FQDNs (S3, GitHub, Docker Hub) preserved for critical CI/CD | §30 (Cost Control), §5 (Network) | Cilium egress policies, Safe-Exit list |
| 33 | APOL quarantines namespace | Anomalous activity in namespace detected by APOL Analyst; namespace cordoned; reasoning trace stored in Sovereign Ledger | §40 (APOL Orchestration) | OPA Gatekeeper, APOL, Sovereign Ledger |
| 34 | PAM break-glass session | SRE initiates break-glass CLI access to node; Teleport records entire session with keystroke logging; audit trail in Sovereign Ledger | §9 (RBAC), §40 (APOL) | Teleport, Hardware keys, Sovereign Ledger |
| 35 | Vendor access via PAM | Third-party vendor granted 2-hour database access; session recorded with MFA; access auto-revoked; no password shared | §9 (RBAC), §40 (APOL) | PAM (Teleport/CyberArk), hardware keys |
| 36 | OPAL updates tenant policy | Tenant switches from `region: any` to `region: apac`; OPAL pushes policy update; all subsequent XR creations validated against new policy | §6 (Security), §11 (Policy-as-Code) | OPAL, OPA Gatekeeper, APOL |

---

## 0.7 Document Hierarchy & Navigation

The CAVE Runbook is structured in a strict hierarchy to ensure consistency and navigability.

### Directory Structure

```
docs/
├── runbook/
│   ├── 00-executive-summary.md          (This file)
│   ├── 01-security-architecture.md      (Zero-trust, supply chain, encryption)
│   ├── 02-identity-access-management.md (Keycloak, RBAC, JIT, break-glass)
│   ├── ...
│   ├── 40-apol-autonomous-operations.md
│   ├── 41-constitutional-guardianship.md
│   ├── 42-compliance-audit.md
│   └── 99-troubleshooting-index.md
├── adr/
│   ├── ADR-001-kubernetes-distribution.md
│   ├── ADR-067-crossplane-abstraction.md
│   └── ...
├── operations/
│   ├── runbooks-incident/
│   │   ├── kong-failure.md
│   │   └── ...
│   └── playbooks/
│       ├── cluster-resize.md
│       └── ...
└── compliance/
    ├── soc2-controls.md
    └── export-templates/
```

### Synchronization Rules

1. **ADRs are canonical**: Every architectural decision is recorded in an ADR. Runbook sections reference ADRs as rationale.
2. **Runbook sections are authoritative**: Operational procedures, configuration examples, and troubleshooting are documented in runbook sections.
3. **Incident runbooks are logs**: When an incident occurs, the response is documented in `runbooks-incident/`, which informs future updates to permanent runbooks.
4. **Quarterly review**: Platform team reviews all documents quarterly, updating based on operational experience.
5. **Roadmap annotation**: Each section links to upcoming changes in the roadmap, so readers understand what is changing and when.

### ADR Categories

- **00–09**: Foundational (Kubernetes, GitOps, IDP definition)
- **10–19**: Control Plane & Developer Experience
- **20–29**: Infrastructure & Provisioning (Crossplane)
- **30–39**: Security & Policy
- **40–49**: Observability & Operations
- **50–59**: AI & Autonomous Systems
- **60–69**: Cloud Providers & Exit Strategy
- **70–79**: Compliance & Audit
- **80–89**: Multi-Tenancy & Isolation
- **90–99**: Advanced Topics (APOL, Chaos, etc.)
- **100+**: Provider-Specific Extensions

---

## 0.8 How to Use This Runbook

The CAVE Platform Runbook is a reference document for developers, operators, security engineers, and platform managers. This section explains how to navigate it effectively.

### For Different Roles

**Developers**:
- Start with §19 (Developer Portal) to understand Backstage.
- Read §23 (Infrastructure as Code) to learn how to declare databases, caches, and queues.
- Refer to §20 (Deployment Pipelines) for CI/CD workflows.
- Consult §30 (FinOps) to understand cost implications of infrastructure choices.

**Operators & SREs**:
- Read §24 (Operational Excellence) for day-to-day operations.
- Study §27 (Incident Response & Chaos) for troubleshooting.
- Review §40 (APOL) to understand autonomous operations.
- Maintain §36 (Disaster Recovery) procedures.

**Security Engineers**:
- Start with §1 (Security Architecture) for zero-trust and threat modeling.
- Study §6 (Authentication, Authorization, Encryption) for cryptographic controls.
- Review §42 (Compliance & Audit) for audit trail and evidence export.
- Monitor §40 (APOL) for security-relevant AI decisions.

**Platform Managers**:
- Read this document (§0) for high-level overview.
- Review §5 (Cost & Compliance Overview) for business metrics.
- Study ADR categories for decision context.
- Check the roadmap quarterly.

### Format of Each Runbook Section

Each runbook section (§N) follows a standard format:

1. **Objective**: One-sentence summary of the section's purpose.
2. **ADR References**: Links to architectural decisions explaining why.
3. **Key Components**: 5–10 components most relevant to this section.
4. **Tool Comparison Table**: If multiple tools are available, a comparison of trade-offs (complexity, cost, security, observability).
5. **Roadmap & Upcoming Changes**: What is planned for the next release.
6. **Architecture Diagram**: High-level diagram of systems involved (ASCII or embedded SVG).
7. **Configuration Examples**: Real, minimal examples (not production-scale, but representative).
8. **Operations & Runbooks**: Step-by-step procedures for common tasks.
9. **Troubleshooting**: Common problems and diagnostics.
10. **Compliance & Audit**: Which controls apply, how to verify.

### Cross-References

Sections reference each other using §N notation (e.g., "see §24 for drift detection"). This allows readers to follow a learning path. For example:

- **Developer onboarding path**: §0.6 (scenarios) → §19 (Backstage) → §23 (infrastructure) → §20 (deployments) → §30 (cost)
- **Security audit path**: §42 (compliance) → §1 (security architecture) → §6 (crypto) → §40 (APOL decisions)
- **Incident response path**: §27 (incident response) → specific runbook section → §24 (postmortem) → §41 (constitutional review)

### How to Find Information

- **By role**: See "For Different Roles" above.
- **By component**: §0.3 (Component Map) lists all components and their runbook sections.
- **By scenario**: §0.6 (Developer Scenarios) lists 30+ workflows and their sections.
- **By error message**: §99 (Troubleshooting Index) is searchable by error text.
- **By compliance requirement**: §42 (Compliance & Audit) lists controls and sections.

### Living Document Practices

- The runbook is version-controlled in Git. Every change is a commit with rationale.
- Runbook updates are validated by platform team and SRE team before merge.
- Quarterly reviews ensure sections stay current with operational reality.
- Incident reports trigger runbook updates. Every post-mortem action item includes runbook updates.

---

## Summary

CAVE is a comprehensive, principled platform designed for operational excellence, security, and developer productivity. This Executive Summary provides the foundation for understanding CAVE's architecture, components, and operational model. Each section in the runbook expands on these principles with concrete procedures, diagrams, and examples.

**Next Steps**:
- Developers: Start with §19 (Developer Portal).
- Operators: Study §24 (Operational Excellence) and §27 (Incident Response).
- Security: Read §1 (Security Architecture) and §42 (Compliance & Audit).
- Managers: Review the roadmap and ADR categories.

---

**Document Info**:
- **Version**: 1.0
- **Last Updated**: 2026-03-06
- **Maintainers**: Platform Engineering Team
- **Status**: Active, reviewed quarterly
- **Next Review**: 2026-06-06
