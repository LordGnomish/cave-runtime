# CAVE Platform Runbook §02 — Deployment Profiles

**Domain:** caveplatform.dev
**Platform:** Cloud-Agnostic Virtualized Environment (CAVE) — Integrated Development Platform
**Version:** 2.0 (March 2026)
**Status:** Stable
**Key ADRs:** ADR-066, ADR-067, ADR-094, ADR-098

---

## 2.1 Overview

CAVE's deployment profile system is the foundational concept that enables the platform's core value proposition: **infrastructure-agnostic, environment-consistent application delivery**. Rather than forcing operators to learn separate tools and procedures for each cloud provider, CAVE abstracts infrastructure differences behind a simple, standardized interface.

The platform supports **seven deployment profiles**: three environment tiers (development, staging, production) across two cloud providers (Hetzner Cloud and Azure), plus a local development profile. Each profile represents a complete, validated infrastructure topology that can be provisioned with a single `cave-ctl` command, scaled according to workload requirements, and promoted through the full software development lifecycle using identical operational procedures.

### The Seven Profiles

**Hetzner Cloud Tier:**
- **hetzner-dev**: Minimal, single-node-pool cluster for rapid iteration and testing
- **hetzner-staging**: Mid-scale, multi-AZ cluster for infrastructure validation
- **hetzner-prod**: High-availability, geo-distributed cluster for production workloads

**Azure Cloud Tier:**
- **azure-dev**: AKS cluster with minimal managed service SKUs
- **azure-staging**: AKS cluster with mid-tier managed services
- **azure-prod**: Zone-redundant AKS with premium managed services

**Local Development:**
- **local**: Lightweight vcluster-based environment for single-developer workflows

### Profile-Driven Operations

CAVE employs a **profile-driven architecture pattern** where the same operational command produces environment-specific behavior based on the active profile. An engineer running `cave-ctl bootstrap` against a staging profile will receive a multi-AZ staging environment. The same command against a dev profile produces a minimal, single-AZ environment. This consistency dramatically reduces cognitive load and error rates during environment transitions.

```
cave-ctl create dev hetzner      # Creates minimal Hetzner environment
cave-ctl create staging azure    # Creates mid-scale Azure environment
cave-ctl create prod azure       # Creates HA Azure production cluster
cave-ctl local up                # Spins up local vcluster in 3-5 minutes
```

The profile system accommodates the **dual SDLC model** that CAVE enforces: Platform SDLC governs infrastructure changes (new profiles, updated machine types, networking topology), while Tenant SDLC governs application workload deployments. This separation ensures that infrastructure stability is not compromised by application deployment velocity.

---

## 2.2 ADR Rationale & Decision Record Synthesis

### ADR-066: Multi-Provider Exit Strategy & Portability

**Problem:** Cloud vendor lock-in represents existential risk. Organizations adopting CAVE needed contractual and operational assurance that workloads could migrate between cloud providers without re-architecting the platform.

**Decision:** Implement dual-provider parity architecture where every Hetzner Cloud service has an equivalent Azure service, with standardized mappings maintained in the platform configuration.

**Rationale:**
- Hetzner offers 40-60% cost savings on compute-intensive workloads
- Azure provides native Microsoft service integrations and compliance certifications
- Rather than accepting trade-offs between cost and compliance, CAVE maintains both options simultaneously
- Operators can make economics-driven decisions during capacity planning cycles

**Evaluated Alternatives:**
- *Single-provider (Hetzner only):* Abandoned because enterprise customers require Microsoft cloud integrations
- *Single-provider (Azure only):* Unacceptable TCO for cost-conscious organizations
- *Three+ providers (AWS, GCP):* Deferred to Phase 3; maintaining two provides proof-of-concept for general multi-cloud pattern
- *Cloud abstraction layer (CNCF clusters):* Insufficient — managed service differences require explicit handling

**Consequences:**
- Platform maintenance burden increases with each new service or feature
- Terraform/OpenTofu code duplication unavoidable; DRY principles applied to values, not infrastructure code
- Quarterly portability drills required to validate bidirectional migration procedures
- Each provider has dedicated platform team (currently notional; scaled at 50+ tenant milestone)

**Mapping Matrix:**
| Service | Hetzner | Azure | Equivalence |
|---------|---------|-------|-------------|
| Compute (IaaS) | Hetzner Cloud VMs | Azure VMs | Direct |
| Kubernetes | Talos Linux + custom | AKS (managed) | Functional |
| PostgreSQL | Hetzner Database | Azure Database for PostgreSQL | Managed service |
| Redis | Hetzner Redis | Azure Cache for Redis | Cache layer |
| Object Storage | S3 (Hetzner S3) | Azure Blob Storage | Object layer |
| Secrets | HashiCorp Vault (self-hosted) | Azure Key Vault | Secrets management |
| Load Balancer | Hetzner Cloud LB | Azure Load Balancer | Layer 4 |
| WAF/CDN | Kong + Cilium | Application Gateway | Layer 7 |
| DNS | Hetzner DNS | Azure DNS | Domain management |

---

### ADR-094: Profile Structure & Environment Topology Definition

**Problem:** Organizations require different infrastructure topologies for different lifecycle stages, but describing these differences in IaC without inconsistency is non-trivial. Absence of explicit profile definitions led to ad-hoc, unmaintainable environment variance.

**Decision:** Define profiles as declarative bundles combining: (a) infrastructure topology, (b) resource sizing, (c) component manifests, and (d) observability/compliance settings. Profiles are immutable and versioned alongside code.

**Motivation for Seven Profiles:**
- Three environments (dev/staging/prod) map to organizational decision-making gates
- Two providers enable cost and compliance optimization
- Seven enables independent scaling decisions: dev and staging can be co-hosted on one provider while prod runs on another
- Seven also enables cost-efficient replication: small teams can omit staging, advanced teams can run A/B prod environments

**Profile Selection Logic:**
```
if [ "$ENVIRONMENT" = "production" ] && [ "$PROVIDER" = "azure" ]; then
  apply azure-prod profile, scaling to 3+ zones, premium service tiers
elif [ "$ENVIRONMENT" = "staging" ] && [ "$PROVIDER" = "hetzner" ]; then
  apply hetzner-staging profile, 2-zone setup, standard service tiers
elif [ "$ENVIRONMENT" = "development" ]; then
  apply minimal profile (either provider), single zone, basic service tiers
fi
```

**Consequences:**
- Seven discrete code paths require comprehensive testing
- Promotion pipeline must validate that promoting code from one profile to the next produces expected infrastructure deltas (terraform plan analysis)
- Profile upgrades trigger sequential validation: dev → staging → prod

---

### ADR-098: Immutable Infrastructure via Talos Linux on Hetzner

**Problem:** Traditional Linux distributions grant operators SSH access, enabling ad-hoc mutations that diverge from declared state. This creates **configuration drift**, security vulnerabilities, and unaudited changes.

**Decision:** Adopt Talos Linux for all Hetzner environments, providing immutable, declarative node management via standard Kubernetes APIs.

**Immutability Benefits:**
- Every node is replaced during updates, not patched in place
- Node state is derived entirely from Talos machine config, no side effects
- SSH access is cryptographically disabled; all changes flow through Kubernetes API or `talosctl`
- Audit trail is complete: every change is logged and traceable

**Why Talos, Not Traditional Linux:**
- Immutable by design; traditional Linux requires external tooling (Ignition, cloud-init scripts) and remains mutable
- Kubernetes-native: nodes are managed as Kubernetes custom resources, not as separate infrastructure
- Smaller attack surface: ~50MB disk footprint vs. 4GB for full Linux
- Automated security patching: Kubernetes orchestrates node drains and reboots

**Hetzner-Specific Justifications:**
- Hetzner Cloud provides bare-metal provisioning APIs compatible with Talos boot process
- Hetzner allocates IP addresses to nodes within seconds, enabling rapid cluster bootstrap
- Unlike managed Kubernetes (AKS, EKS), Hetzner requires explicit node OS management; Talos provides best-practice automation

**Consequences:**
- Azure environments use managed AKS, not Talos (Azure controls node OS)
- Single SSH key exists per cluster: `talosctl` certificate, not traditional keypairs
- Emergency node access requires cluster API connectivity; out-of-band SSH impossible
- Learning curve for operators unfamiliar with declarative infrastructure

---

### ADR-067: Two-Layer Provisioning (OpenTofu Day 0 + Crossplane Day 1+)

