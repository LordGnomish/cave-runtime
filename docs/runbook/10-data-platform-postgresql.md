# 10 — Data Platform: PostgreSQL

## 10.1 Overview

PostgreSQL is the primary relational database for the CAVE platform. It provides the foundational data layer for tenant applications requiring structured, transactional data storage with strict consistency guarantees.

### Why a Relational Database?

The CAVE platform uses PostgreSQL because certain workloads inherently demand relational semantics:

**ACID Transactions.** Many business applications (billing, orders, accounts) cannot tolerate lost or partially-applied state changes. PostgreSQL's ACID guarantees—Atomicity, Consistency, Isolation, Durability—ensure that if a multi-step operation fails halfway through, all changes roll back atomically. This eliminates data corruption from partial failures.

**Structured Data with Schemas.** Relational schemas enforce data shape at the database layer, catching application bugs early. A schema-enforced foreign key constraint prevents orphaned records that would be silent failures in schemaless systems. This is particularly critical in multi-tenant environments where data corruption in one tenant's schema could affect billing calculations or compliance audits.

**Multi-Tenant Isolation.** PostgreSQL's row-level security (RLS) and schema-per-tenant patterns allow strong logical isolation within a single cluster. Combined with CAVE's namespace-based isolation at the Kubernetes level, this provides defense-in-depth: a bug in one tenant's application cannot read another tenant's rows, even if the application framework accidentally queries the shared schema.

**Cost-Effective Scaling.** For the workloads CAVE targets, relational databases scale cost-effectively to mid-scale deployments (hundreds of GB, millions of records per tenant). Advanced indexing, query optimization, and connection pooling allow serving thousands of concurrent users from a single well-tuned cluster. Object storage and search engines handle specialized use cases, but the relational database handles the critical path.

### Two Implementations: Unified Behind Crossplane

CAVE runs PostgreSQL in two distinct environments, each with different operational characteristics:

1. **Hetzner (Self-Hosted via CloudNativePG).** For deployments on Hetzner bare-metal or cloud servers, PostgreSQL runs as a Kubernetes-native cluster using the CloudNativePG operator. This approach gives CAVE complete control over version lifecycle, resource allocation, and backup strategy. CloudNativePG is a CNCF Sandbox project, meaning it has no single vendor dependency and benefits from community stewardship.

2. **Azure (Managed Service via Azure Database for PostgreSQL Flexible Server).** For deployments on Azure, PostgreSQL is provisioned as Microsoft's managed service. This eliminates operator maintenance and leverages Azure's native backup, replication, and monitoring integrations. It's the recommended PostgreSQL offering for Azure workloads.

Both implementations present the same PostgreSQL interface to applications, but differ in operational concerns (backup strategies, failover behavior, cost models). To shield applications from this difference, **Crossplane v2 provides a unified abstraction: the `Database` custom resource.** A developer creates one Database resource; a Composition Function examines the cluster's infrastructure profile and provisions either CloudNativePG (Hetzner) or Azure PostgreSQL Flexible Server (Azure), with identical parameters. This approach avoids duplication and keeps operational knowledge centralized.

### Key Architecture Decision

The selection of CloudNativePG and Azure PostgreSQL Flexible Server, along with their Crossplane abstraction, is documented in **ADR-047: Database Platform Selection and Multi-Region Deployment Strategy.**

### Scenarios from One Prompt

The One Prompt reference implementation demonstrates these common workflows:

- **Developer Creates a Database.** A developer in tenant namespace `tenant-acme-staging` creates a Database resource requesting a `large` instance with `standard` performance. The Composition Function detects the namespace is on a Hetzner profile and creates a CloudNativePG cluster with 4-core CPU, 8 GB RAM, and 3K IOPS. Credentials are automatically rotated every 30 days via External Secrets Operator (ESO) and OpenBao.

- **Automatic IOPS Resize.** Monitoring shows the database is consistently hitting IOPS throttling. An operator modifies the Database resource, changing performanceProfile from `standard` to `high`, which triggers a CloudNativePG PVCResize. The storage expands without downtime; Primary and replicas resize in sequence.

- **Credential Rotation.** Every 30 days, ESO detects a rotation window and requests new credentials from OpenBao, which generates a new password and updates the PostgreSQL role. Applications using the Kubernetes secret never see the rotation; they pick up the new password from the refreshed secret mount.

- **Weekly Maintenance Window.** CloudNativePG's maintenance controller notices that a minor version patch (e.g., 17.1 → 17.2) is available. It orchestrates a controlled upgrade: primary and replicas are patched in sequence, with each replica validated before proceeding. If any replica fails, the upgrade pauses and an alert fires.

---

## 10.2 ADR Rationale (ADR-047)

ADR-047 documents the decision to adopt CloudNativePG for self-hosted environments and Azure Database for PostgreSQL Flexible Server for managed environments, unified through Crossplane.

### 10.2.1 Context & Problem Statement

The CAVE platform must support PostgreSQL across two distinct infrastructure profiles without introducing vendor lock-in or operational silos. The chosen database solution must satisfy these requirements:

**Multi-Infrastructure Support.** PostgreSQL must run on both Hetzner bare-metal infrastructure (where CAVE manages Kubernetes clusters directly) and Azure (where CAVE leverages Azure Kubernetes Service). A single codebase and runbook must document both paths without diverging into separate operational procedures.

**High Availability with Automatic Failover.** Single-node databases are unacceptable. The database must tolerate the loss of one node without user-visible downtime or data loss. For self-hosted, this means leader election and replica failover. For Azure, this means managed zone-redundant HA.

**Backup and Point-in-Time Recovery (PITR).** Recovery objectives require the ability to restore to any point within 7 days. This demands continuous WAL archiving (not just periodic snapshots) and regular base backups. Backups must be portable: restorable to a different cluster or different infrastructure.

**Multi-Tenant Isolation.** Logical isolation between tenants must not rely solely on application-layer enforcement. PostgreSQL should support schema-per-tenant or role-based row-level security. Cross-tenant data leakage must be architecturally difficult, not just operationally discouraged.

**Kubernetes-Native Provisioning (Hetzner).** For self-hosted environments, the database must be provisioned declaratively via Kubernetes CRDs. A developer should not SSH into a server or run imperative scripts. This allows GitOps workflows and enables Crossplane to manage database lifecycle.

**Declarative Provisioning via Crossplane.** Both Hetzner and Azure deployments must be manageable through Crossplane v2, without branching the deployment logic. The platform operator should define a single Database XRD that composes to the correct implementation based on the deployment context.

**Data Classification and Regulatory Compliance.** CAVE must support data residency rules (GDPR: data must remain in EU) and data classification (confidential vs. public). The database solution must integrate with these policies, ideally enforcing them at the infrastructure level.

**Credential and Secret Management.** Database credentials must rotate automatically without application restarts. Integration with External Secrets Operator (ESO) and OpenBao enables this. Credentials must never appear in application configuration, logs, or backup metadata.

### 10.2.2 Decision

**For Hetzner Self-Hosted Environments:** CloudNativePG is the selected database operator. CloudNativePG is a CNCF Sandbox-level project, licensed under Apache 2.0. It provides:
- Kubernetes-native cluster management: databases are defined as CRDs (`Cluster` resource)
- Built-in PgBouncer connection pooling: no separate operator or external sidecar required
- Barman integration: continuous WAL archiving and scheduled base backups, with S3-compatible storage target (MinIO)
- Patroni-based HA: quorum-based leader election, automatic replica promotion
- Prometheus metrics: native observability without additional scraping tools
- Version 1.25+ (as of early 2026): stable feature set, monthly patches, regular minor releases

**For Azure Managed Environments:** Azure Database for PostgreSQL Flexible Server is the selected offering. Azure PostgreSQL Flexible Server is Microsoft's strategic PostgreSQL product (it replaced the deprecated Single Server tier):
- Zone-redundant HA: automatic failover within AZs, RPO <1 second
- Automated backup: 7–35 day configurable retention, geo-redundant copies available
- Built-in monitoring: Azure Monitor integration, query store, performance insights
- Private endpoints: network isolation, no public internet exposure
- Terraform provider: full coverage of SKUs, replicas, and server parameters
- Pricing model: pay-per-hour for compute, storage overages

**Crossplane v2 Abstraction:** Both implementations are hidden behind a single Database XRD (Custom Resource Definition). A Composition Function examines the target namespace's infrastructure profile (derived from a label or namespace selector) and routes the Database resource to either:
- CloudNativePG Cluster resource (Hetzner)
- Azure PostgreSQL Flexible Server managed resource (Azure)

This approach allows developers to use identical Database manifests across both environments. Operational differences (e.g., backup retention, failover behavior) are mapped through Composition Parameters, ensuring consistency.

### 10.2.3 Alternatives Evaluated

Six alternative database systems were seriously evaluated. Each was rejected for specific, documented reasons.

#### 1. Zalando Postgres Operator

**Profile.** Zalando's Postgres Operator is a mature, CNCF Sandbox-level project built by Zalando's engineering team. It uses Patroni for HA, SpaCLOG for backups, and Kubernetes CRDs for cluster definition. It has been in production at Zalando (massive scale: millions of transactions/second) for many years.

**Strengths:**
- Proven at extreme scale; battle-tested in a high-frequency trading environment
- Patroni HA mechanism is elegant: distributed consensus, health-based promotion
- Active community, good documentation
- Multi-cluster deployment patterns well-documented

**Rejection Rationale:**
- **Connection Pooling Gap.** Zalando Operator does NOT include PgBouncer or connection pooling. It assumes a separate pooling layer (managed by the application team or external tool). For CAVE, this creates an additional operational surface: another Kubernetes deployment to manage, another failure point, another scaling policy to tune. CloudNativePG bundles PgBouncer, eliminating this step.
- **Backup Story is Weaker.** Zalando uses SpaCLOG or WAL-E for WAL archiving, but the integration is less opinionated than CloudNativePG's Barman. Barman is a battle-tested backup management suite (used by many enterprises); CloudNativePG's integration is seamless. Zalando requires more manual integration work.
- **Development Momentum.** Zalando's operator is stable but not fast-moving. Critical updates slow down in 2023–2024; the project serves Zalando's immediate needs but doesn't aggressively pursue CNCF Incubation status or feature velocity. CloudNativePG, by contrast, is targeting CNCF Incubation and has a clearer roadmap.
- **No Declarative Backup-to-S3.** Zalando requires operators to define backup schedules outside the CRD. CloudNativePG allows backup scheduling and S3-compatible targets to be declared inline, reducing configuration silos.

**Why NOT Selected:** Zalando is a strong alternative for teams that want maximum HA sophistication, but CAVE prioritizes operational simplicity. The lack of built-in connection pooling and the weaker backup integration mean more toil. CloudNativePG's all-in-one approach—HA, pooling, backup—is better aligned with CAVE's philosophy of reducing operational surfaces.

#### 2. Crunchy Data PostgreSQL Operator (PGO)

**Profile.** Crunchy Data is the leading PostgreSQL services company. Their PGO (PostgreSQL Operator) is enterprise-grade, feature-rich, and includes pgMonitor (superior observability) and pgBackRest (sophisticated backup tooling).

**Strengths:**
- pgMonitor dashboards and alerts are best-in-class for PostgreSQL observability
- pgBackRest is highly configurable: incremental backups, parallel processing, multiple backup targets
- Commercial support available; trusted by large enterprises
- Actively maintained and feature-complete

**Rejection Rationale:**
- **Commercial License Model Creates Lock-In.** While PGO has an open-source version, advanced features (standby clusters, multi-region, advanced monitoring) require a commercial license. CAVE's zero-vendor-lock-in principle forbids adopting a database operator where critical features have a paywall. Even if CAVE commits to the open version today, competitive pressure or feature gaps may force a commercial license later—this is vendor lock-in risk.
- **Operational Footprint is Heavier.** PGO is more opinionated than CloudNativePG. It requires more CustomResource definitions (e.g., separate PGCluster, PGPolicy, PGTaskSchedule resources). CloudNativePG's single Cluster CRD is simpler; it reduces cognitive load and configuration drift.
- **Crunchy's Business Model Dependency.** Crunchy Data is the sole steward. If their business model changes, CAVE loses optionality. CNCF Sandbox projects (like CloudNativePG) are stewarded by community; no single company controls the roadmap. For a foundational system like the database, this community control is critical.

**Why NOT Selected:** Crunchy PGO is an excellent choice for enterprises that can afford commercial support and don't mind vendor dependency. CAVE cannot accept this dependency.

#### 3. Percona Operator for PostgreSQL

**Profile.** Percona offers operators for MySQL, MongoDB, and PostgreSQL. Their PG operator aims to provide a consistent operational experience across all three databases.

**Strengths:**
- Multi-database portfolio: a single operator pattern for MySQL, MongoDB, PG reduces learning curve for teams managing all three
- Mature MongoDB and MySQL operators (battle-tested)
- Good backup integration (Percona Backup for PostgreSQL)

**Rejection Rationale:**
- **PostgreSQL Support is Immature Relative to MySQL.** Percona's strength is MySQL and MongoDB. Their PostgreSQL operator is newer and less battle-tested than their MySQL offering. The community is smaller for PG. If an edge case bug emerges specific to PostgreSQL (e.g., HA failover under high load), Percona's response is slower than CloudNativePG's or Zalando's.
- **Backup Tooling is Less Integrated.** Percona Backup for PostgreSQL is separate from the operator. Like Zalando, this creates an integration boundary and more configuration surface.
- **Limited PostgreSQL-Specific Features.** CloudNativePG and Crunchy Data have deep PostgreSQL expertise baked into their products. Percona's PostgreSQL operator is a "lesser child" in their portfolio; it misses optimizations and PostgreSQL-specific innovations.

**Why NOT Selected:** CAVE has no need for a MySQL or MongoDB operator (those are separate decisions, per ADR-036). Adopting Percona's PostgreSQL operator for consistency with an absent MySQL operator adds no value and sacrifices depth of expertise.

#### 4. CockroachDB

**Profile.** CockroachDB is a distributed SQL database with PostgreSQL wire protocol compatibility. It provides horizontal scaling, multi-region high availability, and ACID transactions across nodes.

**Strengths:**
- Distributed architecture: data is sharded across many nodes, enabling horizontal scaling
- Multi-region HA: can tolerate entire region failures without data loss or manual failover
- ACID transactions: full ACID support across distributed data
- PostgreSQL wire-compatible: many applications can connect unmodified

**Rejection Rationale:**
- **Not 100% Wire-Compatible.** While CockroachDB speaks the PostgreSQL protocol, it is NOT a PostgreSQL dialect. Subtle incompatibilities exist: certain FOREIGN KEY patterns, some aggregate functions, and specific transaction isolation semantics differ. Application teams will face unexpected surprises porting code.
- **Resource Overhead is Significant.** Distributed consensus (Raft) and geographic replication require more CPU and memory per node compared to PostgreSQL. For CAVE's workload profile (many small databases serving mid-market tenants, not massive scale), this overhead is wasteful. CAVE is paying for horizontal scaling it doesn't use.
- **Operational Complexity is High.** CockroachDB's distributed nature means more failure modes: node failures, network partitions, clock skew. Troubleshooting is harder. CloudNativePG's 3-node failover is operationally simpler.
- **CAVE Already Solves Multi-Region at Infrastructure Level.** CAVE's Hetzner deployments span multiple data centers; Azure deployments use availability zones. The database does NOT need to solve multi-region HA; infrastructure provides it. CockroachDB's multi-region strength is a feature CAVE doesn't need, purchased at the cost of complexity.

**Why NOT Selected:** CockroachDB is ideal for applications that truly need distributed SQL (e.g., financial systems with global consistency requirements). CAVE's workloads are regionally scoped (GDPR, data residency). A simpler database (PostgreSQL) with regional HA is better fit.

#### 5. YugabyteDB

**Profile.** YugabyteDB is a distributed SQL database with Cassandra-like partitioning and PostgreSQL wire-compatibility. Like CockroachDB, it offers multi-region HA and horizontal scaling.

**Strengths:**
- PostgreSQL wire-compatible: applications can often connect unchanged
- Distributed consensus: strong multi-region HA
- Flexible replication: can be tuned for latency or consistency

**Rejection Rationale:**
- **Memory and CPU Footprint Per Node is High.** YugabyteDB's Java-based architecture (similar to Cassandra) demands more resources per node than C-based PostgreSQL. For CAVE's typical database size (50–500 GB per tenant), this overhead is disproportionate.
- **Distributed Consensus Adds Complexity.** Like CockroachDB, YugabyteDB requires more operational expertise. Network partitions, clock skew, and consensus failures are less intuitive than PostgreSQL's simpler leader-follower model.
- **CAVE's Multi-Region is Solved at Infrastructure.** YugabyteDB's strength—transparent multi-region replication—is not needed. CAVE handles region isolation explicitly (per ADR-113: data residency). A single-region highly-available database is the right tool.

**Why NOT Selected:** YugabyteDB is a good choice for teams building globally distributed applications on a single database. CAVE's architecture separates regions; YugabyteDB's distributed features are unused and create unnecessary operational burden.

#### 6. MySQL / MariaDB

**Profile.** MySQL is the most widely deployed database globally. MariaDB is a drop-in replacement with additional features.

**Strengths:**
- Massive ecosystem; known by every engineer
- Good HA support (Galera, Group Replication)
- Kubernetes operators available (Percona, Bitnami Helm charts)

**Rejection Rationale:**
- **Inferior JSON Support.** PostgreSQL's JSONB type is a first-class citizen with rich operators and full indexing. MySQL's JSON support is functional but less powerful. CAVE profiles show many modern workloads (configuration, telemetry, semi-structured data) benefit from strong JSON handling.
- **Smaller Extension Ecosystem.** PostgreSQL's extension ecosystem is unmatched: PostGIS (geographic data), pgvector (AI embeddings), pg_cron (scheduled jobs), pg_trgm (full-text search), hstore, uuid-ossp. MySQL has fewer native extensions and relies on external tools for these capabilities.
- **Data Type Richness.** PostgreSQL has RANGE, MULTIRANGE, INET, MACADDR, BYTEA, and other specialized types. MySQL's type system is smaller. For domains like networking or temporal data, PostgreSQL is more naturally expressive.
- **Standards Compliance.** PostgreSQL is the PostgreSQL standard (obviously). MySQL diverges in subtle ways: NULL handling in ORDER BY, subquery behavior, aggregate function semantics. Standards compliance matters when debugging weird edge cases.
- **CNCF and Cloud-Native Preference.** The broader cloud-native ecosystem defaults to PostgreSQL. Kubernetes projects, observability tools, and cloud platforms optimize for PostgreSQL first. CAVE is a cloud-native platform; MySQL would be swimming upstream.

