# CAVE Platform Runbook §01 — Architecture Principles

**Domain:** caveplatform.dev
**Platform:** Cloud-Agnostic Virtualized Environment (CAVE)
**Last Updated:** 2026-03-06
**Audience:** Platform engineers, architects, compliance teams, operations guardians

---

## 1.1 Overview: Why Architecture Principles Matter

Architecture principles are the immune system of an Internal Developer Platform. Without them, CAVE would accumulate technical debt, suffer configuration drift, and eventually collapse under its own complexity. These 18 principles form a constitutional layer that governs every decision—from which tools we adopt to how we remediate failures to who can authorize breaking changes.

For an IDP serving multiple tenants across multiple cloud providers, principles do three critical things:

1. **Prevent Drift.** When a developer or platform engineer faces a decision, they ask: "Which principle applies here?" This creates consistent behavior across all seven deployment profiles (dev/staging/prod × Hetzner/Azure + local). Without principles, teams would solve the same problem differently in different places.

2. **Speed Decision-Making.** Architects don't need to debate the merits of manual kubectl each time. Principle 6 (GitOps Everything) says: use ArgoCD, create signed commits, no exceptions except Emergency CLI under ledger logging. Done.

3. **Enforce Constraints.** Principles make non-negotiable trade-offs visible. Yes, we accept Backstage complexity because Principle 1 (Single Pane of Glass) prevents portal sprawl. Yes, we run two provisioning layers instead of one because Principle 4 (Two-Layer Provisioning) cleanly separates bootstrap concerns from runtime concerns.

This section explains each principle, the problem it solves, how CAVE implements it, and the trade-offs we've made.

---

## 1.2 Principle 1: Single Pane of Glass — Backstage Only

**Principle Statement:**
Backstage is the *only* developer-facing portal for all platform interactions, regardless of underlying cloud provider or infrastructure technology. The same UX applies whether a workload runs on Hetzner or Azure.

**Why This Principle Exists:**

Developer portal sprawl is a platform killer. Without a single pane of glass, teams end up maintaining three portals: Backstage for templates, Azure Portal for debugging, and Hetzner console for networking. Context switching wastes cognitive energy. Inconsistent UX breeds confusion. New developers spend weeks learning three different mental models instead of one.

Moreover, if developers can directly access cloud consoles, they circumvent platform guardrails. They may create untracked resources, bypass policy checks, or accidentally leak secrets. Inconsistent UX also means some teams adopt shortcuts, some don't, creating a fragmented audit trail.

**How CAVE Implements It:**

Every platform capability has a Backstage representation:

- **Catalog:** Microservices, templates, databases, message buses all appear as Backstage entities. Platform teams maintain the source-of-truth; developers consume it.
- **Templates:** `cave-app`, `cave-database`, `cave-vcluster`, etc. Templates call cave-ctl internally but expose a unified UX.
- **TechDocs:** Every component (Grafana, Kong, Tetragon, Crossplane) has documentation baked into Backstage.
- **Plugins:** Custom plugins integrate Grafana dashboards (Principle 10), SLO dashboards (Principle 12), policy status (Principle 11), and AI capabilities (Principle 9) directly into Backstage.
- **Authentication:** Keycloak is the single source of truth for identity; Backstage is the single UX for every action requiring identity.

Behind the scenes, all Backstage interactions translate to cave-ctl calls, ArgoCD commits, or Crossplane XR reconciliation. The developer sees Backstage. The infrastructure sees declarative state.

**Consequences:**

- All new features *must* have Backstage integration. This adds latency to feature delivery but ensures consistency.
- Backstage becomes a critical component. Its outage blocks all self-service. (Mitigated by Principle 10: we have Grafana dashboards for manual interventions.)
- We accept complexity in Backstage plugins rather than expose developers to multiple UIs.
- The principle precludes cloud-provider portals from being the "source of truth" for any developer action. If a developer changes something in Azure Portal, ArgoCD will reconcile it away (or alert on drift).

**Related ADRs:**
ADR-011 (Backstage as Developer Control Plane), ADR-025 (Backstage Plugin Architecture)

---

## 1.3 Principle 2: Three Control Planes

**Principle Statement:**
CAVE exposes three distinct control planes, each optimized for a different use case and user type. All three are necessary; none can be eliminated without losing critical capability.

**Why This Principle Exists:**

Different personas interact with platforms differently. A developer writing a template needs a visual, self-service UX. An AI agent executing a remediation action needs a scriptable, programmatic interface. An on-call engineer dealing with a critical outage needs a read-mostly, forensic-audit-trail interface that logs every action to WORM.

A single control plane either becomes too simple (missing power) or too complex (losing usability). Three control planes let us optimize each independently.

**How CAVE Implements It:**

1. **Backstage (Self-Service GUI Control Plane)**
   - User Type: All developers, product managers, platform tenants
   - Scope: Create apps, databases, vclusters; view dashboards; browse catalogs
   - Authentication: Keycloak OAuth2
   - Audit Trail: ArgoCD git history, Sovereign Ledger for sensitive actions
   - Latency: Human-acceptable (1–5 seconds typical)

2. **cave-ctl (CLI + MCP Server Control Plane)**
   - User Type: Developers in terminal, AI agents (via MCP), CI/CD pipelines
   - Scope: Full CAVE API, including policy checks, deployments, disaster recovery
   - Authentication: Kubeconfig + mTLS, machine identities (workload certificates)
   - Audit Trail: Sovereign Ledger, signed commits to git
   - Latency: Sub-second (CRD watch events)
   - Special Capability: MCP Server allows Claude, ChatGPT, or other AI systems to invoke cave-ctl operations with reasoning traces logged to Sovereign Ledger (see Principle 15).

3. **Emergency CLI (Incident-Only Control Plane)**
   - User Type: On-call engineers, tier-2 ops
   - Scope: Forensics, manual remediation, read-only deep inspection
   - Authentication: mTLS + hardware security key (Yubikey)
   - Audit Trail: *Every* action creates a signed entry in Sovereign Ledger. Changes are not auto-reconciled; they create a "drift audit" that signals to guardians.
   - Latency: Sub-second, zero retry logic (fail fast to prevent accidental cascading changes)
   - Governance: Operations that modify state (e.g., evict a pod, drain a node) require a guardian signature or an ADR justifying the emergency. Caveat: Under Sovereign Ledger logging, an on-call engineer *can* modify state without pre-approval, but all changes are non-repudiated and subject to post-incident review.

**Consequences:**

- Three control planes means three authentication mechanisms, three audit paths, three sets of role definitions. Complexity increases.
- Backstage outage doesn't block emergency CLI. Emergency CLI outage doesn't block cave-ctl.
- We must keep all three in sync. A Backstage template that calls cave-ctl must produce equivalent results to a direct cave-ctl invocation.
- MCP Server (in cave-ctl) enables "CAVE as an AI operating platform"—AI agents run autonomous operations with full reasoning trails. This is powerful but requires strict governance (Principle 15).

**Related ADRs:**
ADR-015 (Control Plane Architecture), ADR-008 (cave-ctl MCP Server Design)

---

## 1.4 Principle 3: Profile-Driven Deployment

**Principle Statement:**
The same cave-ctl commands deploy CAVE to different infrastructure profiles. A developer runs `cave bootstrap --profile=dev` locally; a platform engineer runs `cave bootstrap --profile=prod-azure` on a hyperscaler. The platform adapts to the target environment; the interface stays constant.

**Why This Principle Exists:**

Teams need different infrastructure configurations for different stages. Dev might be a single-node Talos cluster; prod needs a multi-region HA setup. Azure might have managed Postgres; Hetzner doesn't. Without profiles, developers would need to maintain separate tooling, documentation, and mental models for each target.

Profiles also solve onboarding. A new platform engineer can bootstrap a dev environment locally and immediately understand how CAVE works. They then scale to staging and production without conceptual surprises.

**How CAVE Implements It:**

Seven profiles are defined:

1. **dev-local** – Single-node K3s on developer laptop. Minimal resources. For learning and testing templates.
2. **dev-hetzner** – Three-node Talos cluster on Hetzner. For realistic multi-node debugging.
3. **staging-hetzner** – Ten-node Hetzner cluster. Mid-scale, HA data stores, Crossplane enabled, chaos testing allowed.
4. **staging-azure** – Equivalent to staging-hetzner but on Azure. Tests cross-cloud behavior.
5. **prod-hetzner** – Production-grade Hetzner cluster: 30+ nodes, multi-availability-zone networking, Tetragon enabled, chaos testing forbidden.
6. **prod-azure** – Production-grade Azure with managed services (AKS, Azure Database, Cosmos DB).
7. **prod-hybrid** – Workloads span Hetzner and Azure with cross-cloud failover (Phase 4 roadmap).

Each profile is defined by a YAML manifest specifying:
- Cluster node count and machine types
- Data store: managed (Azure) vs self-hosted (Hetzner)
- Network topology: single-AZ, multi-AZ, multi-region
- Observability: sampling rates, retention, alerting thresholds
- Chaos testing policies: enabled (non-prod), disabled (prod)
- Cost limits and FinOps kill-switch thresholds (Principle 12)

When a platform engineer runs `cave bootstrap --profile=prod-azure`, the bootstrap pipeline:
1. Reads the profile manifest
2. Invokes OpenTofu with profile-specific variables
3. Configures Crossplane compositions to use Azure providers
4. Seeds ArgoCD with environment-specific applications
5. Validates the result against Principle 21 (Survivability Invariant): "All tenant workloads continue if any single platform component fails."

**Consequences:**

- Profile definitions must be kept in sync with actual infrastructure. Drift = unreliable bootstrapping.
- Adding a new profile requires updating OpenTofu, Crossplane, and observability configs. High cost for a new environment type.
- We accept "seven versions of the truth" because the benefit—consistent UX + fast onboarding—outweighs the maintenance burden.
- Profiles are not free-form. New ones require an ADR and Platform Architecture Review (PAR).

**Related ADRs:**
ADR-021 (Profile Architecture), ADR-122 (Hetzner Profile Definition), ADR-123 (Azure Profile Definition)

---

## 1.5 Principle 4: Two-Layer Provisioning

**Principle Statement:**
Infrastructure provisioning is split into two layers: *Day 0* (OpenTofu for cluster and networking bootstrap) and *Day 1+* (Crossplane for application dependencies and ongoing operations). ArgoCD orchestrates both layers, ensuring every piece of infrastructure is declaratively defined.