**Problem:** Infrastructure-as-code tools excel at infrastructure provisioning (Day 0), but struggle with ongoing platform engineering tasks (Day 1+). Cloud-native applications need dynamic resource provisioning driven by application state, not static infrastructure definitions.

**Decision:** Implement **two-layer provisioning**:
- **Day 0 (OpenTofu):** Cluster topology, networking, initial service deployments
- **Day 1+ (Crossplane):** Application-driven resource creation (databases per tenant, S3 buckets per workload)

**Day 0: OpenTofu's Role**

OpenTofu bootstraps the complete cluster infrastructure in a single, idempotent operation:

1. **Cluster foundation:** Control plane and worker node placement, storage classes, CNI (Cilium)
2. **Managed services:** Database instances, caches, load balancers, DNS records
3. **Platform services:** Keycloak, Kong, Prometheus, Grafana, Vault (all as Helm charts via Kubernetes provider)
4. **RBAC & networking:** Kubernetes service accounts, network policies, ingress configurations
5. **Observability & compliance:** Prometheus targets, CloudWatch agents, audit logging

OpenTofu executes once per environment, producing a stable, known-good cluster state suitable for running production workloads.

**Day 1+: Crossplane's Role**

Crossplane takes over after cluster exists, enabling **GitOps-native resource provisioning**:

- **Multi-tenant database isolation:** When tenant X is onboarded, a Tenant CR triggers Crossplane to provision a PostgreSQL database, create schemas, and inject connection secrets into the tenant's namespace
- **Storage provisioning:** Tenant uploads YAML requesting S3 bucket with replication; Crossplane creates bucket in real time
- **Managed database parameters:** Changing memory limits, backup retention, or failover behavior updates managed service via Crossplane, not via cloud console
- **Compliance automation:** Applying a policy CR automatically configures encryption, backup windows, and access controls across all resource instances

**Why Two Layers, Not One:**

Single-tool approaches fail because:
- **Pure Terraform/OpenTofu:** Cannot respond to runtime application needs; cluster is static
- **Pure Crossplane:** Cannot initialize cluster (chicken-and-egg: Crossplane runs on cluster, but cluster must exist first)
- **Helm only:** Lacks infrastructure abstraction; cloud-native but operator-hostile

Two layers provide **role-appropriate tooling**: infrastructure engineers provision clusters with OpenTofu, application teams manage their workload infrastructure with Crossplane.

**Evaluated Alternatives:**
- *Terraform + provisioners:* Too imperative; encourages scripts and drift
- *Pulumi:* Excellent conceptually, but Python/TypeScript lock-in conflicts with OpenTofu governance
- *AWS CDK / Azure Bicep:* Vendor-specific; contradicts multi-cloud ADR-066
- *Pure Helm:* Insufficient for infrastructure-level decisions

---

## 2.3 Tool Comparison Matrix

CAVE's technology selections were validated against multi-dimensional criteria. Below is the evaluation matrix that informed IaC tooling and Kubernetes distribution decisions.

### Infrastructure-as-Code Comparison: Day 0 Provisioning

| Criterion | OpenTofu | Terraform | Pulumi | CDK | Score Notes |
|-----------|----------|-----------|--------|-----|-------------|
| **Open Source** | 5 | 5 | 4 | 3 | OpenTofu: Mozilla Public License 2.0, community fork of Terraform; Terraform: BSL (now 1.6+); Pulumi: BSL; CDK: Apache 2.0 but AWS ecosystem lock-in |
| **Multi-Cloud Coverage** | 5 | 5 | 4 | 2 | OpenTofu & Terraform: 2000+ providers; Pulumi: good but Python-first; CDK: AWS native, GCP/Azure support lagging |
| **State Management** | 5 | 5 | 4 | 3 | Stateless providers (Pulumi) lack full consistency model; OpenTofu state backend flexibility unmatched |
| **Modularity & Reusability** | 4 | 4 | 5 | 3 | Pulumi enables functions/classes for reusability; OpenTofu requires module stubs; CDK: black-box synthesis |
| **Community & Support** | 5 | 5 | 3 | 4 | OpenTofu community energized post-fork; Terraform enterprise support from HashiCorp; Pulumi smaller community |
| **Learning Curve** | 4 | 4 | 3 | 2 | HCL minimal learning; Pulumi Python adds context switching; CDK requires TypeScript |
| **GitOps Integration** | 5 | 5 | 3 | 2 | Both natively support plan-commit-apply workflows; Pulumi/CDK require wrapping in CI/CD |
| **Cost (Operational)** | 5 | 3 | 2 | 3 | OpenTofu: free, open-source; Terraform: $0 OSS but Enterprise $10k+; Pulumi: $5k+ platform; CDK: AWS console costs |
| **AVERAGE SCORE** | **4.6** | **4.4** | **3.6** | **2.6** | **Selection: OpenTofu** for cost-conscious, multi-cloud, open-governance organizations |

**CAVE Selection:** OpenTofu (with Terraform compatibility mode for existing modules)

**Justification:** OpenTofu provides the best balance of multi-cloud support, state management sophistication, and cost. The Mozilla Public License ensures community control. Terraform modules remain compatible, reducing migration friction. Pulumi's higher modularity score does not compensate for vendor platform dependency, which contradicts CAVE's portability ethos.

---

## 2.4 Roadmap Analysis: 24-Month Technology Evolution

### OpenTofu Trajectory

**Current State (Q1 2026):** OpenTofu 1.9+, achieving feature parity with Terraform 1.8. CNCF Incubation sandbox acceptance signals vendor-neutral governance.

**H1 2026 Roadmap:**
- Native OIDC provider bindings (eliminating cross-cloud secret sprawl)
- Workspaces 2.0: improved state isolation for multi-tenant environments
- Provider SDK 2.0: faster iteration cycles for community providers

**H2 2026+ Expectations:**
- CNCF Graduated status (estimated Q4 2026), triggering enterprise adoption acceleration
- Integration with Kubernetes Operator SDK for generating infrastructure from CRDs
- Enhanced drift detection without full plan evaluation (critical for large environments)

**CAVE Alignment:** Roadmap trajectory aligns with CAVE's Crossplane strategy. As OpenTofu improves Kubernetes integration, Day 0/Day 1 boundary becomes more flexible.

---

### Talos Linux Evolution

**Current State (Q1 2026):** Talos v1.11 stable, v1.12 beta with breakthrough features.

**v1.12 (Q2 2026) Introduces:**
- **OOM Handler:** Proactive out-of-memory management, preventing node thrashing during memory pressure
- **Staged Networking:** Network interface setup decouples from Kubernetes API readiness, enabling faster pod scheduling
- **User Volumes:** Persistent storage on node local disk (non-volatile, ephemeral cache tier)
- **API Gateway Support:** Talos API can route through Kubernetes ingress, reducing need for bastion hosts

**Multi-Year (2026-2028):**
- Boot time reduction: target 15 seconds (from current 30)
- GPU support maturation: NVIDIA NCCL integration
- Machine Config versioning with automated rollback

**CAVE Strategy:** Hetzner staging/prod profiles will adopt v1.12 in Q3 2026, after 6-week validation in dev environments. User volumes enable stateful workloads (MinIO, PostgreSQL replicas) directly on nodes, reducing managed service dependency.

---

### Azure Kubernetes Service (AKS) Platform Evolution

**Current State (Q1 2026):** AKS supports Kubernetes 1.29 stable, 1.30 preview. Azure Container Instances integration deepening.

**H1 2026 Commitments:**
- **Node pool autoscaling:** Improved bin-packing algorithms, reducing overprovisioned capacity by 15-20%
- **Spot VM integration:** Dedicated spot node pools with automatic failover to standard nodes
- **Application Gateway for Containers:** Native gateway architecture, replacing NGINX/Kong for Azure workloads

**Implications for CAVE:**
- Azure-prod profile will introduce dedicated spot pools for non-critical workloads (platform observability, batch jobs), reducing EUR 200-400/month spend
- Native gateway support removes Kong dependency for Azure-only deployments; CAVE maintains Kong for Hetzner parity
- Node auto-scaling improvements reduce need for manual capacity planning in azure-staging profile

---

### Hetzner Cloud Platform Evolution