**Why NOT Selected:** MySQL is a fine database for certain workloads (web applications, content management). CAVE's workload profile and the cloud-native ecosystem both favor PostgreSQL. Switching to MySQL would require re-architecting integrations and losing access to PostgreSQL-specific innovations.

#### 7. Managed-Only Approach (Skip Self-Hosted)

**Profile.** Use Azure Database for PostgreSQL Flexible Server everywhere, even for Hetzner deployments. Hetzner would provision Azure subscriptions on behalf of tenants, reducing operational burden.

**Strengths:**
- Single operational surface: all databases managed by Microsoft
- High availability and backup handled by Azure
- Simplified runbook: no CloudNativePG operator to manage

**Rejection Rationale:**
- **Violates CAVE's Zero-Vendor-Lock-In Principle.** CAVE's architecture (ADR-001) explicitly rejects single-vendor dependency. Relying on Azure for all databases, even for Hetzner deployments, concentrates risk and limits future infrastructure optionality.
- **Hetzner Customers Expect Self-Hosting.** Hetzner is selected because it offers customer control and data sovereignty. Using Azure from Hetzner contradicts this value proposition. Customers deploying on Hetzner expect their data to stay in Hetzner infrastructure.
- **Proves Portability.** CAVE must demonstrate that the platform is genuinely portable. If the database only works on Azure, CAVE's portability claims are hollow. Supporting self-hosted CloudNativePG proves that the platform is not Azure-dependent.
- **Cost Model Mismatch.** Managed services charge for idle time and have minimum fees. Self-hosted allows closer resource matching to actual usage, important for cost-sensitive deployments.

**Why NOT Selected:** Managed services are excellent for complexity reduction, but CAVE cannot sacrifice architectural principles for operational convenience. The self-hosted option must exist and must be operational.

---

## 10.3 Tool Comparison Matrix

The table below scores each option on critical dimensions. Scoring uses a 1–5 scale: 1 = poor fit, 5 = excellent fit. Scores are justified below the table.

| Dimension | CloudNativePG | Zalando Op. | Crunchy PGO | CockroachDB | YugabyteDB | Azure PG Flex. |
|---|:---:|:---:|:---:|:---:|:---:|:---:|
| **K8s-Native Operation** | 5 | 4 | 5 | 4 | 3 | N/A |
| **HA & Automatic Failover** | 5 | 4 | 5 | 5 | 4 | 5 |
| **Backup & Point-in-Time Recovery** | 5 | 3 | 4 | 4 | 3 | 5 |
| **Built-In Connection Pooling** | 5 | 2 | 4 | N/A | N/A | 4 |
| **Observability & Monitoring** | 4 | 3 | 5 | 4 | 3 | 4 |
| **Community & Vendor Lock-In** | 5 | 4 | 2 | 2 | 2 | 1 |
| **License & Commercial Restrictions** | 5 | 5 | 3 | 3 | 3 | N/A |
| **Crossplane Compatibility** | 5 | 3 | 3 | 2 | 2 | 5 |
| **Multi-Tenant Isolation** | 4 | 3 | 4 | 3 | 3 | 5 |
| **Operational Complexity** | 4 | 3 | 2 | 1 | 1 | 5 |
| **Resource Efficiency** | 5 | 5 | 4 | 2 | 2 | 5 |
| **PostgreSQL Ecosystem Fit** | 5 | 5 | 5 | 2 | 2 | 5 |
| **TOTAL** | **53** | **43** | **45** | **33** | **32** | **48** |

### Scoring Rationale

**K8s-Native Operation (CloudNativePG: 5, Zalando: 4, Crunchy: 5, Others: varies).**
CloudNativePG and Crunchy both score 5: they are designed from the ground up as Kubernetes operators with CRDs. Zalando scores 4: it's Kubernetes-native but requires integration with external tooling (connection pooling). CockroachDB scores 4: Kubernetes support exists but isn't the primary deployment model (it's designed for distributed infrastructure beyond K8s). YugabyteDB scores 3: similar—Kubernetes support is secondary. Azure PG Flexible is scored N/A (not applicable): it's a managed service, not K8s-native.

**HA & Automatic Failover (CloudNativePG: 5, Zalando: 4, Crunchy: 5, CockroachDB: 5, YugabyteDB: 4, Azure: 5).**
CloudNativePG, Crunchy, and Azure all score 5: built-in, automatic, battle-tested failover. Zalando scores 4: Patroni HA is excellent but requires more operator tuning and cluster-wide configuration. YugabyteDB scores 4: distributed HA works but adds complexity; less proven than PostgreSQL's simpler HA.

**Backup & Point-in-Time Recovery (CloudNativePG: 5, Zalando: 3, Crunchy: 4, CockroachDB: 4, YugabyteDB: 3, Azure: 5).**
CloudNativePG scores 5: Barman integration is seamless, opinionated, and enterprise-proven. Azure scores 5: Azure-managed backups with configurable retention and geo-redundancy. Crunchy scores 4: pgBackRest is excellent but requires more manual integration. Zalando scores 3: WAL archiving and backup are possible but less opinionated; more integration work. CockroachDB and YugabyteDB score 3–4: built-in snapshots and backups work but are less flexible than Barman/pgBackRest.

**Built-In Connection Pooling (CloudNativePG: 5, Zalando: 2, Crunchy: 4, Others: N/A).**
CloudNativePG scores 5: PgBouncer is included and automatically configured. Crunchy scores 4: PgBouncer can be added but requires separate configuration. Zalando scores 2: connection pooling is NOT built-in; must be managed separately (a significant operational gap). CockroachDB, YugabyteDB, and distributed DBs score N/A: they don't use connection pooling in the traditional sense.

**Observability & Monitoring (CloudNativePG: 4, Zalando: 3, Crunchy: 5, CockroachDB: 4, YugabyteDB: 3, Azure: 4).**
Crunchy scores 5: pgMonitor is best-in-class observability. CloudNativePG and Azure both score 4: native Prometheus metrics (CloudNativePG) or Azure Monitor integration (Azure) are good but less comprehensive than pgMonitor. Zalando and YugabyteDB score 3: adequate monitoring but less opinionated.

**Community & Vendor Lock-In (CloudNativePG: 5, Zalando: 4, Crunchy: 2, CockroachDB: 2, YugabyteDB: 2, Azure: 1).**
CloudNativePG scores 5: CNCF Sandbox project, no single vendor. Zalando scores 4: maintained by Zalando but open-source and community-friendly. Crunchy scores 2: Crunchy Data is the sole steward; commercial license model creates dependency. CockroachDB and YugabyteDB score 2: venture-backed companies; funding changes could shift priorities. Azure scores 1: Microsoft dependency; no optionality if Azure changes strategy.

**License & Commercial Restrictions (CloudNativePG: 5, Zalando: 5, Crunchy: 3, CockroachDB: 3, YugabyteDB: 3, Azure: N/A).**
CloudNativePG and Zalando score 5: Apache 2.0 and MIT licenses, no commercial restrictions. Crunchy, CockroachDB, and YugabyteDB score 3: open-source base but commercial licenses for advanced features (vendor lock-in risk). Azure scores N/A: managed service pricing model, not license-based.

**Crossplane Compatibility (CloudNativePG: 5, Zalando: 3, Crunchy: 3, CockroachDB: 2, YugabyteDB: 2, Azure: 5).**
CloudNativePG and Azure score 5: excellent Crossplane provider support, full resource coverage. Zalando and Crunchy score 3: CRDs are manageable via Crossplane but require custom Compositions due to operator-specific APIs. CockroachDB and YugabyteDB score 2: Crossplane providers are less mature or community-maintained.

**Multi-Tenant Isolation (CloudNativePG: 4, Zalando: 3, Crunchy: 4, CockroachDB: 3, YugabyteDB: 3, Azure: 5).**
Azure scores 5: managed service with strong tenant isolation at the infrastructure level. CloudNativePG and Crunchy score 4: support schema-per-tenant and RLS patterns well. Zalando, CockroachDB, and YugabyteDB score 3: capable but less integrated with platform-level isolation strategies.

**Operational Complexity (CloudNativePG: 4, Zalando: 3, Crunchy: 2, CockroachDB: 1, YugabyteDB: 1, Azure: 5).**
Azure scores 5 (least complex): Microsoft handles all operator duties. CloudNativePG scores 4: simple, single CRD, good defaults. Zalando scores 3: more configuration surface (connection pooling separate). Crunchy scores 2: many CRD types, more operational knobs. CockroachDB and YugabyteDB score 1: distributed consensus requires deep expertise.

**Resource Efficiency (CloudNativePG: 5, Zalando: 5, Crunchy: 4, CockroachDB: 2, YugabyteDB: 2, Azure: 5).**
CloudNativePG, Zalando, and Azure score 5: lean resource usage. Crunchy scores 4: slightly heavier due to additional tooling. CockroachDB and YugabyteDB score 2: distributed consensus and replication require more CPU, memory, and network.

**PostgreSQL Ecosystem Fit (CloudNativePG: 5, Zalando: 5, Crunchy: 5, CockroachDB: 2, YugabyteDB: 2, Azure: 5).**
All native PostgreSQL options score 5 (they ARE PostgreSQL). CockroachDB and YugabyteDB score 2: wire-compatible but not dialect-compatible; extensions and edge cases differ.

**Conclusion from Matrix:** CloudNativePG (53 points) and Azure PostgreSQL Flexible Server (48 points) emerge as clear winners. Together, they provide excellent coverage for self-hosted and managed scenarios, with strong community backing, operational simplicity, and ecosystem fit. This alignment validates the ADR-047 decision.

---

## 10.4 24-Month Roadmap Analysis

Understanding the direction of each component helps predict operational changes, feature availability, and support longevity. The following roadmap analysis spans approximately 24 months from early 2026.

### 10.4.1 CloudNativePG (Hetzner Self-Hosted)

**Current State (Early 2026).**
- Version: 1.25.x
- CNCF Status: Sandbox (promoted December 2024)
- Release Cadence: minor release every ~3 months, patch releases monthly

**Roadmap (2026–2028).**

*Declarative Tablespace Management.* Currently, tablespaces (distinct storage volumes for indexes, hot data, cold data) require manual PostgreSQL commands or custom scripts. Future versions will support tablespace declarations in the Cluster CRD, allowing operators to define storage tiers and automatic data migration without downtime.

*Enhanced Online Resize.* Current online resize (PVCResize) works but requires careful monitoring. Future versions will add progress tracking, estimated time-to-completion, and automatic rollback if errors occur during resize.

*Kubernetes 1.32+ Compatibility.* Kubernetes 1.31+ removes certain v1 APIs. CloudNativePG will maintain compatibility while adding support for newer API groups. This is routine maintenance but requires attention during major K8s upgrades.

*CNCF Incubation Application.* The CloudNativePG project has indicated intent to apply for CNCF Incubation status (currently at Sandbox). Incubation status increases funding, governance rigor, and long-term project stability guarantees. Expected timeline: late 2026 or early 2027.

**Risk Assessment: LOW.**
- Development is active (EDB and community contributors)
- CNCF backing provides governance and funding stability
- No competing fork or alternative with superior features
- Adoption is growing; projects like Crunchy Data (a competing operator) and Bitnami include CloudNativePG as an option

**Migration Path if Needed (Worst Case).**
If CloudNativePG became unmaintained (unlikely but possible), the fallback is Zalando Postgres Operator. Both use Patroni for HA; the migration path is:
1. Set up a Zalando Operator cluster in parallel
2. Use pg_basebackup to restore from CloudNativePG replica
3. Switch application connections to Zalando cluster
4. Decommission CloudNativePG cluster

This is a significant undertaking but not impossible; it's why CAVE maintains documentation of Zalando as an alternative.

### 10.4.2 Azure Database for PostgreSQL Flexible Server

**Current State (Early 2026).**
- Supported Versions: PostgreSQL 13, 14, 15, 16, 17
- HA: Zone-redundant standby in separate AZ, automatic failover
- SKUs: Burstable (B1s, B2s), General Purpose (D2s_v3 to D96s_v3), Memory-Optimized (E2s_v3 to E104is_v3)
- Max Storage: 16 TB
- Max Connections: varies by SKU (typically 300–5000)

**Roadmap (2026–2028).**

*Enhanced Vector Search Integration.* Azure is investing in AI/ML database features. Future versions will optimize pgvector performance with native vector indexing (HNSW, IVFFlat) and GPU-accelerated similarity search. This benefits CAVE tenants building AI-native applications (embeddings, RAG systems).

*AI-Assisted Query Optimization.* Azure Monitor will provide AI-powered query recommendations: detecting missing indexes, suggesting partitioning strategies, and identifying optimization opportunities. This feature will be optional (opt-in) and will not affect existing workloads.

*Terraform Provider Expansion.* Full coverage of read replicas, logical replication slots, and parameter groups is in progress. Terraform coverage will reach feature parity with Azure CLI by 2027.

*Continued Single Server Deprecation.* Azure's legacy PostgreSQL Single Server is deprecated and will be fully delisted in 2027. All CAVE customers must migrate to Flexible Server (or Hyperscale, which CAVE does NOT use). This migration is mostly transparent (Flexible Server is backward-compatible) but requires testing.

**Risk Assessment: LOW.**
- Microsoft's strategic commitment to PostgreSQL is strong (it's the primary PG offering)
- Backward compatibility is maintained; Azure doesn't break existing workloads
- Managed service model means Microsoft handles version upgrades, security patches, and HA orchestration

**Lock-In Risk: MEDIUM (but manageable).**
Azure PostgreSQL Flexible Server is a managed service, so CAVE has a dependency on Azure's availability and pricing. However, the lock-in is NOT tight:
- Data is standard PostgreSQL; logical replication or pg_dump/pg_restore can migrate to any PostgreSQL cluster
- Credentials and configurations are portable
- The only migration cost is the one-time data transfer and application reconfiguration

If Azure made unacceptable pricing changes, CAVE could migrate databases to self-hosted CloudNativePG on Hetzner within weeks. The lock-in is operational, not technical.

### 10.4.3 PostgreSQL Core (Upstream)

Understanding the PostgreSQL community roadmap informs feature availability and operational changes.

**PostgreSQL 17 (Current, Released 2024 Q4).**
- Incremental backups: greatly reduces backup storage and time
- JSON_TABLE function: SQL standard JSON processing
- Improved partitioning: better query planner for partitioned tables
- Performance improvements: vacuum, index creation, executor

**PostgreSQL 18 (Expected 2025 Q4 / Early 2026).**
- Asynchronous I/O improvements: faster bulk operations
- Enhanced logical replication: lower replication lag, better failover
- Transparent Data Encryption (TDE): encryption-at-rest without application changes (IF included)

**Impact on CAVE.**
- Incremental backups (PG 17) reduce backup storage by 80–90% compared to full backups, lowering cost
- TDE (if released in PG 18) simplifies encryption compliance for confidential data, reducing need for application-layer encryption
- Enhanced logical replication enables multi-region read replicas without strong replication lag

**Upgrade Strategy.**
CAVE will track PostgreSQL releases but will NOT immediately adopt new versions. The strategy is:
1. CloudNativePG and Azure both release support within 2–3 months of PostgreSQL GA
2. CAVE's staging environments test new versions for 1–2 months
3. Production rollout occurs on a per-database basis; not all tenants upgrade simultaneously
4. Major version upgrades (e.g., 17→18) are opt-in for tenants; CAVE provides migration playbooks but doesn't force upgrades

### 10.4.4 Crossplane and Database Abstraction

**Crossplane v2 Maturity (2026+).**
- Namespace-first, function-based composition is stable (v2 released 2024, broadly adopted by 2025)
- Composition Functions (Go-based) are the preferred authoring model; classic Compositions are legacy
- MRAP (Multi-Resource Attachment Point) adoption is increasing, reducing CRD sprawl on dev profiles

**Database XR Implications.**
As Crossplane matures, CAVE's Database XR will:
- Migrate to Composition Functions (if not already) for more flexible provider routing
- Leverage MRAP to avoid loading unneeded providers (dev-hetzner doesn't load Azure CRDs; dev-azure doesn't load CloudNativePG CRDs)
- Integrate with Crossplane's secret management for credential rotation (complementing ESO)

**Risk Assessment: LOW.**
- Crossplane is CNCF Incubating (stable, well-governed)
- Database abstraction patterns are proven across many CAVE deployments
- Backward compatibility is maintained; Composition Function migration is non-disruptive

---

## 10.5 Architecture

### 10.5.1 Hetzner Architecture (CloudNativePG)

The Hetzner deployment model runs PostgreSQL as a Kubernetes-native cluster, with automated failover, built-in connection pooling, and external backup to MinIO (S3-compatible object storage).

**Topology Overview.**