**Why This Principle Exists:**

A single provisioning tool (e.g., Terraform for everything) creates a false boundary between "one-time infrastructure" and "recurring operations." In practice:

- **Day 0 concerns** (spinning up a cluster, creating VPCs, configuring DNS) require imperative orchestration. You can't idempotently create a cluster because "create a cluster" is inherently stateful.
- **Day 1+ concerns** (provisioning a database for an app, creating a message bus, scaling services) should be declarative. A Crossplane ComposedResourceDefinition (XRD) lets developers request a database by submitting a YAML; reconciliation loops handle the rest.

Mixing both in one tool (like Terraform) leads to confusion: which state is safe to re-run idempotently? Which requires special handling? Separating them makes the boundary explicit.

**How CAVE Implements It:**

**OpenTofu Layer (Day 0):**
- Creates the Kubernetes cluster (Talos on Hetzner or AKS on Azure)
- Creates networking (VPCs, subnets, security groups)
- Creates foundational managed services (e.g., Azure Database for Postgres, Hetzner load balancers)
- Installs the Kubernetes networking add-ons (Cilium)
- Installs ArgoCD itself
- Produces a kubeconfig and secrets (stored in Sovereign Ledger)
- Runs *once per cluster*, then becomes read-only for auditing

**Crossplane Layer (Day 1+):**
- Runs continuously. Applications request databases, caches, and message buses by submitting XRs (Custom Resources).
- Crossplane compositions translate XRs into cloud-provider-specific resources (e.g., an XRD "Database" becomes an Azure Database or a Hetzner-managed Postgres instance).
- Crossplane Operations (CronOperation, WatchOperation) handle recurring tasks: backups, scaling decisions, remediation (Principle 16).
- All Crossplane operations are GitOps-driven: a developer's commit to the Git repo triggers an ArgoCD sync, which reconciles the desired Crossplane XRs.

**ArgoCD's Role:**
- Syncs the OpenTofu root module to the cluster (as a Helm chart). If the YAML drifts, ArgoCD re-applies it.
- Syncs all Crossplane XRs from Git to the cluster. When a developer commits a new Database XR, ArgoCD reconciles it and Crossplane provisions the actual DB.
- Syncs OPA bundles, Tetragon configs, Kong routes—everything declaratively.

**Consequences:**

- Two tools to maintain and upgrade (OpenTofu + Crossplane). More operational burden.
- The boundary between layers must be rigidly enforced. A temptation exists to use OpenTofu for "just one more thing" (e.g., a daily scaling job). Resist it. Crossplane is the right tool for Day 1+ operations.
- Developers don't touch OpenTofu; they only submit Crossplane XRs. This simplifies their mental model.
- Recovering from Day 0 failures (e.g., cluster creation failed) is manual. Recovering from Day 1 failures (e.g., a database XR has an invalid claim) is automatic (Crossplane reconciliation).

**Related ADRs:**
ADR-067 (OpenTofu + Crossplane Boundary), ADR-119 (Crossplane v2 Architecture)

---

## 1.6 Principle 5: Crossplane-First Abstraction

**Principle Statement:**
Applications never depend on cloud-provider APIs or console names. All infrastructure is accessed through Crossplane Composite Resource Definitions (XRDs), which are namespace-scoped, provider-agnostic abstractions. ManagedResourceActivationPolicy (MRAP) prevents resource footprint explosion.

**Why This Principle Exists:**

If an application's code references an Azure resource name (e.g., `my-storage-account`), migrating to Hetzner requires changing application configs. Worse, if ten teams each define their own custom resources for "a database," we end up with a fragmented ecosystem—some databases have backup policies, others don't; some are encrypted, others aren't.

Crossplane XRDs solve this by providing a *canonical language* for expressing infrastructure dependencies. An application doesn't ask for "an Azure SQL database"; it asks for a "Database with 100 GiB, multi-AZ, 99.9% uptime SLA."

**How CAVE Implements It:**

**Core XRDs (Canonical Abstractions):**

- **Database** – Relational database (Postgres, MySQL, MariaDB). XR specifies size, backup retention, HA mode. Composition automatically chooses Azure Database (managed) or Hetzner + self-hosted Postgres based on the target profile.
- **Bucket** – Object storage (S3-compatible). Same abstraction across Azure Blob Storage and Hetzner S3-compatible object stores.
- **Cache** – Redis-compatible caching layer. Managed Azure Cache for Redis or self-hosted Redis.
- **MessageBus** – Event streaming (Kafka-compatible or managed message queues).
- **Search** – Elasticsearch-compatible full-text search. OpenSearch on Hetzner, Azure Cognitive Search or managed OpenSearch on Azure.
- **VectorDB** – Vector storage for embeddings (used by Principle 9, AI). Pinecone (managed) or Weaviate (self-hosted).

Each XRD is authored once, tested in all profiles, and reused everywhere. The composition function (written in CEL, not Rego) maps the XRD to provider-specific resources.

**Namespace-Scoped (Principle of Least Surprise):**

All XRs are namespaced. A developer in the `acme-team` namespace cannot see or interfere with XRs in the `competitor-team` namespace. The Crossplane controller respects RBAC boundaries.

**ManagedResourceActivationPolicy (MRAP):**

Problem: If applications can submit arbitrary CRDs, someone will define a `PostgresInstanceManual` CRD that directly exposes Azure connection strings, bypassing CAVE's abstractions. MRAP prevents this.

MRAP is a policy layer (enforced via Gatekeeper, Principle 11) that:
1. Allowlists which CRD kinds are activatable in each namespace. E.g., the `acme-team` namespace can only activate `Database`, `Bucket`, `MessageBus`—not `PostgresInstance` or `AzureDatabase`.
2. Prevents CRD kinds that expose cloud provider internals. A developer can't submit a raw Azure CRD.
3. Requires a reason and justification (governance ticket) to add a new activatable CRD.

**Consequences:**

- Abstraction is powerful but requires investment. Writing a new XRD takes ~2 weeks (including compositions, tests, docs).
- Developers experience a slight delay in getting new infrastructure types—they can't request "raw Azure SQL" on day 1.
- The abstraction layer hides cloud-specific optimizations. We accept some efficiency loss for portability.
- Crossplane compositions must be tested in all profiles. A composition bug affects all users. High stakes, but high confidence in changes.

**Related ADRs:**
ADR-067 (Crossplane Compositions), ADR-119 (Crossplane v2 Namespace-First Design), ADR-124 (MRAP Policy)

---

## 1.7 Principle 6: GitOps Everything

**Principle Statement:**
Every piece of CAVE infrastructure is defined in Git, synced via ArgoCD, and applied using server-side apply. Manual kubectl is forbidden except in Emergency CLI under Sovereign Ledger logging. Drift detection is enabled by default; drift reconciliation is automatic.

**Why This Principle Exists:**

Configuration drift is the root cause of most production outages and compliance failures. Without GitOps, teams end up with "snowflake" systems: someone runs a kubectl command to fix a problem, no one documents it, and six months later a junior engineer spins up a new cluster that lacks the fix.

GitOps also provides compliance evidence. Auditors ask: "Who changed the firewall rules?" With manual kubectl, the answer is "someone, sometime." With Git commits, the answer is "Alice, at 2026-03-03 14:22 UTC, with commit message 'Increase Kong timeout for slow clients.'"

**How CAVE Implements It:**

**Git + OCI Registry Model:**

- All CAVE configurations live in a Git repository (cabability-cave-config).
- ArgoCD pulls from Git and from an OCI registry (Harbor) for container images and Helm chart artifacts.
- Applications are deployed via ArgoCD ApplicationSet CRDs, which define the desired state.
- ArgoCD compares desired (in Git) vs actual (in cluster) every 3 minutes. If they diverge, ArgoCD auto-reconciles.

**Server-Side Apply:**

- ArgoCD uses `kubectl apply --server-side`, not client-side apply.
- Server-side apply prevents client-side field conflicts (e.g., two controllers trying to own the same field). The Kubernetes API arbitrates.
- Manual kubectl is forbidden because every change must flow through Git. A developer who wants to change something must commit to Git first, and ArgoCD reconciles the change.

**Emergency CLI Exception:**

- Incident response sometimes requires immediate action (e.g., scale down a misbehaving pod).
- Emergency CLI can modify state directly *without* going through Git first.
- However, every Emergency CLI action creates a *signed Git commit* in a special `emergency-actions` branch, pushed automatically by the Emergency CLI controller.
- ArgoCD then reconciles this commit. If the change is correct, it gets merged to main. If it's wrong, it gets reverted.
- The entire audit trail is signed and stored in Sovereign Ledger.

**Drift Detection:**

- ArgoCD alerts on drift (desired ≠ actual).
- Drift can mean two things: (a) someone modified the cluster manually (bad), or (b) a cert expired and was auto-renewed by a controller (expected, documented).
- ArgoCD distinguishes with annotation-driven exceptions: resources annotated `argocd.argoproj.io/compare-result: ignore` are not drift-flagged.

**Consequences:**

- Developers must wait for ArgoCD sync. Typical latency: 3–5 minutes. For day-2 operations, this is acceptable. For emergency changes, Emergency CLI bypasses this.
- Git becomes a critical system. If the config repo is corrupted, the entire platform can be corrupted. Mitigated by: (a) backup branches, (b) Sovereign Ledger signing, (c) read-only Guardian audit.
- All configuration is versioned and auditable. "Who deployed what" is a git log query.

**Related ADRs:**
ADR-026 (GitOps Architecture), ADR-120 (ArgoCD v3.3+ Server-Side Apply)

---

## 1.8 Principle 7: Security by Default

**Principle Statement:**
Security is built into every profile from the ground up. No "security is an add-on for production." Zero-trust networking, signed container images, SBOMs, SLSA Level 3 attestations, and runtime enforcement apply to dev, staging, and production equally.

**Why This Principle Exists:**

Postponing security to production is a siren song. "We'll add authentication later, we'll scan images in prod, we'll harden networking next quarter." In practice, later never comes. Developers write code against insecure dev environments and end up shipping vulnerabilities.

By baking security into all profiles, we ensure:
1. Developers test against realistic security constraints from day 1.
2. Security tooling is battle-tested in dev before prod relies on it.
3. No surprises when workloads move to production.

**How CAVE Implements It:**

