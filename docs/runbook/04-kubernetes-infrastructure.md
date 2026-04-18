# CAVE Platform Runbook §04 — Kubernetes Infrastructure

**Document Version:** 2.1
**Last Updated:** 2026-03-08
**Status:** Production
**Audience:** Platform Engineers, Site Reliability Engineers, Operations Teams

---

## 4.1 Overview

Kubernetes is the foundational orchestration layer of the CAVE (Cloud-Agnostic Virtualized Environment) Internal Developer Platform. The CAVE platform implements a deliberately dual-infrastructure strategy—not to accommodate multiple vendors, but to validate architectural decisions and maintain deployment flexibility as organizational requirements evolve.

The CAVE platform operates two distinct Kubernetes implementations optimized for their respective cloud providers:

**Hetzner Deployment:** Self-managed Kubernetes clusters built on Talos Linux, an immutable, API-first Linux distribution designed specifically for Kubernetes. Talos eliminates the entire SSH/shell attack surface by providing infrastructure state management exclusively through the Kubernetes API. Clusters run bare-metal or dedicated cloud instances with complete operational control.

**Azure Deployment:** Managed Kubernetes via Azure Kubernetes Service (AKS), eliminating control-plane operational overhead while maintaining identical developer experiences through consistent networking, storage, and security abstractions above the K8s layer.

Both implementations are provisioned and managed identically via OpenTofu declarative infrastructure-as-code, ensuring that regardless of which cloud environment a team deploys to, the operational patterns, troubleshooting procedures, and upgrade sequences remain consistent. This "infrastructure agnosticism" is not about vendor independence for its own sake—it is about reducing cognitive load on engineering teams and preventing platform silos that create technical debt.

---

## 4.2 Architectural Decisions & ADR Rationale

This section explains the design decisions that shaped CAVE's Kubernetes infrastructure, with explicit rejection of viable alternatives.

### ADR-003: Kubernetes Distribution Selection

**Context:** CAVE required a self-managed Kubernetes distribution for Hetzner deployments that could scale to hundreds of nodes while remaining operationally simple. The organization needed immutable infrastructure—no SSH access, no drift risk, no late-night troubleshooting sessions in production systems.

**Decision:** Talos Linux for Hetzner; AKS for Azure.

**Rationale:** Talos Linux is the only production-grade Kubernetes distribution that treats the host operating system as an implementation detail, not a configuration surface. Every aspect of a Talos node—kernel parameters, container runtime, kubelet configuration—is declared through the Kubernetes Talos API and applied atomically. When an update is required, the entire node state is rolled forward to a new immutable image. If a node becomes unstable, it is destroyed and recreated within minutes. This eliminates entire categories of production incidents caused by configuration drift.

**Rejected Alternatives:**

1. **kubeadm on Ubuntu/Debian:** While kubeadm is the reference implementation for Kubernetes, it leaves the host OS as a mutable configuration surface. SSH access is required for troubleshooting, which introduces security risk and enables ad-hoc configuration changes that diverge from infrastructure-as-code. Once three or four SSH troubleshooting sessions have accumulated on a production node, reproducibility is lost.

2. **k3s (Rancher):** Lightweight, single-binary distribution optimized for edge and resource-constrained environments. Not suitable for CAVE because: (a) not production-grade HA for large clusters, (b) missing enterprise features like advanced etcd management and certificate rotation, (c) single-vendor (Rancher) dependency creates lock-in that contradicts CAVE's cloud-agnostic philosophy.

3. **RKE2 (Rancher):** More capable than k3s but still embedded in Rancher's ecosystem. Operational procedures, upgrade sequences, and troubleshooting are tied to Rancher's release cycle. The distribution requires adoption of Rancher's security and networking models.

4. **MicroK8s (Canonical):** Snap-based distribution from Canonical. Operationally, snaps introduce a second package manager (alongside apt/dpkg) which complicates deployment and increases the attack surface. Canonical's EOL timeline and commercial support model are separate concerns from Kubernetes.

5. **Kubespray (Ansible-based):** Comprehensive but fundamentally mutable. Kubespray uses Ansible to configure nodes, which means configuration lives partially in Ansible playbooks and partially in Kubernetes manifests. Drift emerges naturally as engineers SSH into nodes to troubleshoot and apply ad-hoc fixes.

**Why Talos Linux Wins:** Talos is immutable by design, not by policy. The Linux kernel, kubelet, container runtime, networking stack, and system services are all configured through the Talos API—no SSH, no shell, no exceptions. This eliminates entire categories of incidents. Upgrades are atomic: either the node transitions to the new state or it rolls back automatically. There is no in-between state where a partial upgrade leaves the node unstable. Talos is also deeply aligned with CNCF principles: it is maintained by Siderolabs (formerly SideroLabs) and pursues the lowest-possible-toil operational model.

### ADR-062: Azure Infrastructure Configuration

**Context:** CAVE's Azure deployments needed the simplicity of a managed Kubernetes service while retaining network and storage configuration flexibility.

**Decision:** Azure Kubernetes Service (AKS) with custom networking and storage drivers.

**Rationale:** AKS abstracts away the Kubernetes control plane (API server, scheduler, etcd, controller manager), removing the operational burden of managing these components. Microsoft guarantees SLAs for control-plane availability, which CAVE does not need to re-engineer. What CAVE retains operational control over: node pools (size, scaling, lifecycle), networking (Cilium CNI, network policies), storage (CSI drivers), RBAC (Entra ID integration), and observability. This division of responsibility is clean and maintainable.

**Rejected Alternatives:**

1. **Self-Managed Kubernetes on Azure VMs:** Possible, but introduces operational toil identical to Hetzner without the immutability guarantees of Talos. CAVE would need to manage kubelet versions, etcd backups, and certificate rotation manually—unnecessary work when Azure offers AKS.