```
┌─────────────────────────────────────────────────────────────────────┐
│                     Hetzner Kubernetes Cluster                       │
│                  (Multiple Failure Domains)                          │
│                                                                       │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                    Tenant Namespace                          │   │
│  │                 (e.g., tenant-acme-prod)                     │   │
│  │                                                               │   │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │   │
│  │  │ PostgreSQL   │  │ PostgreSQL   │  │ PostgreSQL   │      │   │
│  │  │ Primary (RW) │ ←→│ Replica 1(RO)│ ←→│ Replica 2(RO)│     │   │
│  │  │              │  │              │  │              │      │   │
│  │  │ v17.2        │  │ v17.2        │  │ v17.2        │      │   │
│  │  │ (Pool Slot)  │  │ (Pool Slot)  │  │ (Pool Slot)  │      │   │
│  │  └──────┬───────┘  └──────────────┘  └──────────────┘      │   │
│  │         │ Replication                                        │   │
│  │  ┌──────┴────────────────────────────────────────────┐      │   │
│  │  │          Built-In PgBouncer (Connection Pool)     │      │   │
│  │  │  Read-Write (Primary) | Read-Only (Replicas)     │      │   │
│  │  │  Max Conn: 300       | Queue: 5000               │      │   │
│  │  └──────┬─────────────────────────────────────────┬──┘      │   │
│  │         │                                         │          │   │
│  │         │ Applications Connect                    │          │   │
│  │         │ (Credential Secret)                     │          │   │
│  │                                                    │          │   │
│  │  ┌──────────────────────┐   ┌──────────────────────┐        │   │
│  │  │ WAL Archiving        │   │ Barman Backup        │        │   │
│  │  │ (Every 16 MB or 5s)  │   │ (Daily base backup)  │        │   │
│  │  └─────────┬────────────┘   └──────────┬───────────┘        │   │
│  │            │                           │                    │   │
│  │            └───────────────┬───────────┘                    │   │
│  │                            │                                │   │
│  │            ┌───────────────▼──────────────┐                 │   │
│  │            │    MinIO S3-Compatible       │                 │   │
│  │            │    Backup Bucket             │                 │   │
│  │            │ (7-day PITR retention)       │                 │   │
│  │            └──────────────────────────────┘                 │   │
│  │                                                               │   │
│  │  ┌──────────────┐  ┌──────────────────────────────────────┐ │   │
│  │  │ ServiceMonitor│ │ CloudNativePG Cluster CRD            │ │   │
│  │  │ (Pod Metrics) │ │ - size: large                        │ │   │
│  │  └──────┬────────┘ │ - performanceProfile: high           │ │   │
│  │         │          │ - dataResidency: eu                  │ │   │
│  │         │          │ - classification: confidential        │ │   │
│  │         │          └──────────────────────────────────────┘ │   │
│  │         │                                                    │   │
│  │         │ Scrape Metrics                                    │   │
│  │         │                                                    │   │
│  │  ┌──────▼─────────────────────────────────────────────────┐ │   │
│  │  │ Prometheus (Cluster-Wide)                              │ │   │
│  │  │ Dashboards: QPS, Replication Lag, Cache Hit Rate       │ │   │
│  │  └──────────────────────────────────────────────────────┘ │   │
│  │                                                               │   │
│  │  ┌──────────────────────────────────────────────────────┐   │   │
│  │  │ External Secrets Operator (ESO)                      │   │   │
│  │  │ Syncs credentials from OpenBao                       │   │   │
│  │  │ Rotation: every 30 days                              │   │   │
│  │  │ Secret: database-user-password (mounted to pods)     │   │   │
│  │  └──────────────────────────────────────────────────────┘   │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                                                                       │
└─────────────────────────────────────────────────────────────────────┘
```

**Component Breakdown.**

**PostgreSQL Cluster (3 Nodes).**
- **Primary.** Accepts read-write transactions. Pool slot reserves one connection for superuser management.
- **Replica 1 & 2.** Hot standbys in different Hetzner failure domains (racks or data centers). Each maintains a full copy of the database and streams from primary via synchronous replication. If primary fails, quorum-based Patroni election promotes the best replica.
- **Why 3 Nodes.** With 3 nodes, the cluster tolerates 1 failure without losing quorum. If 2 nodes fail, the cluster stops accepting writes (preventing split-brain), but the 1 remaining node is still readable. This is the minimum safe configuration for production.

**PgBouncer (Built-In Connection Pool).**
- Runs as a sidecar container in each node pod (injected by CloudNativePG)
- Multiplexes many client connections onto fewer server-side connections
- Configuration is automatically generated from the Cluster CRD; no manual tuning needed
- Routes read-write queries to primary, read-only queries to replicas (via a `ro` endpoint)
- Applications see three endpoints:
  - `database-rw`: read-write, connects to primary
  - `database-ro`: read-only, load-balances across replicas
  - `database-r`: routed via explicit pool selector (advanced)

**WAL Archiving and Barman Backup.**
- **WAL (Write-Ahead Log).** Every transaction is written to WAL before being applied to the table. WAL files (16 MB each) are immutable and form a continuous log. Archiving ensures every WAL file is persisted to MinIO within 5 seconds or 16 MB (whichever comes first).
- **Barman.** A dedicated backup utility that maintains a full backup repository. Barman performs:
  - Daily base backups (copy of the entire database at a point-in-time)
  - WAL archiving and compression
  - Point-in-time recovery (PITR) by replaying WAL up to any desired timestamp
- **PITR Window.** With continuous WAL archiving, you can recover to any second within the last 7 days (configurable). Base backups are retained for 7 days; older WAL is deleted if not needed for PITR.

**MinIO Backup Target.**
- S3-compatible object storage running on Hetzner or as managed service
- Stores compressed WAL files and base backup manifests
- Supports lifecycle policies (e.g., delete objects older than 30 days) to control costs
- Data is encrypted at rest using MinIO's KMS integration

**ServiceMonitor and Prometheus.**
- CloudNativePG exposes Prometheus metrics on port 9187 (by default)
- A ServiceMonitor CRD instructs Prometheus to scrape these metrics
- Key metrics: connection count, replication lag, cache hit rate, slow queries (if pg_stat_statements is enabled)
- Alerting rules trigger on replication lag > 10 seconds, connection exhaustion, or backup failures

**External Secrets Operator (ESO) and OpenBao.**
- ESO periodically polls OpenBao for new credentials
- When credentials are rotated, OpenBao generates a new password and updates the PostgreSQL role
- ESO syncs the new credential into a Kubernetes secret in the tenant namespace
- Applications read from the Kubernetes secret; no restart required (if using dynamic secret mounting)

**Failure Scenarios and Recovery.**

| Scenario | Outcome | RTO | RPO |
|---|---|---|---|
| Replica fails | Cluster degrades to 2 nodes. Primary healthy, single replica. No data loss. | N/A | 0s |
| Primary fails (with standby in-sync) | Fastest replica promoted to primary via Patroni election (automatic). Failover takes 10–30 seconds. | 30s | 0s (if async replication is NOT used) |
| Entire cluster lost | Restore from latest base backup + WAL replay. Can recover to any point in last 7 days. | 10–30 min (depending on data size) | Configurable; typically <1 min |
| Credential compromised | ESO detects rotation window, generates new password, updates secret. Old connections are terminated after a grace period. | N/A | N/A |

### 10.5.2 Azure Architecture (Azure PG Flexible Server)

The Azure deployment model uses Azure's managed PostgreSQL service, eliminating the need for a dedicated operator. HA is provided by Azure's zone-redundant standby, and backup is handled by Azure's automated backup system.

**Topology Overview.**

```
┌──────────────────────────────────────────────────────────────────┐
│                    Azure Resource Group                          │
│              (Tenant's Managed Resources)                        │
│                                                                  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │              PostgreSQL Flexible Server                   │  │
│  │                                                            │  │
│  │  ┌─────────────────────┐  ┌──────────────────────────┐   │  │
│  │  │ Primary Replica     │  │ Standby Replica (HA)     │   │  │
│  │  │ (AZ-1, US East)     │←→│ (AZ-2, US East)         │   │  │
│  │  │                     │  │                          │   │  │
│  │  │ SKU: D4s_v3         │  │ Synchronized (RTO <1s)   │   │  │
│  │  │ (4 vCores, 16 GB)   │  │ Auto-failover enabled    │   │  │
│  │  │                     │  │                          │   │  │
│  │  │ PostgreSQL 17       │  │ PostgreSQL 17            │   │  │
│  │  │ Storage: 256 GB     │  │ Storage: 256 GB (replica)│   │  │
│  │  │                     │  │                          │   │  │
│  │  └──────────┬──────────┘  └──────────────────────────┘   │  │
│  │             │ Replication                                 │  │
│  │  ┌──────────┴──────────────────────────────────┐          │  │
│  │  │   Private Endpoint                          │          │  │
│  │  │   (VNet-integrated, no public IP)           │          │  │
│  │  │   AKS pods connect via service endpoint     │          │  │
│  │  └──────────┬──────────────────────────────────┘          │  │
│  │             │ Applications Connect (Port 5432)            │  │
│  │                                                            │  │
│  │  ┌──────────────────────┐  ┌────────────────────────────┐ │  │
│  │  │ Automated Backups    │  │ Azure Storage Account       │ │  │
│  │  │ (Daily base backup)  │→ │ (Geo-redundant storage)    │ │  │
│  │  │ (Hourly incremental) │  │ 7-35 day retention         │ │  │
│  │  │ (WAL archiving)      │  │ (Long-term backups: 1y)    │ │  │
│  │  └──────────────────────┘  └────────────────────────────┘ │  │
│  │                                                            │  │
│  │  ┌──────────────────────┐  ┌────────────────────────────┐ │  │
│  │  │ Azure Key Vault      │←─│ ESO (Credentials)          │ │  │
│  │  │ Stores credentials   │  │ Syncs passwords & secrets  │ │  │
│  │  │ (RBAC-protected)     │  │ Rotation: 30 days         │ │  │
│  │  └──────────────────────┘  └────────────────────────────┘ │  │
│  │                                                            │  │
│  │  ┌──────────────────────────────────────────────────────┐ │  │
│  │  │ Azure Monitor + Application Insights                │ │  │
│  │  │ Metrics: CPU, memory, disk I/O, connections        │ │  │
│  │  │ Query Performance Insights: slow query analysis     │ │  │
│  │  │ Alerts: CPU > 80%, connections > 80%, backup failed│ │  │
│  │  └──────────────────────────────────────────────────────┘ │  │
│  │                                                            │  │
│  │  ┌──────────────────────────────────────────────────────┐ │  │
│  │  │ Server Parameters (Managed)                         │ │  │
│  │  │ max_connections: 300 (Burstable B2s)                │ │  │
│  │  │ shared_buffers: 6 GB (managed)                       │ │  │
│  │  │ Random-Access Memory (RAM): 16 GB                   │ │  │
│  │  │ Maintenance Window: Sun 2-4 AM UTC (configurable)   │ │  │
│  │  └──────────────────────────────────────────────────────┘ │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ Networking (VNet)                                         │  │
│  │ - Private Endpoint: traffic never leaves Azure backbone  │  │
│  │ - NSG: inbound restricted to AKS subnet only             │  │
│  │ - No public endpoint; no internet exposure               │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

**Component Breakdown.**

**Flexible Server with Zone-Redundant HA.**
- **Primary Replica.** Runs in Availability Zone 1. Accepts all read-write transactions.
- **Standby Replica.** Runs in Availability Zone 2. Maintains a synchronous copy via Azure's internal replication. If primary fails, automatic failover promotes standby to primary (typically <1 second RTO).
- **SKU (Compute Tier).** Choices include:
  - **Burstable (B1s, B2s).** For development and light workloads. CPU is shared; burstable credits accumulate during idle time. Cost-effective but unpredictable under sustained load.
  - **General Purpose (D2s_v3 to D96s_v3).** Balanced compute, memory, and network. Recommended for most production workloads.
  - **Memory-Optimized (E2s_v3 to E104is_v3).** For workloads with large working sets (analytics, in-memory caching). Higher cost but lower I/O latency.
- **Storage.** Automatically scales up to 16 TB. IOPS scale with storage tier (e.g., P10=100 IOPS, P50=7500 IOPS). No manual intervention required.

**Automated Backup System.**
- **Base Backups.** Daily full backup (compression reduces size by 80–90%). Retained for configurable period (7–35 days).
- **Incremental Backups.** Hourly snapshots of changed blocks (available in some SKU tiers). Reduces backup time and storage.
- **WAL Archiving.** Continuous streaming of transaction logs to Azure Storage. Enables PITR.
- **PITR Window.** Configurable 7–35 days. You can restore to any second within this window using Azure CLI, Terraform, or the Portal.
- **Geo-Redundant Backups.** Backups are replicated to a secondary region (e.g., US West) to protect against regional disasters. (Optional; incurs additional cost.)

**Private Endpoint.**
- An Azure Private Endpoint is a network interface that provides private connectivity to the PostgreSQL server.
- The endpoint exists in the customer's VNet. The AKS cluster has a route to this endpoint via its VNet integration.
- Traffic never leaves Azure's private backbone; no internet exposure.
- DNS resolution: `database.c.postgres.database.azure.com` resolves to the private endpoint IP within the VNet, not a public IP.

**Azure Key Vault and Credentials.**
- Credentials (username and password) are stored in Azure Key Vault, an Azure-managed secrets store.
- RBAC policies control who can read credentials.
- ESO polls Key Vault periodically and syncs new credentials into Kubernetes secrets.
- Password rotation: every 30 days, a new password is generated and stored in Key Vault. Applications read the refreshed Kubernetes secret automatically.

**Azure Monitor Integration.**
- **Metrics.** CPU, memory, disk I/O, active connections, replication lag (if read replica is enabled).
- **Query Performance Insights.** Analyzes the query store and identifies slow queries, missing indexes, and optimization opportunities.
- **Alerts.** Configured via Azure Monitor action groups. Examples: CPU >80%, connections >80%, backup failure.
- **Logs.** PostgreSQL logs are streamed to Azure Log Analytics. Queries like "find all password change events" or "list failed authentication attempts" are possible.

**Maintenance Window.**
- Azure periodically applies security patches, minor version updates, and infrastructure maintenance.
- A configurable maintenance window (e.g., Sunday 2–4 AM UTC) specifies when this work can occur.
- The standby is patched first, then promoted temporarily; primary is patched; original primary is restored. Downtime is typically <1 minute.

**Failure Scenarios and Recovery.**

| Scenario | Outcome | RTO | RPO |
|---|---|---|---|
| Standby fails | Primary continues; HA is disabled. Azure automatically reprovisiones a standby. | N/A | 0s |
| Primary fails (with standby in-sync) | Automatic failover to standby. Failover takes <1 second. Clients reconnect within 10–30 seconds. | 30s | 0s |
| Entire server lost | Restore from automated backup. Specify a point-in-time (within 7–35 days), and Azure restores to a new server. | 30–60 min | 0–60 min (depending on chosen restore point) |
| Region failure | Use geo-redundant backup. Restore to a new server in a different region. | Hours | Depends on backup frequency |

### 10.5.3 Crossplane Abstraction: Unified Database XR

Crossplane's core role is to hide the differences between Hetzner and Azure deployments, allowing developers to request databases with a single resource definition.

**XRD (Composite Resource Definition).**

The Database XRD defines the schema for the Composite Resource (XR). Here is the structure:

```yaml
apiVersion: cave.dev/v1alpha1
kind: Database
metadata:
  name: billing-db
  namespace: tenant-acme-prod
spec:
  parameters:
    size: large                    # small|medium|large|xlarge
    classification: confidential   # public|internal|confidential
    performanceProfile: high       # standard|high|extreme
    dataResidency: eu              # eu|us-east|us-west|ap (ADR-113)
    backups: enabled               # enabled|disabled
    backupRetention: 7             # days (optional; default 7)
status:
  conditions:
    - type: Ready
      status: "True"
    - type: BackupHealthy
      status: "True"
  connectionSecret:
    name: billing-db-credentials
    namespace: tenant-acme-prod
  database:
    hostname: billing-db-rw.tenant-acme-prod
    readOnlyHostname: billing-db-ro.tenant-acme-prod (Hetzner only)
    port: 5432
```

**Field Semantics.**

- **size.** Defines CPU, memory, and storage allocation:
  - `small`: 1 CPU, 2 GB RAM, 10 GB storage, 1000 IOPS
  - `medium`: 2 CPU, 4 GB RAM, 50 GB storage, 3000 IOPS
  - `large`: 4 CPU, 8 GB RAM, 256 GB storage, 10000 IOPS
  - `xlarge`: 8 CPU, 16 GB RAM, 512 GB storage, 30000 IOPS

  Mapping to infrastructure:
  - Hetzner (CloudNativePG): maps to node resource requests and storage class
  - Azure: maps to SKU family (Burstable, GP, MO) based on workload profile

- **classification.** Enforced by OPA (ADR-102):
  - `public`: no encryption required, standard backup retention
  - `internal`: encryption-at-rest recommended, 7-day backup retention
  - `confidential`: mandatory encryption-at-rest, 30-day backup retention, audit logging enabled

- **performanceProfile.** Determines IOPS and connection limits:
  - `standard`: 3K IOPS (Hetzner), Burstable SKU (Azure), 100 concurrent connections
  - `high`: 10K IOPS (Hetzner), GP SKU (Azure), 300 concurrent connections
  - `extreme`: 30K IOPS (Hetzner), MO SKU (Azure), 1000 concurrent connections

- **dataResidency.** Enforces geographic placement (ADR-113):
  - `eu`: data must remain in EU regions (GDPR compliance). For Hetzner, restricted to EU data centers. For Azure, restricted to Europe regions.
  - `us-east`: US East Coast. HIPAA workloads often require this.
  - `us-west`: US West Coast. Cost optimization for US-based tenants.
  - `ap`: Asia-Pacific. Serves APAC customers; data doesn't leave the region.

- **backups.** Enable/disable automated backups. Disabled is rarely used (only for ephemeral dev databases).

- **backupRetention.** Number of days to retain backups. Defaults to 7; can be extended to 30 or 60 for compliance-sensitive workloads.

**Composition Function (Go-Based).**

The Composition Function is a Go program that runs inside the Crossplane controller. It examines the Database XR and decides which infrastructure-specific resource to create.

Pseudocode logic:

```
func Compose(ctx, database *v1.Database, desired *v1.DesiredResourceSet) {
  // Detect provider from namespace label or profile
  provider := detectProvider(database.Namespace)

  if provider == "hetzner" {
    // Create CloudNativePG Cluster
    cluster := &cnpg.Cluster{
      Spec: {
        Instances: 3,
        PostgresVersion: "17",
        Resources: mapSize(database.Spec.Size),
        StorageConfiguration: {
          Size: mapStorageSize(database.Spec.Size),
          StorageClass: "fast-ssd",
        },
        Bootstrap: {
          Backup: {
            Source: "barman",
            BarmanObjectStore: {
              Destination: "s3://backup-bucket/acme-prod/billing-db",
            },
          },
        },
      },
    }
    desired.Resources = append(desired.Resources, cluster)
  }

  if provider == "azure" {
    // Create Azure PostgreSQL Flexible Server
    server := &azure.PostgreSQLServer{
      Spec: {
        ResourceGroupName: database.Namespace,
        SKU: mapPerformanceProfile(database.Spec.PerformanceProfile),
        Storage: {
          SizeGB: mapStorageSize(database.Spec.Size),
        },
        HighAvailability: {
          Mode: "ZoneRedundant",
        },
        Backup: {
          BackupRetentionDays: database.Spec.BackupRetention,
          GeoRedundantBackupEnabled: true,
        },
      },
    }
    desired.Resources = append(desired.Resources, server)
  }

  // Conditionally add ESO SecretStore and ExternalSecret for credential rotation
  if database.Spec.CredentialRotation {
    externalSecret := &eso.ExternalSecret{...}
    desired.Resources = append(desired.Resources, externalSecret)
  }
}
```

In reality, the logic is more sophisticated: it handles data residency restrictions, validates that the requested size is available in the target region, and configures monitoring integrations.

**Example: Developer Creates a Database.**

A developer in namespace `tenant-acme-prod` (running on Hetzner) creates:

```yaml
apiVersion: cave.dev/v1alpha1
kind: Database
metadata:
  name: billing-db
  namespace: tenant-acme-prod