**Zero-Trust Networking (Istio mTLS):**
- Every workload-to-workload connection is encrypted with mTLS (mutual TLS).
- Istio sidecar proxies enforce mTLS at the application layer. Even if an attacker compromises a pod, they can't decrypt traffic to adjacent services.
- Applied to all profiles, including dev-local. A developer's local microservice architecture has Istio sidecars.

**Signed Container Images (cosign Keyless):**
- Every image pushed to Harbor must be signed. Signatures are verified by Kyverno (policy controller) at admission time.
- cosign keyless signing uses OIDC + Sigstore keyless infrastructure. No private keys stored in CI/CD.
- If an image is unsigned or signature verification fails, Kyverno rejects the pod. Pod stays in ImagePullBackOff.

**SBOM Generation (CycloneDX):**
- Every image build produces a CycloneDX SBOM (Software Bill of Materials).
- The SBOM is attached to the image via Rekor (Sigstore). It's auditable and tamper-evident.
- Compliance teams can query "What versions of OpenSSL are running in prod?" across all images.

**SLSA Level 3 Attestations:**
- CI/CD pipeline follows SLSA Level 3 requirements: (a) all sources reviewed, (b) all build steps audited, (c) build provenance signed.
- Every image gets a SLSA provenance attestation, signed and immutable in Rekor.
- No image can be deployed without provenance. Impossible to deploy undocumented artifacts.

**27-Stage CI/CD Pipeline with Security Gates:**

1. Code commit
2. Pre-commit hook: format check, secret scan (Gitleaks)
3. Lint and unit test
4. SAST (static analysis): Semgrep, SonarQube
5. Dependency scan: Dependabot, OWASP Dependency-Check
6. License check: REUSE compliance
7. Container build (OCI image)
8. Image scan (Trivy): vulnerabilities, misconfigurations
9. Sign image (cosign)
10. Generate SBOM (cyclonedx)
11. Attest provenance (SLSA)
12. Push to Harbor
13. Policy evaluation (OPA): "Is this image allowed?"
14. Kyverno admission check (if deploying immediately)
15. Chaos testing validation (staging-hetzner, staging-azure)
16. E2E test suite
17. SLO validation
18. Cost impact assessment (FinOps)
19. Manual approval gate (prod-only)
20. ArgoCD sync trigger
21. Canary deployment (prod-only, Principle 16)
22. Automated rollback validation
23. Incident simulation (chaos)
24. Golden signal monitoring
25. Audit log validation
26. Sovereign Ledger attestation
27. Post-deployment OSINT (supply chain monitoring)

Each gate can block deployment. A single failed SAST rule blocks promotion. A cost spike of >20% blocks promotion (Principle 12).

**Runtime Enforcement (Tetragon):**
- Tetragon enforces Linux Security Module (LSM) policies at runtime.
- Example policy: a pod running `curl` inside a container violates policy (unexpected external network call). Tetragon alerts or blocks.
- Used in all profiles. In dev, violations produce warnings. In prod, violations may block or kill the pod (configurable per policy, per tenant).

**Consequences:**

- Security tooling adds latency to every pipeline. CI/CD is slower. Accepted trade-off.
- Developers must understand mTLS, image signing, and OPA policies. Initial onboarding is harder. Mitigated by Backstage plugins that simplify the experience.
- Security gates may block legitimate workloads. Requires human review and policy adjustments. Expected operational cost.
- The surface area of "security tools that must not fail" is large. A broken Kyverno admission controller can prevent all deployments. Mitigated by Principle 21 (Survivability Invariant): Kyverno failure doesn't prevent emergency pod restarts.

**Related ADRs:**
ADR-077 (Zero-Trust Networking), ADR-101 (Image Signing), ADR-105 (SBOM Strategy), ADR-106 (SLSA Level 3)

---

## 1.9 Principle 8: Multi-Tenant Isolation

**Principle Statement:**
CAVE supports three isolation tiers—Namespace (soft), vCluster (hard), and Dedicated (maximum)—with graduated network policies and resource quotas. A tenant chooses the tier based on data sensitivity and workload criticality.

**Why This Principle Exists:**

Multi-tenancy on shared infrastructure is inherently risky. A "noisy neighbor" (a tenant consuming 90% of CPU) can starve other tenants. A compromised tenant might exfiltrate data from adjacent namespaces. We need isolation levels that scale from "low-sensitivity dev tenants on shared hardware" to "highly regulated financial tenants in isolated clusters."

Three tiers let teams right-size isolation costs. A startup testing a prototype uses Namespace isolation (cheap). A financial services firm uses Dedicated isolation (expensive but maximum assurance).

**How CAVE Implements It:**

**Tier 1: Namespace (Soft Isolation)**
- Tenants share the same Kubernetes cluster.
- Network isolation: Cilium NetworkPolicy enforces tenant-scoped traffic rules. Pod-to-pod traffic between namespaces is forbidden by default; only explicitly allowed flows work.
- Resource isolation: Kubernetes ResourceQuota limits CPU, memory, ephemeral storage per namespace.
- Identity isolation: Keycloak RBAC restricts which tenants can access which namespaces.
- Cost model: Cheapest. Shared infrastructure depreciation spread across all tenants.
- Use case: Shared dev environments, low-sensitivity workloads, startups.
- Trade-off: Namespace isolation is cryptographically weak. A determined attacker with pod-level access *might* escape and compromise adjacent namespaces. Mitigated by runtime enforcement (Tetragon, Principle 7).

**Tier 2: vCluster (Hard Isolation)**
- Each tenant gets a virtual Kubernetes cluster (vCluster) running inside a pod on the shared host cluster.
- The tenant's API server, etcd, and kubelet run *inside* their vCluster pod. The host cluster is hidden.
- Network isolation: The vCluster pod is subject to host Cilium NetworkPolicy. Traffic between vClusters is forbidden by default.
- Resource isolation: The vCluster pod gets a guaranteed resource slice (CPU, memory). If the tenant exhausts it, their workloads are OOM-killed, not neighboring workloads.
- Blast radius: A compromised tenant workload is confined to their vCluster; it cannot reach the host cluster's API server or neighboring vClusters.
- Cost model: ~10–15% overhead per vCluster (API server + etcd replica).
- Use case: Mid-sensitive workloads, teams requiring defense-in-depth.

**Tier 3: Dedicated (Maximum Isolation)**
- Each tenant gets a dedicated Kubernetes cluster (separate set of nodes, separate networking, separate data stores).
- No shared components except the control plane gateway (Kong, Principle 2) and observability collectors (Prometheus agents).
- Nodes may be geographically isolated (e.g., separate region, separate hyperscaler).
- Network isolation: Physical or logical network separation (VPC isolation, cross-account isolation on Azure).
- Resource isolation: Absolute. One tenant's cluster has zero impact on another's.
- Compliance advantage: Dedicated clusters simplify regulatory evidence. Auditors see "separate infrastructure per tenant."
- Cost model: Highest. Minimum cluster size (3 nodes × 2 profiles × 2 clouds) = expensive.
- Use case: Financial, healthcare, government customers. Multi-billion-dollar contracts that require isolation guarantees.

**Cross-Tier Network Policies:**

All tiers enforce these Cilium policies:
- Egress to external networks (internet) is restricted by tenant criticality (Principle 12).
- Ingress from external networks routes through Kong with authentication.
- Inter-tenant traffic is forbidden by default.
- Communication with platform components (Prometheus scraping, log shipping to Loki) is explicitly allowed per tenant.

**Consequences:**

- Tier 2 and Tier 3 cost more. We accept this because data protection and compliance are non-negotiable.
- Tier 1 requires strong runtime security (Tetragon). Weak runtime enforcement makes Tier 1 indefensible.
- Tenant onboarding must clearly establish isolation requirements. A misconfigured tenant (e.g., sensitive data in Tier 1) is a compliance risk. Mitigated by Principle 11 (OPA policy enforcement).

**Related ADRs:**
ADR-012 (Multi-Tenancy Architecture), ADR-084 (vCluster Design)

---

## 1.10 Principle 9: Self-Hosted AI with Governance

**Principle Statement:**
AI capabilities are available to the platform and to developers, but data classification drives routing. Restricted data goes to self-hosted Ollama. Confidential data goes to Azure OpenAI with Data Processing Agreements. Public and internal data can use any model. LiteLLM acts as a unified gateway.

**Why This Principle Exists:**

AI is essential to modern platforms. We need it for Principle 15 (autonomous operations), for developer productivity (code generation), and for compliance automation (policy drift detection). But AI models are trained on the internet, and "confidential customer data" cannot be sent to a public LLM API.

The solution: maintain a portfolio of models. Self-hosted Ollama for sensitive data. Azure OpenAI (with DPA) for highly confidential. Public APIs for public data.

**How CAVE Implements It:**

**LiteLLM Gateway:**
- LiteLLM is a unified proxy for multiple LLM providers. Developers submit requests to `https://litellm.caveplatform.dev/v1/chat/completions` without caring which backend handles it.
- LiteLLM decides routing based on:
  1. Data classification (from Principle 11, embedded in the request context)
  2. Model capability requirements (some requests need reasoning, others need speed)
  3. Cost optimization (cheaper models for simple tasks)

**Data Classification-Driven Routing:**

- **Restricted (PII, customer data, secrets):** Routes to self-hosted Ollama (Mistral 7B or Llama2 13B, fine-tuned on internal corpus). Data never leaves CAVE infrastructure. Models run on GPU nodes in the cluster (or dedicated GPU machines on Hetzner).

- **Confidential (product roadmap, financial forecasts, contracts):** Routes to Azure OpenAI with a Data Processing Agreement (DPA) in place. Data is encrypted in-transit and at-rest. Azure OpenAI does not retain data for model improvement. Slower but maximum privacy.

- **Public/Internal (code snippets, architecture docs, non-sensitive logs):** Routes to whichever model is cheapest and available (OpenAI GPT-4, Claude Opus, open-source models via Together.ai, etc.).

**PII Redaction (Presidio):**

Before sending any data to an LLM, Presidio (a PII detection library) redacts sensitive information:
- Phone numbers → `[PHONE_NUMBER]`
- Email addresses → `[EMAIL_ADDRESS]`
- Names of employees (from Keycloak LDAP) → `[PERSON_NAME]`
- Internal IPs and hostnames → `[HOSTNAME]`

Even if a developer accidentally includes PII in a request, Presidio strips it before the request leaves CAVE.

**Usage Examples (Principle 15):**