**2025-2026 Roadmap:**
- **New Regions:** Helsinki (EU-FI) and Ankara (TR) regions coming H2 2026
- **Hetzner Cloud DNS:** Managed DNS service (currently third-party only)
- **Network Isolation:** Enhanced private network capabilities for multi-tenant isolation
- **Kubernetes Cost Transparency:** Quarterly billing API improvements for consumption tracking

**CAVE Opportunity:** New regions enable GDPR data residency options. Multi-region CAVE clusters (Hetzner dev in DE, staging in FI, prod replicated across both) become viable without provider switch.

---

## 2.5 Architecture: Profile Specifications

### Hetzner Dev Profile

**Cluster Topology**

The Hetzner dev profile is CAVE's "minimum viable cluster" — the smallest infrastructure capable of running all platform components for local iteration and testing.

```
Control Plane:  1× CX32 (Hetzner general-purpose VM)
                4 vCPU, 8GB RAM, 40GB NVMe
Worker Nodes:   2× CX42 (scale-up VM, 6 vCPU, 16GB RAM, 60GB NVMe)
Availability:   Single AZ (Frankfurt eu-central-1 default, changeable)
Storage:        Local volumes on worker nodes, no persistent managed storage
Total Monthly:  EUR 50-80 (compute only; includes bandwidth to 20TB limit)
```

**Operating System & Node Management**

Talos Linux immutable, declarative nodes. Machine config versioned in Git. Updates via `talosctl` trigger automatic node reboots with no manual intervention.

**Networking**

- **Container Networking:** Cilium eBPF-based CNI, single-node network policies fully evaluated
- **Load Balancing:** Kong ingress controller deployed as single replica; suitable for internal testing, not external traffic
- **Service Exposure:** ClusterIP and NodePort services only; LoadBalancer type services fail gracefully (no cloud LB provisioned)

**Platform Components (Phase 1 Only)**

Hetzner dev cluster runs CAVE Phase 1 components exclusively:
- Kubernetes control plane (etcd, API server, kubelet)
- Cilium & Cilium Network Policies
- Kong Ingress Controller
- HashiCorp Vault (self-hosted, single replica)
- Keycloak (pre-seeded with local dev tenant)
- Prometheus (metrics scraping)
- PostgreSQL (Helm-deployed StatefulSet, uses local volume)
- MinIO (object storage, local node storage)
- Valkey (in-memory cache, local storage)

Phase 2 components (advanced observability, multi-cluster federation, GitOps) are omitted to reduce resource contention.

**Storage**

All storage is ephemeral or local:
- PostgreSQL uses local PV backed by node disk
- MinIO uses local PV; loss of node = loss of data (acceptable for dev)
- ConfigMaps and Secrets stored in etcd only
- No cloud provider snapshot capabilities

**Use Cases**

- Fresh development environment: empty slate for feature testing
- Integration testing: Kubernetes API available, realistic cluster behavior
- Configuration validation: test OpenTofu module changes before applying to staging
- Cost minimization: lowest cost CAVE deployment for testing organization policies

**Limitations**

- Single control plane is SPOF; node loss = cluster unavailable
- No cross-AZ networking; cannot test multi-region patterns
- Limited to internal testing; external traffic requires manually exposing Kong via `kubectl port-forward`
- Local storage loss = data loss; not suitable for persistent workload testing

---

### Hetzner Staging Profile

**Cluster Topology**

Staging replicates production architecture at reduced scale, enabling realistic infrastructure validation before production deployment.

```
Control Plane:  3× CX32 (identical to dev compute, but high availability)
                Distributed across 2 AZs (Frankfurt eu-central-1a/1b)
                Etcd stores replicated 3-way
Worker Nodes:   3× CX52 (6 vCPU, 16GB RAM, 80GB NVMe)
                Mixed AZ distribution
Availability:   2 AZs (3-way replication across zones)
Storage:        Hetzner Managed Volumes (multi-AZ replicated block storage)
Total Monthly:  EUR 120-180 (includes managed volumes, inter-AZ transfer)
```

**Networking**

- **Container Networking:** Cilium with full NetworkPolicy enforcement; multi-AZ networking fully evaluated
- **Load Balancing:** Hetzner Cloud Load Balancer (L4) fronts Kong; TLS termination at LB, passthrough to Kong
- **Ingress:** Kong configured with multi-replica deployment for HA
- **Service Mesh (optional):** Istio ambient mode available for advanced routing testing

**Platform Components (Phase 1 + Phase 2)**

Staging runs full platform stack:
- Phase 1: Kubernetes, Cilium, Kong, Vault, Keycloak, Prometheus, PostgreSQL, MinIO, Valkey
- Phase 2: Prometheus Operator, Grafana, Alertmanager, Loki, OpenTelemetry collectors
- Service Mesh: Istio (lightweight ambient mode, ztunnel only)
- Crossplane: Ready for Day 1+ provisioning validation

**Storage & Persistence**

- PostgreSQL: Managed Hetzner Database instance (replicated, daily backups)
- MinIO: Hetzner Managed Volume-backed, multi-replica MinIO cluster
- Persistent Volumes: 5-20GB available, suitable for test workload state

**Use Cases**

- **Platform infrastructure validation:** Verify that OpenTofu changes produce expected topology before production
- **High-availability testing:** Multi-AZ failover scenarios, network partition recovery
- **Component integration testing:** Full platform stack available for integration testing
- **Load testing:** Staging cluster can sustain 1-5K requests/sec before saturation
- **Disaster recovery drills:** Practice backup restoration, cluster recovery procedures

**Not For:**
- Production workload hosting (no compliance guardrails, not multi-tenant secure)
- Long-running state: staging cluster can be destroyed and recreated daily

---

### Hetzner Prod Profile

**Cluster Topology**

Production cluster is CAVE's most robust configuration, designed for continuous operation serving thousands of requests and hosting dozens of independent tenants.

```
Control Plane:  3× CX32 dedicated (no workload scheduling)
                Distributed across 3 AZs (Frankfurt eu-central-1a/1b/1c)
                Etcd stored with 3-way replication, daily snapshots to S3
Worker Nodes:   5+ (autoscaling 5-30 based on tenant load)
                Mix of CX52 (general), CPX42 (CPU-optimized), and CX62 (memory-optimized)
                Mixed AZ distribution, spreading load evenly
Availability:   3 AZs, any single AZ loss does not impact workload availability
Storage:        Hetzner Managed Volumes for all stateful components
Total Monthly:  EUR 350-500 base (excludes tenant autoscaling, inter-AZ transfer)
```

**Networking**

- **Container Networking:** Cilium with eBPF acceleration, strict isolation between tenant namespaces
- **Load Balancing:** Hetzner Cloud Load Balancer fronts Kong with geographically distributed health checks
- **Ingress:** Kong deployed with 3+ replicas across AZs; automatic failover
- **Network Policies:** Comprehensive tenant isolation enforced at eBPF level (not just software)
- **Service Mesh:** Istio ambient mode optional; ambient provides mTLS, observability, advanced routing

**Platform Components (All Phases)**

- Phase 1: All components as in staging
- Phase 2: Full observability stack, service mesh, multi-cluster federation preparation
- Phase 3: Multi-cluster control plane, federation API, cross-region tenant migration

**Storage & Persistence**

- PostgreSQL: Hetzner Database instance with daily automated backups, 30-day retention
- MinIO: Managed Multi-Cloud Gateway (MCG) with bucket lifecycle policies, versioning
- Persistent Volumes: Dynamically provisioned Hetzner Managed Volumes, 100GB+ aggregate capacity available
- Backup: Daily full cluster state exported to Hetzner S3 (Hetzner Cloud) bucket, 90-day retention

**Compliance & Security**

- Network policies enforce tenant namespace isolation
- RBAC restricts cross-tenant operations
- Audit logging captures all API operations
- TLS 1.3 required for all inter-component communication
- Secrets encrypted at rest using sealed-secrets with cluster-specific keys

**Use Cases**

- **Production workload hosting:** Dedicated infrastructure for tenant workloads
- **Regulatory compliance:** SOC 2, ISO 27001, NIS2 audited and certified
- **High availability:** Single-AZ failure does not impact availability
- **Scalability:** Autoscaling accommodates 10x traffic spikes within 5 minutes

---

### Azure Dev Profile

**Cluster Topology**

Azure dev profile uses managed AKS, eliminating node OS management overhead compared to Hetzner.

```
AKS Cluster:    1 node pool, 1× Standard_D4s_v5 (4 vCPU, 16GB RAM)
                Linux nodes only (no Windows), Ephemeral OS disk
Availability:   Single AZ (default eastus, configurable)
Container Registry: Azure Container Registry (Basic tier, 10GB storage)
```