spec:
  parameters:
    size: large
    classification: confidential
    performanceProfile: high
    dataResidency: eu
    backups: enabled
```

Within seconds:
1. The Crossplane controller detects the Database resource
2. The Composition Function identifies the namespace is on Hetzner (via a label or namespace selector)
3. The function creates a CloudNativePG Cluster resource:
   - 3 instances, PostgreSQL 17
   - 4 CPU, 8 GB RAM per instance
   - 256 GB SSD storage (fast-ssd storage class)
   - 10K IOPS (high performance profile)
   - Barman backup to MinIO with WAL archiving
   - Encryption-at-rest enabled (mandatory for confidential)
   - Audit logging enabled (confidential classification)
4. CloudNativePG operator reads the Cluster CRD and provisions the cluster
5. Within 1–2 minutes, the cluster is healthy
6. A Kubernetes secret `billing-db-credentials` is created with connection details
7. ESO syncs the credentials from OpenBao; password rotation is configured for every 30 days

The developer can then deploy an application with:

```yaml
spec:
  containers:
  - name: app
    image: app:1.0
    env:
    - name: DATABASE_URL
      valueFrom:
        secretKeyRef:
          name: billing-db-credentials
          key: connection-string
```

The application connects to `billing-db-rw.tenant-acme-prod:5432` (read-write endpoint) or `billing-db-ro.tenant-acme-prod:5432` (read-only endpoint for reporting queries).

**Performance Profile Mapping.**

| Profile | Hetzner (CloudNativePG) | Azure (PG Flexible) | Typical Use Case |
|---|---|---|---|
| **standard** | 3K IOPS, 100 conn, 1 CPU / 2 GB | Burstable B1s (1 vCore, 1 GB) | Development, CI/CD, small tenants |
| **high** | 10K IOPS, 300 conn, 2 CPU / 4 GB | GP D4s_v3 (4 vCore, 16 GB) | Production small-to-mid workloads |
| **extreme** | 30K IOPS, 1000 conn, 8 CPU / 16 GB | MO E8s_v3 (8 vCore, 64 GB) | High-concurrency, analytics |

**Advantages of This Abstraction.**

1. **Developer doesn't care about provider.** Identical YAML works on Hetzner or Azure.
2. **Operational knowledge is centralized.** All backup, HA, and credential policies are defined in one Composition Function.
3. **Easy to audit and audit.** A single Composition Function source is the source of truth for how databases are provisioned.
4. **Testable.** Composition Functions can be unit-tested before rollout.

---

## 10.6 Use Cases & Developer Scenarios

This section walks through real-world workflows that developers and operators encounter when working with CAVE's PostgreSQL abstraction. Each scenario is a narrative that shows the system in action—both the happy path and where things can go wrong.

### 10.6.1 Developer Creates a Database via Crossplane XR

A developer on tenant `acme-corp` (running in the Hetzner environment) needs a new billing database to track customer invoices. The database must be classified as confidential because it contains financial data. Here's how the provisioning flow unfolds.

The developer prepares a Database manifest:

```yaml
apiVersion: cave.dev/v1alpha1
kind: Database
metadata:
  name: billing-db
  namespace: tenant-acme-prod
spec:
  size: large
  performanceProfile: high
  classification: confidential
  dataResidency: eu
  backups:
    enabled: true
    retentionDays: 90
```

**Step 1: Submit to Kubernetes.** The developer applies this YAML to the cluster (via `kubectl apply`, GitOps, or Backstage template). The Kubernetes API server accepts the resource because the CustomResourceDefinition for Database already exists.

**Step 2: OPA Validation.** Before the Crossplane controller sees the Database, the OPA policy engine (enforced as a ValidatingWebhookConfiguration) intercepts the create request. OPA checks:
- Is `classification` present? Yes (`confidential`).
- Is `classification` one of the allowed values (`public`, `internal`, `confidential`, `restricted`)? Yes.
- Does the `dataResidency` (`eu`) match the tenant's allowed regions (from a ConfigMap or LDAP)? Yes (assume acme-corp is EU-only).
- Does the requested `size` and `performanceProfile` fit within the tenant's quota (from ADR-124 MRAP policy)? Yes, acme-corp has budget for a high-performance instance.

If any check fails, OPA rejects the request with a human-readable error (e.g., `"classification is required but missing"`). The developer sees this immediately and corrects the YAML.

**Step 3: Crossplane Detects the XR.** Once OPA approves the create, Crossplane's Package Manager (provider-cave) watches for Database resources. When it sees the new `billing-db` resource, it retrieves the Composition that defines how to compose a Database from primitive cloud resources.

**Step 4: Composition Function Routes to Provider.** The Composition Function (running as a WASM module or Go binary) executes. It reads the Database spec and determines:
- The tenant namespace `tenant-acme-prod` is labeled `provider=hetzner` (or the Composition queries a label-based registry).
- Therefore, create a CloudNativePG Cluster (not an Azure PostgreSQL Flexible Server).
- Map the `large` size to a 3-instance cluster with 4 CPU and 8 GB RAM per instance.
- Map the `high` performanceProfile to 10K IOPS SSD storage.
- Set `dataResidency: eu` by selecting the Hetzner data center in Nuremberg (not Ashburn, Virginia).

The Composition Function generates three desired resources:
1. **CloudNativePG Cluster.** A Kubernetes Cluster CR with the HA configuration, bootstrap SQL, backup policy, and monitoring settings.
2. **MinIO Bucket (via Crossplane AWS provider).** A private S3-compatible bucket for WAL archiving and backups.
3. **ExternalSecret (via ESO).** A Kubernetes resource that tells ESO to fetch the initial superuser password from OpenBao, rotate it every 30 days, and update the Kubernetes Secret each time.

**Step 5: Crossplane Reconciles.** Crossplane's provider-kubernetes controller applies the CloudNativePG Cluster to the cluster. The CloudNativePG operator reads the Cluster CR and:
- Provisions three PostgreSQL 17 pods with StatefulSet
- Initializes the first instance as the primary, the other two as hot-standby replicas
- Configures streaming replication with synchronous commit (one replica must acknowledge writes before primary commits)
- Sets up Barman continuous archiving to MinIO
- Deploys a PgBouncer sidecar in each pod for connection pooling

This happens in parallel: CloudNativePG pods are starting while the MinIO bucket is being created. Typical time-to-healthy: 1–3 minutes (SLO: < 3 minutes per ADR-067).

**Step 6: ESO Syncs Credentials.** ESO's SecretStore (configured to speak to OpenBao via mTLS) fetches the initial database password from OpenBao's `database/` secrets engine. ESO creates a Kubernetes Secret:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: billing-db-credentials
  namespace: tenant-acme-prod
type: Opaque
data:
  username: c3VwZXJ1c2Vy  # base64: superuser
  password: <random-64-char-string>
  host: YmlsbGluZy1kYi1ydwotaGVhZHk=  # base64: billing-db-rw-headless
  port: NTQzMg==  # 5432
  connection-string: cG9zdGdyZXM6Ly9zdXBlcnVzZXI6PHBhc3N3b3JkPkBiaWxsaW5nLWRiLXJ3LWhlYWR5LnRlbmFudC1hY21lLXByb2Q6NTQzMi9zdWJzdHJhdGU/c3NsbW9kZT12ZXJpZnktZnVsbA==
```

ESO also sets up a watch: every 30 days, ESO wakes up, generates a new password in OpenBao, updates the Secret, and the old password is revoked after a 5-minute grace period (allowing in-flight connections to close).

**Step 7: Application Connects.** A developer deploys an application that reads the secret:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: billing-service
  namespace: tenant-acme-prod
spec:
  template:
    spec:
      containers:
      - name: app
        image: billing-service:v1.2.3
        env:
        - name: DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: billing-db-credentials
              key: connection-string
```

The application pod starts, reads the connection string from the mounted Secret volume, and opens a TCP connection to `billing-db-rw-headless.tenant-acme-prod:5432`. The connection is routed through PgBouncer, which authenticates the user and assigns a connection from the pool. The application is connected within 100 ms.

**The Happy Path Summary.** From YAML submission to application connected: 2–3 minutes, fully automated. No manual database provisioning, no operator running SQL scripts. OPA has validated the policy constraints, Crossplane has provisioned the infrastructure, and ESO has injected credentials. The entire flow is declarative and auditable (every step is a Kubernetes resource in etcd).

**What If Classification Is Missing?**

Now suppose a developer forgets to add the `classification` field:

```yaml
apiVersion: cave.dev/v1alpha1
kind: Database
metadata:
  name: billing-db
  namespace: tenant-acme-prod
spec:
  size: large
  performanceProfile: high
  dataResidency: eu
  # ERROR: classification is missing
```

When they submit this, OPA's ValidatingWebhookConfiguration intercepts the create and checks the OPA policy:

```rego
# Simplified OPA policy
deny[msg] {
    input.request.kind.kind == "Database"
    not input.request.object.spec.classification
    msg := "classification field is required on all Database resources (ADR-102)"
}
```

The webhook returns HTTP 403 Forbidden with the message:

```
Admission webhook "validate-database-classification.cave.dev" denied the request:
classification field is required on all Database resources (ADR-102)
```

The developer sees this error immediately in their terminal and updates the YAML to include `classification: confidential`. This prevents a subtle compliance violation where a confidential database was provisioned without audit logging enabled.

### 10.6.2 PostgreSQL IOPS Saturates → Reflex Auto-Resize

The billing database from section 10.6.1 has been running in production for three weeks. On a Tuesday afternoon, the finance team runs their end-of-month billing report, which queries the `invoices`, `line_items`, and `payments` tables with heavy joins. The database's storage volume begins saturating its IOPS quota.

**Step 1: Prometheus Detects the Problem.** CloudNativePG exports Prometheus metrics for I/O performance. The instance running PgBouncer exports `pgbouncer_stats_io_block_cycles_total`, which increments when a statement stalls waiting for disk I/O. The Hetzner monitoring exporter also scrapes the underlying block device and exports `node_disk_write_rate_bytes`.

Prometheus records:
```
pg_io_wait_ratio{instance="billing-db-0.billing-db.tenant-acme-prod"} = 0.85
```

(85% of samples show the pod waiting on I/O.)

The prometheus alert rule is:

```yaml
alert: PostgreSQLIOPSSaturated
expr: pg_io_wait_ratio > 0.8
for: 5m
annotations:
  summary: "PostgreSQL instance {{ $labels.instance }} I/O utilization at {{ $value | humanizePercentage }}"
  runbook: "https://docs.cave.dev/runbooks/postgresql-iops-saturated"
```

After 5 minutes of sustained high I/O wait, the alert fires.

**Step 2: KEDA ScaledObject Triggers Reflex Engine.** The Reflex Engine (as described in ADR-095) is configured to listen for Prometheus alerts. A ScaledObject in the cluster (managed by KEDA) is watching for the `PostgreSQLIOPSSaturated` alert:

```yaml
apiVersion: keda.sh/v1alpha1
kind: ScaledObject
metadata:
  name: postgresql-iops-reflex
  namespace: reflex-system
spec:
  scaleTargetRef:
    name: reflex-engine
    kind: Deployment
  triggers:
  - type: prometheus
    metadata:
      query: 'ALERTS{alertname="PostgreSQLIOPSSaturated", severity="warning"}'
      threshold: '1'
```

When the alert is active, KEDA increments the Reflex Engine deployment's replica count (or signals it via a message queue). The Reflex Engine wakes up and dequeues the remediation workflow.

**Step 3: Argo Workflow Executes Multi-Step Remediation.** The Reflex Engine spawns an Argo Workflow:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Workflow
metadata:
  name: pg-iops-resize-billing-db
spec:
  entrypoint: main
  templates:
  - name: main
    dag:
      tasks:
      - name: verify-not-transient
        template: check-spike
      - name: on-verified
        template: check-budget
        dependencies: verify-not-transient
      - name: on-budget-ok
        template: resize-profile
        dependencies: on-budget-ok
      - name: on-resize-complete
        template: verify-iops-normalized
        dependencies: on-resize-complete
      - name: on-verified-normalized
        template: attestation
        dependencies: on-verified-normalized

  - name: check-spike
    script:
      image: curlimages/curl
      command: [curl]
      args:
      - -s
      - 'http://prometheus:9090/api/v1/query_range?query=pg_io_wait_ratio%7Binstance%3D%22billing-db-0%22%7D&start=<now-30m>&end=<now>&step=1m'
      source: |
        # Parse the response. If the last 30 minutes show sustained high I/O (not just a 5-min spike),
        # continue. Otherwise, exit with "transient spike detected; no action needed".
        ...

  - name: check-budget
    script:
      image: bitnami/kubectl:latest
      command: [kubectl]
      args:
      - get
      - configmap
      - tenant-budgets
      - -o
      - jsonpath='{.data.acme-corp-iops-remaining}'
      source: |
        # Verify that tenant acme-corp has budget remaining to upgrade from "high" to "extreme".
        # If budget is depleted, log and exit with error "budget exhausted".
        ...

  - name: resize-profile
    script:
      image: bitnami/kubectl:latest
      command: [kubectl]
      source: |
        # Patch the Database XR: change performanceProfile from "high" to "extreme"
        kubectl patch database billing-db \
          -n tenant-acme-prod \
          -p '{"spec":{"performanceProfile":"extreme"}}' \
          --type=merge
        # Wait for the Composition to apply the Cluster patch (10K -> 30K IOPS).
        # CloudNativePG will add a new hot-spare with higher IOPS and rebalance.
        # Typical time: 1-2 minutes.
        kubectl wait --for=condition=Ready pod \
          -l app.kubernetes.io/name=cloudnativepg,pg-cluster=billing-db \
          --timeout=300s
        ...

  - name: verify-iops-normalized
    script:
      image: curlimages/curl
      source: |
        # Query Prometheus again: is pg_io_wait_ratio now < 0.3?
        # If yes, success. If no after 5 minutes, escalate to on-call engineer.
        ...

  - name: attestation
    script:
      image: bitnami/kubectl:latest
      source: |
        # Create a SelfHealed resource (ADR-041):
        # Log the fact that Reflex auto-remediated an IOPS saturation event.
        # Include: timestamp, alert fired at, verification passed at, new performanceProfile,
        # tenant charged for upgrade cost, remediation duration.
        # Send this to the Sovereign Ledger for compliance/audit trail.
        kubectl create -f - <<EOF
        apiVersion: cave.dev/v1alpha1
        kind: SelfHealed
        metadata:
          name: pg-iops-resize-billing-db-$(date +%s)
          namespace: audit
        spec:
          alertName: PostgreSQLIOPSSaturated
          affectedResource:
            kind: Database
            name: billing-db
            namespace: tenant-acme-prod
          remediationType: autoresize
          oldConfig:
            performanceProfile: high
          newConfig:
            performanceProfile: extreme
          firedAt: <timestamp>
          remediatedAt: <timestamp>
          verifiedAt: <timestamp>
          tenantChargedFor: "iops-upgrade-high-to-extreme"
        EOF
```

The workflow runs to completion. If any step fails (e.g., budget exhausted), the workflow pauses and an alert is sent to the on-call database engineer, who manually evaluates the situation.

**Step 4: Tenant Notified.** Once the Reflex workflow completes successfully, a notification is sent to the tenant's ops team:

```
Subject: Auto-Remediation: Database IOPS Upgraded
From: reflex-engine@cave.dev

At 2026-03-06 14:23 UTC, the billing-db database in production experienced
I/O saturation (85% wait time for 5+ minutes).

Action Taken:
- Diagnosed sustained I/O spike (not transient)
- Verified tenant budget remaining
- Upgraded performanceProfile from "high" (10K IOPS) to "extreme" (30K IOPS)
- Verified I/O wait normalized to 12% within 90 seconds

New Cost:
- IOPS charge increased from $180/month to $520/month
- Billed to: acme-corp (account ID: acc-12345)

Further Action:
- Consider optimizing the billing report query (add indexes, partition large tables)
- Or schedule a permanent upgrade if this workload is now baseline
- Review with your database architect

Attestation: https://ledger.cave.dev/events/pg-iops-resize-billing-db-1741350180
```

The tenant can click the attestation link and see a cryptographically signed record of the auto-remediation, which satisfies SOC 2 CC7.3 (incident response) and demonstrates that the platform detected and fixed a problem autonomously.

### 10.6.3 Database Credential Auto-Rotated via ESO

The billing-db database has been running for 60 days. Its initial superuser password was set by OpenBao when the database was first provisioned. ESO's rotation schedule is configured for every 30 days.

**Step 1: Rotation Schedule Triggers.** ESO (ExternalSecrets Operator) has a SecretStore pointing to OpenBao:

```yaml
apiVersion: external-secrets.io/v1beta1
kind: SecretStore
metadata:
  name: openbao-store
  namespace: external-secrets-system
spec:
  provider:
    vault:
      server: "https://openbao.cave.dev:8200"
      path: "secret"
      auth:
        kubernetes:
          mountPath: "kubernetes"
          role: "external-secrets"
      caProvider:
        key: ca.crt
        name: openbao-ca
        type: Secret
```

And an ExternalSecret for the billing-db:

```yaml
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata:
  name: billing-db-credentials
  namespace: tenant-acme-prod
spec:
  refreshInterval: 30d  # Refresh every 30 days
  secretStoreRef:
    name: openbao-store
    kind: SecretStore
  target:
    name: billing-db-credentials
    creationPolicy: Owner
    template:
      engineVersion: v2
      data:
        username: "{{ .username }}"
        password: "{{ .password }}"
        host: "billing-db-rw-headless.tenant-acme-prod"
        port: "5432"
        connection-string: "postgres://{{ .username }}:{{ .password }}@billing-db-rw-headless.tenant-acme-prod:5432/postgres?sslmode=verify-full"
  data:
  - secretKey: username
    remoteRef:
      key: database/static-roles/billing-db/username
  - secretKey: password
    remoteRef:
      key: database/static-roles/billing-db/password
```

At the 30-day mark, ESO's reconciliation loop wakes up. It queries OpenBao's `database/static-roles/billing-db/password` endpoint. OpenBao's database secrets engine (which is configured to manage PostgreSQL credentials) generates a new password and stores it. ESO retrieves it.

**Step 2: ESO Updates the Kubernetes Secret.** ESO updates the `billing-db-credentials` Secret in-place with the new password:

```bash
$ kubectl get secret billing-db-credentials -n tenant-acme-prod -o yaml
apiVersion: v1
kind: Secret
metadata:
  name: billing-db-credentials
  namespace: tenant-acme-prod
data:
  password: <NEW-BASE64-PASSWORD>  # Changed
  connection-string: <NEW-BASE64-CONNECTION-STRING-WITH-NEW-PASSWORD>  # Changed
```

The Secret update is atomic (etcd transaction). All watchers (applications) are notified via a Kubernetes watch event.

**Step 3: Application Reconnects.** The application (billing-service) is configured to watch for Secret changes. One way is to use a library like `external-secrets-reload` or `stakater/Reloader`. When the Secret is updated, Reloader:

1. Detects the Secret change (via label `reloader.stakater.com/match: "true"` on the Secret)
2. Touches a ConfigMap or annotation on the Deployment to trigger a rollout
3. Kubernetes rolls out new pods with the fresh Secret volume mount

Alternatively, if the application is stateless and uses a connection pool, it can be configured to:
- Periodically re-read the Secret from the file system (via inotify or polling)
- Close and re-establish idle connections

The application code does this:

```python
import os
import psycopg2

def get_connection():
    # Read the connection string from the Secret volume mount
    with open('/var/run/secrets/billing-db-credentials/connection-string', 'r') as f:
        conn_string = f.read().strip()
    return psycopg2.connect(conn_string)

def maintain_pool():
    # Every 5 minutes, close idle connections and recreate the pool
    while True:
        time.sleep(300)
        pool.recycle_all()  # Close all connections
        pool = psycopg2.pool.SimpleConnectionPool(...)
```

Within 5 minutes of the Secret update, the application's connection pool is refreshed with the new credentials.

**Step 4: Old Credential Revoked.** OpenBao's database secrets engine is configured with a grace period. After ESO updates the Secret, OpenBao waits 5 minutes, then revokes the old password by running an `ALTER ROLE` command on PostgreSQL:

```sql
ALTER ROLE superuser VALID UNTIL '2026-03-06 15:05:00 UTC';
```

Any connection using the old password will be rejected after this time. In-flight connections are allowed to finish (TCP connection still exists), but new login attempts fail. This ensures that:
- Old credentials are eventually invalidated (defense in-depth)
- Rogue processes using the old password are forced to reconnect and discover they no longer have credentials
- The security posture is continuously improved without full service disruption

### 10.6.4 Crossplane CronOperation Runs Weekly DB Maintenance

The billing-db database needs regular maintenance to stay performant. A CronOperation (part of ADR-119) is configured to run every Sunday at 3:00 AM UTC.

```yaml
apiVersion: cavern.crossplane.io/v1alpha1
kind: CronOperation
metadata:
  name: billing-db-maintenance
  namespace: crossplane-system
spec:
  schedule: "0 3 * * 0"  # Sunday, 3:00 AM UTC
  template:
    spec:
      forProvider:
        query: |
          -- VACUUM and ANALYZE all tables to reclaim space and update statistics
          VACUUM ANALYZE;

          -- Reindex if bloat > 20% (via pg_stat_user_indexes)
          SELECT schemaname, tablename, indexname
          FROM pg_stat_user_indexes
          WHERE idx_blks_hit::float / (idx_blks_hit + idx_blks_read) < 0.8
          LIMIT 10;

          -- For each bloated index, run REINDEX CONCURRENTLY
          -- (This must be done in a second pass to avoid deadlocks)

          -- Update table statistics
          ANALYZE;

          -- Check backup completeness
          SELECT backup_id, backup_status FROM barman_backups
          WHERE backup_status != 'DONE'
          ORDER BY backup_id DESC
          LIMIT 1;

resourceSelector:
  matchLabels:
    database-class: production
    maintenance: enabled
```

**Sunday 3:00 AM UTC Arrives.** The Crossplane CronOperation controller (running in the crossplane-system namespace) detects that the scheduled time has arrived. It creates a Job resource:

```yaml
apiVersion: batch/v1
kind: Job
metadata:
  name: billing-db-maintenance-20260306-030000
  namespace: crossplane-system
spec:
  template:
    spec:
      containers:
      - name: executor
        image: postgres:17-alpine
        command:
        - psql
        - -h
        - billing-db-rw.tenant-acme-prod
        - -U
        - superuser
        - -d
        - postgres
        - -c
        - |
          VACUUM ANALYZE;
          SELECT schemaname, tablename, indexname FROM pg_stat_user_indexes
          WHERE idx_blks_hit::float / (idx_blks_hit + idx_blks_read) < 0.8 LIMIT 10;
          ANALYZE;
          SELECT backup_id, backup_status FROM barman_backups
          WHERE backup_status != 'DONE' ORDER BY backup_id DESC LIMIT 1;
        env:
        - name: PGPASSWORD
          valueFrom:
            secretKeyRef:
              name: billing-db-credentials
              key: password
      restartPolicy: OnFailure
```

The Job runs to completion. The VACUUM ANALYZE takes about 45 seconds (scans all tables, reclaims dead rows, and re-calculates query planner statistics). The reindex step checks for bloat. If found, REINDEX CONCURRENTLY runs (allowing reads during the reindex). If no bloat, this step is skipped.

**Monitoring the Maintenance Window.** The Job's logs are captured:

```
2026-03-06 03:00:15 UTC - VACUUM ANALYZE started
2026-03-06 03:00:58 UTC - VACUUM ANALYZE completed (43 tables, 890 MB reclaimed)
2026-03-06 03:01:02 UTC - Checking for index bloat...
2026-03-06 03:01:05 UTC - No significant bloat detected (all indexes < 80% fill)
2026-03-06 03:01:06 UTC - ANALYZE started
2026-03-06 03:01:11 UTC - ANALYZE completed
2026-03-06 03:01:12 UTC - Backup status: DONE (last backup: 2026-03-05 23:00:00 UTC)
```

**Attestation to Sovereign Ledger.** Once the Job completes, a SelfHealed attestation is created:

```yaml
apiVersion: cave.dev/v1alpha1
kind: SelfHealed
metadata:
  name: billing-db-maintenance-20260306-030000
  namespace: audit
spec:
  operationType: CronOperation
  resourceName: billing-db-maintenance
  affectedResources:
  - kind: Database
    name: billing-db
    namespace: tenant-acme-prod
  operationDetails:
    maintenanceSteps:
    - step: vacuum-analyze
      duration: 43s
      rowsReclaimed: 890MB
      status: success
    - step: reindex-check
      duration: 3s
      indicesChecked: 47
      bloatDetected: false
      status: success
    - step: analyze
      duration: 5s
      tablesAnalyzed: 43
      status: success
    - step: backup-verification
      duration: 1s
      backupStatus: DONE
      lastBackupTime: "2026-03-05T23:00:00Z"
      status: success
  completedAt: "2026-03-06T03:01:12Z"
  attestor: "crossplane-controller-manager"
```

This attestation is immutably logged to the Sovereign Ledger, satisfying the audit trail requirement. Operators can query the Ledger and see a complete history of all maintenance operations performed on the database.

### 10.6.5 Schema Migration with Flyway

A developer needs to add a new column to the `invoices` table to track invoice status (`draft`, `sent`, `paid`, `cancelled`). The migration is managed by Flyway (ADR-119 related; schema versioning is a key Day 1+ operation).

**Developer Workflow.**

1. The developer creates a new migration file in the application's `db/migration/` directory:

```sql
-- db/migration/V006__add_invoice_status.sql

-- Forward migration: Add status column with default value
ALTER TABLE invoices
ADD COLUMN status VARCHAR(20) DEFAULT 'draft' NOT NULL;

-- Create index for status filtering (common in billing reports)
CREATE INDEX idx_invoices_status ON invoices(status);

-- Update existing invoices to 'paid' (they have been settled)
UPDATE invoices
SET status = 'paid'
WHERE payment_received_date IS NOT NULL;

-- Make a checkpoint for rollback (see undo script below)
COMMIT;
```

2. The developer also creates a matching undo/rollback script (required by ADR-115):

```sql
-- db/migration/U006__add_invoice_status.sql

-- Rollback: Remove the column and index
DROP INDEX idx_invoices_status;
ALTER TABLE invoices DROP COLUMN status;
```

3. The developer commits both files to Git:

```bash
git add db/migration/V006__add_invoice_status.sql db/migration/U006__add_invoice_status.sql
git commit -m "feat: add status tracking to invoices table (ADR-115)"
git push origin feature/invoice-status
```

**CI Pipeline Stage 6: Schema Validation.**

When the developer opens a pull request, the CI pipeline (§07 CI/CD Pipeline) is triggered. Stage 6 is dedicated to schema migration validation:

```yaml
# .github/workflows/ci.yaml
stages:
  - stage: "6-schema-validation"
    jobs:
    - job: "FlywyMigrationValidate"
      steps:
      - task: DownloadArtifact@2
        inputs:
          artifactName: "built-app"
      - task: Bash@3
        inputs:
          script: |
            # Start a temporary PostgreSQL container for migration testing
            docker run -d \
              --name postgres-test \
              -e POSTGRES_PASSWORD=testpass \
              postgres:17

            sleep 5

            # Run Flyway forward migration on the test database
            docker run --rm \
              --link postgres-test \
              -e FLYWAY_URL=jdbc:postgresql://postgres-test:5432/test \
              -e FLYWAY_USER=postgres \
              -e FLYWAY_PASSWORD=testpass \
              -e FLYWAY_OUT_OF_ORDER=false \
              -v $(pwd)/db/migration:/flyway/sql \
              flyway/flyway:latest \
              migrate

            if [ $? -ne 0 ]; then
              echo "Forward migration FAILED"
              exit 1
            fi

            # Now test the rollback (undo)
            flyway/flyway undo

            if [ $? -ne 0 ]; then
              echo "Rollback migration FAILED"
              exit 1
            fi

            # Run the forward migration again to ensure idempotence
            flyway/flyway migrate

            if [ $? -ne 0 ]; then
              echo "Re-run of forward migration FAILED (idempotence check)"
              exit 1
            fi

            echo "✓ Forward migration OK"
            echo "✓ Rollback migration OK"
            echo "✓ Idempotence check OK"
            docker stop postgres-test
```

The CI job verifies:
1. Forward migration runs without error
2. Rollback script runs without error
3. Forward migration can be re-run (idempotence)
4. No SQL syntax errors
5. No constraint violations

If any step fails, the PR cannot be merged. The developer must fix the migration script.

**Staging Validation.**

Once the PR is approved and merged to `main`, the application is deployed to the `staging` environment (§07). The staging deployment pipeline includes a pre-deployment step that applies the Flyway migration to the staging database:

```yaml
apiVersion: batch/v1
kind: Job
metadata:
  name: billing-service-flyway-migrate-staging-v1.3.0
spec:
  template:
    spec:
      containers:
      - name: flyway
        image: flyway/flyway:latest
        env:
        - name: FLYWAY_URL
          valueFrom:
            secretKeyRef:
              name: staging-db-credentials
              key: connection-string
        - name: FLYWAY_BASELINE_ON_MIGRATE
          value: "true"  # Allow baselining if this is the first migration
        - name: FLYWAY_OUT_OF_ORDER
          value: "false"  # Enforce strict ordering
        volumeMounts:
        - name: migrations
          mountPath: /flyway/sql
      volumes:
      - name: migrations
        configMap:
          name: billing-service-migrations-v1.3.0
          items:
          - key: V006__add_invoice_status.sql
            path: V006__add_invoice_status.sql
      restartPolicy: OnFailure
```

The Job runs Flyway against the staging database. It applies V006 and any other pending migrations. The application's Pods are held in a `Pending` state until the Job completes successfully.

Once the Job finishes, the application Pods are scheduled and can connect to the updated schema.

**Production Canary Deployment.**

In production, the migration is applied as part of a canary deployment. The production database is live and serving traffic. Flyway is run in a blue-green pattern:

1. A small canary Pod (1/100 replicas) is deployed with the new application version
2. The canary Pod runs Flyway to apply V006 to the production database
3. If Flyway succeeds, traffic is gradually shifted to canary Pods (10% → 50% → 100%)
4. The old application version's Pods (using the old schema) are drained and removed

The key insight: Flyway applies the migration to a shared production database, and the old and new application versions can coexist (if the schema change is backwards-compatible). Once 100% traffic is on the new version, the old version's Pods are removed.

If Flyway fails in production (e.g., constraint violation), the canary deployment is immediately rolled back. The production database is unchanged, and the old application version continues running against the old schema.

### 10.6.6 Multi-Tenant Data Isolation

CAVE enforces multi-tenant data isolation at two layers: Kubernetes namespaces (resource isolation) and PostgreSQL row-level security (data isolation). This section explains how they work together.

**Namespace-Level Isolation.**

Each tenant has a dedicated Kubernetes namespace (e.g., `tenant-acme-prod`, `tenant-customer-b-staging`). Kubernetes RBAC and network policies ensure:
- Only the tenant's Pods can access the tenant's namespace
- Network policy blocks inter-tenant traffic
- Only the tenant's service accounts can access the tenant's secrets

A Kubernetes NetworkPolicy in each tenant namespace:

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: deny-cross-tenant
spec:
  podSelector: {}  # Apply to all Pods in this namespace
  policyTypes:
  - Ingress
  - Egress
  ingress:
  - from:
    - namespaceSelector:
        matchLabels:
          tenant-id: acme  # Only Pods from acme tenant's namespaces
  egress:
  - to:
    - namespaceSelector:
        matchLabels:
          tenant-id: acme
  - to:
    - namespaceSelector:
        matchLabels:
          name: kube-system  # Allow DNS
    ports:
    - protocol: UDP
      port: 53
```

**Database-Level Isolation: Row-Level Security (RLS).**

At the PostgreSQL level, RLS is a per-table policy that limits which rows a user can see. Even if a developer makes a mistake in their SQL query (e.g., `SELECT * FROM invoices` without a WHERE clause), RLS ensures the query only returns rows belonging to the authenticated tenant.

The setup:

1. **Create a tenant-specific role.** For tenant acme-corp:

```sql
CREATE ROLE tenant_acme_corp WITH LOGIN ENCRYPTED PASSWORD '<rotated-by-ESO>';
GRANT CONNECT ON DATABASE postgres TO tenant_acme_corp;
```

2. **Enable RLS on sensitive tables.** For the `invoices` table:

```sql
ALTER TABLE invoices ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation ON invoices
  USING (tenant_id = current_setting('app.tenant_id')::uuid)
  WITH CHECK (tenant_id = current_setting('app.tenant_id')::uuid);
```

The policy says: a row in `invoices` is visible only if its `tenant_id` column matches the `app.tenant_id` setting. The setting is provided by the application at connection time.

3. **Application sets the tenant context.** When the application connects:

```python
import psycopg2

def get_connection():
    conn = psycopg2.connect(
        host="billing-db-rw.tenant-acme-prod",
        user="tenant_acme_corp",
        password=os.getenv("DB_PASSWORD"),
        database="postgres"
    )
    # Set the tenant context
    with conn.cursor() as cur:
        cur.execute("SET app.tenant_id = 'acme-corp-uuid'")
    return conn

# Now any query on invoices is automatically filtered
with get_connection() as conn:
    with conn.cursor() as cur:
        cur.execute("SELECT * FROM invoices")
        rows = cur.fetchall()  # Only returns invoices where tenant_id = acme-corp-uuid
```

**When to Use Separate Databases vs. Shared Database with RLS.**

| Scenario | Recommendation | Rationale |
|---|---|---|
| **Regulatory isolation required** (HIPAA, PCI-DSS) | Separate database per tenant | A database-level breach cannot leak data from other tenants |
| **Tenant wants isolated backups** | Separate database | Each tenant's backups can be stored separately, encrypted with tenant's key |
| **Thousands of small tenants** | Shared database with RLS | Cost-effective; RLS provides logical isolation |
| **Performance: high IOPS/connections** | Separate database | Each database has independent resource limits, no contention |
| **Compliance: data residency (GDPR)** | Could be either; depends on sensitivity | If separate, each database can be in the required region |
| **Tenant wants ability to export all data** | Separate database | Easier to pg_dump and provide to tenant |

For billing data (financial, sensitive), many organizations choose separate databases per tenant. For operational data (logs, telemetry), a shared database with RLS is cost-effective.

### 10.6.7 Backup Verification and PITR Restore Test

Backups are useless if they're never tested. CAVE runs an automated weekly backup verification drill.

**Weekly Restore Test (Saturday 2:00 AM UTC).**

A Crossplane WatchOperation (ADR-119) monitors the Barman backup status. When a new backup is marked `DONE`, a CronOperation is triggered to run the restore test:

```yaml
apiVersion: cavern.crossplane.io/v1alpha1
kind: WatchOperation
metadata:
  name: backup-completion-trigger
spec:
  watch:
    kind: CloudNativePGBackup
    selector:
      status: DONE
  onEvent:
    createCronOperation:
      name: restore-test-{{ .metadata.name }}
      template:
        spec:
          schedule: "0 2 * * 6"  # Saturday 2 AM
```

The CronOperation spawns an Argo Workflow:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Workflow
metadata:
  name: billing-db-restore-test-20260308
spec:
  entrypoint: main
  templates:
  - name: main
    dag:
      tasks:
      - name: fetch-latest-backup
        template: get-backup-metadata
      - name: restore-to-temp
        template: restore
        dependencies: fetch-latest-backup
      - name: verify-integrity
        template: integrity-check
        dependencies: restore-to-temp
      - name: destroy-temp
        template: cleanup
        dependencies: verify-integrity
      - name: record-result
        template: attestation
        dependencies: destroy-temp

  - name: get-backup-metadata
    script:
      image: curlimages/curl
      source: |
        # Query barman backup list
        curl -s http://barman-api:5000/backups/billing-db | jq '.[0]'

  - name: restore
    script:
      image: postgres:17-alpine
      source: |
        # Create a temporary PostgreSQL instance from the latest backup
        # (In practice, this uses Barman's recover command or CloudNativePG's ClusterRecovery CR)
        barman recover billing-db latest /tmp/restored-data
        # Start the temporary instance
        postgres -D /tmp/restored-data &
        sleep 5
        # Wait for it to be ready
        pg_isready -h localhost -U postgres

  - name: integrity-check
    script:
      image: postgres:17-alpine
      source: |
        # Run integrity checks on the restored database
        psql -h localhost -U postgres -d postgres -c "SELECT COUNT(*) FROM invoices" > /tmp/count.txt
        psql -h localhost -U postgres -d postgres -c "REINDEX DATABASE CONCURRENTLY"
        psql -h localhost -U postgres -d postgres -c "ANALYZE"
        # Verify foreign key constraints
        psql -h localhost -U postgres -d postgres -c "SELECT COUNT(*) FROM pg_constraint WHERE contype = 'f'"

  - name: cleanup
    script:
      image: postgres:17-alpine
      source: |
        # Destroy the temporary instance
        pkill -9 postgres
        rm -rf /tmp/restored-data

  - name: attestation
    script:
      image: bitnami/kubectl:latest
      source: |
        # Create SelfHealed attestation
        kubectl create -f - <<EOF
        apiVersion: cave.dev/v1alpha1
        kind: SelfHealed
        metadata:
          name: billing-db-restore-test-20260308
          namespace: audit
        spec:
          operationType: BackupVerification
          backupId: billing-db-20260307-2300
          testType: FullRestore
          restoreTime: "2026-03-08T02:15:00Z"
          verificationDetails:
            recordCount: 1234567
            indexesVerified: 47
            foreignKeysVerified: 23
            integrityChecksPassed: true
          testDurationSeconds: 180
          status: success
        EOF
```