- **AI SRE:** Asks Ollama, "Why is the p99 latency spike correlated with traffic spike?" Ollama reasons over Prometheus metrics (which are internal, non-sensitive) and suggests: "Scale the backend service." Request is local, no external API call needed.

- **AI FinOps Governor:** Asks Azure OpenAI, "Right-size this Azure VM given its 2-week utilization profile." Request includes confidential cost data and resource consumption data. Azure OpenAI is DPA-backed, trusted for confidential.

- **Developer:** Asks "Generate a Kubernetes manifest for a caching layer." LiteLLM routes to the cheapest available model (might be Ollama, might be Claude). The manifest is generic code, no sensitive data.

**Consequences:**

- Multiple model providers means integration complexity. LiteLLM adds a network hop but simplifies API surface.
- Self-hosted Ollama requires GPU hardware. Cost and complexity for CAVE infrastructure. Hetzner GPUs are rented; Azure has managed GPU services.
- Presidio redaction is imperfect. It might over-redact (e.g., misidentify a hostname) or under-redact (e.g., miss a unique identifier). Requires tuning and human oversight.
- Teams must understand data classification to make good routing decisions. A developer who doesn't classify data correctly might send restricted data to a public API. Mitigated by OPA policies that enforce classification.

**Related ADRs:**
ADR-009 (Self-Hosted AI Strategy), ADR-013 (LiteLLM Gateway), ADR-103 (Data Classification Routing), ADR-111 (Presidio Integration)

---

## 1.11 Principle 10: Full Observability

**Principle Statement:**
Every bit of data flowing through CAVE is observable: logs, metrics, traces, and events. All observability is multi-tenant by design, scoped by tenant-id. The same Grafana dashboards work across all profiles (dev-local to prod-azure) without modification.

**Why This Principle Exists:**

Multi-tenant systems are hard to debug. When prod is down, on-call engineers need to answer: "Which tenant is affected? Is it their workload or platform infrastructure? What changed in the last 10 minutes?" Without observability, troubleshooting becomes guesswork.

Full observability also supports FinOps (Principle 12): we must track resource consumption per tenant to bill fairly and right-size infrastructure.

**How CAVE Implements It:**

**Prometheus (Metrics):**
- All components (Kubernetes, Kong, Crossplane, Tetragon, Ollama, etc.) expose Prometheus metrics.
- Metrics are scraped every 15 seconds across all profiles.
- Multi-tenancy: every metric includes a `tenant_id` label. Queries like `container_cpu_usage_seconds_total{tenant_id="acme-team"}` isolate one tenant's CPU usage.
- Data retention: 15 days at 15-second granularity, 1 year at 1-hour granularity (using Thanos downsampling).
- Alerting: Prometheus alert rules (e.g., "p99 latency >500ms") are defined once and fire for all tenants if their individual metrics exceed thresholds.

**Loki (Logs):**
- All container logs (stdout, stderr) are streamed to Loki via Promtail DaemonSet.
- Logs are indexed by labels: `tenant_id`, `pod`, `namespace`, `cluster`, `profile`.
- Multi-tenancy: developers only see logs from their namespace. Cross-tenant log queries are blocked at the API layer (Loki RBAC).
- Sampling: In dev, all logs are retained (unlimited). In prod, logs are sampled: only ERROR and WARNING logs are retained indefinitely; INFO and DEBUG are sampled at 10%. Reduces cost without losing critical signals.

**Tempo (Traces):**
- Distributed tracing using OpenTelemetry (OTEL) instrumentation in all applications.
- Traces are 1:1 correlated with spans. A single HTTP request generates a trace with multiple spans (service-to-service calls, database queries, cache lookups).
- Multi-tenancy: traces include `tenant_id` in baggage. A developer traces a request end-to-end within their tenant.
- Sampling: 100% in dev, 5% in prod (adaptive sampling based on error rate). If a trace errors, it's always sampled.

**Thanos (Long-Term Storage):**
- Thanos is an ultra-long-term metrics store (object storage backed).
- Data is deduplicated across multiple Prometheus instances. If dev-hetzner and dev-azure both scrape the same application (for comparison testing), Thanos deduplicates.
- Compliance value: auditors can query metrics from 2 years ago to prove SLA compliance or investigate historical cost anomalies.

**Grafana Dashboards:**

All dashboards are profile-agnostic. They work in dev-local, staging-hetzner, and prod-azure without changes:

- **Golden Signals Dashboard:** Latency, error rate, saturation, throughput. Tenant-scoped via `tenant_id` variable.
- **FinOps Dashboard:** CPU, memory, egress, storage per tenant. Feeds Principle 12 (SLO-Driven FinOps).
- **Security Dashboard:** Tetragon events, policy violations, unsigned images. Principle 7 observability.
- **Platform Health Dashboard:** ArgoCD sync status, API server latency, etcd performance, Crossplane reconciliation lag.

Dashboards use Grafana variables (templating) to let users filter by tenant, profile, or time range.

**Consequences:**

- Observability infrastructure (Prometheus, Loki, Tempo, Thanos) is itself a critical platform component. Its failure impacts debugging capability but not tenant workloads (by Principle 21).
- Observability generates a lot of data. Costs scale with cluster size and cardinality (number of unique label combinations). Sampling in prod is necessary.
- Multi-tenant observability requires strict RBAC. A bug in Loki RBAC could leak logs across tenants. High stakes, carefully tested.
- Developers expect Grafana dashboards to work immediately. If a dashboard is "template-driven" (good for reuse, bad for discoverability), we must provide guided dashboards for each use case.

**Related ADRs:**
ADR-029 (Observability Architecture), ADR-109 (Grafana Template Design)

---

## 1.12 Principle 11: Policy-as-Code

**Principle Statement:**
All platform policies—from RBAC rules to allowed container registries to data classification requirements—are expressed as code, version-controlled, and enforced automatically. OPA Gatekeeper enforces Kubernetes admission policies. OPAL distributes real-time policy and data updates from external sources (Keycloak, tenant metadata stores) to OPA without requiring ArgoCD sync delays.

**Why This Principle Exists:**

Manual policy enforcement doesn't scale. "Thou shalt only deploy images from the approved registry" is a guideline; without enforcement, someone will circumvent it. And if a policy changes (e.g., "approved registries" list grows), updating 50 wikis is error-prone.

Policy-as-code makes policies executable, testable, and auditable. A policy is either enforced or it isn't; there's no ambiguity.

**How CAVE Implements It:**

**OPA Gatekeeper (Policy Enforcement):**

OPA is a policy engine that uses Rego (a declarative language) to express rules:

```
package kubernetes.admission

deny[msg] {
    input.request.kind.kind == "Pod"
    image := input.request.object.spec.containers[_].image
    not startswith(image, "harbor.caveplatform.dev/")
    msg := sprintf("Image %s is not from approved registry", [image])
}
```

This rule denies any pod with an image outside the approved registry. Enforcement is automatic: Kubernetes API rejects the pod with the error message.

**Policies Enforced by Gatekeeper:**

1. **Image Policy:** Only images signed with cosign (Principle 7). Only images from Harbor (approved registry). Only images with SBOM attached.
2. **RBAC Policy:** Developers cannot create ClusterRole or ClusterRoleBinding. Only platform engineers can. (Prevents privilege escalation.)
3. **Network Policy:** Cross-namespace NetworkPolicy is forbidden unless explicitly allowed (tight isolation, Principle 8).
4. **Resource Policy:** Pods must have CPU and memory requests/limits. Prevents noisy neighbor (Principle 12).
5. **Data Classification:** Pods must be labeled with `data-classification` (public, internal, confidential, restricted). Drives LiteLLM routing (Principle 9).
6. **Backup Policy:** Databases (Crossplane XRs) must specify `backupRetentionDays` and `backupLocation`. Prevents data loss.
7. **MRAP Policy:** Prevents unauthorized CRD activation (Principle 5).

Policies live in Git (capability-cave-policies). Changes go through CI/CD (PR review, merge, automatic Gatekeeper bundle update).

**OPAL (Real-Time Policy Distribution):**

Problem: OPA Gatekeeper traditionally distributes policies via "bundles"—static files synced every 3 minutes (ArgoCD sync interval). If Keycloak roles change (e.g., Alice is promoted to "data-governance" role), Gatekeeper doesn't know for 3 minutes.

OPAL solves this by:
1. **Subscribing to external data sources:** OPAL connects to Keycloak, the tenant metadata store (a database), and the data classification system.
2. **Pushing updates in real-time:** When Alice's role changes in Keycloak, OPAL immediately pushes the new role data to all OPA instances.
3. **Embedding data in policies:** Rego policies can reference this real-time data. Example: "deny if user does not have data-governance role AND resource is marked confidential."

Latency: <100ms from external data change to policy enforcement. Compared to ArgoCD (3–5 minute sync), OPAL is dramatically faster.

**Policy Bundles Signed (Non-Repudiation):**

All policy bundles are signed with a key managed by Guardian (Principle 15). If someone tries to slip in an unsigned policy bundle, OPA rejects it. This prevents a compromised CI/CD system from sneaking in malicious policies.

**Consequences:**

- Rego is a new language. Developers and platform engineers must learn it. Mitigation: CAVE provides policy templates; teams copy and customize.
- Overly strict policies block legitimate use cases. "No root pods ever" is simple; "No root pods except for this one critical service" requires policy exceptions (Constraint templates with exclusions). Exceptions reduce policy simplicity.
- OPA + OPAL adds operational complexity. Both are critical for admission control. Failure = admission control broken. Mitigated by policy "fail-open" mode (allow requests if policy evaluation times out), and by Principle 21 (Survivability).
- Policy distribution is decoupled from GitOps. OPAL pushes updates out-of-band. Auditors must understand that Gatekeeper policies *include* dynamic data from external sources, not just Git.

**Related ADRs:**
ADR-030 (Policy-as-Code Architecture), ADR-089 (OPAL Real-Time Distribution), ADR-131 (Policy Bundle Signing)

---

## 1.13 Principle 12: SLO-Driven FinOps

**Principle Statement:**
Costs are managed by defining Service Level Objectives (SLOs) for each tenant, then automatically enforcing cost limits tied to criticality. Business-critical workloads are never killed due to cost; standard workloads are limited to 150% of budgeted cost; batch workloads are limited to 120%. Per-tenant egress quotas prevent external traffic abuse.

**Why This Principle Exists:**

Cloud costs spiral without governance. A misconfigured autoscaler spins up 100 unnecessary instances. A runaway machine learning job consumes 10 TB of egress. Without controls, these incidents cost thousands of dollars and damage the platform's credibility with finance.