**Managed Services**

Azure dev profile leverages managed services unsuitable for Hetzner:
- **PostgreSQL:** Azure Database for PostgreSQL Flexible Server (Basic tier, 1 vCore, single replica)
- **Redis:** Azure Cache for Redis (Basic, C0 instance, 250MB)
- **Blob Storage:** Azure Storage Account (Standard tier, locally redundant)
- **DNS:** Azure Private DNS (for internal service discovery)

**Platform Components**

Identical to Hetzner dev: Phase 1 components only. Azure-managed services replace self-hosted Helm charts where applicable.

**Use Cases**

- Integration with Microsoft services (Office 365 sync, Teams notifications)
- Organizations standardized on Azure infrastructure
- Learning Kubernetes on Azure-native managed services

---

### Azure Staging Profile

**Cluster Topology**

```
AKS Cluster:    2 node pools
                Pool 1: 2× Standard_D4s_v5 (general workloads)
                Pool 2: 1× Standard_D8s_v5 (memory-intensive services)
Availability:   Single AZ, but AKS provides implicit node redundancy
```

**Managed Services**

- **PostgreSQL:** Standard tier, 1 vCore, multi-AZ failover capable
- **Redis:** Standard tier, 1GB
- **Blob Storage:** Standard tier, geo-redundant (stored in paired region automatically)
- **Application Gateway:** WAF enabled, request inspection

**Networking**

- Azure Virtual Network with subnets for control plane, worker nodes, managed services
- Network Security Groups restrict inter-subnet traffic
- Azure Firewall optional for outbound filtering

**Use Cases**

- Testing AKS features (node pools, managed identity integration, GPU pools)
- Validating Azure RBAC and security policies
- Load testing up to 5K requests/second

---

### Azure Prod Profile

**Cluster Topology**

```
AKS Cluster:    3+ node pools
                Pool 1: 3× Standard_D4s_v5 (general workloads, zone-redundant)
                Pool 2: 3× Standard_D8s_v5 (memory-intensive, zone-redundant)
                Pool 3: 1-3× Standard_NC6s_v3 (GPU-enabled, optional)
Availability:   Zone-redundant: nodes distributed across 3 availability zones
                Any single zone loss does not impact workload availability
Kubernetes:     v1.30 stable, auto-upgrade enabled for patch versions
```

**Managed Services**

All services deployed with premium tier and zone redundancy:
- **PostgreSQL:** Premium tier, 2+ vCores, geo-redundant backups, 35-day retention
- **Redis:** Premium tier, 2GB, zone-redundant
- **Blob Storage:** Premium tier, geo-redundant, failover to paired region
- **Application Gateway:** Enterprise grade, 3+ instances across zones, WAF advanced rules
- **Azure Firewall:** Standard tier, Central Hub & Spoke topology for multi-environment isolation

**Compliance & Monitoring**

- Azure Policy enforces compliance (no unapproved SKUs, encryption required, etc.)
- Azure Security Center: continuous threat assessment
- Azure Monitor: centralized logging and metrics from all Azure resources
- Defender for Kubernetes: container image scanning, runtime threat detection

**Use Cases**

- Production workload hosting for enterprise tenants
- Regulatory compliance (SOC 2, ISO 27001, HIPAA-eligible)
- High availability across availability zones
- Automatic scaling to 30+ nodes during peak demand

---

### Local Profile

**Cluster Topology**

The local profile is CAVE's rapid development environment, designed to spin up in 3-5 minutes on a developer's laptop.

```
Container Runtime: Docker Desktop or kind (Kubernetes in Docker)
Cluster:          vcluster (lightweight, single-namespace cluster)
Nodes:            1 virtual node (laptop CPU/memory backing)
Total Memory:     2-4GB required from host machine
Disk:             1-2GB for container images, state
```

**Bootstrap Command**

```bash
cave-ctl local up
# Spins up vcluster, deploys Phase 1 components, seeds tenant data
# Completes in 3-5 minutes on MacBook Pro, 5-10 minutes on older hardware
```

**Included Components**

Phase 1 components, all containerized:
- PostgreSQL: In-container, local volume, no persistence across restarts
- MinIO: In-container object storage
- Valkey: In-container cache
- Keycloak: Pre-seeded with platform admin + 1-3 local tenants for testing
- Kong: Exposed on `localhost:8443` with self-signed TLS (via mkcert)
- Prometheus: Basic metrics scraping
- Grafana: Pre-configured dashboards, `localhost:3000`

**Service Mesh Options**

- **Lightweight (default):** Istio ambient mode with ztunnel only (~100MB overhead)
- **Full ambient:** `cave-ctl mesh full --profile local` enables full sidecar injection, additional 500MB-1GB overhead

**TLS & DNS**

- mkcert generates self-signed CA on first run, installs to host OS trust store
- All services accessible via `https://localhost:*` or `https://service-name.local` (depends on service)
- Certificate renewal automatic on `cave-ctl local restart`

**Data Seeding**

On first boot, `cave-ctl local up` seeds:
- Platform admin user (username: `admin@caveplatform.dev`, password: `changeme`)
- Test tenant (username: `tenant-001@example.com`)
- Pre-created OIDC app for development (redirect URIs pointing to localhost)
- Sample workloads in manifests repository (optional)

**Storage & Persistence**

- Defaults to ephemeral: restarting vcluster loses all data
- Optional `--persistent` flag: uses Docker volume for PostgreSQL, survives restarts
- `cave-ctl local destroy` explicitly removes volumes (can be recovered from Git if workloads committed)

**Use Cases**

- **Initial setup:** New platform engineer can have working environment in 10 minutes
- **Feature development:** Code, test, redeploy locally before pushing to staging
- **Integration testing:** Full API contract validation against live services
- **Documentation examples:** All runbook examples work on local profile
- **Offline development:** No internet required after initial image pull

**Limitations**

- Single-node cluster: cannot test multi-node failure scenarios
- Resource-constrained: not suitable for load testing above 100 req/sec
- Ephemeral by default: not suitable for long-running workloads
- No cross-machine networking: cannot test multi-cluster patterns

---

## 2.6 Use Cases & Developer Scenarios

### Scenario 1: Platform Engineer Creates Hetzner Dev Environment from Scratch

**Context:** New platform engineer onboards. Needs a personal development cluster to iterate on OpenTofu modules.

**Procedure:**

```bash
# 1. Clone platform repository
git clone https://github.com/caveplatform/platform.git
cd platform/cave-ctl

# 2. Initialize Hetzner credentials
export HCLOUD_TOKEN="$(pass show hetzner/platform-token)"

# 3. Create dev cluster
cave-ctl create dev hetzner \
  --ssh-key-name "my-workstation" \
  --region "eu-central" \
  --project-name "alice-dev-cluster"

# 4. Cluster bootstrap progresses automatically
# OpenTofu: provisions VMs, networking, storage (5 min)
# Kubernetes: bootstraps etcd, API server (2 min)
# Helm: deploys Phase 1 components (3 min)
# Total: 10 minutes

# 5. Verify cluster is ready
cave-ctl kubeconfig export > ~/.kube/config-alice-dev
kubectl get nodes
# Shows 3 nodes (1 control, 2 workers) running Talos Linux

# 6. Access platform services
kubectl port-forward -n cavity svc/keycloak 8080:80
# Keycloak available at http://localhost:8080
```

**Outcomes:**
- Cluster provisioned deterministically; no manual steps
- Engineer can iterate on OpenTofu code locally, test against live infrastructure
- Cluster can be destroyed with `cave-ctl destroy dev hetzner` (5 minutes), recreated within 10 minutes

---

### Scenario 2: Promoting Code from Hetzner Staging to Azure Prod

**Context:** Platform team has validated a new Prometheus alert configuration in staging. Ready to promote to prod with confidence that the same infrastructure change applies to Azure.

**Procedure:**

```bash
# 1. Review changes
git log --oneline staging..main
# Shows 3 commits: Prometheus alert rules, Prometheus storage scaling, Loki config

# 2. Verify staging behavior matches expected
cave-ctl validate staging hetzner --terraform-plan-only
# Terraform plan shows: +3 disk volumes, +0 compute (HA alert replicas)

# 3. Promotion gate: test against prod profile
cave-ctl validate prod azure --terraform-plan-only
# Terraform plan output matches staging (except Azure service SKUs differ)
# Prometheus alert rules identical across both plans

# 4. Deploy to prod
git merge staging main
cave-ctl deploy prod azure --wait-for-reconciliation

# 5. Verify deployment
kubectl get pods -n cavity | grep prometheus
# Shows 3 Prometheus replicas running on prod AKS cluster
```