The restore test takes about 3 minutes. If it succeeds, the attestation is logged. If it fails, an alert is immediately sent to the database team (e.g., "Backup integrity check failed for billing-db").

### 10.6.8 Database Migration During Portability Drill

Once a year, CAVE runs a portability drill (ADR-006) to ensure that no vendor lock-in exists. For databases, this means migrating from Hetzner (CloudNativePG) to Azure (PostgreSQL Flexible Server), or vice versa, and verifying that the application can connect to the migrated database without code changes.

**Drill Scenario.**

The billing-db database currently runs on Hetzner. The drill's goal is to export the data, import it to Azure, and verify the application can connect.

**Step 1: Prepare Azure Target.** A temporary Azure PostgreSQL Flexible Server is provisioned with the same schema version and performance profile:

```yaml
apiVersion: cave.dev/v1alpha1
kind: Database
metadata:
  name: billing-db-portability-test
  namespace: tenant-acme-prod
spec:
  size: large
  performanceProfile: high
  classification: confidential
  dataResidency: eu  # Azure EU region
  provider: azure  # Force Azure (normally auto-detected)
  backups:
    enabled: true
    retentionDays: 90
```

Within 3 minutes, the Azure database is ready to receive data.

**Step 2: Export Data from Hetzner.** Using pg_dump (logical backup):

```bash
# On Hetzner database
pg_dump \
  -h billing-db-rw.tenant-acme-prod \
  -U superuser \
  -d postgres \
  --no-owner \
  --no-privileges \
  --format=custom \
  --compress=9 \
  > /tmp/billing-db.dump

# Size: typically 5-50 GB for a production database
# Time: 10-30 minutes
```

Alternatively, using logical replication (faster for large databases):

```sql
-- On Hetzner database
CREATE PUBLICATION billing_db_migration FOR ALL TABLES;
SELECT * FROM pg_publication;
```

Then, on the Azure side:

```sql
-- On Azure database
CREATE SUBSCRIPTION billing_db_migration
  CONNECTION 'host=billing-db-rw.tenant-acme-prod port=5432 user=superuser password=... dbname=postgres'
  PUBLICATION billing_db_migration;

-- Monitor replication progress
SELECT slot_name, restart_lsn, confirmed_flush_lsn FROM pg_replication_slots;
```

**Step 3: Import Data to Azure.** Using pg_restore:

```bash
pg_restore \
  -h billing-db-portability-test.postgres.database.azure.com \
  -U superuser \
  -d postgres \
  --single-transaction \
  /tmp/billing-db.dump

# Time: 20-40 minutes (depending on size and index rebuild)
```

The `--single-transaction` flag ensures that if the restore fails partway, the entire import rolls back (no partial state).

**Step 4: Verify Data Integrity.** A set of SQL integrity checks are run on both the source (Hetzner) and target (Azure):

```sql
-- Check row counts
SELECT tablename, COUNT(*) as rows
FROM pg_tables
JOIN information_schema.tables ON (tablename = table_name)
GROUP BY tablename
ORDER BY tablename;

-- Check checksums on critical tables
SELECT
  tablename,
  COUNT(*) as row_count,
  SUM(LENGTH(CAST(row_to_json(t.*) AS TEXT))) as total_bytes
FROM billing_db.*
GROUP BY tablename;

-- Compare between source and target (via a query that runs on both)
```

The results are compared using a diff tool. If the row counts and checksums match, data integrity is verified.

**Step 5: Verify Application Connectivity.** The application's database URL is temporarily changed to point to the Azure database:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: billing-service-portability-test
spec:
  template:
    spec:
      containers:
      - name: app
        image: billing-service:v1.3.0
        env:
        - name: DATABASE_URL
          value: "postgres://superuser:password@billing-db-portability-test.postgres.database.azure.com:5432/postgres?sslmode=verify-full"
        livenessProbe:
          httpGet:
            path: /health
            port: 8080
          initialDelaySeconds: 10
          periodSeconds: 5
```

The application starts and performs a health check. If the health check passes (which includes a test query to the database), the connectivity is verified.

**Step 6: Reverse Migration (Optional).** To prove that the reverse direction also works, the drill can then migrate back from Azure to Hetzner using the same steps.

**Step 7: Report and Archive.** The drill results are documented:

```yaml
apiVersion: cave.dev/v1alpha1
kind: PortabilityDrill
metadata:
  name: billing-db-2026-03-annual
  namespace: audit
spec:
  databaseName: billing-db
  sourceProvider: hetzner
  sourceRegion: eu-nuremberg
  targetProvider: azure
  targetRegion: eu-west
  drillDate: "2026-03-08"
  drillDuration: "120 minutes"
  steps:
  - step: DataExport
    duration: "25 minutes"
    dataSize: "12 GB"
    status: success
  - step: DataImport
    duration: "35 minutes"
    status: success
  - step: IntegrityVerification
    status: success
    rowCountMatch: true
    checksumMatch: true
  - step: ConnectivityTest
    status: success
    healthCheckPassed: true
    latencyMs: 45
  - step: ReverseImport
    duration: "33 minutes"
    status: success
  conclusion: "No vendor lock-in detected. Database successfully migrated between providers."
  attestedBy: "compliance-engine"
```

This attestation is stored in the Sovereign Ledger and serves as evidence that the platform has been tested for portability.

## 10.7 Configuration Reference

This section provides detailed configurations for provisioning and managing PostgreSQL in CAVE, with annotations explaining the rationale for each setting.

### 10.7.1 Crossplane Database XR

The Database XR (custom resource) is the developer-facing API for provisioning a database. Here's the annotated spec:

```yaml
apiVersion: cave.dev/v1alpha1
kind: Database
metadata:
  name: billing-db
  namespace: tenant-acme-prod
spec:
  # PARAMETERS: High-level, developer-friendly configuration
  parameters:
    # SIZE: Determines compute (CPU, RAM), storage capacity, and connection pool size
    # Values: "micro" (dev), "small", "medium", "large" (production)
    # Mapping is done by Composition Function based on provider
    size: large

    # PERFORMANCE_PROFILE: I/O and throughput tier
    # "standard" (3K IOPS): suitable for read-heavy or low-concurrency workloads
    # "high" (10K IOPS): typical production workload (this value)
    # "extreme" (30K IOPS): high-concurrency, analytics-heavy workloads
    # Cost increases with IOPS tier; Reflex Engine can auto-upgrade based on metrics
    performanceProfile: high

    # CLASSIFICATION: Mandatory label for data sensitivity (ADR-102)
    # "public": no sensitive data, no special encryption/audit required
    # "internal": internal use, encrypted at rest, basic audit logging
    # "confidential": sensitive data (PII, financial), must have encryption + audit + RLS
    # "restricted": highly sensitive (healthcare, state secrets), most stringent controls
    # OPA policy enforces this field is always present (prevents accidental misconfiguration)
    classification: confidential

    # DATA_RESIDENCY: Geographic constraint for compliance (GDPR, NIS2)
    # "eu": must reside in EU data center (Hetzner Nuremberg for Hz; Azure West Europe for Az)
    # "us": must reside in US data center
    # "apac": Asia-Pacific
    # OPA validates that the tenant's data residency preference matches this value
    dataResidency: eu

    # BACKUPS: High-level backup strategy
    enabled: true
    # Retention: how long to keep backups (impacts storage cost, aids compliance)
    retentionDays: 90

  # CREDENTIALS_ROTATION: Enable automatic password rotation via ESO
  # When enabled, ESO polls the secret store (OpenBao or Azure Key Vault) every 30 days
  # and rotates the database user password, revoking the old one after a grace period
  # This satisfies SOC 2 CC6.7 (secure password practices) and NIST 800-53 (credential management)
  credentialsRotation: true

  # (The Composition Function will fill in provider-specific details below based on
  # the tenant's namespace labels and Composition routing logic)
```

**Developer CLI Commands (cave-ctl).**

Developers interact with the Database XR using the cave-ctl CLI:

```bash
# CREATE a new database
cave-ctl xr create db \
  --name billing-db \
  --size large \
  --env prod \
  --classification confidential \
  --namespace tenant-acme-prod

# LIST all databases in a tenant
cave-ctl xr list db --tenant acme

# DESCRIBE a specific database (view full spec, status, events)
cave-ctl xr describe db billing-db --namespace tenant-acme-prod

# EDIT a database (e.g., upgrade performance profile)
cave-ctl xr edit db billing-db --namespace tenant-acme-prod
# Opens $EDITOR with current spec; save to apply changes via Crossplane

# DELETE a database (confirms before proceeding; creates backup automatically)
cave-ctl xr delete db billing-db --namespace tenant-acme-prod
```

### 10.7.2 CloudNativePG Cluster (Hetzner)

When a Database XR is provisioned on Hetzner, the Composition Function creates a CloudNativePG Cluster CR. Here's the annotated manifest:

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: billing-db
  namespace: tenant-acme-prod
spec:
  # INSTANCES: Number of PostgreSQL instances in the cluster
  # 3 = 1 primary + 2 replicas (survives loss of any 1 node without data loss)
  # Replication is synchronous, so writes wait for ≥1 replica to acknowledge
  # Increases HA and durability; cost scales with instance count
  instances: 3

  # IMAGENAME: Which PostgreSQL version to run
  # Must match Hetzner's supported versions (currently 14, 15, 16, 17)
  # Security patches are applied during maintenance windows
  imageName: ghcr.io/cloudnative-pg/postgresql:17.1-1

  # POSTGRESQL: PostgreSQL server parameters (tuned for this workload)
  postgresql:
    parameters:
      # SHARED_BUFFERS: Fraction of RAM dedicated to PostgreSQL buffer pool
      # Best practice: 25% of available RAM (for 8GB, use 2GB)
      # Larger buffer = fewer disk I/O, fewer page faults; must leave RAM for OS/Kubernetes overhead
      shared_buffers: "2GB"

      # WORK_MEM: Memory per query executor (hash joins, sorting, aggregates)
      # Per connection: (RAM - shared_buffers) / max_connections
      # Example: (8GB - 2GB) / 300 = ~20MB per connection
      # Too low = spill to disk (slower); too high = OOM killer
      work_mem: "20MB"

      # EFFECTIVE_CACHE_SIZE: Hint to query planner about total available cache
      # Usually: shared_buffers + OS page cache
      # Larger = planner chooses index scans over sequential scans
      effective_cache_size: "6GB"

      # MAX_CONNECTIONS: Maximum concurrent connections to the database
      # CloudNativePG + PgBouncer front-end the connection pool, so this can be moderate
      # Too low = connection refused errors; too high = memory overhead
      max_connections: 300

      # CHECKPOINT settings: Control how often dirty pages are flushed to disk
      # More frequent checkpoints = slower writes, faster recovery from crash
      # Less frequent = faster writes, longer recovery time
      checkpoint_completion_target: "0.9"  # Spread checkpoint I/O over 90% of interval
      checkpoint_timeout: "15min"

      # WAL settings: Write-Ahead Log (recovery and replication)
      # WAL records all changes before they're applied to data pages
      # Required for ACID durability and streaming replication
      wal_level: "replica"  # Enable replication (not just "minimal")
      wal_keep_size: "1GB"  # Keep 1GB of old WAL for standby catchup

      # AUTOVACUUM settings: Background cleanup of deleted rows
      # Deleted rows are marked "dead" but space is not reclaimed until VACUUM runs
      # Autovacuum runs periodically to reclaim space and update query stats
      autovacuum: "on"
      autovacuum_max_workers: "3"
      autovacuum_naptime: "30s"

      # LOGGING: Query performance and slow queries
      # Essential for performance debugging and compliance audits
      log_statement: "mod"  # Log non-SELECT statements (INSERT, UPDATE, DELETE)
      log_duration: "on"
      log_min_duration_statement: "1000"  # Log queries taking >1 second
      log_checkpoints: "on"
      log_connections: "on"
      log_disconnections: "on"
      log_lock_waits: "on"

  # POSTGRESQL.PG_HBA: Host-based access control (authentication)
  postgresql:
    pg_hba:
    # Allow local connections from Kubernetes pods in the same cluster
    - type: host
      database: all
      user: all
      address: "10.0.0.0/8"  # Pod CIDR range
      auth_method: scram-sha-256  # Require password with SCRAM-SHA-256 hashing
    # Deny all other connections
    - type: host
      database: all
      user: all
      address: "0.0.0.0/0"
      auth_method: reject

  # BOOTSTRAP: Initialization when cluster is first created
  bootstrap:
    initdb:
      database: postgres
      owner: superuser
      encoding: UTF8
      locale: C.UTF-8  # UTF-8 locale for international text support
      collation: C.UTF-8
      # Custom initialization SQL to run after initdb
      postInitTemplateSQL:
      - CREATE SCHEMA IF NOT EXISTS public;
      - CREATE EXTENSION IF NOT EXISTS pg_stat_statements;  # Track query performance
      - CREATE EXTENSION IF NOT EXISTS pgaudit;  # Audit logging (confidential data)

  # MONITORING: Prometheus metrics and alerting
  # Enables PodMonitor for scraping metrics and allows pg_stat_statements queries
  monitoring:
    enabled: true
    podMonitorName: billing-db-monitor

  # STORAGE: Where to store the database files
  # PVC (PersistentVolumeClaim) provides durable block storage
  primaryUpdateStrategy: unsupervised  # Let CloudNativePG manage failover
  postgresql:
    # ... (parameters above)

  storage:
    size: 256Gi  # Allocate 256 GB of persistent storage
    # storageClassName: fast-ssd  # Use SSD storage class (high IOPS, low latency)
    # This is critical for the "high" performance profile

  # MONITORING_SETUP: Configure pod monitoring
  monitoring:
    enabled: true

  # BACKUP: Automated continuous backup to MinIO (S3-compatible object storage)
  # Barman (archiver) is used to continuously archive WAL and full backups
  # This enables Point-In-Time Recovery (PITR) with fine granularity
  backup:
    barmanObjectStore:
      # S3 endpoint: MinIO running in the cluster (or external S3)
      destinationPath: "s3://hetzner-backups/billing-db"
      endpointURL: "https://minio.cave.dev:9000"
      s3Credentials:
        accessKeyId:
          name: minio-credentials
          key: access-key
        secretAccessKey:
          name: minio-credentials
          key: secret-key
      wal:
        # WAL archiving: every completed WAL segment is uploaded to S3
        # WAL segment size is 16 MB; new segment every few seconds (during active load)
        # Enables PITR to any point within the last N days
        compression: gzip  # Compress WAL for storage efficiency
        maxParallel: 4     # Upload up to 4 WAL files in parallel
      # Data backups: full snapshots taken periodically
      data:
        compression: gzip
        immediateCheckpoint: false  # Don't force checkpoint before backup (slower on prod)
      # Retention policy: how long to keep backups
      retentionPolicy: "RECOVERY WINDOW OF 7 days"  # Keep backups for 7 days (PITR available for 7 days)
      # Barman reads this policy and purges old backups automatically
      googleCredentials: null  # Not using Google Cloud; could use instead of S3

  # RESOURCES: CPU and memory requests/limits for the Postgres pod
  # Requests: minimum guaranteed; Limits: maximum allowed (OOMKilled if exceeded)
  # For a "large" instance: 4 CPU, 8 GB RAM
  resources:
    requests:
      cpu: "4"
      memory: "8Gi"
    limits:
      cpu: "4"
      memory: "8Gi"

  # AFFINITY: Pod scheduling rules
  affinity:
    podAntiAffinity:
      preferredDuringSchedulingIgnoredDuringExecution:
      # Try to spread replicas across different nodes for HA
      # (Not required; preferred for cost/latency optimization)
      - weight: 100
        preference:
          matchExpressions:
          - key: kubernetes.io/hostname
            operator: NotIn
            values: []  # Will be filled with node names
```

### 10.7.3 Azure PostgreSQL Flexible Server (Azure)

When a Database XR is provisioned on Azure, the Composition Function creates an Azure PostgreSQL Flexible Server. Here's the Crossplane Managed Resource (or equivalent):