SLOs (Service Level Objectives) are the bridge between business requirements and cost constraints. Every workload has a criticality: business-critical (e.g., customer-facing API), standard (e.g., internal tools), or batch (e.g., nightly analytics job). Cost limits scale with criticality.

**How CAVE Implements It:**

**Criticality Tiers and Cost Budgets:**

Each tenant defines workloads in tiers:
- **Business-Critical (Tier 0):** Customer revenue depends on this. Uptime >99.95%. Kill switch: never. Example: payment processing, product recommendation engine.
- **Standard (Tier 1):** Internal operation, some resilience. Uptime >99%. Kill switch: never, but cost enforcement activates at 150% of budget.
- **Batch (Tier 2):** Nightly jobs, low priority. Uptime varies. Kill switch: activates at 120% of budget.

A tenant submits a budget plan: "We run 3 payment microservices (Tier 0) at ~$2K/month, 10 internal services (Tier 1) at ~$5K/month, and daily ML jobs (Tier 2) at ~$1K/month. Total: $8K/month."

CAVE tracks actual spending. When Tier 0 hits 150% of $2K ($3K), nothing happens. When Tier 1 hits 150% of $5K ($7.5K), CAVE alerts but workloads continue. When Tier 2 hits 120% of $1K ($1.2K), CAVE alerts; if spending continues climbing, batch pods are evicted (killed) to stop the bleed.

**Per-Tenant Egress Quotas (Cilium):**

Egress costs are often the biggest surprise. A developer leaves a loop that hammers a third-party API 1000x per second. Suddenly, TB of egress. Per-tenant egress quotas prevent this:

- Each tenant gets a monthly quota (e.g., 100 GB egress). Cilium (the network policy engine) tracks bytes leaving the cluster.
- When a tenant approaches their quota (e.g., 90%), they're alerted.
- When they exceed it, Cilium can either block further egress (fail-closed) or alert + allow (fail-open). Configurable per tenant.

**Workload Scaling Policies:**

Autoscaling decisions are scoped by cost. A Horizontal Pod Autoscaler (HPA) will scale up to meet demand, but not beyond the tenant's cost budget.

Example policy: "Scale the API from 2 to 10 pods based on CPU. But if scaling would exceed our cost budget by >10%, stay at current replicas."

Implemented via Crossplane Operations (Principle 16): a CronOperation evaluates SLO + cost constraints and decides scaling.

**Real-Time Cost Attribution:**

Every pod, every database, every egress byte is attributed to a tenant. Costs are calculated in real-time (using Prometheus metrics) and compared to budgets every 5 minutes.

Dashboard shows:
- Actual cost YTD
- Projected cost (run rate)
- Budget
- Variance
- Breakdown by workload criticality

**Consequences:**

- Cost limits can be painful. A tenant's batch job gets evicted at 120% of budget. Requires careful budget planning.
- Cost estimation is imperfect. A tenant might provision a database that costs $500/month but Prometheus metrics don't capture all costs (e.g., backup storage). Accepted risk; cost visibility improves over time.
- Egress quotas can cause unexpected outages. If a critical workload legitimately needs more egress, quota enforcement is too strict. Requires ongoing tuning.
- FinOps becomes a cultural change. Teams must think about cost, not just performance. Training and dashboards are essential.

**Related ADRs:**
ADR-096 (SLO-Driven FinOps), ADR-110 (Cost Attribution), ADR-126 (Kill Switch Mechanism)

---

## 1.14 Principle 13: Immutable Infrastructure

**Principle Statement:**
All infrastructure is immutable. Talos Linux is deployed on all Hetzner nodes. Talos provides an API-only interface with no SSH, no shell, no package manager. Nodes are never patched in place; they are destroyed and recreated (Immutable Lifecycle).

**Why This Principle Exists:**

Traditional Linux (Ubuntu, CentOS) allows SSH access and in-place patching. This leads to "snowflake" servers where each node diverges slightly from others. When a security patch is deployed, did it apply correctly to all nodes? Did a sysadmin run a custom script that isn't documented? Debugging becomes a nightmare.

Immutable infrastructure eliminates these issues. Every node is built from a declarative spec. To update a node, you rebuild the entire image from scratch and recreate the node. No patches, no scripts, no ambiguity.

**How CAVE Implements It:**

**Talos Linux:**

Talos Linux is a minimal Linux distribution designed specifically for Kubernetes. Key properties:

- **API-Only:** Talos nodes are managed entirely via gRPC API. There is no SSH server, no bash shell, no interactive login. All operations (reboot, upgrade, generate kubeconfig, inspect logs) go through the Talos API.
- **Immutable:** The root filesystem is read-only. Containers run in overlayfs. If a workload tries to modify `/etc/some-config`, the write fails (or goes to ephemeral storage, lost on reboot).
- **Minimal:** Talos ships with only what's needed to run Kubernetes. No systemd, no pip, no curl. Reduces attack surface by ~90% compared to Ubuntu.
- **Machine Config as Code:** Talos nodes are defined in YAML (machine config). The YAML specifies kernel parameters, kubelet config, container runtime config, networking. OpenTofu generates this YAML and applies it via the Talos API.

**Immutable Lifecycle:**

1. A new Talos version is released (e.g., v1.8.1). CAVE maintainers test it in dev-hetzner.
2. If tests pass, a commit updates the machine config YAML to specify `talosVersion: v1.8.1`.
3. OpenTofu applies this change. Talos initiates a rolling upgrade: each node pulls the new Talos image, reboots, and joins the cluster.
4. During the upgrade, the node is cordoned (no new pods scheduled) and drained (existing pods are evicted to other nodes).
5. If the upgrade fails, OpenTofu rollback is invoked, and nodes are reverted to the previous version.

There is no "patch" step. The entire node image is immutable and versioned.

**Eliminates Categories of Security Issues:**