**Outcomes:**
- Same Prometheus configuration deployed to both Hetzner staging and Azure prod
- Terraform plan validated changes before actual deployment
- Rollback available: `git revert <commit>`, redeploy

---

### Scenario 3: Local Developer Starts Working on New Feature

**Context:** Developer needs to implement new Kong authentication plugin. Wants rapid iteration loop locally before testing against staging.

**Procedure:**

```bash
# 1. Start local cluster
cave-ctl local up --persistent

# 2. Clone workload repository
git clone https://github.com/caveplatform/kong-plugins.git
cd kong-plugins

# 3. Implement feature
# Edit: kong-auth-plugin/main.go
# Add test: kong-auth-plugin/main_test.go

# 4. Test locally
make test
# Runs Go tests, linters, security scanning

# 5. Build and deploy to local cluster
make docker-build IMG=localhost:5000/kong-plugin:dev
make deploy IMG=localhost:5000/kong-plugin:dev

# 6. Validate behavior via Kong API
curl -X POST https://localhost:8443/admin/plugins \
  -H "Content-Type: application/json" \
  -d '{"name": "auth-custom", ...}'

# 7. Push to staging when confident
git push origin feature/auth-plugin
# CI/CD triggers against staging cluster, tests against Hetzner
```

**Outcomes:**
- Local iteration loop: code → test → deploy → validate (5 minutes)
- Feature validated on real Kubernetes API before staging
- Staging tests catch multi-node failure modes, network policies, resource limits

---

### Scenario 4: Annual Portability Drill — Hetzner to Azure Migration

**Context:** Security compliance requires quarterly proof that CAVE can migrate a tenant's workload between providers. Platform team schedules annual drill.

**Procedure:**

```bash
# 1. Create snapshot of Hetzner prod cluster state
cave-ctl snapshot hetzner-prod --name "pre-migration-state"
# Exports: database dump, object store listing, secrets manifest

# 2. Validate Azure prod cluster ready
cave-ctl validate azure-prod
# Checks: all services healthy, storage capacity available

# 3. Create new Azure environment matching Hetzner topology
cave-ctl create prod azure --copy-state-from snapshot:pre-migration-state
# Restores: databases from dump, object storage from S3 export, secrets

# 4. Validate consistency
cave-ctl validate consistency hetzner-prod azure-prod
# Compares: database row counts, object store file listings, tenant configurations

# 5. If validation passes: switch DNS records to Azure
# (In real scenario, this involves external DNS changes)

# 6. Cleanup Hetzner cluster
cave-ctl destroy hetzner-prod
```

**Outcomes:**
- Annual proof that workloads can migrate between providers
- Data consistency validated by automated tooling, not manual inspection
- Runbook validated and tested

---

### Scenario 5: Scaling Prod Cluster After Large Tenant Onboarding

**Context:** New enterprise customer onboarded; workload projects 10x traffic spike within 2 weeks. Platform team proactively scales prod cluster.

**Procedure:**

```bash
# 1. Forecast capacity needs
cave-ctl forecast --tenant enterprise-customer-xyz \
  --growth-rate 2x-per-week --horizon 8-weeks
# Output: Recommends scaling worker nodes to 25 (from 5)

# 2. Update profile configuration
# Edit: profiles/prod.hetzner.tfvars
# Change: worker_count = 5 → worker_count = 25

# 3. Stage change
git add profiles/prod.hetzner.tfvars
git commit -m "chore(infra): scale hetzner-prod to 25 workers for enterprise-customer-xyz"

# 4. Validate change in CI
# Terraform plan shows: +20 CX52 VMs, +storage volumes, networking unchanged

# 5. Apply change (during maintenance window)
cave-ctl deploy prod hetzner --apply-changes

# 6. Monitor rollout
kubectl get nodes -w
# Shows 20 new nodes joining cluster, Kubernetes distributing workloads

# 7. Verify stability
cave-ctl monitor --duration 1h
# Checks: CPU utilization <60%, memory <70%, no errors in logs
```

**Outcomes:**
- Scaling plan validated before execution
- Gradual node join prevents API server overload
- Workloads automatically distributed by Kubernetes scheduler
- Rollback: `git revert commit`, redeploy (takes 15 minutes)

---

## 2.7 Configuration Reference

CAVE deployment profiles are defined via Terraform variables organized in a standardized directory structure. Each profile is a collection of values that configure OpenTofu modules.

### Directory Structure

```
cave-ctl/
  infra/
    hetzner/
      main.tf                    # Provider config, module instantiation
      versions.tf                # Terraform version constraints
      modules/
        cluster/
          main.tf                # Cluster topology
          variables.tf
          outputs.tf
        networking/
          main.tf
        managed-services/
          main.tf                # Database, Redis, object storage
    azure/
      main.tf
      modules/
        aks/                      # AKS-specific cluster provisioning
        managed-services/
          main.tf                # AKS companion services
    profiles/
      dev.hetzner.tfvars
      staging.hetzner.tfvars
      prod.hetzner.tfvars
      dev.azure.tfvars
      staging.azure.tfvars
      prod.azure.tfvars
      local.docker.tfvars        # Values for local vcluster provisioning
```

### Profile File Structure & Annotations

#### Example: `profiles/dev.hetzner.tfvars`

```hcl
# ============================================================================
# CAVE Deployment Profile: Hetzner Dev
# Minimal, single-AZ cluster for rapid iteration
# ============================================================================

# CLUSTER TOPOLOGY
# Why: Single control plane reduces cost; 2 workers enable workload distribution
# Consequence: Single CP is SPOF; cluster unavailable if CP node fails
cluster_name    = "cave-dev"
worker_count    = 2
control_plane_count = 1
control_plane_machine_type = "cx32"    # 4vCPU, 8GB RAM, EUR 7/month
worker_machine_type = "cx42"           # 6vCPU, 16GB RAM, EUR 12/month

# AVAILABILITY
# Why: Single AZ for dev; multi-AZ not required for rapid iteration
# Consequence: Cannot test multi-AZ failover on dev cluster
region          = "eu-central-1"       # Frankfurt (lowest latency for Germany)
availability_zones = ["eu-central-1a"] # Only one zone

# OPERATING SYSTEM
# Why: Talos Linux enforces immutability; prevents configuration drift
# Consequence: No SSH access; all changes via talosctl or Kubernetes API
talos_linux_version = "v1.11"

# NETWORKING
# Why: Cilium CNI + Cilium Network Policies provide fine-grained isolation
# Consequence: Requires learning eBPF concepts for advanced network policies
enable_cilium       = true
cilium_version      = "1.17"
enable_network_policy_enforcement = true  # Drop traffic not explicitly allowed

# LOAD BALANCING
# Why: Kong as single replica (single point of failure OK for dev)
# Consequence: External traffic requires manual port-forward in dev
enable_kong         = true
kong_replicas       = 1
kong_enable_waf     = false             # WAF disabled for dev (performance)

# STORAGE
# Why: Local volumes on worker nodes (no cost, fast for iteration)
# Consequence: Data loss if node fails; not suitable for persistent state
storage_backend = "local"               # Use node disk as backing store
storage_class_name = "local-storage"

# MANAGED SERVICES
# Why: Minimal SKUs reduce cost; suitable for testing, not production
managed_postgresql_enabled = false      # Use self-hosted Helm chart instead
managed_redis_enabled      = false

# SECURITY & COMPLIANCE
# Why: Dev profile omits compliance overhead; compliance enforced in staging/prod
enable_audit_logging = false
enable_encryption_at_rest = false
enable_pod_security_policy = false

# OBSERVABILITY
# Why: Prometheus + Grafana sufficient for dev; no enterprise APM needed
enable_prometheus   = true
prometheus_replicas = 1
prometheus_retention_days = 7            # Minimal retention for cost
enable_grafana      = true
grafana_replicas    = 1

# COST OPTIMIZATION
# Why: Dev clusters can be destroyed and recreated for cost savings
# Delete after development session ends
enable_scheduling_down_time = false
cluster_autoscaling_enabled = false

# Tags for cost tracking
tags = {
  environment = "development"
  managed-by  = "cave-ctl"
  profile     = "hetzner-dev"
  cost-center = "engineering"
}
```