```yaml
apiVersion: dbforpostgresql.azure.upbound.io/v1beta1
kind: Server
metadata:
  name: billing-db
  namespace: tenant-acme-prod
spec:
  # PROVIDER_CONFIG: Azure subscription and authentication
  providerConfigRef:
    name: azure-default

  forProvider:
    # RESOURCE_GROUP: Azure resource group (billing/cost allocation)
    # Typically one resource group per tenant for RBAC and cost tracking
    resourceGroupName: "rg-tenant-acme"

    # LOCATION: Azure region (must match dataResidency constraint)
    # "eu-west" for EU, "eastus" for US, etc.
    location: "westeurope"  # EU-West (Ireland) - complies with dataResidency: eu

    # STORAGE: Database storage capacity and performance
    storageSize: 256  # 256 GB (matches CloudNativePG)

    # SKU: Compute tier and performance class
    # Mapping from Database XR sizes to Azure SKUs:
    # - "dev" / "micro"    → Burstable_B1s (1 vCore, 1 GB RAM) - low cost, burstable
    # - "small"            → GeneralPurpose_D2s_v3 (2 vCore, 8 GB RAM)
    # - "medium"           → GeneralPurpose_D4s_v3 (4 vCore, 16 GB RAM)
    # - "large"            → GeneralPurpose_D8s_v3 (8 vCore, 32 GB RAM) - production
    #
    # "Burstable" SKUs: burst briefly above baseline; cost-effective for variable load
    # "GeneralPurpose" SKUs: full performance guaranteed; suitable for production
    # "Memory Optimized": rare; for very heavy in-memory workloads

    sku: "GeneralPurpose_D8s_v3"  # "large" database = D8s_v3

    # VERSION: PostgreSQL version
    version: "17"  # PostgreSQL 17 (matches Hetzner)

    # HIGH_AVAILABILITY: Azure-managed HA with zone redundancy
    # Zone-redundant HA = primary and standby in different Azure AZs
    # Azure handles failover automatically within seconds
    # Increases cost but provides automatic failover (no manual action needed)
    highAvailability:
      mode: "ZoneRedundant"  # "SameZone" for dev/staging (lower cost)

    # BACKUP: Azure-managed automated backups
    # Azure automatically takes full backups daily and transaction log backups every 5 minutes
    # Enables PITR (point-in-time recovery) within the retention window
    backup:
      backupRetentionDays: 90  # Keep backups for 90 days (matches Hetzner config)
      geoRedundantBackupEnabled: true  # Replicate backups to another region (disaster recovery)

    # AUTHENTICATION: User authentication method
    # "None" = integrated Azure AD authentication; "All" = Azure AD + local users
    authenticationConfig:
      activeDirectoryAuthEnabled: true  # Use Azure AD for user management
      passwordAuthEnabled: true  # Also allow local (non-AD) users if needed

    # NETWORK: Connectivity and access control
    # Private Endpoint: restrict access to Azure VNet only (not open to internet)
    # Improves security but requires VNet-to-VNet peering or ExpressRoute for remote access
    network:
      delegatedSubnetResourceId: "/subscriptions/sub-id/resourceGroups/rg-tenant-acme/providers/Microsoft.Network/virtualNetworks/vnet-acme/subnets/db-subnet"
      privateDnsZoneArmResourceId: "/subscriptions/sub-id/resourceGroups/rg-tenant-acme/providers/Microsoft.Network/privateDnsZones/database.postgres.database.azure.com"

    # DATABASE_PARAMETERS: Configuration parameters (similar to CloudNativePG)
    # Note: Some parameters in Azure managed version have restricted ranges
    # (e.g., max_connections is limited based on SKU; can't be arbitrarily large)
    parameters:
      max_connections: 300  # D8s_v3 SKU allows up to 400; we set 300 for safety
      shared_buffers: "16GB"  # 25% of 32 GB RAM for D8s_v3
      effective_cache_size: "24GB"
      work_mem: "80MB"  # (32GB - 16GB) / 300 = ~53 MB; round to 80 MB with overhead
      maintenance_work_mem: "1GB"
      random_page_cost: "1.25"  # Azure storage is faster than traditional SSD; reduce cost
      jit: "on"  # Just-in-time compilation for complex queries (PostgreSQL 11+)
      log_min_duration_statement: "1000"
      log_statement: "mod"
      log_duration: "on"

  # CROSS_TENANT_AUTH: Use pod identity (Managed Identity) for cross-tenant Azure access
  # (Alternative to static credentials; more secure)
  # Example: Pod uses its Managed Identity to authenticate to Azure,
  # which then authenticates to the PostgreSQL Flexible Server
  # via Azure Entra ID (formerly Azure AD).
  # This is complex; static credentials (managed by ESO) are simpler for single-tenant databases.
```

### 10.7.4 ESO Credential Rotation

External Secrets Operator (ESO) manages database credentials and rotates them on a schedule. Here's the configuration:

```yaml
# SecretStore: Tells ESO where and how to fetch secrets from the backing secret manager
apiVersion: external-secrets.io/v1beta1
kind: SecretStore
metadata:
  name: openbao-store
  namespace: external-secrets-system
spec:
  provider:
    vault:  # OpenBao is Vault-compatible
      # Server: OpenBao instance running in the cluster (or external)
      server: "https://openbao.cave.dev:8200"
      path: "secret"

      # Authentication: How ESO proves its identity to OpenBao
      auth:
        kubernetes:
          mountPath: "kubernetes"  # Kubernetes auth method mount point in OpenBao
          role: "external-secrets"  # Role that allows reading database/* secrets

      # TLS: Verify OpenBao's certificate (prevent MITM attacks)
      caProvider:
        key: ca.crt
        name: openbao-ca  # ConfigMap holding OpenBao's CA certificate
        type: ConfigMap

---

# ExternalSecret: Tells ESO which secrets to fetch and how to store them in Kubernetes
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata:
  name: billing-db-credentials
  namespace: tenant-acme-prod
spec:
  # REFRESH_INTERVAL: How often ESO checks for updates (rotation trigger)
  # 30d = once per month; ESO fetches the latest secret version from OpenBao
  # If OpenBao has rotated the password, ESO syncs the new one to Kubernetes
  refreshInterval: 30d

  # SECRET_STORE_REF: Which SecretStore to fetch from
  secretStoreRef:
    name: openbao-store
    kind: SecretStore

  # TARGET: Where to store the synced secret in Kubernetes
  target:
    name: billing-db-credentials  # Name of the K8s Secret
    creationPolicy: Owner  # ESO owns this secret; if ESO is deleted, secret is deleted
    template:
      # TEMPLATE: Build a connection string from secret components
      # Allows storing username and password separately in OpenBao,
      # but combining them in the K8s Secret for the application to use
      engineVersion: v2
      data:
        username: "{{ .username }}"
        password: "{{ .password }}"
        host: "billing-db-rw-headless.tenant-acme-prod"  # Static (from Kubernetes)
        port: "5432"
        connection-string: |
          postgres://{{ .username }}:{{ .password }}@billing-db-rw-headless.tenant-acme-prod:5432/postgres?sslmode=verify-full

  # DATA: Fetch these secrets from OpenBao
  data:
  - secretKey: username
    remoteRef:
      key: database/static-roles/billing-db/username  # OpenBao path
  - secretKey: password
    remoteRef:
      key: database/static-roles/billing-db/password  # OpenBao path (rotated here)

---

# OpenBao Configuration (runs in the cluster; external setup required)
# This is a simplified example; real setup is more complex
apiVersion: v1
kind: ConfigMap
metadata:
  name: openbao-config
  namespace: openbao-system
data:
  openbao.hcl: |
    # Database secrets engine: manages PostgreSQL user credentials
    path "database/*" {
      capabilities = ["read", "update"]
    }

    # Policy for ESO: allows reading database secrets
    path "database/static-roles/*/username" {
      capabilities = ["read"]
    }
    path "database/static-roles/*/password" {
      capabilities = ["read"]
    }

---

# OpenBao PostgreSQL Configuration (command to run once)
# vault write database/config/billing-db \
#   plugin_name=postgresql-database-plugin \
#   allowed_roles="superuser" \
#   connection_url="postgresql://superuser:password@billing-db-rw:5432/postgres" \
#   username="superuser" \
#   password="initial-password"
#
# vault write database/static-roles/billing-db \
#   db_name="billing-db" \
#   username="superuser" \
#   rotation_statements="ALTER USER superuser WITH PASSWORD '{{ password }}';" \
#   rotation_period="30d"
```

### 10.7.5 Flyway Integration

Flyway manages schema versioning and migrations. Here's how it integrates with the CI pipeline:

```yaml
# CI Pipeline Stage 6: Schema Validation (Azure DevOps example)
trigger:
  branches:
    include:
    - main
    - feature/*

pool:
  vmImage: 'ubuntu-latest'

variables:
  DOCKER_REGISTRY: 'docker.io'
  FLYWAY_VERSION: '10.1.0'
  TEST_DB_PASSWORD: 'test-password-12345'

stages:
- stage: Build
  jobs:
  - job: CompileAndTest
    # ... earlier build stages ...

- stage: SchemaValidation
  dependsOn: Build
  jobs:
  - job: FlywaySchemaMigration
    steps:
    - checkout: self
      fetchDepth: 0  # Full history for migration verification

    - task: Docker@2
      displayName: Start PostgreSQL Test Container
      inputs:
        command: build
        containerRegistry: $(DOCKER_REGISTRY)
        repository: postgres
        tags: |
          17-test
          latest

    - script: |
        # Start a temporary PostgreSQL 17 container for migration testing
        docker run -d \
          --name postgres-test \
          --env POSTGRES_DB=test_db \
          --env POSTGRES_PASSWORD=$(TEST_DB_PASSWORD) \
          --publish 5432:5432 \
          postgres:17-alpine

        # Wait for PostgreSQL to be ready
        for i in {1..30}; do
          echo "Waiting for PostgreSQL... (attempt $i/30)"
          pg_isready -h localhost -U postgres && break
          sleep 2
        done
      displayName: Start PostgreSQL Container

    - script: |
        # Download Flyway if not cached
        mkdir -p /opt/flyway
        cd /opt/flyway

        if [ ! -f flyway ]; then
          curl -L https://repo1.maven.org/maven2/org/flywaydb/flyway-commandline/$(FLYWAY_VERSION)/flyway-commandline-$(FLYWAY_VERSION)-linux-x64.tar.gz \
            | tar xz
          ln -s flyway-$(FLYWAY_VERSION)/flyway flyway
        fi

        export PATH=/opt/flyway:$PATH
      displayName: Install Flyway

    - script: |
        export FLYWAY_URL="jdbc:postgresql://localhost:5432/test_db"
        export FLYWAY_USER="postgres"
        export FLYWAY_PASSWORD="$(TEST_DB_PASSWORD)"
        export FLYWAY_LOCATIONS="filesystem:$(Build.SourcesDirectory)/db/migration"
        export FLYWAY_OUT_OF_ORDER="false"

        # FORWARD MIGRATION: Validate all pending migrations
        echo "=== Phase 1: Forward Migration ==="
        /opt/flyway/flyway migrate

        migration_result=$?
        if [ $migration_result -ne 0 ]; then
          echo "❌ Forward migration FAILED with exit code $migration_result"
          exit 1
        fi
        echo "✓ Forward migration succeeded"
      displayName: Run Forward Migration

    - script: |
        export FLYWAY_URL="jdbc:postgresql://localhost:5432/test_db"
        export FLYWAY_USER="postgres"
        export FLYWAY_PASSWORD="$(TEST_DB_PASSWORD)"
        export FLYWAY_LOCATIONS="filesystem:$(Build.SourcesDirectory)/db/migration"
        export FLYWAY_OUT_OF_ORDER="false"

        # UNDO MIGRATION: Validate all rollback scripts
        echo "=== Phase 2: Undo (Rollback) Migration ==="
        /opt/flyway/flyway undo

        undo_result=$?
        if [ $undo_result -ne 0 ]; then
          echo "❌ Undo migration FAILED with exit code $undo_result"
          echo "This means the rollback script is missing or invalid."
          echo "Every V*.sql migration MUST have a corresponding U*.sql undo script (ADR-115)."
          exit 1
        fi
        echo "✓ Undo migration succeeded"
      displayName: Run Undo (Rollback) Migration
      condition: succeeded()

    - script: |
        export FLYWAY_URL="jdbc:postgresql://localhost:5432/test_db"
        export FLYWAY_USER="postgres"
        export FLYWAY_PASSWORD="$(TEST_DB_PASSWORD)"
        export FLYWAY_LOCATIONS="filesystem:$(Build.SourcesDirectory)/db/migration"
        export FLYWAY_OUT_OF_ORDER="false"

        # IDEMPOTENCE CHECK: Verify forward migration can run again (without error)
        echo "=== Phase 3: Idempotence Check (re-run forward migration) ==="
        /opt/flyway/flyway migrate

        idempotence_result=$?
        if [ $idempotence_result -ne 0 ]; then
          echo "❌ Re-run of forward migration FAILED with exit code $idempotence_result"
          echo "This indicates the migration script has side effects or is not idempotent."
          echo "Migrations must be safe to run multiple times (if interrupted, etc.)."
          exit 1
        fi
        echo "✓ Idempotence check passed"
      displayName: Idempotence Check
      condition: succeeded()

    - script: |
        # Verify migration schema is valid (basic sanity check)
        export PGPASSWORD="$(TEST_DB_PASSWORD)"
        psql -h localhost -U postgres -d test_db -c "
          SELECT table_name FROM information_schema.tables
          WHERE table_schema = 'public'
          ORDER BY table_name;
        " > /tmp/schema_after.txt

        # Count tables (should be non-zero)
        table_count=$(wc -l < /tmp/schema_after.txt)
        echo "Schema contains $table_count tables after migration"

        if [ $table_count -lt 5 ]; then
          echo "⚠️  Warning: Very few tables in schema. Verify migration is complete."
        fi
      displayName: Verify Schema Post-Migration
      condition: succeeded()

    - script: |
        # Generate migration report for audit
        export FLYWAY_URL="jdbc:postgresql://localhost:5432/test_db"
        export FLYWAY_USER="postgres"
        export FLYWAY_PASSWORD="$(TEST_DB_PASSWORD)"
        export FLYWAY_LOCATIONS="filesystem:$(Build.SourcesDirectory)/db/migration"

        /opt/flyway/flyway info > $(Build.ArtifactStagingDirectory)/flyway_info.txt
      displayName: Generate Flyway Info Report
      condition: succeeded()

    - task: PublishBuildArtifacts@1
      displayName: Publish Migration Report
      inputs:
        pathToPublish: '$(Build.ArtifactStagingDirectory)'
        artifactName: schema-validation-report
      condition: succeeded()

    - script: |
        # Cleanup
        docker stop postgres-test || true
        docker rm postgres-test || true
      displayName: Cleanup Test Container
      condition: always()

- stage: DeployStaging
  dependsOn: SchemaValidation
  condition: succeeded()
  jobs:
  - deployment: DeployToStaging
    environment: staging
    strategy:
      runOnce:
        preDeploy:
          steps:
          - script: echo "Pre-deploy checks for staging..."
        deploy:
          steps:
          # Deploy includes a Kubernetes Job that runs Flyway on the staging database
          - task: KubernetesManifest@0
            inputs:
              action: deploy
              kubernetesServiceConnection: staging-cluster
              manifests: |
                k8s/flyway-job-staging.yaml

          # Wait for Flyway Job to complete
          - script: |
              kubectl wait --for=condition=complete job/billing-service-flyway-migrate-staging \
                --timeout=300s -n staging
```

Migration files are stored in the application repository:

```
db/migration/
├── V001__initial_schema.sql
├── U001__initial_schema.sql
├── V002__add_customer_table.sql
├── U002__add_customer_table.sql
├── V003__create_indexes.sql
├── U003__create_indexes.sql
├── V004__add_audit_logging.sql
├── U004__add_audit_logging.sql
├── V005__add_payment_table.sql
├── U005__add_payment_table.sql
├── V006__add_invoice_status.sql
└── U006__add_invoice_status.sql
```

**Migration Convention.**

- **V-prefix**: Versioned forward migration (V001, V002, …, V999)
  - Naming: `V{version}__{description}.sql`
  - Applied in order; cannot skip versions
  - Example: `V006__add_invoice_status.sql`
- **U-prefix**: Undo/rollback script (matching version)
  - Naming: `U{version}__{description}.sql`
  - Applied when rolling back; must be present for every V-script (ADR-115)
  - Example: `U006__add_invoice_status.sql`

Flyway tracks applied migrations in a `flyway_schema_history` table in the database, preventing re-execution of the same migration.

## 10.8 Operations

### 10.8.1 Day-2 Operations

Once a database is provisioned, it must be operated, maintained, and optimized. This section covers common operational tasks.

**Scaling: Resizing via XR Spec Change.**

When the billing-db needs more compute or storage (e.g., to handle increased transaction volume), the developer or operator modifies the Database XR:

```bash
# Edit the XR via YAML
kubectl patch database billing-db \
  -n tenant-acme-prod \
  -p '{"spec":{"size":"extreme"}}' \
  --type=merge
```

Or using cave-ctl:

```bash
cave-ctl xr edit db billing-db --namespace tenant-acme-prod
# Opens editor; change size from "large" to "extreme", save
```

Crossplane detects the change and triggers the Composition Function:
1. For Hetzner: CloudNativePG adds a new Pod with higher resource limits and rebalances the replica set
2. For Azure: Triggers a SKU change (e.g., D8s_v3 → D16s_v3); Azure handles the resize (may have brief downtime)

Typical time to complete: 2–5 minutes for Hetzner (rolling restart), 5–15 minutes for Azure (SKU change).

**Major Version Upgrades.**

PostgreSQL releases major versions (14, 15, 16, 17) approximately annually. Upgrading is a binary-incompatible change that requires special handling.

*Hetzner (CloudNativePG):* Rolling upgrade

CloudNativePG manages the upgrade automatically with zero downtime:
1. Update the Cluster CR's `imageName: postgresql:18-1`
2. CloudNativePG controller detects the change
3. Stops each replica, restarts with the new binary, verifies it can connect to the primary (still on old version)
4. Stops and upgrades the primary
5. Entire process takes 5–10 minutes; no write downtime (replicas fall slightly behind, then catch up)

*Azure:* Managed upgrade (may have brief downtime)

Azure's managed upgrade process varies by HA configuration:
- Zone-redundant HA: Failover to standby, upgrade primary, failover back (0–5 min downtime)
- Same-zone HA: Coordinated upgrade (brief connection interruptions possible)
- Single-node: Single downtime window for upgrade (15–30 min)

Upgrades are scheduled during the maintenance window (e.g., Sunday 3–4 AM UTC) and cannot be deferred indefinitely (Azure enforces them for security patches).

**Connection Pool Tuning: PgBouncer Settings.**

CloudNativePG deploys PgBouncer as a sidecar for connection pooling. If applications experience connection refused errors, adjust the pool:

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: billing-db
spec:
  # ... other settings ...

  monitoring:
    pgbouncerMonitoring:
      enabled: true
      poolMode: transaction  # transaction-level pooling (vs. session-level)
      poolSize: 50           # Base pool size per backend
      reservePoolSize: 10    # Reserve pool for admin connections
      reservePoolTimeoutCheck: 3  # Check reserve pool every 3 seconds
```

- **pool_mode: transaction**: Each application connection borrows a database connection for a single transaction, then returns it. Multiplexes many app connections onto fewer DB connections. Good for OLTP (many short transactions).
- **pool_mode: session**: Each app connection gets a dedicated DB connection for the session lifetime. One-to-one mapping; more DB connections needed, but compatible with all application patterns.

**Performance Monitoring: pg_stat_statements, EXPLAIN ANALYZE.**

Enable pg_stat_statements extension to track query performance:

```sql
-- Enable the extension (one-time)
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