By removing SSH, shell, and package managers, entire attack vectors disappear:
- No SSH brute-force attacks (because SSH doesn't exist).
- No kernel exploits via local privilege escalation (because there are no local users).
- No supply chain attacks via compromised packages (because packages can't be installed after image build).
- No accidental misconfiguration (because there's no manual config).

**Consequences:**

- **Learning curve:** Operators must unlearn "SSH to server, debug, fix" workflows. Instead, they must understand: "Rebuild the image, redeploy the node."
- **Debugging is different:** You can't SSH to a node and `tail -f /var/log/syslog`. Instead, you use Talos API: `talosctl logs <node>`. All logs go to Loki (Principle 10). Adjustment period is ~2 weeks for ops teams.
- **Not suitable for all cloud providers:** Talos is production-ready on Hetzner (bare metal). On Azure (managed AKS), Talos is overkill because Azure already enforces immutability. We use Talos on Hetzner and AKS on Azure (different philosophies, same outcome).
- **Hardware specifics:** Talos must be compiled for the Hetzner CPU architecture. If a Hetzner generation changes, the Talos image might need recompilation. Mitigated by Hetzner's stability (they rarely change CPU).

**Related ADRs:**
ADR-098 (Immutable Infrastructure), ADR-122 (Talos on Hetzner), ADR-123 (AKS on Azure)

---

## 1.15 Principle 14: Sovereign Auditability

**Principle Statement:**
An immutable, non-repudiable audit trail is maintained for every action taken on CAVE. The Sovereign Ledger (Write-Once-Read-Many + Sigstore) records all actions: Kubernetes API calls, ArgoCD commits, Emergency CLI operations, and AI decisions (Principle 15). Runtime forensics (Tetragon + Hubble) capture system-level activity. Every entry is signed and cryptographically bound to the actor and timestamp.

**Why This Principle Exists:**

Auditability is non-negotiable for compliance. When a regulator asks, "Who deleted customer data and when?" the answer must be immediate and irrefutable. If logs are mutable, the answer is always suspect.

Sovereign auditability also enables forensics. When a security breach occurs, investigators can reconstruct: "What did the attacker do, on which nodes, with which tools?"

**How CAVE Implements It:**

**Sovereign Ledger (WORM Backend):**

The Sovereign Ledger is a custom CAVE component that stores an immutable append-only log. It's built on top of a WORM (Write-Once-Read-Many) backend (e.g., Azure Blob Storage with immutable retention, or Hetzner object storage with object lock).

Every audit event is a ledger entry:
- **Kubernetes API calls:** When a developer deploys a pod, creates a service, or deletes a namespace, the API server sends an event to the Ledger.
- **ArgoCD commits:** When a developer commits a change to the config repo and ArgoCD syncs it, an entry is logged.
- **Emergency CLI operations:** When an on-call engineer uses Emergency CLI to scale a pod, an entry is logged.
- **Policy violations:** When OPA Gatekeeper denies a pod (Principle 11), an entry is logged.
- **AI actions:** When an AI SRE (Principle 15) decides to scale a service, the decision and reasoning are logged.

Each entry includes:
- **Action:** What happened (e.g., "Create Pod")
- **Actor:** Who/what did it (Keycloak user, service account, AI SRE, Emergency CLI user)
- **Timestamp:** When (UTC, ntp-synchronized)
- **Object:** What resource was affected (namespace, pod name, etc.)
- **Result:** Success or failure
- **Signature:** Cryptographically signed by the Ledger (using Sigstore). Changes are impossible without re-signing, which requires the Ledger's private key.

**Sigstore Integration:**

All ledger entries are countersigned using Sigstore (the Linux Foundation's open-source project for supply chain security). Sigstore provides:
- Keyless signing (no private key files; signing is authenticated via OIDC)
- Transparency log (Rekor): Every signature is published to a global transparency log, making backdating impossible.

Example: If someone claims "we have logs showing Alice deleted the database," you can verify:
1. The ledger entry is signed by the Ledger private key (verifiable with the public cert).
2. The signature is in Rekor (global transparency log), timestamped by Rekor, making it impossible to create a false entry with an older timestamp.

**Runtime Forensics (Tetragon + Hubble):**

Tetragon (eBPF-based runtime security) monitors system-level activity:
- Which processes executed
- Which files were accessed/modified
- Which network connections were made
- Which system calls were invoked

All events are logged to Hubble (observability backend) and ultimately to the Sovereign Ledger. When an attacker compromises a pod, Tetragon records every action they take. Post-breach forensics are detailed.

**Audit Trail Walkthrough (End-to-End):**

A developer deploys a new version of `payment-api`:

1. Developer commits a YAML change to Git: `image: harbor.caveplatform.dev/payment-api:v2.3`
2. Git hook (Sigstore) signs the commit. Entry in Sovereign Ledger: "Git commit by Alice, hash abc123, timestamp 14:22:00"
3. ArgoCD detects the change, syncs the repo, and applies the YAML to Kubernetes.
4. Kubernetes API server receives the apply request. Entry in Sovereign Ledger: "Create Deployment by ArgoCD, timestamp 14:22:15"
5. Kyverno (policy controller) validates the image signature (Principle 7). Entry in Sovereign Ledger: "Image signature verified for harbor.caveplatform.dev/payment-api:v2.3, timestamp 14:22:16"
6. Kubelet pulls the image and starts a container.
7. Tetragon monitors the container: network calls, file access, etc. Runtime events streamed to Ledger.

Auditors can now query: "Show all actions on payment-api in the last hour." Response includes git commits, Kubernetes API calls, policy evaluations, and runtime activity. The trail is continuous, signed, and non-repudiable.

**Consequences:**

- The Sovereign Ledger is critical infrastructure. Its outage doesn't block workloads (by Principle 21) but makes forensics impossible until it's recovered.
- WORM storage has costs and retention limits. Ledger data is queryable for 1 year, archived for 7 years (compliance requirement). After 7 years, entries are deleted.
- Cryptographic verification is computationally expensive. Auditors querying large time ranges may experience slow queries. Accepted trade-off for security.
- Privacy: All entries are logged, including PII (if an attacker accesses customer data, logs contain customer PII). Requires careful access control to audit logs (only authorized auditors can view them).

**Related ADRs:**
ADR-093 (Sovereign Ledger Architecture), ADR-106 (Sigstore Integration), ADR-090 (Runtime Forensics)

---

## 1.16 Principle 15: Autonomous Operations — APOL

**Principle Statement:**
The CAVE Autonomous Platform Operations Layer (APOL) enables four AI roles—AI SRE, AI Compliance Officer, AI FinOps Governor, and AI Change Manager—to perform routine operations with zero human approval for low-risk decisions. All AI decisions produce reasoning traces logged to the Sovereign Ledger. Constitutional protections prevent AI from modifying critical artifacts. Target: 0 FTE ops, 1+ constitutional guardian.

**Why This Principle Exists:**

A human on-call engineer can respond to ~3–5 incidents per week before fatigue sets in. A team of 10 engineers handles ~50 incidents/week. But a growing platform generates hundreds of operational events daily (alerts, deployments, policy drifts, cost anomalies).

Autonomous AI operations handle routine decisions 24/7 without fatigue. Humans remain for exceptional decisions and strategy.

**How CAVE Implements It:**

**Four AI Roles:**

Each role has a well-defined scope, authority, and audit trail. Implemented via cave-ctl MCP (Principle 2):

1. **AI SRE (Site Reliability Engineer Role)**
   - Scope: Scaling, restarts, anomaly detection, incident mitigation
   - Authority: Adjust replica counts, restart pods, drain nodes, scale databases (up to 20% of budget)
   - Tools: Prometheus metrics (Principle 10), Prophet (time series forecasting), River (online ML for drift detection)
   - Example decision: "CPU usage is trending upward for the past hour. Projected to exceed 85% in 15 minutes. Scaling payment-api from 5 to 8 replicas. Reason: maintain golden signal (CPU <80%)."
   - Logging: Decision + reasoning → Sovereign Ledger, signed by AI SRE cert
   - Audit: If the scaling decision was wrong (e.g., caused more instability), it's auditable. Humans can review and adjust AI policies.

2. **AI Compliance Officer Role**
   - Scope: Policy drift detection, RBAC anomalies, certificate expiry, data classification drift
   - Authority: Alert, block invalid policies, require remediation within 24 hours
   - Tools: OPAL (Principle 11), policy evaluations, Keycloak RBAC queries
   - Example decision: "Pod in acme-team namespace does not have data-classification label. This violates OPA policy. Deleting pod (after 24-hour warning). Reason: policy enforcement without exception."
   - Logging: Policy violation + AI decision → Sovereign Ledger
   - Audit: Humans can override (with explicit approval). If a pod was deleted incorrectly, remediation is to re-deploy and file a bug.

3. **AI FinOps Governor Role**
   - Scope: Cost predictions, right-sizing recommendations, zombie tenant cleanup, egress quota enforcement (Principle 12)
   - Authority: Forecast costs, recommend right-sizing, kill long-idle resources, emit alerts for budget overruns
   - Tools: Cost metrics from Prometheus, linear regression, spend baselines
   - Example decision: "tenant-xyz has 3 pods running at <5% CPU utilization for 30+ days. Cost: $200/month. Recommendation: consolidate to 1 pod (save $133/month). Humans must approve consolidation (to preserve dev environments), but the analysis is automatic."
   - Logging: Cost analysis → Sovereign Ledger
   - Audit: Recommendations are not enforced automatically; humans decide. Prevents AI from aggressively deleting resources.

4. **AI Change Manager Role**
   - Scope: Dependency updates, canary deployments, chaos validation, SLO verification, attestation
   - Authority: Merge Renovate PRs (dependency updates) → trigger CI → deploy canary → run chaos tests → verify SLO → merge to main if all checks pass
   - Tools: Renovate (dependency updater), GitOps (ArgoCD), chaos testing (via Principle 16), SLO verifier
   - Example decision: "Renovate detected openssl 3.1 → 3.2 update. Merged PR. CI passed. Deploying canary (10% traffic). Running 5-minute chaos test (network failures, latency injection). SLO p99 latency: 95ms (target 100ms). SLO error rate: 0.01% (target <1%). Chaos test passed. Merged to main. Reason: automation speeds safe releases."
   - Logging: Every step (merge, CI, canary, chaos, SLO check) → Sovereign Ledger with timestamps
   - Audit: If a canary deployment went wrong, the full trace is available. Rollback is immediate (ArgoCD reverts the commit).

**Constitutional Layer (Protection Against AI Misuse):**

Not all artifacts can be modified by AI. The Constitutional Layer protects critical artifacts:

- **XRDs (Crossplane Resource Definitions):** Define the platform's abstraction layer. Only humans (2-of-3 guardian multi-sig + hardware key) can modify.
- **Core OPA Policies:** Gatekeeper admission policies. Only humans (2-of-3 guardian) can modify.
- **ADRs (Architecture Decision Records):** The platform's principles and constraints. Only humans (2-of-3 guardian) can add/modify.
- **Sovereign Ledger Config:** The audit system itself. Only humans (2-of-3 guardian) can modify.
- **Identity Root:** Keycloak root certs and trust anchors. Only humans (2-of-3 guardian + hardware key) can modify.

If an AI attempts to modify a constitutional artifact, the request is denied at the API layer. Signed by the Constitution Layer enforcer.

**Reasoning Traces + Redaction (ADR-128):**

Every AI decision produces a reasoning trace: "I observed X metric, compared to baseline Y, concluded Z, took action W."

These traces are logged to Sovereign Ledger. However, traces might contain sensitive data (e.g., "User Alice's password was logged in an error, which AI SRE saw while analyzing logs"). Traces are redacted before logging:

- PII is redacted (Presidio, Principle 9)
- Customer data is summarized (e.g., "10 queries in top 1% latency" instead of customer names)
- Internal secrets are stripped

Redacted traces are suitable for audit review. Non-redacted traces are kept for ~24 hours in memory, then deleted. If a breach occurs, forensics can recover non-redacted traces from backups (under strict access control).

**Target: 0 FTE Ops, 1+ Constitutional Guardian:**

The vision: CAVE runs itself. Four AI roles handle 99% of operational tasks. Humans transition to:
- Constitutional Guardians (2–3 FTE): Approve XRD changes, policy updates, ADR modifications. Review exceptional AI decisions. On-call for true emergencies.
- Platform Architects (2–3 FTE): Design new features, evaluate new tools, long-term roadmap.
- Developer Experience Engineers (1–2 FTE): Improve Backstage UI, fix template bugs, support developers.

No "site reliability engineers" writing on-call runbooks or manually remediating alerts. Total ops cost drops from 10–15 FTE to 5–8 FTE, redirected to higher-value work.

**Consequences:**

- AI misuse is a real risk. If AI is too aggressive (e.g., scaling services without understanding side effects), platform reliability suffers. Requires careful tuning of AI policies and close human oversight initially.
- Constitutional Layer adds governance overhead. Changing an OPA policy requires 2-of-3 human approval. Slower than ad-hoc changes, but prevents accidental misconfigurations.
- Reasoning traces are auditable but not fool-proof. An AI might have faulty logic that passes all checks. Requires continuous monitoring of AI performance (e.g., "What % of AI scaling decisions were correct in hindsight?").
- Early adoption of autonomous operations is risky. We recommend: Phase 1 (AI SRE + Compliance Officer, low-risk tasks), Phase 2 (AI FinOps Governor, cost recommendations only, no enforcement), Phase 3 (AI Change Manager, canary deployments), Phase 4 (full autonomy with constitutional safeguards).

**Related ADRs:**
ADR-112 (APOL Architecture), ADR-125 (Constitutional Layer), ADR-128 (Reasoning Trace Redaction)

---

## 1.17 Principle 16: Two-Tier Automated Remediation

**Principle Statement:**
Remediation complexity is matched to tools: simple, single-resource issues use Crossplane Operations (CronOperation, WatchOperation). Complex, multi-step workflows use the Reflex Engine (KEDA triggers + Argo Workflows). Both produce attestations logged to Sovereign Ledger.

**Why This Principle Exists:**

Some remediation is simple: "If CPU >85%, scale up." Others are complex: "If a deployment is stuck (pods pending for >5min), check resource quotas, check node capacity, check image pull secrets, suggest remediation." Different complexity levels need different tools.

Crossplane Operations are declarative and Kubernetes-native. Argo Workflows are full-featured orchestration engines. Using the right tool for each job prevents mismatch (using Argo for simple tasks wastes complexity; using Crossplane for complex tasks causes frustration).

**How CAVE Implements It:**

**Tier 1: Crossplane Operations (Simple Remediation)**

Crossplane Operations are declarative, single-step remediations:

- **CronOperation:** Runs on a schedule. Example: "Every day at 2 AM, prune old backups older than 30 days."
- **WatchOperation:** Runs when a watched resource changes. Example: "If a Database XR is marked for deletion, trigger a backup before tearing down."

Example CronOperation (YAML):

```yaml
apiVersion: batch.crossplane.io/v1alpha1
kind: CronOperation
metadata:
  name: backup-cleanup
spec:
  schedule: "0 2 * * *"  # Daily at 2 AM
  template:
    spec:
      operation: DeleteOldBackups
      parameters:
        olderThanDays: 30
        snapshotLocation: s3://cave-backups/
```

Crossplane interprets this, runs the operation, and logs the result to Sovereign Ledger. If the operation fails, it's retried with exponential backoff.

**Tier 2: Reflex Engine (Complex Remediation)**

The Reflex Engine is a workflow orchestrator built on Argo Workflows. It handles multi-step decisions:

Example workflow (pseudocode):
```
Event: Pod stuck in Pending for 5+ minutes
Trigger: Watch Pod events, if pending >5min, start workflow

Step 1: Check resource quotas (is namespace quota exceeded?)
  If yes: Alert tenant, recommend scaling quota
  If no: Continue to Step 2

Step 2: Check node capacity (is there space to schedule?)
  If no: Alert platform team, recommend scaling cluster
  If yes: Continue to Step 3

Step 3: Check image pull (is image pulling failing?)
  If yes: Alert tenant, recommend checking image registry access
  If no: Continue to Step 4

Step 4: Human escalation
  Create incident ticket, assign on-call engineer
  Log all diagnostic info collected above
```

The Reflex Engine is programmatic (not declarative). Workflow definitions are in Argo Workflow YAML or Go SDK. Supports conditionals, loops, and external API calls. Much more powerful than Crossplane Operations.

**Both Produce Attestations:**

Whether remediation is via Crossplane Operation or Reflex Engine, the result is logged to Sovereign Ledger:
- What problem was detected
- What remediation was taken
- Whether it succeeded
- Any side effects (e.g., pod was evicted)

Attestations are signed. Auditors can trace every auto-remediation decision.

**Consequences:**

- Reflex Engine is more powerful but harder to reason about. A workflow with many conditional branches is easy to get wrong.
- Crossplane Operations are simple but limited. Not all remediations fit the CronOperation or WatchOperation model. Temptation to use Reflex Engine for everything.
- Both require testing and validation. A broken remediation is worse than no remediation (it might make things worse). Extensive chaos testing is required (Principle 7).

**Related ADRs:**
ADR-095 (Remediation Strategy), ADR-119 (Crossplane Operations), ADR-128 (Attestation Logging)

---

## 1.18 Principle 17: Exit Strategy Built-In

**Principle Statement:**
Every Azure managed service has an equivalent on Hetzner, and vice versa. The platform is designed for provider portability. An annual portability drill exports workload definitions, data, and keys, then re-imports them into the alternate provider to verify no lock-in. Failure to complete the drill in one week is a platform failure.

**Why This Principle Exists:**

Vendor lock-in is an existential risk. If CAVE is tightly coupled to Azure (using AKS, Azure SQL, Azure Key Vault, Azure Cognitive Services), migrating to Hetzner becomes a multi-year effort. Costs can skyrocket if a provider increases prices. Service disruptions (e.g., region outage) are unrecoverable.

Annual portability drills enforce the exit strategy. They also drive a design principle: keep abstraction layers (Principle 5, Crossplane XRDs) clean so cross-provider portability is a matter of rewriting Compositions, not rewriting applications.

**How CAVE Implements It:**

**Provider-Equivalent Mapping:**

- **Azure Database for Postgres** ↔ **Hetzner Managed Postgres (or self-hosted via Crossplane)**
- **Azure Blob Storage** ↔ **Hetzner S3 (MinIO or Wasabi)**
- **Azure Cosmos DB** ↔ **MongoDB (self-hosted or managed)**
- **Azure Cache for Redis** ↔ **Hetzner Managed Redis (or self-hosted)**
- **Azure Cognitive Search** ↔ **OpenSearch (managed or self-hosted)**
- **AKS** ↔ **Talos on Hetzner**
- **Azure Key Vault** ↔ **HashiCorp Vault (self-hosted)**

Crossplane Compositions ensure applications don't know which provider they're using. A Composition can target either provider based on profile selection.

**Annual Portability Drill:**

Every year (typically during a scheduled maintenance window):

1. **Export Phase (1 day):** ArgoCD exports all workload definitions (Deployments, Services, ConfigMaps, Secrets). Data is exported from databases (pg_dump, mysqldump). TLS keys are exported. Result: a portable backup.

2. **Validation Phase (1 day):** The backup is validated: all required keys present, all images accessible, all config valid.

3. **Re-Import Phase (2 days):** A temporary Hetzner cluster (or Azure cluster, if current cluster is on Hetzner) is spun up. Workloads and data are imported. Sanity tests run (can pods start? Can they reach databases? Can they serve traffic?).

4. **Cross-Provider Verification (2 days):** Traffic is gradually shifted from the original cluster to the re-imported cluster. If everything works, the drill succeeds.

5. **Rollback (1 day):** Traffic is shifted back. The temporary cluster is deleted.

Failure at any stage is a Platform Failure. The root cause is investigated. Changes are made to improve portability. The drill is retried.

**Benefits of Annual Drills:**

- Discover lock-in early. If re-importing fails, we know months before we need to actually migrate.
- Train the team. Engineers practice migrations. When a real migration is needed, they're experienced.
- Validate backups. Export/import is tested annually, so we know our backup strategy works.
- Update documentation. Each drill reveals outdated runbooks, undocumented assumptions, etc.

**Consequences:**

- Annual drills are expensive (temporary infrastructure, engineering time). But cheaper than discovering lock-in during a crisis.
- If the drill fails, platform work halts until it's fixed. High priority but disruptive.
- Provider incompatibilities might be discovered late. Example: Azure has a managed Postgres feature X, Hetzner doesn't. Working around it requires application changes. Ideal: discover during drill, not in production.

**Related ADRs:**
ADR-066 (Portability and Exit Strategy), ADR-122 (Hetzner Profile), ADR-123 (Azure Profile)

---

## 1.19 Principle 18: ADRs for Every Decision

**Principle Statement:**
Every significant architectural decision is documented in an Architecture Decision Record (ADR). 130+ ADRs form the platform's decision history. Constitutional artifacts (XRDs, core OPA policies, ADR set, Ledger config, identity root) require 2-of-3 guardian multi-sig approval.

**Why This Principle Exists:**

Architecture decisions are easy to forget. A year after CAVE is built, new team members ask: "Why do we use two provisioning layers instead of one?" If the decision is not documented, the answer is guesswork. Cargo cult development follows.

ADRs create institutional memory. They also slow down decisions (writing an ADR takes time), which is intentional: important decisions deserve deliberation.

**How CAVE Implements It:**

**ADR Template and Content:**

Each ADR follows a standard template:

1. **Title:** "ADR-XXX: [Short decision description]"
2. **Status:** Draft | Proposed | Accepted | Deprecated
3. **Context:** Why this decision was needed. What problem does it solve?
4. **Decision:** What was decided. State clearly and concisely.
5. **Alternatives:** Other options that were considered and rejected. Include rejection rationale.
6. **Consequences:** Positive and negative outcomes. Trade-offs. What becomes harder/easier?
7. **Compliance Mapping:** Which compliance frameworks (SOC 2, ISO 27001, NIS2, GDPR) does this decision address?
8. **Resolved By:** Which ADR supersedes this one (if any)?

Example ADR (stub):

```
ADR-067: Two-Layer Provisioning (OpenTofu Day 0, Crossplane Day 1+)

Status: Accepted

Context:
Infrastructure provisioning spans two phases:
- Day 0: Cluster bootstrap (one-time, imperative)
- Day 1+: Application dependencies (ongoing, declarative)

Mixing both in Terraform creates unclear boundaries.

Decision:
Use OpenTofu for Day 0. Use Crossplane for Day 1+. ArgoCD orchestrates both.

Alternatives:
- Single Tool (Terraform for everything): Rejected because it blurs the boundary between bootstrap and operations.
- Infrastructure as Containers (Operator-only): Rejected because it's immature; OpenTofu is battle-tested.

Consequences:
+ Clear separation of concerns
+ Easier to understand Day 0 vs Day 1 operations
- Two tools to maintain
- Developers must understand both layers (brief learning curve)

Compliance Mapping:
- SOC 2: Supports auditability (each layer is versioned)
- ISO 27001: Supports change management (ArgoCD provides versioning and review)
```

**130+ ADRs Covering:**

- Architecture patterns (Principles 1–7, 10, 11)
- Provisioning and infrastructure (Principles 4, 5, 13)
- Security and compliance (Principles 7, 8, 14, 18)
- Operations and reliability (Principles 12, 15, 16)
- Specific tool choices (Backstage, ArgoCD, Crossplane, OPA, Tetragon, Kong, etc.)
- Profile definitions (Principle 3)
- Data classification (Principle 9)
- Cost models (Principle 12)
- Governance (Principle 11, 15, 18)

**Constitutional Artifacts Require Guardian Multi-Sig:**

Five artifact categories are "constitutional." Changes require 2-of-3 guardian approval (each guardian has a hardware security key):

1. **XRDs:** Crossplane Composite Resource Definitions. These define the platform's abstraction language.
2. **Core OPA Policies:** Gatekeeper admission policies protecting CAVE integrity.
3. **ADR Set:** The ADRs themselves. Adding a principle or decision is a meta-decision requiring guardians.
4. **Ledger Config:** Sovereign Ledger retention, signing, and archival policies.
5. **Identity Root:** Keycloak root certs, trust anchors, IdP integrations.

Changes to constitutional artifacts go through a formal review:
1. Proposer drafts the change (ADR + code).
2. All three guardians review (async, 1 week deadline).
3. At least 2 guardians sign the change (via hardware key).
4. Change is committed to Git with 2-of-3 signatures.
5. ArgoCD syncs the change.

This slows down changes but prevents accidental platform breakage (e.g., an overzealous engineer deletes a critical OPA policy, breaking admission control for all tenants).

**Consequences:**

- ADR process adds process overhead. A simple decision might require a week (for 2-of-3 guardian review) instead of being implemented in a day.
- ADRs accumulate over time. After 3 years, 200+ ADRs exist. Finding the relevant one requires good indexing (CMS, search tools).
- Guardian quorum is critical. If a guardian is unavailable for a week, platform changes stall. Mitigated by rotating guardians and overlapping availability.

**Related ADRs:**
ADR-020 (ADR Process), ADR-125 (Guardian and Constitutional Layer)

---

## 1.20 Complexity Budget

**Principle Statement:**
CAVE has a finite complexity budget. No new core component is added without removing or consolidating an existing one (unless justified by a compliance ADR). Complexity is measured by Guardian Onboarding Time (GOT): the time for a new guardian to understand and operate CAVE. GOT ≤ 2 weeks is the target. If a change increases GOT by >10%, it exceeds the budget and requires escalation.

**Why This Principle Exists:**

Every tool added to CAVE increases operational burden. Kong manages traffic, ArgoCD manages deployments, Crossplane manages infrastructure, OPA manages policy, Tetragon manages runtime security, Ollama manages AI. Each tool requires learning, debugging, and maintenance. Too many tools = burnout.

A complexity budget forces prioritization. Before adding a new tool, ask: "What will we remove?" This discipline keeps CAVE lean and maintainable.

**How CAVE Implements It:**

**Guardian Onboarding Time (GOT):**

GOT is measured quarterly. A new guardian is given CAVE infrastructure (empty Hetzner cluster, documentation, Backstage access) and asked to perform five operational tasks:

1. Deploy a sample application (Backstage template → deployment)
2. Debug a performance issue (use Grafana, identify root cause)
3. Approve a policy change (read ADR, review proposed Gatekeeper policy, sign it)
4. Perform a disaster recovery drill (restore from backup, verify)
5. Investigate a security incident (use Tetragon logs, Sovereign Ledger, identify attacker actions)

GOT is the median time to complete all five tasks across all new guardians (last 3 hired). Target: ≤ 2 weeks. Current (as of 2026): ~9 days.

**Measuring Complexity:**

- **Tool count:** How many core tools (Kubernetes, ArgoCD, Crossplane, OPA, Kong, Ollama, Tetragon, etc.)? Target: ≤ 12.
- **Configuration files:** How many ConfigMaps, Secrets, CRDs? Aim for <5000 total across all namespaces.
- **Alert rules:** How many Prometheus alert rules? Target: <200 (reduce noise, false positives).
- **Custom policies (Rego):** How many lines of Rego policy code? Target: <10K lines (tight, readable policies).

**Phase 4 Exempt:**

Phase 4 (prod-hybrid, cross-cloud failover) is allowed to temporarily exceed the complexity budget because regulatory compliance may require it. But Phase 4 is subject to annual GOT review. If GOT exceeds 2 weeks due to Phase 4, remediation is required (e.g., automation, better documentation).

**Annual Pruning Review:**

Every year, architecture review identifies tools/components that are underutilized or duplicative. Examples:

- If Prometheus and Thanos are both deployed but only Thanos is queried, consolidate.
- If two secret management tools are running, pick one and deprecate the other.
- If an older ADR is superseded by a newer decision, deprecate the old ADR.

Pruning reduces complexity and improves GOT.

**Consequences:**

- Adding new tools is hard (requires removing something). Slows innovation.
- GOT is imperfect. A metric doesn't capture all complexity (e.g., a tool with 100 knobs is more complex than its LOC suggests).
- Tight complexity budget might force suboptimal choices. Example: "We'd prefer Tool B, but Tool A is already deployed, so we'll stick with Tool A." Accept some inefficiency for simplicity.

**Related ADRs:**
ADR-014 (Complexity Budget), ADR-127 (Tool Consolidation)

---

## 1.21 Survivability Invariant

**Principle Statement:**
No single control-plane component failure may cause tenant data loss or forced workload termination. Tenant workloads continue running during any single platform component failure. Only Kong (API gateway) failure causes external traffic interruption. All other failures degrade management capabilities, not tenant runtime.

**Why This Principle Exists:**

An IDP's primary function is to run tenant workloads reliably. Platform failures are acceptable if they don't cascade to tenants. A broken ArgoCD doesn't kill running pods. A crashed Prometheus doesn't interrupt traffic. Only external traffic (through Kong) loss is acceptable as a minor degradation.

This principle drives resilience design for every platform component.

**How CAVE Enforces It:**

**Failure Modes:**

1. **Kubernetes API Server Failure:** Workloads continue. New pods can't be scheduled. Management operations fail. Acceptable. Mitigation: API server is HA (3+ replicas with etcd quorum).

2. **Kong (API Gateway) Failure:** External traffic halts until Kong recovers. Acceptable because Kong is per-profile HA (multiple replicas). Single Kong pod failure = other Kong pod handles traffic.

3. **ArgoCD Failure:** Workloads continue. Git changes don't auto-sync (manual kubectl required, via Emergency CLI). Acceptable. Mitigation: ArgoCD is HA (multiple replicas, separate etcd).

4. **Prometheus Failure:** Workloads continue. Observability is unavailable. Acceptable (short term). Mitigation: Prometheus is replicated; queries hit backup if primary fails.

5. **Tetragon Failure:** Workloads continue. Runtime security is unavailable. Degradation accepted (unmonitored workloads). Mitigation: Tetragon runs on every node; failure of one node's Tetragon is isolated.

6. **Crossplane Failure:** Existing databases/buckets run. New provisioning is blocked. Acceptable (day 1+ operations degrade, day 0 workloads unaffected). Mitigation: Crossplane is stateless; restart and reconciliation is fast.

7. **OPA Gatekeeper Failure:** Existing pods continue. New pod admission fails (fail-open or fail-closed, configurable). If fail-closed, workload deployments are blocked (unacceptable). Mitigated by: Gatekeeper has multiple replicas; failure of one replica doesn't block.

**Verification via Chaos Testing:**

The Survivability Invariant is tested quarterly via chaos testing (see §43):

- Kill a random Kubernetes API server pod. Verify workloads continue, new scheduling is blocked.
- Kill all Kong pods. Verify external traffic halts, internal traffic (pod-to-pod) continues.
- Kill ArgoCD pod. Verify workloads continue, git sync is unavailable.
- Simulate network partition (partition a node from the cluster). Verify workloads on the node continue, but node doesn't accept new scheduling.
- Corrupt an etcd database. Verify the corrupted data is recoverable from backup, and workloads don't suffer during recovery.

If chaos testing violates the Survivability Invariant, it's a critical bug. P0 severity.

**Consequences:**

- HA for all critical components is expensive. Multiple replicas, load balancers, and backup storage.
- Some failures cause degradation (loss of observability, loss of new scheduling). Not ideal, but acceptable.
- Chaos testing is time-consuming (1 week per quarter). But it's the only way to verify survivability at scale.

**Related ADRs:**
ADR-054 (Resilience Architecture), ADR-099 (Chaos Testing Strategy)

---

## 1.22 Compliance Mapping

CAVE's architecture principles map to major compliance frameworks:

| Framework | Principles | Key Controls |
|-----------|-----------|--------------|
| **SOC 2 Type II** | 6 (GitOps), 14 (Sovereign Ledger), 18 (ADRs) | Audit trail, change management, access control |
| **ISO 27001** | 7 (Security), 8 (Isolation), 11 (Policy), 14 (Auditability) | Information security, access management, incident response |
| **NIS2 Directive** | 7, 14, 15, 16 | Cybersecurity governance, incident reporting, supply chain |
| **GDPR** | 8 (Isolation), 9 (Data Classification), 11 (Policy), 14 (Ledger) | Data protection, consent, right to erasure, DPA |
| **HIPAA** | 7 (Encryption), 8 (Isolation), 14 (Ledger) | Audit controls, encryption, access controls |

Platform engineers and compliance teams collaborate to map each principle to specific compliance requirements.

---

## 1.23 Related ADRs

**Grouped by Category:**

**Architecture & Design:**
ADR-014 (Complexity Budget), ADR-015 (Control Plane Architecture), ADR-020 (ADR Process), ADR-021 (Profile Architecture), ADR-025 (Backstage Plugin Architecture)

**Provisioning & Infrastructure:**
ADR-067 (OpenTofu + Crossplane Boundary), ADR-119 (Crossplane v2), ADR-122 (Hetzner Profile), ADR-123 (Azure Profile), ADR-124 (MRAP Policy)

**Security & Compliance:**
ADR-007 (Secret Management), ADR-077 (Zero-Trust Networking), ADR-101 (Image Signing), ADR-105 (SBOM), ADR-106 (SLSA + Sigstore), ADR-093 (Sovereign Ledger), ADR-090 (Runtime Forensics)

**Operations & Reliability:**
ADR-026 (GitOps), ADR-029 (Observability), ADR-054 (Resilience), ADR-095 (Remediation), ADR-096 (FinOps), ADR-110 (Cost Attribution), ADR-099 (Chaos Testing)

**Multi-Tenancy & Isolation:**
ADR-012 (Multi-Tenancy), ADR-084 (vCluster Design)

**AI & Autonomy:**
ADR-009 (Self-Hosted AI), ADR-013 (LiteLLM), ADR-103 (Data Classification Routing), ADR-111 (Presidio), ADR-112 (APOL), ADR-125 (Constitutional Layer), ADR-128 (Reasoning Trace Redaction)

**Policy & Governance:**
ADR-030 (Policy-as-Code), ADR-089 (OPAL Real-Time Distribution), ADR-131 (Policy Bundle Signing)

**Portability & Exit:**
ADR-066 (Portability Strategy)

---

## 1.24 Related Runbook Sections

- **§02 — Installation & Bootstrapping:** How to deploy CAVE using profiles
- **§05 — Backstage Developer Portal:** Navigating the UI, creating templates
- **§10 — Crossplane XRD Reference:** All available resource definitions
- **§11 — GitOps Workflows:** Committing changes, ArgoCD sync, drift resolution
- **§15 — OPA Policy Reference:** Writing and debugging policies
- **§20 — Observability & Dashboards:** Prometheus, Loki, Grafana queries
- **§25 — FinOps & Cost Management:** Tracking costs, setting budgets, kill switches
- **§30 — Security Hardening:** mTLS, image signing, Tetragon policies
- **§35 — Incident Response & Emergency CLI:** Using the incident-only control plane
- **§40 — Autonomous Operations (APOL):** Monitoring AI SRE, Compliance Officer, FinOps Governor
- **§43 — Chaos Testing & Validation:** Running the quarterly survivability test
- **§50 — Disaster Recovery:** Backups, restores, data exports for portability drills

---

**End of §01**

*Next: §02 — Installation & Bootstrapping*