#### Example: `profiles/prod.hetzner.tfvars`

```hcl
# ============================================================================
# CAVE Deployment Profile: Hetzner Production
# High-availability, 3-AZ cluster for enterprise workloads
# ============================================================================

cluster_name    = "cave-prod"
worker_count    = 5                     # Scales to 30 via autoscaling
control_plane_count = 3                 # Odd number for etcd quorum
control_plane_machine_type = "cx32"     # 4vCPU, 8GB RAM each

# Different worker types for different workload classes
worker_machine_types = {
  general = {
    count         = 3
    machine_type  = "cx52"              # 6vCPU, 16GB RAM, EUR 15/month
    labels        = { workload = "general" }
  }
  compute_optimized = {
    count         = 1
    machine_type  = "cpx42"             # 8vCPU, 32GB RAM, EUR 25/month
    labels        = { workload = "compute" }
  }
  memory_optimized = {
    count         = 1
    machine_type  = "cx62"              # 8vCPU, 32GB RAM, EUR 20/month
    labels        = { workload = "memory" }
  }
}

# AVAILABILITY (Critical for prod)
# Why: 3 AZs provide fault tolerance against single-zone failure
# Consequence: Inter-AZ networking charges ~EUR 0.01/GB (included in estimate)
availability_zones = ["eu-central-1a", "eu-central-1b", "eu-central-1c"]

# OPERATING SYSTEM
talos_linux_version = "v1.11"
enable_auto_update = true               # Automatic security patches

# NETWORKING
enable_cilium       = true
cilium_version      = "1.17"
enable_network_policy_enforcement = true
enable_pod_security_policy = true       # Enforce container security

# LOAD BALANCING
enable_kong         = true
kong_replicas       = 3                 # HA across AZs
kong_enable_waf     = true              # Web Application Firewall

# STORAGE
# Why: Hetzner Managed Volumes for high durability; replicated across zones
storage_backend = "managed-volumes"
storage_class_name = "managed-replicated"
storage_snapshot_enabled = true         # Daily snapshots
storage_snapshot_retention_days = 30

# MANAGED SERVICES
# Why: Production-grade services with failover, backups, monitoring
managed_postgresql_enabled = true
managed_postgresql_sku = "Standard"     # Multi-AZ replication
managed_postgresql_backup_retention_days = 30

managed_redis_enabled = true
managed_redis_sku = "Standard"          # Replication across AZs

# SECURITY & COMPLIANCE
enable_audit_logging = true
enable_encryption_at_rest = true        # Secrets encrypted
enable_pod_security_policy = true
enable_network_segmentation = true      # Tenant isolation

# OBSERVABILITY (Enterprise-grade)
enable_prometheus   = true
prometheus_replicas = 3                 # HA
prometheus_retention_days = 90          # Long-term trend analysis
enable_grafana      = true
grafana_replicas    = 3
enable_prometheus_operator = true       # Advanced alerting

# AUTOSCALING
# Why: Prod cluster automatically scales to match demand
cluster_autoscaling_enabled = true
min_worker_nodes = 5
max_worker_nodes = 30                   # Scale up to 30 under load

# BACKUP & DISASTER RECOVERY
enable_cluster_backups = true
backup_schedule = "0 2 * * *"           # Daily at 2am UTC
backup_retention_days = 90
backup_destination = "s3"               # Hetzner S3 bucket

tags = {
  environment = "production"
  managed-by  = "cave-ctl"
  profile     = "hetzner-prod"
  cost-center = "infrastructure"
  compliance-level = "SOC2"
}
```

#### Example: `profiles/prod.azure.tfvars`

```hcl
# ============================================================================
# CAVE Deployment Profile: Azure Production
# Zone-redundant AKS cluster with managed services
# ============================================================================

cluster_name            = "cave-prod-aks"
kubernetes_version      = "1.30"        # Latest stable from Azure

# Node pools provide compute class separation
node_pools = {
  system = {
    name           = "systempool"
    vm_size        = "Standard_D4s_v5"  # 4 vCPU, 16GB RAM
    min_count      = 3                  # Always 3 for system services
    max_count      = 5
    zones          = ["1", "2", "3"]    # Zone redundancy
  }
  workload = {
    name           = "workloadpool"
    vm_size        = "Standard_D8s_v5"  # 8 vCPU, 32GB RAM
    min_count      = 3
    max_count      = 20
    zones          = ["1", "2", "3"]    # Spread across zones
  }
  memory = {
    name           = "memorypool"
    vm_size        = "Standard_E8s_v5"  # 8 vCPU, 64GB RAM
    min_count      = 1
    max_count      = 5
    zones          = ["1", "2", "3"]
  }
}

# NETWORKING
# Why: Azure Virtual Network provides layer 2 isolation
vnet_address_space = ["10.0.0.0/16"]
enable_azure_firewall = true
firewall_sku = "Standard"               # Layer 4 filtering

# MANAGED SERVICES
# Why: Azure-managed services reduce operational overhead
managed_postgresql_enabled = true
postgresql_sku = "Standard_D4s"         # High memory for analytics
postgresql_geo_redundant_backup_enabled = true
postgresql_backup_retention_days = 35

managed_redis_enabled = true
redis_sku = "Premium"
redis_shard_count = 3                   # Multi-partition for scale

managed_storage_enabled = true
storage_redundancy = "GZRS"             # Geo-zone-redundant storage
storage_tier = "Premium"

# APPLICATION GATEWAY (Layer 7 load balancing)
enable_application_gateway = true
gateway_sku = "WAF_v2"
gateway_capacity = 3                    # Minimum HA
enable_waf_rules = true

# SECURITY
enable_pod_security_policy = true
enable_network_policy = true
enable_microsoft_defender = true        # Kubernetes threat detection
enable_disk_encryption = true           # AES-256 CMK encryption

# COMPLIANCE
enable_audit_logging = true
enable_compliance_dashboard = true
compliance_standards = ["SOC2", "ISO27001", "HIPAA"]

# BACKUP
enable_managed_backup = true
backup_vault_redundancy = "GeoRedundant"

tags = {
  environment = "production"
  managed-by  = "cave-ctl"
  profile     = "azure-prod"
  cost-center = "infrastructure"
}
```

---

## 2.8 Operations

Day-2 operations on CAVE clusters involve profile-driven procedures: changing cluster scale, upgrading Kubernetes, modifying networking topology, or migrating workloads between profiles.

### Profile Switching (Hetzner to Azure)

Profile switching moves a workload cluster from one provider to another. This is distinct from provider vendor lock-in; it's a deliberate operational decision driven by cost, compliance, or capacity.

**Procedure Overview:**

1. Create target profile in parallel
2. Migrate state (databases, persistent data)
3. Validate consistency
4. Switch application traffic
5. Decommission source profile

**Estimated Duration:** 4-6 hours for small tenants, 24 hours for large multi-TB databases.

**Example: Migrating hetzner-staging to azure-staging**

```bash
# 1. Create Azure staging cluster in parallel
cave-ctl create staging azure

# 2. Wait for cluster stability (15 minutes)
kubectl --context=azure-staging get nodes
# All nodes ready

# 3. Backup Hetzner staging state
cave-ctl snapshot hetzner-staging --output-path /tmp/hetzner-staging-snapshot

# 4. Restore to Azure cluster
cave-ctl restore azure-staging --snapshot /tmp/hetzner-staging-snapshot

# 5. Validate data consistency
cave-ctl validate consistency hetzner-staging azure-staging
# Compare: row counts, object store listings, config maps

# 6. Switch DNS (if applicable)
# Update cave-dns records to point to Azure staging cluster

# 7. Monitor for 1 hour
cave-ctl monitor --duration 1h

# 8. Decommission Hetzner cluster
cave-ctl destroy hetzner-staging
```

### Scaling Worker Nodes

Horizontal scaling (adding/removing compute nodes) is profile-specific and idempotent.

```bash
# Query current cluster size
cave-ctl info prod hetzner --format table
# Output: 5 worker nodes, 60% CPU utilization, 45% memory

# Forecast: 2-week projection shows need for 10 nodes
cave-ctl forecast prod hetzner --horizon 2-weeks
# Recommendation: scale to 10 nodes

# Update profile
sed -i 's/worker_count = 5/worker_count = 10/' profiles/prod.hetzner.tfvars

# Apply
cave-ctl deploy prod hetzner --approve
# Takes 15 minutes; nodes join cluster gradually

# Verify
kubectl get nodes
# Shows 15 total (3 control + 12 workers, 2 joining in progress)
```