2. **EKS (Amazon):** Not applicable; CAVE is not deployed on AWS in the initial phase.

3. **GKE (Google Cloud):** Not applicable; CAVE is not deployed on GCP in the initial phase.

4. **OpenShift on Azure:** Enterprise Kubernetes platform from Red Hat. Rejected because: (a) Red Hat licensing cost is substantial, (b) OpenShift enforces its own security model and container runtimes, limiting flexibility, (c) operational complexity exceeds CAVE's requirement for simplicity.

### ADR-098: Immutable Infrastructure Paradigm

**Context:** CAVE infrastructure incidents historically stem from configuration drift, where production systems diverge from version-controlled infrastructure definitions due to manual changes, partial upgrades, or incomplete rollback attempts.

**Decision:** Treat all Kubernetes nodes as cattle, not pets. Never patch a node; instead, destroy and recreate it with the desired state.

**Rationale:** Immutability eliminates 80% of infrastructure troubleshooting. When a node has an unknown configuration state, it cannot be patched back to "normal"—there is no normal. Instead, it is deprovisioned and replaced with a new instance built from a immutable image. This is fast (often 2-5 minutes for Talos nodes), reliable, and reproducible. Every node is created from the same image, ensuring deterministic behavior across the cluster.

This paradigm requires: (a) fast node provisioning, which Talos and AKS both support, (b) workload scheduling that tolerates node churn, which is native Kubernetes behavior, (c) no persistent state on nodes themselves, which is enforced by stateless container design and persistent volume claims (PVCs) for storage.

---

## 4.3 Distribution Comparison Matrix

The following matrix evaluates self-hosted Kubernetes distributions on criteria relevant to CAVE's operational model:

| Criterion | Talos | kubeadm | k3s | RKE2 | Kubespray |
|-----------|-------|---------|-----|------|-----------|
| **Security (immutability)** | 5 | 1 | 2 | 2 | 1 |
| **SSH Attack Surface** | 5 (none) | 1 (required) | 2 (optional) | 2 (optional) | 1 (required) |
| **HA Readiness** | 5 | 4 | 2 | 4 | 4 |
| **API Management** | 5 | 2 | 1 | 2 | 1 |
| **Upgrade Simplicity** | 5 | 2 | 3 | 3 | 2 |
| **Community & Support** | 4 | 5 | 5 | 4 | 4 |
| **Resource Footprint (small)** | 4 | 3 | 5 | 4 | 3 |
| **GitOps Integration** | 5 | 3 | 2 | 3 | 2 |
| **Observability** | 5 | 4 | 3 | 4 | 4 |
| **Production Track Record** | 4 | 5 | 4 | 3 | 4 |

**Key Observations:**

- Talos scores highest on security, immutability, and API-driven management, which aligns with CAVE's operational philosophy.
- kubeadm scores highest on community support and production track record, but lowest on immutability—a critical requirement for CAVE.
- k3s excels in resource efficiency and simplicity, but lacks enterprise HA features.
- RKE2 and Kubespray both introduce vendor or configuration complexity that CAVE seeks to avoid.

---

## 4.4 Twenty-Four-Month Roadmap Analysis

The CAVE Kubernetes infrastructure is designed to accommodate anticipated developments in the ecosystem over 24 months (through March 2028).

### Talos Linux Evolution (2026–2028)

**Current Stable Version:** Talos 1.12.x (as of Q1 2026)

**Anticipated Features in Talos v1.13–v1.15:**

- **OOM Handler Enhancements:** Talos v1.12 introduced an improved out-of-memory (OOM) handler that gracefully terminates low-priority workloads before system-critical services are affected. Future versions will refine scoring heuristics to consider workload quality-of-service (QoS) class, ensuring best-effort pods are reaped before burstable pods.

- **Staged Networking:** A feature in development that allows incremental network configuration updates without requiring full node restart. This enables canary deployments of networking changes (MTU adjustments, encryption parameters) to be tested on a single node before rolling out cluster-wide.

- **User Volumes:** Talos v1.12 introduced support for custom persistent volumes mounted at the node level, enabling use cases where workloads require host-level storage that is not suitable for Kubernetes PVCs (e.g., caching layers, ephemeral bulk processing).

- **Omni Platform Integration:** SideroLabs' Omni platform provides a unified control plane for managing multiple Talos clusters across clouds. In the 24-month window, Omni will mature to GA with features including: multi-cluster workload scheduling, global load balancing, and centralized policy enforcement. CAVE may adopt Omni if managing 5+ Kubernetes clusters becomes operational overhead.

- **CNCF Alignment:** Talos is pursuing deeper integration with CNCF projects (Cilium, Kubewarden, etc.) and contributing upstream to Kubernetes to influence platform-level improvements.

### AKS Evolution (2026–2028)

**Anticipated Developments:**

- **Kubernetes Version Support:** Azure maintains support for three minor versions of Kubernetes. AKS will upgrade to v1.33, v1.34, v1.35 as they are released upstream, with typical release cycle of ~3 months between minor versions.

- **Karpenter General Availability:** Karpenter (AWS/open-source node autoprovisioning tool) is being adopted by AKS as a native feature. By 2027, expect Karpenter to be GA for AKS, replacing the legacy cluster autoscaler for advanced workload-aware node scaling.

- **Node Autoprovision:** AKS is developing "just-in-time" VM provisioning—workloads request node specs (CPU, memory, accelerators), and AKS automatically provisions matching VMs on-demand. This eliminates the need to pre-configure node pools for heterogeneous workloads.