-- View top 10 slow queries
SELECT
  query,
  calls,
  mean_exec_time as avg_ms,
  max_exec_time as max_ms,
  total_exec_time as total_ms
FROM pg_stat_statements
ORDER BY mean_exec_time DESC
LIMIT 10;

-- Reset statistics (useful before testing)
SELECT pg_stat_statements_reset();
```

For a specific slow query, use EXPLAIN ANALYZE to understand the query plan:

```sql
EXPLAIN (ANALYZE, BUFFERS, TIMING)
SELECT invoices.id, COUNT(line_items.id) as line_count
FROM invoices
LEFT JOIN line_items ON invoices.id = line_items.invoice_id
WHERE invoices.status = 'unpaid'
GROUP BY invoices.id;
```

The output shows:
- Estimated vs. actual row counts (if far off, statistics are stale; run ANALYZE)
- Buffer hits vs. disk reads (high disk reads → missing index)
- Execution time per node

**Storage Growth Monitoring and Auto-Expansion.**

Monitor storage usage:

```sql
-- Database size
SELECT
  datname,
  pg_size_pretty(pg_database_size(datname)) as size
FROM pg_database
ORDER BY pg_database_size DESC;

-- Table size (including indexes)
SELECT
  schemaname,
  tablename,
  pg_size_pretty(pg_total_relation_size(schemaname||'.'||tablename)) as size
FROM pg_tables
ORDER BY pg_total_relation_size DESC
LIMIT 10;
```

Configure storage auto-expansion in Crossplane:

```yaml
apiVersion: cave.dev/v1alpha1
kind: Database
metadata:
  name: billing-db
spec:
  storage:
    size: 256Gi
    # Azure-specific: auto-expand when usage reaches 90%
    autoGrowSize: 50Gi  # Add 50 GB each time
    autoGrowMaxSize: 1024Gi  # Don't auto-expand beyond 1 TB (manual approval required)
```

Hetzner (CloudNativePG) supports PVC auto-expansion via the storage provider if the underlying Hetzner volume supports it. Azure's managed storage auto-expands to the max allowed by the SKU.

### 10.8.2 Crossplane Operations (ADR-119)

Crossplane provides CronOperation and WatchOperation for automating recurring and event-driven tasks.

**CronOperation Examples.**

Weekly VACUUM (reclaim dead row space and update statistics):

```yaml
apiVersion: cavern.crossplane.io/v1alpha1
kind: CronOperation
metadata:
  name: weekly-vacuum-all-dbs
spec:
  schedule: "0 3 * * 0"  # Sunday 3 AM UTC
  template:
    spec:
      forProvider:
        query: |
          -- Run on all databases
          DO $$
          DECLARE
            db RECORD;
          BEGIN
            FOR db IN SELECT datname FROM pg_database WHERE datistemplate = false LOOP
              EXECUTE 'VACUUM ANALYZE ' || quote_ident(db.datname);
            END LOOP;
          END $$;
  resourceSelector:
    matchLabels:
      database-tier: production
```

Daily backup integrity check:

```yaml
apiVersion: cavern.crossplane.io/v1alpha1
kind: CronOperation
metadata:
  name: daily-backup-verify
spec:
  schedule: "0 4 * * *"  # 4 AM UTC daily
  template:
    spec:
      forProvider:
        verifyBackup: true
        backupType: "barman"  # Barman backups on Hetzner
  resourceSelector:
    matchLabels:
      backup-strategy: barman
```

Monthly certificate expiry check:

```yaml
apiVersion: cavern.crossplane.io/v1alpha1
kind: CronOperation
metadata:
  name: monthly-cert-expiry-check
spec:
  schedule: "0 9 1 * *"  # 1st of month, 9 AM UTC
  template:
    spec:
      forProvider:
        checkSSLCertExpiry: true
        alertDaysBeforeExpiry: 30
  resourceSelector:
    matchLabels:
      database-tier: production
```

**WatchOperation Examples.**

Trigger REINDEX when index bloat detected:

```yaml
apiVersion: cavern.crossplane.io/v1alpha1
kind: WatchOperation
metadata:
  name: trigger-reindex-on-bloat
spec:
  watch:
    kind: CloudNativePGCluster
    selector:
      matchExpressions:
      - key: status.indexBloatPercent
        operator: GreaterThan
        values: ["20"]  # >20% bloat
  onEvent:
    template:
      spec:
        forProvider:
          reindexBloatedIndices: true
```

**Relationship to Reflex Engine.**

- **Crossplane Operations (CronOperation, WatchOperation)**: Simple, single-step remediation
  - Example: Run VACUUM, check backup status, trigger REINDEX
  - Appropriate for deterministic, side-effect-free operations
- **Reflex Engine**: Complex, multi-step remediation with conditional logic
  - Example: IOPS saturation → verify not transient → check budget → resize → verify normalized
  - Appropriate for operations that require decision-making, error recovery, human escalation

For simple DB maintenance, Crossplane Operations are sufficient. For complex scenarios (like the IOPS auto-resize in 10.6.2), Reflex Engine is required.

### 10.8.3 Backup & Restore

**Hetzner: Barman Continuous Backup to MinIO.**

Barman (Backup and Recovery Manager) is deployed alongside the CloudNativePG cluster. It performs:
1. Continuous WAL archiving: Every completed WAL segment (16 MB) is uploaded to MinIO
2. Full backups: Daily or weekly full snapshots (pg_basebackup format)
3. WAL retention: Keeps WAL segments according to the retention policy

Point-in-Time Recovery (PITR) procedure:

```bash
# 1. Identify the target recovery time
# (You want to recover to 2026-03-05 14:30:00 UTC)

# 2. Trigger recovery from the Kubernetes barman pod
kubectl exec -it billing-db-barman-0 -c barman -- \
  barman recover billing-db latest /tmp/recovery --remote-ssh-command "ssh backup@billing-db-0"

# 3. Once recovered, barman creates a recovery.conf (PostgreSQL config)
# 4. Start the recovered instance and verify data

# For Azure: Use the Azure CLI to restore from automated backups
az postgres flexible-server restore \
  --resource-group rg-tenant-acme \
  --name billing-db-restored \
  --source-server billing-db \
  --restore-point-in-time "2026-03-05T14:30:00+00:00"
```

**RPO/RTO Per Profile.**

| Profile | RPO | RTO | Rationale |
|---|---|---|---|
| **dev** | 24 hours | 4 hours | Acceptable for development; backups less frequent to save cost |
| **staging** | 1 hour | 1 hour | Test environment; hourly backups, reasonable recovery time |
| **prod** | 5 minutes | 15 minutes | Critical workload; WAL archiving every 5 min, recovery <15 min SLO |

The RPO (Recovery Point Objective) is the amount of data loss acceptable in a disaster scenario (e.g., can we afford to lose 5 minutes of transactions?). The RTO (Recovery Time Objective) is how fast we need to recover (e.g., must be back online in 15 minutes).

CAVE's architecture achieves these RTO/RPO targets through:
- Continuous WAL archiving (minimizes RPO)
- Automated backup testing (ensures RTO is achievable)
- Multi-region replication (for production, optional)

## 10.9 Troubleshooting

When things go wrong with PostgreSQL, this table provides diagnosis and resolution steps.

| # | Symptom | Provider | Cause | Resolution |
|---|---|---|---|---|
| 1 | XR stuck in "Creating" state for >5 minutes | Both | Crossplane provider pod is not running or has invalid credentials | Check provider pod logs: `kubectl logs -n crossplane-system deployment/provider-cave`. Verify credentials (cloud provider auth) in the ProviderConfig. If credentials are missing, update the ProviderConfig with valid API keys/tokens. |
| 2 | Connection refused on port 5432 | Hetzner | PgBouncer sidecar has not finished initializing after a failover | Wait 30 seconds; PgBouncer reconfigures after detecting the new primary. Check CloudNativePG Cluster events: `kubectl describe cluster billing-db -n tenant-acme-prod`. If events show stuck "Setting PgBouncer configuration", restart the affected Pod. |
| 3 | Error: "classification field is required" | Both | OPA policy validation rejected the Database XR spec | Add `classification` field to the XR spec. Valid values: `public`, `internal`, `confidential`, `restricted`. See ADR-102 for guidance. Always include this field for compliance tracking. |
| 4 | Database IOPS capped at 3000 despite high performanceProfile | Hetzner | Underlying storage volume type does not support higher IOPS | Upgrade the Hetzner volume type or upgrade to a different instance type. Alternatively, upgrade the Database XR's performanceProfile to "extreme" and allow Reflex Engine to auto-resize (section 10.6.2). |
| 5 | Replication lag >30 seconds (standby falling behind primary) | Hetzner | Network congestion, slow replica Pod, or insufficient replica resources | Check replica Pod resource usage: `kubectl top pod billing-db-2 -n tenant-acme-prod`. If CPU/memory are maxed, increase resource limits in the Cluster CR. Verify Cilium network policy allows replication traffic (TCP 5432). Check PostgreSQL logs for replication lag details: `kubectl logs billing-db-0 -n tenant-acme-prod | grep replication`. |
| 6 | Backup to MinIO failed | Hetzner | MinIO bucket does not exist, credentials expired, or network unreachable | Verify MinIO bucket exists: `mc ls hetzner-backups/billing-db`. Check ESO secret rotation: `kubectl get secret minio-credentials -n external-secrets-system -o yaml`. Verify Barman can reach MinIO: `kubectl exec -it billing-db-barman-0 -- s3cmd ls s3://hetzner-backups/`. If credentials are stale, ESO's next rotation cycle will refresh them; or manually trigger: `kubectl annotate externalsecret billing-db-credentials -n tenant-acme-prod force-rotate=now --overwrite`. |
| 7 | PITR restore fails with "WAL gap detected" | Both | WAL segment missing from archive; data between last backup and now is unrecoverable | Check barman WAL archive completeness: `barman list-backup billing-db`. If gaps exist, recovery is only possible to the last complete backup before the gap. Investigate why WAL was not archived (e.g., Barman Pod crashed, MinIO was unavailable). Check Barman logs: `kubectl logs -f -n cloudnativepg billing-db-barman-0`. |
| 8 | Azure connection timeout after 30 seconds | Azure | Private endpoint DNS not resolving in the VNet; application cannot reach the database | Verify Private DNS Zone is linked to the VNet: `az network private-dns zone list --resource-group rg-tenant-acme`. Check DNS resolution: `nslookup billing-db.postgres.database.azure.com <dns-ip>`. Verify network security group (NSG) allows outbound traffic on port 5432. Increase the connection timeout in the application: `postgresql://...?connect_timeout=60`. |
| 9 | Flyway migration blocked: "Migration failed" | Both | Rollback script (U*.sql) missing or invalid; forward migration has a syntax error or constraint violation | Every forward migration V*.sql must have a corresponding rollback U*.sql. Check the error message in CI logs to identify the failing SQL statement. Common causes: adding a NOT NULL column without a default, invalid syntax, foreign key constraint violations. Fix the migration script and rerun. |
| 10 | Application Pod: OOMKilled | Hetzner | PostgreSQL shared_buffers parameter too high for the container memory limit | Calculate shared_buffers as 25% of container memory limit. Example: if container limit is 8GB, set shared_buffers to 2GB. Update the Cluster CR's `postgresql.parameters.shared_buffers`. CloudNativePG will roll restart the cluster with the new setting. |
| 11 | Too many connections; connection pool exhausted | Both | PgBouncer pool_size is too small for the application workload; or application has a connection leak | Check current connections: `SELECT count(*) FROM pg_stat_activity;`. Check pool utilization: `kubectl exec -it billing-db-0 -c pgbouncer -- pgbouncer -R`. Increase pool_size in the Cluster CR: `spec.monitoring.pgbouncerMonitoring.poolSize: 100`. Also check the application for connection leaks (connections not returned to pool). |
| 12 | Credential rotation broke the application | Both | Application is not watching for Secret volume changes; still holds old connection with old password | Ensure the application is configured to reload the Secret. Use `Stakater/Reloader` annotation on the Deployment: `reloader.stakater.com/match: "true"`. Or, configure the application to periodically close idle connections and re-read the Secret. Test by triggering a manual rotation: `kubectl annotate externalsecret billing-db-credentials force-rotate=now --overwrite`. |
| 13 | Data leak: application queried a row from another tenant | Both | Row-Level Security (RLS) policy is missing or incorrect on that table | Enable RLS on the table: `ALTER TABLE <table_name> ENABLE ROW LEVEL SECURITY;`. Create the policy: `CREATE POLICY tenant_isolation ON <table_name> USING (tenant_id = current_setting('app.tenant_id')::uuid);`. Enforce this via OPA policy on all schema migrations (ADR-102 related). Check existing tables: `SELECT * FROM pg_policies;`. |
| 14 | Slow queries after PostgreSQL major version upgrade | Both | Query planner statistics are stale; planner makes suboptimal decisions | Run ANALYZE on all tables: `ANALYZE;`. This updates table and index statistics. Run EXPLAIN ANALYZE on slow queries to see the new plan. If plans are still suboptimal, consider adding indexes or rewriting the query. |
| 15 | Disk full; database cannot write | Hetzner | WAL retention setting (wal_keep_size) is too high; Barman is not archiving WAL effectively | Check WAL directory size: `kubectl exec -it billing-db-0 -- du -sh /data/pg_wal/`. Reduce wal_keep_size: change from 2GB to 512MB or lower. Verify Barman is archiving: `kubectl logs -f billing-db-barman-0 | grep archive`. Check MinIO bucket has space. Once archiving is catching up, disk pressure will decrease. Add a PVC size increase as a longer-term fix. |

## 10.10 Compliance Mapping

CAVE's PostgreSQL layer satisfies compliance frameworks by design. This table maps controls to mechanisms.

| Control | Framework | How PostgreSQL Addresses It |
|---|---|---|
| **Encryption at rest** | SOC 2 CC6.7, ISO 27001 A.10.1, GDPR Art. 32 | Hetzner: LUKS encryption enabled on all volumes. Azure: Azure Storage Service Encryption (SSE) with customer-managed keys (CMK) available. Database files are encrypted before being written to disk. |
| **Encryption in transit** | SOC 2 CC6.7, ISO 27001 A.13.1, GDPR Art. 32 | TLS 1.3 enforced for all client connections (sslmode=verify-full). Connection certificates are validated against CA. Kubernetes Istio mTLS enforces encrypted pod-to-pod communication. |
| **Access control** | SOC 2 CC6.1, ISO 27001 A.9.2, GDPR Art. 32 | RBAC per tenant namespace; Kubernetes RBAC prevents cross-tenant access. Database RBAC via PostgreSQL roles (one role per tenant). Row-Level Security (RLS) enforces row-level access control. Credentials are rotated every 30 days (ESO). |
| **Backup & recovery** | SOC 2 A1.2, ISO 27001 A.12.3, GDPR Art. 32 | Automated continuous backups (Barman). Weekly automated restore tests verify backups are usable (section 10.6.7). RPO per profile (5 min for prod). RTO <15 minutes for prod. |
| **Audit logging** | SOC 2 CC7.2, ISO 27001 A.12.4, GDPR Art. 30 | pgaudit extension logs DDL/DML per user. Logs forwarded to Sovereign Ledger. Connection/disconnection logging enabled. All Crossplane operations logged as SelfHealed attestations. |
| **Data residency** | GDPR Art. 44-49, NIS2 Art. 13 | dataResidency field in XR enforced by OPA. Hetzner/Azure deployment location is restricted per tenant. Backups are stored in the same region. |
| **Data classification** | ISO 27001 A.8.2, GDPR Art. 9 | Mandatory classification label (ADR-102) on all Database XRs. Classification drives encryption/audit/RLS policies. |
| **Change management** | SOC 2 CC8.1, ISO 27001 A.12.1, GDPR Art. 32 | All schema changes via Flyway migrations (version-controlled in Git). CI validates forward/rollback migrations before deployment. Canary deployment strategy limits blast radius. |
| **Vulnerability management** | SOC 2 CC7.1, ISO 27001 A.12.6 | Trivy container image scanning for CloudNativePG images. Azure patches PostgreSQL engine automatically. Security updates are applied within SLA. |
| **Incident response** | SOC 2 CC7.3, ISO 27001 A.16.1 | Reflex Engine auto-remediates common issues (e.g., IOPS saturation). All remediations logged as SelfHealed attestations to Sovereign Ledger. Manual escalation available if auto-remediation fails. |

## 10.11 Related ADRs

| ADR | Relevance |
|---|---|
| **ADR-047** | Primary architecture decision: PostgreSQL as the relational database for CAVE |
| **ADR-067** | Crossplane v2 as the Day 1+ provisioning and lifecycle management layer |
| **ADR-102** | Mandatory data classification labels on all Database XRs (OPA validation) |
| **ADR-113** | Data residency enforcement; geographic constraints on database placement |
| **ADR-119** | Crossplane Operations (CronOperation, WatchOperation) for scheduled and event-driven DB maintenance |
| **ADR-095** | Reflex Engine for complex remediation (IOPS auto-resize, budget checks) |
| **ADR-083** | External Secrets Operator (ESO) for credential management and rotation |
| **ADR-115** | CI/CD secret injection; no static credentials in pipelines; Flyway integration |
| **ADR-124** | MRAP (Multi-Resource Access Policy) to limit concurrent CRDs per profile/tenant |
| **ADR-080** | Backup strategy and retention policy; automated restore testing |
| **ADR-006** | Portability drills; annual testing of data migration between providers |

## 10.12 Related Runbook Sections

| Section | Relationship |
|---|---|
| **§06 Security Architecture** | Encryption at rest/in-transit, OPA validation, ESO credential injection |
| **§07 CI/CD Pipeline** | Stage 6: Flyway schema migration validation; no static secrets in environment |
| **§23 Crossplane v2 Composition** | XRD definition, Composition Function implementation for Database abstraction |
| **§24 Crossplane Operations** | CronOperation for DB maintenance; WatchOperation for event-driven remediation |
| **§27.2 Automated Remediation** | Reflex Engine triggering on Prometheus alerts (IOPS saturation example) |
| **§28 Backup & DR** | Velero for cluster backups + Barman/Azure for database backups; PITR strategies |
| **§29 Migration & Portability** | Database migration during annual portability drills; pg_dump / logical replication |
| **§30 FinOps** | Database cost in tenant unit economics; rightsizing performance profiles |
| **§41 Sovereign Ledger** | Self-Healed attestations from DB operations; immutable audit trail |