### Upgrading Kubernetes Version

Kubernetes version upgrades are controlled via profile variables and executed through Kubernetes-native procedures (cordoning, draining, upgrading).

**Hetzner (Talos) Upgrade:**

```bash
# 1. Update profile
sed -i 's/talos_linux_version = "v1.11"/talos_linux_version = "v1.12"/' \
  profiles/prod.hetzner.tfvars

# 2. Validate plan
cave-ctl plan prod hetzner
# Output: Will upgrade 8 nodes sequentially

# 3. Apply upgrade (iterative node replacement)
cave-ctl deploy prod hetzner --approve
# Each node is replaced: drained, replaced with new image, rejoin

# 4. Verify upgrade
talosctl version
# Shows v1.12 on all nodes

# 5. Rollback (if issues detected)
# Git revert, redeploy
git revert HEAD
cave-ctl deploy prod hetzner --approve
```

**Azure (AKS) Upgrade:**

```bash
# 1. Update profile
sed -i 's/kubernetes_version = "1.29"/kubernetes_version = "1.30"/' \
  profiles/prod.azure.tfvars

# 2. Apply
cave-ctl deploy prod azure --approve
# AKS automatically drains and upgrades nodes across availability zones
```

### Disaster Recovery: Restoring from Backup

CAVE maintains automatic backups. Restoring from backup recovers cluster to a point-in-time state.

```bash
# 1. List available backups
cave-ctl backup list prod hetzner
# Latest backup: 2 hours ago

# 2. Restore to new cluster
cave-ctl restore prod hetzner \
  --backup-id cave-prod-2026-03-08-0200 \
  --cluster-name cave-prod-recovered

# 3. Verify restored data
kubectl --context=cave-prod-recovered get namespaces
# All namespaces restored

# 4. Route traffic to recovered cluster
# (Manual DNS/traffic switch)
```

---

## 2.9 Troubleshooting

### Issue 1: Cluster Bootstrap Timeout (OpenTofu)

**Symptom:** `cave-ctl create dev hetzner` hangs at "Waiting for Kubernetes API ready."

**Root Cause:** Hetzner API rate-limiting, or insufficient IP pool availability.

**Diagnosis:**

```bash
# Check OpenTofu logs
cave-ctl create dev hetzner --verbose
# Look for "API rate limit exceeded" or "IP pool exhausted"

# Verify Hetzner account quota
hcloud quotas list
```

**Resolution:**
- Wait 60 seconds, retry: `cave-ctl create dev hetzner --retry`
- Contact Hetzner support if quota exhausted
- Use different region: `--region eu-west-1` (switch from Frankfurt)

---

### Issue 2: Cilium NetworkPolicy Blocking Pod Communication

**Symptom:** Pods in different namespaces cannot communicate; `curl` between services times out.

**Root Cause:** Cilium network policies are strict by default; whitelist-based (deny-all unless explicitly allowed).

**Diagnosis:**

```bash
# Check Cilium logs
kubectl logs -n cavity -l k8s-app=cilium --tail=100

# Verify policy in namespace
kubectl get networkpolicies -n workload
# Lists all NetworkPolicy objects

# Inspect policy rules
kubectl describe networkpolicy allow-frontend -n workload
```

**Resolution:**
- Create NetworkPolicy allowing traffic:

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: allow-frontend-to-backend
spec:
  podSelector:
    matchLabels:
      app: backend
  policyTypes:
  - Ingress
  ingress:
  - from:
    - podSelector:
        matchLabels:
          app: frontend
```

- Apply: `kubectl apply -f policy.yaml`

---

### Issue 3: Persistent Volume Claim Stuck "Pending"

**Symptom:** Pod cannot start; PVC remains "Pending" despite available storage.

**Root Cause:** Storage class mismatch, or StorageBackend configuration error.

**Diagnosis:**

```bash
# Describe PVC
kubectl describe pvc my-data-claim -n workload
# Check "Events" section for error message

# List storage classes
kubectl get storageclass
# Verify expected storage class exists

# Check PV availability
kubectl get pv
```

**Resolution (Hetzner local storage):**
- Ensure pod is scheduled on node with available local volume:

```yaml
affinity:
  nodeAffinity:
    requiredDuringSchedulingIgnoredDuringExecution:
      nodeSelectorTerms:
      - matchExpressions:
        - key: kubernetes.io/hostname
          operator: In
          values: ["worker-1"]  # Pin to specific node with storage
```

**Resolution (Azure managed volumes):**
- Verify node pool has sufficient capacity:

```bash
kubectl describe nodes | grep -A 5 "Allocated resources"
```

---

### Issue 4: Kong Ingress Not Receiving Traffic

**Symptom:** Ingress object created, but external requests fail ("connection refused").

**Root Cause (dev profile):** Kong is single replica; load balancer not exposed. Manual port-forward required.

**Diagnosis:**

```bash
# Verify Kong pod running
kubectl get pods -n cavity | grep kong

# Check Ingress status
kubectl describe ingress my-app -n workload
# Check "Address" field; should show LB IP (prod) or pending (dev)
```

**Resolution (dev profile):**
```bash
# Port-forward Kong service
kubectl port-forward -n cavity svc/kong 8443:443 &
# Access via https://localhost:8443
```

**Resolution (staging/prod):**
```bash
# Verify Hetzner Load Balancer created
hcloud loadbalancer list
# Should show "kong-ingress" LB

# Get LB public IP
hcloud loadbalancer describe kong-ingress
# Use this IP in DNS records
```

---

### Issue 5: Out-of-Memory Errors on Worker Nodes

**Symptom:** Pods evicted with "MemoryPressure" node condition.

**Root Cause:** Tenant workloads exceed available memory; cluster lacks memory-optimized nodes.

**Diagnosis:**

```bash
# Check node memory
kubectl top nodes
# Shows per-node memory utilization

# Check pod memory usage
kubectl top pods --all-namespaces | sort -k 3 -n
# Identify memory hogs
```

**Resolution:**
- Scale cluster (add memory-optimized nodes):

```bash
# Hetzner
sed -i 's/worker_count = 5/worker_count = 7/' profiles/prod.hetzner.tfvars
# Azure
sed -i 's/min_count      = 3/min_count      = 5/' profiles/prod.azure.tfvars
```

- Or: request tenants optimize workload memory footprint

---

### Issue 6: Database Connection Pool Exhaustion

**Symptom:** Applications fail with "too many connections" error.

**Root Cause:** Managed database connection limit reached; PostgreSQL max_connections exceeded.

**Diagnosis:**

```bash
# Query active connections
kubectl exec -it postgresql-0 -n cavity -- \
  psql -U postgres -c "SELECT datname, count(*) FROM pg_stat_activity GROUP BY datname;"

# Check max_connections setting
kubectl exec -it postgresql-0 -n cavity -- \
  psql -U postgres -c "SHOW max_connections;"
```

**Resolution (Hetzner self-hosted PostgreSQL):**
```bash
# Increase max_connections
kubectl patch statefulset postgresql -n cavity --type=json \
  -p '[{"op":"replace","path":"/spec/template/spec/containers/0/env/0/value","value":"200"}]'
```

**Resolution (Azure managed PostgreSQL):**
```bash
# Scale up SKU via Azure portal or terraform
sed -i 's/Standard_B2s/Standard_D2s_v3/' profiles/prod.azure.tfvars
```

---

### Issue 7: Talos Node Fails to Boot

**Symptom:** Node appears in Hetzner console as "Started," but Kubernetes shows "NotReady."

**Root Cause:** Talos machine config not applied; node booted without cluster credentials.

**Diagnosis:**

```bash
# Check Talos API connectivity
talosctl -n <node-ip> health
# Should show "OK" for all services

# Check logs
talosctl -n <node-ip> logs
# Look for Kubernetes API connection errors
```

**Resolution:**
- Reboot node via Hetzner API; cluster bootstrap will retry

```bash
hcloud server reboot <server-id>
```

---

### Issue 8: Istio Ambient Mesh Certificate Rotation Failure

**Symptom:** mTLS connections fail with "certificate expired" after 30+ days.

**Root Cause:** Certificate rotation cron job failed; no new certs issued.

**Diagnosis:**

```bash
# Check cert age
kubectl get cert -n istio-system
# Look for "NotReady" status