- **Confidential Compute:** Azure's confidential computing capabilities (Intel SGX, AMD SEV) will be deeper integrated into AKS. CAVE will evaluate these for workloads handling sensitive data.

### Kubernetes Upstream (2026–2028)

**Anticipated Versions:** Kubernetes v1.32–v1.35

**Notable Features to Monitor:**

- **Gateway API Maturity:** Gateway API is the successor to Ingress, providing better abstractions for HTTP routing, load balancing, and cross-namespace configuration. By v1.33–v1.34, expect Gateway API to reach GA and become the default ingress abstraction.

- **Structured Logging:** Kubernetes logging is transitioning from unstructured text to structured JSON. This improves observability and enables more sophisticated log aggregation in CAVE's observability stack (Datadog, Prometheus, Loki).

- **Kubelet Credentialing Improvements:** Ongoing work to improve how workloads authenticate to cloud APIs (AWS IAM, Azure Entra ID, GCP Workload Identity). CAVE will benefit from simpler, more secure workload credential management.

---

## 4.5 Kubernetes Architecture

### Hetzner Deployment Architecture

The Hetzner deployment consists of a self-managed Talos Kubernetes cluster running on Hetzner Cloud dedicated instances or bare-metal servers.

**Control Plane Nodes:** Three dedicated Talos nodes (typically CX32 or larger in "production" profile, smaller in "dev" profile) run the Kubernetes control-plane components: API server, scheduler, controller manager, and etcd. Talos bundles these into a single immutable image that runs on each control-plane node. There is no separate "master" role—Talos nodes run all components and self-elect leaders.

**etcd:** Embedded in Talos control-plane nodes. Three-node consensus ensures quorum for split-brain protection. Encryption at rest is enabled using AES-256-GCM. Regular snapshots (hourly) are taken to an external object storage (Hetzner S3-compatible storage) to enable disaster recovery.

**API Server Load Balancing:** The Kubernetes API is exposed via a Hetzner Load Balancer (Layer 4 TCP load balancer) that distributes traffic across the three control-plane API servers. This eliminates the need for external load-balancer software (like HAProxy on a separate VM) and reduces operational complexity.

**Worker Nodes:** A scalable pool of Talos worker nodes (no control-plane components) run tenant workloads. Node count ranges from 3 (minimal) to 50+ (production) depending on workload requirements. All nodes run identical Talos images, ensuring consistent container runtime versions, kubelet behavior, and network configuration.

**Container Runtime:** Talos uses containerd (CNCF standard), not Docker, eliminating the overhead and complexity of the Docker daemon.

**CNI (Container Network Interface):** Cilium is the network plugin, providing eBPF-powered network policies, service load balancing, and observability. Calico and Flannel were evaluated but rejected in favor of Cilium for superior eBPF-based performance and policy expressiveness.

**Storage:** Hetzner CSI driver provides persistent volume support. Volumes are Hetzner block storage instances that are attached to worker nodes on demand. etcd backups are stored in Hetzner S3-compatible object storage.

**Upgrades & Rotations:** Kubernetes upgrades on Talos are atomic and rolling. The cluster triggers a rolling upgrade of the Talos image across all nodes sequentially. Each node reboots once and either transitions to the new image or rolls back automatically if health checks fail. The entire upgrade process is orchestrated through the Talos API—no manual intervention required.

### Azure Deployment Architecture

**AKS Managed Control Plane:** The Kubernetes control plane (API server, scheduler, etcd, controller manager) is entirely managed by Azure. CAVE does not provision, upgrade, or troubleshoot these components; Azure provides SLA guarantees (typically 99.95% availability for production SKU).

**System Node Pool:** 3 nodes (by default) reserved for platform system components: DNS (CoreDNS), network policy enforcement (Cilium), and observability agents. These nodes run managed images provided by AKS and are upgraded by Azure in coordination with control-plane updates.

**User Node Pools:** Additional node pools for tenant workloads. Each pool can have independent scaling policies, VM types, and availability zones. For instance, a "compute-optimized" pool might run C-series VMs for CPU-intensive workloads, while a "gpu-pool" might run GPU-accelerated VMs for ML workloads.

**Karpenter (or Autoscaler):** For Azure deployments, CAVE uses Karpenter (in preview or GA, depending on the month) for intelligent node autoprovisioning. Karpenter watches for pending workloads and automatically provisions appropriately-sized VMs to schedule them, consolidating workloads to fewer nodes over time to reduce waste.

**Networking:** Azure CNI (Azure-native networking plugin) in overlay mode with Cilium overlay for advanced network policies. This provides both Azure native integrations (e.g., Network Security Groups) and Cilium's eBPF-powered policies.

**Storage:** Azure Disk CSI driver for block storage (similar to persistent volumes on Hetzner), and Azure Files CSI for NFS/SMB file shares. All storage is managed through Kubernetes PersistentVolumeClaims and StorageClasses.