# Check cert-manager logs
kubectl logs -n cert-manager deploy/cert-manager | grep -i error
```

**Resolution:**
```bash
# Trigger manual cert rotation
kubectl patch cert -n istio-system <cert-name> --type=json \
  -p '[{"op":"replace","path":"/spec/renewBefore","value":"168h"}]'

# Verify new certs issued
kubectl get cert -n istio-system -w
```

---

### Issue 9: Azure AKS Pod Failed to Pull Image

**Symptom:** Pod stuck in "ImagePullBackOff"; container image not found.

**Root Cause:** Azure Container Registry (ACR) credentials not available; pod cannot authenticate.

**Diagnosis:**

```bash
# Check pull secrets
kubectl get secrets -n workload | grep azure-registry

# Verify ACR login credentials
kubectl get secret azure-registry -n workload -o yaml | base64 -d
```

**Resolution:**
```bash
# Create pull secret
kubectl create secret docker-registry azure-registry \
  --docker-server=myregistry.azurecr.io \
  --docker-username=<username> \
  --docker-password=<password> \
  -n workload

# Reference in pod spec
containers:
- name: app
  image: myregistry.azurecr.io/app:latest
  imagePullPolicy: Always
imagePullSecrets:
- name: azure-registry
```

---

### Issue 10: Prometheus Scrape Targets Down

**Symptom:** Prometheus shows "UP: 0/100 targets," no metrics collected.

**Root Cause:** Service discovery misconfigured; Prometheus cannot reach scrape targets.

**Diagnosis:**

```bash
# Check ServiceMonitor objects
kubectl get servicemonitor -n cavity

# Verify scrape targets in Prometheus UI
# Navigate to http://prometheus:9090/targets
# Check "Endpoint" column for DNS resolution failures
```

**Resolution:**
```bash
# Verify service exists and is accessible
kubectl get svc -n cavity | grep prometheus

# Re-apply Prometheus Operator configuration
kubectl rollout restart deploy/prometheus-operator -n cavity
```

---

### Issue 11: Local vcluster Docker Volume Exhaustion

**Symptom:** `cave-ctl local up` fails with "docker: no space left on device."

**Root Cause:** Docker Desktop allocated disk space exhausted by vcluster images.

**Diagnosis:**

```bash
# Check Docker disk usage
docker system df
# Shows per-component disk usage

# List vcluster volumes
docker volume ls | grep vcluster
```

**Resolution:**
```bash
# Clean up unused volumes
docker volume prune

# Increase Docker Desktop disk allocation
# Settings → Resources → Disk image size (increase to 50GB)

# Restart Docker Desktop
killall Docker
open /Applications/Docker.app
```

---

### Issue 12: Terraform State Corruption

**Symptom:** `cave-ctl` operations fail with "invalid state file" error.

**Root Cause:** Concurrent modifications to state file; multiple operators applying changes simultaneously.

**Diagnosis:**

```bash
# Check state file validity
terraform validate -var-file=profiles/dev.hetzner.tfvars
# Reports state format errors

# List state backups
ls -la terraform.tfstate*
```

**Resolution:**
```bash
# Restore from latest backup
cp terraform.tfstate.backup.1 terraform.tfstate

# Re-apply changes
cave-ctl deploy <profile> <provider>

# Implement locking to prevent concurrent modifications
# (Ensure only one operator runs cave-ctl at a time)
```

---

## 2.10 Compliance Mapping

CAVE's deployment profiles are designed with regulatory requirements in mind. This section maps profile capabilities to compliance frameworks.

### SOC 2 Type II (Security, Availability, Integrity)

**Requirement:** Encryption in transit (TLS 1.2+), encryption at rest, access controls, audit logging.

**Profile Mapping:**
- **hetzner-dev:** TLS for Kong ingress, no encryption at rest (dev acceptable)
- **hetzner-staging:** TLS + audit logging, encryption optional
- **hetzner-prod:** TLS 1.3 required, encryption at rest (AES-256), comprehensive audit logging
- **azure-prod:** TLS 1.3, CMK encryption, Azure Policy compliance checks

**Validation:** Annual SOC 2 audit of prod profiles confirms compliance.

---

### ISO 27001 (Information Security Management)

**Requirement:** Access controls, incident response, backup/recovery, vulnerability management.

**Profile Mapping:**
- **RBAC:** All profiles enforce Kubernetes RBAC; role-based access to cluster operations
- **Secrets Management:** Vault available on all profiles; encryption enforced on prod
- **Backup:** hetzner-prod and azure-prod maintain daily backups, 90-day retention
- **Vulnerability Scanning:** Container images scanned on staging/prod before deployment
- **Incident Response:** Comprehensive audit logs on prod profiles enable forensic analysis

---

### NIS2 Directive (Network & Information Security)

**Requirement:** (EU) Critical infrastructure protection, incident notification, supply chain security.

**Profile Mapping:**
- **Hetzner prod:** Data residency in Frankfurt (Germany); GDPR-aligned
- **Azure prod:** Multiple EU regions available; Azure compliance certifications
- **Multi-provider:** Portability eliminates vendor lock-in risk (supply chain resilience)

---

### GDPR (General Data Protection Regulation)

**Requirement:** (EU) Data residency, right to deletion, data subject access requests.

**Profile Mapping:**
- **Data Residency:** Hetzner prod clusters in Frankfurt (DE); Azure clusters in WestEurope region
- **Right to Deletion:** Kube-delete-tls tool enables pod-level data purge; backup deletion honored (90-day retention)
- **Data Subject Access:** Audit logs provide complete access trail; requests fulfilled within 30 days
- **Encryption:** GDPR-compliant encryption (AES-256) on all production profiles

---

## 2.11 Related ADRs

- **ADR-066:** Multi-Provider Exit Strategy & Portability — Foundation for seven-profile model
- **ADR-067:** Two-Layer Provisioning (OpenTofu + Crossplane) — Day 0/Day 1 division
- **ADR-094:** Profile Structure & Environment Definition — Profile design rationale
- **ADR-098:** Immutable Infrastructure via Talos Linux — Hetzner node management

---

## 2.12 Related Runbook Sections

- **§0.1 Introduction to CAVE:** High-level platform concepts
- **§4 cave-ctl Command Reference:** Detailed command syntax for profile operations
- **§29 Disaster Recovery & Backup Procedures:** Profile-specific backup/restore procedures
- **§35 Kubernetes Cluster Upgrades:** Version upgrade procedures per profile
- **§36 Multi-Cloud Operations:** Cross-provider workload migration

---

## Deployment Profile Cost Estimates

Cost estimates are based on March 2026 pricing (Hetzner EU, Azure EU-West regions). Actual costs may vary based on commitment discounts, reserved instances, and consumption patterns.

| Profile | Base Compute | Managed Services | Storage | Networking | Total EUR/mo |
|---------|--------------|------------------|---------|------------|--------------|
| **hetzner-dev** | 40 (1 CP + 2 workers) | 0 (self-hosted) | 5 | 5 | 50-80 |
| **hetzner-staging** | 90 (3 CP + 3 workers) | 40 (PostgreSQL, Redis) | 30 | 20 | 120-180 |
| **hetzner-prod** | 200 (3 CP + 5+ workers) | 100 (managed services) | 80 (snapshots) | 50 | 350-500 |
| **azure-dev** | 150 | 100 (Basic AKS, managed services) | 30 | 20 | 400-800 |
| **azure-staging** | 250 | 200 (Standard tier) | 50 | 30 | 800-1,500 |
| **azure-prod** | 500+ | 400+ (Premium tier) | 150 | 100 | 2,000-5,000 |
| **local** | 0 (runs on laptop) | 0 (containerized) | 2 (Docker images) | 0 | Free |

**Cost Optimization Strategies:**
- Co-host dev and staging on single Hetzner region to amortize inter-AZ transfer
- Use spot instances (Azure Spot VMs) for non-critical workloads (saves 50-70%)
- Delete dev/staging clusters during off-hours (nights, weekends) to reduce EUR 150-250/month
- Annual reserved capacity discounts available from both providers (15-25% savings)

---

**Document Version:** 2.0
**Last Updated:** March 2026
**Next Review:** June 2026
**Owner:** Platform Engineering
**Status:** Approved for Production Use