**RBAC & Identity:** AKS integrates with Azure Entra ID (formerly Azure Active Directory). Workloads authenticate to Azure services using Workload Identity (Azure's implementation of OIDC-based service account credentials). This eliminates the need to manage API keys or connection strings in Kubernetes secrets.

### Per-Profile Node Sizing

CAVE defines seven operational profiles, each with different node counts and sizes:

| Profile | Control Plane | Worker Nodes | CPU per Node | RAM per Node | Use Case |
|---------|---------------|--------------|--------------|--------------|----------|
| **dev-local** | 1 | 1 | 2 | 4 GB | Local development, Hetzner |
| **dev-azure** | 1 | 2 | 2 | 4 GB | Azure development, managed control plane |
| **staging-hetzner** | 3 | 5 | 4 | 8 GB | Pre-production testing, self-managed |
| **staging-azure** | — | 5 | 4 | 8 GB | Pre-production testing, AKS |
| **production-hetzner** | 3 | 10–20 | 8 | 16 GB | Production workloads, autoscaling enabled |
| **production-azure** | — | 10–20 | 8 | 16 GB | Production workloads, Karpenter autoscaling |
| **high-perf** | 3 | 5–15 | 16 | 32 GB | GPU workloads, large compute jobs |

Nodes are provisioned on demand via OpenTofu, with autoscaling rules triggered by CPU/memory utilization and pending workload queue depth.

---

## 4.6 Use Cases & Developer Scenarios

### Scenario 1: Bootstrapping a New Hetzner Cluster

A new development team joins CAVE and needs a dedicated Hetzner cluster for their workloads.

**Process:**
1. Platform engineer defines cluster configuration in OpenTofu (node count, region, Talos version).
2. OpenTofu creates the underlying Hetzner infrastructure: VMs, load balancer, networks.
3. Talos machines are initialized with their configuration (API server endpoints, kubelet join tokens) via the Talos API.
4. Kubernetes nodes form quorum, etcd initializes, and the API server becomes available.
5. Container network (Cilium) is deployed via Helm.
6. The cluster is operational, accessible via kubectl, with a dashboard (e.g., Kubernetes Dashboard or a custom UI) available.
7. The entire process takes 15–20 minutes, fully automated.

### Scenario 2: Kubernetes Version Upgrade on Talos

A new Kubernetes version (e.g., 1.33) is released. CAVE operators decide to upgrade the Hetzner cluster.

**On Talos:**
1. OpenTofu or a Kubernetes operator (e.g., Renovate) detects the new Talos version (which bundles a new Kubernetes version).
2. The new Talos image is downloaded to each node sequentially.
3. Nodes reboot in a controlled, rolling fashion. Worker nodes reboot first (with workload eviction); control-plane nodes follow.
4. Health checks verify that the node rejoined the cluster and is reporting ready status.
5. If any node fails health checks, it automatically rolls back to the previous image.
6. Total time: 5–10 minutes for a 10-node cluster.

**On AKS:**
1. Azure detects that a new Kubernetes version is available and notifies CAVE operators.
2. CAVE operators trigger a cluster upgrade via Azure portal or CLI.
3. Azure upgrades the managed control plane (no downtime, handled by Azure internally).
4. System node pool is upgraded automatically.
5. User node pools are upgraded on a schedule defined by CAVE (e.g., rolling, canary, or batch).
6. Total time: 30–60 minutes depending on node pool size and deployment strategy.

### Scenario 3: Scaling AKS Node Pool for New Tenant

A new tenant signs up for CAVE and their workloads require 8 additional nodes.

**Process:**
1. Tenant workloads are deployed to AKS with resource requests (CPU, memory).
2. Karpenter observes that workloads are pending (cannot be scheduled due to insufficient node capacity).
3. Karpenter provisions new VMs in AKS automatically to accommodate the workloads.
4. Workloads are scheduled and running within 2–3 minutes.
5. As tenant workloads scale down, Karpenter consolidates workloads to fewer nodes and deprovisions unused VMs, reducing costs.

### Scenario 4: Handling Node Failure on Hetzner

A Talos node experiences a disk failure and becomes unresponsive.

**Process:**
1. The kubelet on the failed node fails to report back to the API server.
2. Kubernetes marks the node as NotReady after a grace period (default: 5 minutes).
3. Workloads on the failed node are evicted and rescheduled to healthy nodes.
4. A platform engineer triggers node deletion via OpenTofu or a Kubernetes operator.
5. OpenTofu destroys the failed Hetzner VM.
6. OpenTofu creates a new Hetzner VM, initializes Talos, and joins it to the cluster.
7. The cluster returns to full capacity. Total recovery time: 10–15 minutes.

### Scenario 5: etcd Backup and Restore

Operators perform regular etcd backups to protect against catastrophic cluster failure.

**Process:**
1. A scheduled Kubernetes CronJob runs hourly on a cluster control-plane node.
2. The job calls `talosctl etcd snapshot` to export an encrypted snapshot.
3. The snapshot is uploaded to Hetzner S3-compatible object storage.
4. If disaster strikes (e.g., all three control-plane nodes fail), the snapshot is retrieved.
5. A new cluster is bootstrapped from the snapshot, restoring all Kubernetes objects and application data.
6. RTO (Recovery Time Objective): ~30 minutes; RPO (Recovery Point Objective): ~1 hour (depending on backup frequency).

### Scenario 6: Adding a GPU Node Pool

A tenant workload requires GPU acceleration. CAVE operators add a GPU-enabled node pool.

**Process:**
1. Platform engineer defines a new node pool in OpenTofu with GPU-equipped VMs (e.g., NVIDIA A100).
2. Nodes are provisioned, joined to the cluster, and labeled with `accelerator=gpu`.
3. Tenant specifies GPU requirements in their workload manifest (e.g., `nvidia.com/gpu: 1`).
4. Kubernetes scheduler routes the workload to the GPU node pool.
5. NVIDIA GPU device plugin exposes GPU resources to containers.
6. The workload accesses GPU hardware directly, enabling ML inference/training.

---

## 4.7 Configuration Reference

This section explains the OpenTofu configuration that provisions Hetzner and Azure infrastructure. Key decisions and rationale are annotated.

### Hetzner OpenTofu Configuration (Excerpt)

```hcl
# Hetzner Talos Cluster Definition
# Why: Self-managed K8s provides operational control and immutability via Talos
resource "hcloud_network" "cave" {
  name = "cave-network-${var.profile}"
  # Network isolation: Each profile gets a dedicated VPC-equivalent (Hetzner network)
  # Why: Prevents cross-profile traffic leakage, enables multi-tenancy at infra level
}

resource "hcloud_server" "control_plane" {
  count           = var.control_plane_count # 3 for HA
  name            = "talos-cp-${count.index}"
  server_type     = var.control_plane_instance_type # CX32, CX42 depending on profile
  image           = "custom_talos_image_id" # Immutable Talos image (built upstream)
  network_id      = hcloud_network.cave.id

  # Why CX32+: Talos control plane (API server, scheduler, etcd) requires stable CPU
  # 2 vCPU is minimum; 4+ vCPU recommended for production (handles surge traffic)

  labels = {
    "node-type" = "control-plane"
    "profile"   = var.profile
  }

  # User data: Talos machine configuration passed via Hetzner API
  user_data = file("talos-controlplane-config.yaml")
  # Why YAML API: Machine config is immutable; changes require node recreation, not patch
}

resource "hcloud_load_balancer" "api_lb" {
  name              = "talos-api-${var.profile}"
  load_balancer_type = "lb11" # Layer 4, TCP passthrough
  # Why Layer 4: API server needs raw TCP; Layer 7 would add latency and inspection overhead

  algorithm {
    type = "round_robin"
  }

  service {
    protocol = "tcp"
    listen_port = 6443 # Kubernetes API default port
    destination_port = 6443
    health_check {
      protocol = "tcp"
      port     = 6443
      interval = 10
      timeout  = 5
    }
  }

  target {
    type = "server"
    server_id = hcloud_server.control_plane[*].id # All control-plane servers as targets
  }
}

resource "hcloud_volume" "etcd_backup_storage" {
  name     = "etcd-backups-${var.profile}"
  size     = 100 # GB; stores compressed etcd snapshots
  format   = "xfs"
  # Why separate volume: etcd backups are critical; isolating storage prevents loss if node disk fills
}

resource "hcloud_server" "workers" {
  count       = var.worker_node_count
  name        = "talos-worker-${count.index}"
  server_type = var.worker_instance_type # CX22, CX32, CX42 depending on workload
  image       = "custom_talos_image_id"
  network_id  = hcloud_network.cave.id

  user_data = file("talos-worker-config.yaml")

  labels = {
    "node-type" = "worker"
    "profile"   = var.profile
  }
}
```

**Key Rationale:**

- **Immutable Images:** Talos images are built from source (via `talosctl gen image`) and uploaded to Hetzner. No runtime configuration is stored on nodes; all state is in the image.
- **Load Balancer:** API server is exposed through Hetzner's managed load balancer, eliminating the need to run external LB software.
- **Network Isolation:** Each CAVE profile (dev, staging, production) gets a separate Hetzner network to prevent cross-profile traffic and enable soft multi-tenancy.

### Azure OpenTofu Configuration (Excerpt)

```hcl
# Azure AKS Cluster Definition
# Why: Managed control plane reduces operational burden; AKS handles HA, upgrades, etcd
resource "azurerm_resource_group" "cave" {
  name            = "rg-cave-${var.profile}"
  location        = var.azure_region
}

resource "azurerm_kubernetes_cluster" "cave" {
  name                = "aks-cave-${var.profile}"
  location            = azurerm_resource_group.cave.location
  resource_group_name = azurerm_resource_group.cave.name
  dns_prefix          = "cave-${var.profile}"

  kubernetes_version = "1.33" # AKS-supported version; auto-managed by Azure
  # Why managed: CAVE doesn't manage control-plane upgrades; Azure does

  default_node_pool {
    name            = "system"
    node_count      = 3 # System workloads (DNS, networking, monitoring)
    vm_size         = "Standard_B4ms" # Burstable, cost-optimized for system pods
    # Why separate: System pods (CoreDNS, Cilium, etc.) are isolated from user workloads
  }

  identity {
    type = "SystemAssigned" # AKS-managed identity for cloud API access
    # Why: Avoids need to manage service account keys; tied to cluster lifecycle
  }

  network_profile {
    network_plugin = "azure" # Azure CNI for native VNet integration
    network_policy = "azure" # Cilium in overlay mode for advanced policies
    # Why Azure CNI + Cilium: Native Azure networking + eBPF-powered policies

    service_cidr       = "10.0.0.0/16"
    dns_service_ip     = "10.0.0.10"
    docker_bridge_cidr = "172.17.0.1/16"
  }

  oms_agent {
    log_analytics_workspace_id = azurerm_log_analytics_workspace.cave.id
    # Why: Observability built-in; logs sent to Azure Monitor for alerting
  }

  depends_on = [
    azurerm_role_assignment.aks_sp_role
  ]
}

resource "azurerm_kubernetes_cluster_node_pool" "compute" {
  name                   = "compute"
  kubernetes_cluster_id  = azurerm_kubernetes_cluster.cave.id
  node_count             = var.compute_pool_node_count
  vm_size                = "Standard_D4s_v3" # General-purpose VMs for typical workloads
  auto_scaling_enabled   = true
  min_count              = 2
  max_count              = 20 # Allow elasticity during peak demand

  labels = {
    "workload" = "compute"
  }

  # Why autoscaling: Reduces cost during off-peak; scales elastically during demand spikes
}

resource "azurerm_kubernetes_cluster_node_pool" "gpu" {
  name                   = "gpu"
  kubernetes_cluster_id  = azurerm_kubernetes_cluster.cave.id
  node_count             = 0 # Karpenter will provision on-demand
  vm_size                = "Standard_NC6s_v3" # NVIDIA GPU VMs
  auto_scaling_enabled   = true
  min_count              = 0
  max_count              = 10

  labels = {
    "accelerator" = "gpu"
  }

  # Why GPU node pool: Keeps expensive GPU nodes separate; scales to zero when idle
}
```

**Key Rationale:**

- **Managed Control Plane:** AKS handles Kubernetes API server, etcd, scheduler, controller manager. CAVE focuses on node pools and networking.
- **System Node Pool:** Reserved for platform system components (DNS, networking, monitoring). Isolated from user workloads to prevent resource contention.
- **User Node Pools:** Separate pools for different workload types (compute, GPU, memory-intensive). Each can have independent scaling policies.
- **Networking:** Azure CNI provides native VNet integration (traffic routed via Azure Network Security Groups). Cilium overlay mode adds eBPF-powered network policies.

---

## 4.8 Operations

### Kubernetes Version Upgrades

**On Hetzner (Talos):**

Talos embeds a specific Kubernetes version in each release. Upgrading Kubernetes is inseparable from upgrading Talos.

1. **Check Available Versions:**
   ```
   talosctl upgrade --help
   ```
   Lists available Talos versions (each bundles a Kubernetes version).

2. **Plan the Upgrade:**
   Operators review the release notes and decide on a maintenance window (typically outside business hours).

3. **Initiate Rolling Upgrade:**
   ```
   talosctl upgrade --nodes <IP> --image ghcr.io/siderolabs/talos:v1.13.0
   ```
   Talos orchestrates a rolling update: worker nodes first, then control-plane nodes, one at a time.

4. **Verify Health:**
   ```
   kubectl get nodes
   kubectl get pods -A
   ```
   After each node reboots, verify it rejoined the cluster and workloads are healthy.

5. **Rollback (if necessary):**
   If issues arise, nodes can be rolled back to the previous Talos version:
   ```
   talosctl upgrade --nodes <IP> --image ghcr.io/siderolabs/talos:v1.12.0
   ```

**On Azure (AKS):**

AKS control plane upgrades are handled by Azure; node pool upgrades are coordinated by CAVE.

1. **Monitor Available Upgrades:**
   Azure notifies CAVE when new Kubernetes versions are available. Operators plan the upgrade window.

2. **Upgrade Control Plane:**
   ```
   az aks upgrade --resource-group rg-cave-prod --name aks-cave-prod --kubernetes-version 1.34
   ```
   Azure updates the managed control plane (no cluster downtime). Takes 10–20 minutes.

3. **Upgrade Node Pools:**
   System node pool upgrades automatically with control plane. User node pools require explicit upgrade:
   ```
   az aks nodepool upgrade --resource-group rg-cave-prod --cluster-name aks-cave-prod --name compute --kubernetes-version 1.34
   ```
   Nodes are replaced in a rolling fashion (old nodes drained, new nodes provisioned).

4. **Verify Cluster Health:**
   ```
   kubectl get nodes
   kubectl get pods -A
   ```

### Node Rotation

**Hetzner (Talos):**

Nodes are rotated as part of regular maintenance or when security patches require a full image rebuild.

1. **Cordon the Node** (prevent new workloads from scheduling):
   ```
   kubectl cordon <node-name>
   ```

2. **Drain Existing Workloads:**
   ```
   kubectl drain <node-name> --ignore-daemonsets --delete-emptydir-data
   ```
   Workloads are evicted and rescheduled on other nodes.

3. **Destroy the Node:**
   ```
   talosctl reset --nodes <IP> --graceful=true --reboot=true
   ```
   Talos performs a graceful shutdown and wipes the disk.

4. **Remove from Cluster:**
   ```
   kubectl delete node <node-name>
   ```

5. **Provision Replacement:**
   OpenTofu creates a new Hetzner VM, initializes Talos, and joins the cluster:
   ```
   terraform apply -target=hcloud_server.workers[N]
   ```

**Azure (AKS):**

Azure manages system node pool rotation as part of control-plane upgrades. User node pools can be rotated manually:

1. **Cordon and Drain** (same as Hetzner).
2. **Scale Down Node Pool:**
   ```
   az aks nodepool scale --resource-group rg-cave-prod --cluster-name aks-cave-prod --name compute --node-count <reduced-count>
   ```
3. **Scale Back Up:**
   Azure provisions fresh VMs in the node pool, and Kubernetes schedules workloads.

### etcd Maintenance

**Backup Strategy:**

Hetzner clusters perform hourly etcd snapshots:

1. **Automated Backup:**
   A Kubernetes CronJob runs on a control-plane node:
   ```
   talosctl -n <control-plane-IP> etcd snapshot
   ```

2. **Store Snapshot:**
   Snapshots are compressed and uploaded to Hetzner S3-compatible object storage with encryption.

3. **Retention Policy:**
   Snapshots are kept for 30 days (after which they are deleted automatically).

**Restore Procedure:**

If etcd becomes corrupted or cluster is lost:

1. **Retrieve Snapshot:**
   Download the latest snapshot from object storage.

2. **Provision New Control-Plane Nodes:**
   Create three new Talos nodes.

3. **Restore from Snapshot:**
   ```
   talosctl -n <new-control-plane-IP> etcd restore <snapshot-file>
   ```

4. **Verify Cluster:**
   The new cluster is restored with all Kubernetes objects from the time of the snapshot.

**Azure Note:** AKS manages etcd backup automatically; CAVE operators are not responsible for etcd snapshots.

### Certificate Rotation

**Hetzner (Talos):**

Talos automatically rotates Kubernetes API certificates before expiration.

1. **Monitor Certificate Expiry:**
   ```
   talosctl kubeconfig > ~/.kube/config
   openssl x509 -in ~/.kube/config -text -noout | grep -A 2 "Not Valid"
   ```

2. **Automatic Rotation:**
   Talos triggers certificate rotation (valid for 365 days by default) 90 days before expiration.

3. **Manual Rotation (if needed):**
   ```
   talosctl gen certs
   ```

**Azure (AKS):** AKS manages certificate rotation automatically; no operator action required.

---

## 4.9 Troubleshooting

### Issue 1: Node NotReady

**Symptom:** `kubectl get nodes` shows a node in NotReady state.

**Diagnosis:**
```
kubectl describe node <node-name>
kubectl get events --all-namespaces | grep <node-name>
```

**Common Causes:**
- Node is out of memory or disk space.
- Kubelet process has crashed.
- Network connectivity is lost.

**Resolution:**
- For Talos: Drain the node, destroy it, and provision a replacement (immutability principle).
- For AKS: Azure may automatically replace the node. If not, manually delete the node via Azure CLI.

### Issue 2: Pending Workloads (Cannot Schedule)

**Symptom:** Pods remain in Pending state even though nodes are ready.

**Diagnosis:**
```
kubectl describe pod <pod-name>
kubectl get events --namespace <namespace>
```

**Common Causes:**
- Insufficient CPU/memory resources on nodes.
- PVC not provisioned (storage issue).
- Taints on nodes that don't match pod tolerations.

**Resolution:**
- Scale worker nodes to add capacity.
- Verify PVCs are bound to storage backends.
- Check node taints: `kubectl describe node <node-name> | grep Taints`.

### Issue 3: etcd High Latency

**Symptom:** API server becomes slow; kubectl commands timeout.

**Diagnosis:**
```
talosctl -n <control-plane-IP> etcd member list
talosctl -n <control-plane-IP> etcd alarm list
```

**Common Causes:**
- High load on control-plane nodes.
- Disk I/O bottleneck.
- Network latency between control-plane nodes.

**Resolution:**
- Increase control-plane node resources (CPU, RAM).
- Defragment etcd: `talosctl etcd defrag`.
- Migrate workloads from control-plane nodes (if running user workloads there, which is not recommended).

### Issue 4: CNI Plugin (Cilium) Pod Crashes

**Symptom:** Cilium pods in kube-system namespace are in CrashLoopBackOff.

**Diagnosis:**
```
kubectl logs -n kube-system ds/cilium
kubectl describe pod -n kube-system <cilium-pod-name>
```

**Common Causes:**
- Insufficient node resources (CNI requires kernel memory).
- eBPF kernel version too old (Cilium requires Linux 4.9+).
- Conflicting network policy configuration.

**Resolution:**
- Ensure nodes are running Talos/Linux kernel 5.10+.
- Reduce Cilium verbosity: adjust helm values, redeploy.
- Check for conflicting NetworkPolicy objects: `kubectl get networkpolicies -A`.

### Issue 5: Hetzner Load Balancer Health Check Failures

**Symptom:** Load balancer marks API servers as unhealthy, despite nodes being ready.

**Diagnosis:**
```
talosctl -n <control-plane-IP> health
kubectl cluster-info
```

**Common Causes:**
- API server is slow to respond (high load).
- Network policies are blocking health check traffic.
- Load balancer target port (6443) is misconfigured.

**Resolution:**
- Verify API server is responsive: `curl -k https://localhost:6443/healthz`.
- Check health check timeout in Hetzner LB configuration; increase if necessary.
- Ensure network policies allow health check traffic on port 6443.

### Issue 6: Persistent Volume Attachment Failure

**Symptom:** Pod cannot mount PVC; CSI driver reports attachment failure.

**Diagnosis:**
```
kubectl describe pvc <pvc-name>
kubectl describe pod <pod-name>
kubectl logs -n kube-system <csi-attacher-pod>
```

**Common Causes:**
- CSI driver is not deployed or running.
- Volume limit exceeded on the node.
- Cloud API (Hetzner/Azure) rejected the attachment request.

**Resolution:**
- Verify CSI driver is running: `kubectl get daemonset -n kube-system | grep csi`.
- Check node volume limits: `kubectl describe node <node-name> | grep Allocated`.
- Check cloud API logs (Hetzner/Azure portal) for errors.

### Issue 7: High CPU Usage on Control-Plane

**Symptom:** Control-plane nodes consume 80%+ CPU continuously.

**Diagnosis:**
```
talosctl -n <control-plane-IP> top
kubectl top nodes
kubectl top pods -A --sort-by=cpu
```

**Common Causes:**
- Excessive API requests from applications.
- CrashLoopBackOff pods repeatedly creating/destroying.
- Large cluster (100+ nodes) straining the controller manager.

**Resolution:**
- Identify and fix misbehaving workloads.
- Implement API server rate limiting: `.apiServer.flags.enable-priority-and-fairness=true`.
- Scale control-plane vertically (larger instance types).

### Issue 8: Network Policy Blocks Inter-Pod Communication

**Symptom:** Pods cannot communicate even though they should be allowed by NetworkPolicy.

**Diagnosis:**
```
kubectl get networkpolicies -A
kubectl describe networkpolicy <policy-name> -n <namespace>
cilium policy trace -s <source-pod> -d <destination-pod>
```

**Common Causes:**
- NetworkPolicy selector is too restrictive.
- Cilium is denying by default (no allow rules).
- Pod labels don't match policy selector.

**Resolution:**
- Review and relax NetworkPolicy selectors.
- Ensure pods have labels matching the policy.
- Test with an explicit allow rule: `kubectl apply -f allow-all-policy.yaml`.

### Issue 9: AKS Node Pool Fails to Scale

**Symptom:** Node pool does not provision new nodes despite Karpenter/autoscaler requests.

**Diagnosis:**
```
az aks nodepool show --resource-group rg-cave-prod --cluster-name aks-cave-prod --name compute
kubectl logs -n karpenter deployment/karpenter
```

**Common Causes:**
- VM quota exceeded in Azure subscription.
- Node pool max-count limit reached.
- Insufficient Azure permissions for the identity.

**Resolution:**
- Request quota increase from Azure portal.
- Increase max-count: `az aks nodepool update --max-count 30`.
- Verify AKS identity has necessary permissions.

### Issue 10: Workload Eviction During Node Upgrade (Talos)

**Symptom:** Critical workloads are killed during rolling node upgrades.

**Diagnosis:**
```
kubectl describe pod <critical-pod> | grep "Reason\|Message"
```

**Common Causes:**
- Pod disruption budget (PDB) not configured.
- Pod has no eviction-friendly topology spread.

**Resolution:**
- Define a PodDisruptionBudget: `minAvailable: 2` for critical workloads.
- Use topology spread constraints to distribute pods across multiple nodes.
- Example:
  ```yaml
  apiVersion: policy/v1
  kind: PodDisruptionBudget
  metadata:
    name: critical-app-pdb
  spec:
    minAvailable: 2
    selector:
      matchLabels:
        tier: critical
  ```

---

## 4.10 Compliance Mapping

CAVE's Kubernetes infrastructure is designed to satisfy compliance frameworks relevant to enterprise software platforms.

### CIS Kubernetes Benchmark

CAVE follows CIS (Center for Internet Security) recommendations:

- **Access Control:** AKS integrates with Entra ID for RBAC. Talos disables SSH, reducing attack surface.
- **Network Policies:** Cilium enforces NetworkPolicy resources, enabling micro-segmentation.
- **Pod Security Standards:** Workloads run in restricted Pod Security Standards (no privileged containers, no host mounts).
- **Audit Logging:** Kubernetes API audit logs are collected in central observability stack (Datadog/Loki).
- **Image Scanning:** All container images are scanned for vulnerabilities before deployment.

**Score:** Estimated 85%+ compliance with CIS Kubernetes Benchmark v1.2.1.

### SOC 2 Type II

CAVE's infrastructure supports SOC 2 Type II compliance:

- **Access Control:** Only authorized platform engineers can provision/modify clusters. Changes are audit-logged.
- **Change Management:** All infrastructure changes via OpenTofu are version-controlled and require review (pull request).
- **Disaster Recovery:** etcd backups enable RTO <1 hour and RPO <1 hour.
- **Monitoring & Alerting:** Real-time alerts for security events, node failures, and application errors.
- **Encryption:** Data in transit (TLS), data at rest (volume encryption), etcd encryption at rest.

### ISO 27001

CAVE aligns with ISO 27001 Information Security Management:

- **Asset Management:** Infrastructure inventory is maintained in OpenTofu, enabling asset tracking.
- **Access Control:** RBAC via Entra ID (Azure) or Kubernetes RBAC (Hetzner). Principle of least privilege enforced.
- **Cryptography:** TLS 1.3 for all APIs. AES-256 for etcd encryption.
- **Monitoring:** 24/7 monitoring via observability stack. Incident response playbooks in place.

### NIS2 (Network and Information Security Directive 2)

CAVE meets NIS2 requirements (EU Directive 2022/2555):

- **Incident Response:** Automated alerting and escalation for security incidents.
- **Risk Assessment:** Regular penetration testing and vulnerability assessments.
- **Supply Chain Security:** Container images are scanned; dependencies are tracked.
- **Backup & Recovery:** Disaster recovery procedures tested quarterly.

---

## 4.11 Related ADRs

- **ADR-003:** Kubernetes Distribution Selection (Talos Linux vs. alternatives)
- **ADR-062:** Azure Infrastructure Configuration (AKS design decisions)
- **ADR-098:** Immutable Infrastructure Paradigm (destroy-and-recreate philosophy)
- **ADR-015:** Container Runtime Selection (containerd)
- **ADR-024:** Network Plugin Selection (Cilium)
- **ADR-087:** Storage CSI Strategy (persistent volume architecture)

---

## 4.12 Related Runbook Sections

- **§2 — Infrastructure Fundamentals:** OpenTofu basics, cloud provider setup, DNS configuration.
- **§5 — Container Registries & Image Management:** Building and pushing container images, scanning for vulnerabilities.
- **§36 — Observability & Monitoring:** Collecting metrics, logs, traces from Kubernetes clusters.
- **§43 — Disaster Recovery & Backup:** etcd snapshots, cross-region failover, application-level backups.

---

## Conclusion

The CAVE Kubernetes infrastructure is purpose-built for operational simplicity, security, and multi-cloud flexibility. By standardizing on Talos Linux for self-managed clusters and AKS for managed deployments, CAVE achieves a consistent developer experience while maintaining the operational efficiency and immutability guarantees required for enterprise production workloads.

The use of declarative infrastructure-as-code (OpenTofu), immutable node images, and policy-driven security (Cilium, Kubernetes RBAC, Pod Security Standards) eliminates entire categories of production incidents and enables teams to scale their workloads with confidence.

As the Kubernetes ecosystem matures—particularly with features like Karpenter, Gateway API, and Workload Identity—CAVE's architecture is designed to evolve without disrupting running workloads or requiring architectural rethinking.

---

**Document History**

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 2.1 | 2026-03-08 | Platform Team | Added Talos v1.12 features, Karpenter integration, 24-month roadmap |
| 2.0 | 2025-12-15 | Platform Team | Initial production release, Cilium CNI documentation |
| 1.0 | 2025-09-01 | Platform Team | Draft, early stage CAVE development |

---

**Approvals**

- **Platform Architecture Lead:** [Signature] — Date: 2026-03-08
- **SRE Lead:** [Signature] — Date: 2026-03-08
- **Security Lead:** [Signature] — Date: 2026-03-08
