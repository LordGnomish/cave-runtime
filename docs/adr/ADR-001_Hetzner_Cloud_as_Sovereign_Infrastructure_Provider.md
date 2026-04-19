# ADR-001: Hetzner Cloud as Sovereign Infrastructure Provider

**Status:** Accepted

**Scope:** Hetzner

**Category:** Infrastructure

**Related ADRs:** 002 (Azure), 003 (Talos), 061 (Storage), 065 (Load Balancer), 066 (Provider Choice), 098 (Immutable Infra)

## Context

CAVE requires a primary infrastructure provider for the sovereign (self-hosted) deployment target. The provider must support:

- EU data sovereignty (GDPR Art.44-49 compliance without adequacy decision concerns)
- Bare-metal or VM compute for Kubernetes
- Cost efficiency for a startup/small-team platform engineering environment
- API-driven infrastructure provisioning (OpenTofu compatibility)
- Network isolation capabilities (firewalls, private networking, load balancers)
- Object storage or block storage for persistent workloads

The sovereign profile is CAVE's "home" environment — the platform engineering playground where full control is non-negotiable.

---


## Candidates

### 3.1 Provider Comparison Matrix

| Criteria | Hetzner Cloud | AWS EU (Frankfurt) | GCP EU | OVHcloud | Scaleway | DigitalOcean |
|---|---|---|---|---|---|---|
| **EU HQ & data sovereignty** | ✅ German company, Falkenstein/Nuremberg/Helsinki DCs | ❌ US company, EU region available but CLOUD Act applies | ❌ US company, CLOUD Act | ✅ French company | ✅ French company | ❌ US company |
| **Managed K8s** | ❌ No managed K8s (Talos self-managed) | ✅ EKS | ✅ GKE | ✅ OVH Managed K8s | ✅ Kapsule | ✅ DOKS |
| **VM cost (4 vCPU, 16GB)** | ~€16/mo (CX42) | ~€120/mo (m5.xlarge) | ~€110/mo (e2-standard-4) | ~€25/mo (B2-15) | ~€20/mo (DEV1-L) | ~€48/mo (s-4vcpu-8gb) |
| **Dedicated vCPU cost** | ~€35/mo (CCX23) | ~€140/mo (c5.xlarge) | ~€130/mo (c2-standard-4) | ~€40/mo | ~€35/mo | N/A |
| **Block storage** | €0.052/GB/mo | €0.10/GB/mo (gp3) | €0.08/GB/mo | €0.04/GB/mo | €0.08/GB/mo | €0.10/GB/mo |
| **Egress** | 20TB included/server | €0.09/GB after 1GB | €0.12/GB after 1GB | Varies | 75GB included | 1TB included pool |
| **Load balancer** | €5.49/mo | €16/mo (ALB) + €0.008/LCU-hr | €18/mo + per-rule | Included | €10/mo | €12/mo |
| **Floating IP** | €4/mo | €3.60/mo (EIP) | Free (ephemeral) | Free | Free | Free (reserved) |
| **OpenTofu provider** | ✅ hetznercloud/hetzner (mature) | ✅ hashicorp/aws | ✅ hashicorp/google | ⚠️ Community provider | ✅ scaleway/scaleway | ✅ digitalocean/digitalocean |
| **API maturity** | Good (v1, stable) | Excellent | Excellent | Moderate | Good | Good |
| **Firewall** | ✅ Cloud Firewall (free) | ✅ Security Groups | ✅ VPC Firewall | ✅ | ✅ | ✅ |
| **Private networking** | ✅ vSwitch / Networks | ✅ VPC | ✅ VPC | ✅ vRack | ✅ Private Networks | ✅ VPC |
| **Object storage** | ✅ S3-compatible (new, limited regions) | ✅ S3 | ✅ GCS | ✅ S3-compatible | ✅ S3-compatible | ✅ Spaces (S3) |
| **GPU instances** | ❌ Not available | ✅ Full range | ✅ Full range | ⚠️ Limited | ⚠️ Limited | ❌ |
| **SLA** | 99.9% (VMs) | 99.99% (EC2) | 99.99% (GCE) | 99.95% | 99.9% | 99.95% |
| **Support** | Email (free), phone (paid) | Business/Enterprise plans | Premium support | Paid tiers | Community | Paid tiers |

### 3.2 Cost Projection (CAVE Prod Profile)

| Resource | Hetzner | AWS Frankfurt | GCP Belgium | OVHcloud |
|---|---|---|---|---|
| 3x CX32 control plane | €29.70/mo | €270/mo | €252/mo | €54/mo |
| 3x CX42 workers | €47.70/mo | €360/mo | €330/mo | €75/mo |
| 2x CCX53 dedicated (AI/ML) | €119.80/mo | €560/mo | €520/mo | €160/mo |
| 3x Load balancer | €16.47/mo | €48/mo + LCU | €54/mo + rules | Included |
| Block storage (500GB) | €26/mo | €50/mo | €40/mo | €20/mo |
| Floating IPs (3) | €12/mo | €10.80/mo | Free | Free |
| Egress (2TB/mo) | Included | €180/mo | €240/mo | Varies |
| **Monthly total** | **~€252/mo** | **~€1,479/mo** | **~€1,436/mo** | **~€309/mo** |
| **Annual total** | **~€3,024** | **~€17,748** | **~€17,232** | **~€3,708** |

Hetzner is **5-6x cheaper** than hyperscalers for equivalent compute. Even OVHcloud is ~20% more expensive.

### 3.3 Data Sovereignty Assessment

| Provider | HQ | Primary law | CLOUD Act / FISA 702 exposure | EU adequacy |
|---|---|---|---|---|
| Hetzner | Germany | GDPR, BDSG | None — German company, no US jurisdiction | Native EU |
| AWS | USA | US law, CLOUD Act applies | **Yes** — US gov can compel data disclosure from EU regions | EU-US DPF (challenged) |
| GCP | USA | US law, CLOUD Act applies | **Yes** | EU-US DPF (challenged) |
| OVHcloud | France | GDPR, French law | None — French company | Native EU |
| Scaleway | France | GDPR, French law | None — French company | Native EU |

For CAVE's sovereign profile, US CLOUD Act exposure is a disqualifying factor. This eliminates AWS, GCP, Azure (for sovereign), and DigitalOcean.

---


## Decision

**Hetzner Cloud** for all sovereign deployment profiles (dev, staging, prod).

---


## Rejected Options

### 4.1 AWS EU (Frankfurt) — Rejected

**Primary:** CLOUD Act exposure. US government can compel Amazon to disclose data from EU regions regardless of GDPR. Schrems II precedent makes EU-US data transfers legally uncertain. CAVE's sovereign profile requires zero US jurisdiction exposure.

**Secondary:** 5-6x cost premium. Managed K8s (EKS) is convenient but CAVE self-manages Talos (ADR-003) — EKS advantage is nullified. Egress costs (€0.09/GB) make multi-profile data transfer expensive.

### 4.2 GCP EU — Rejected

**Primary:** Same CLOUD Act exposure as AWS. Google's data practices under additional regulatory scrutiny in EU.

**Secondary:** Similar cost premium to AWS. GKE Autopilot is opinionated and incompatible with Talos/Cilium/Istio ambient customization needs.

### 4.3 OVHcloud — Rejected

**Primary:** OpenTofu provider maturity. Community-maintained provider with fewer contributors and slower feature adoption than Hetzner's official provider. CAVE's OpenTofu-first Day 0 provisioning (ADR-067) requires reliable provider.

**Secondary:** OVH's 2021 Strasbourg datacenter fire (SBG2) raised operational reliability concerns. While OVH has improved, Hetzner's track record is cleaner. OVH Managed K8s would be unused (Talos). Pricing ~20% higher than Hetzner for equivalent compute.

### 4.4 Scaleway — Rejected

**Primary:** Smaller infrastructure footprint. Fewer datacenter locations, smaller instance variety. GPU availability limited. For CAVE's AI/ML workloads (Ollama inference), Hetzner's CCX dedicated instances with AMD EPYC are more cost-effective than Scaleway's GPU offering.

**Secondary:** Object storage less mature than Hetzner's or MinIO self-hosted. CAVE uses MinIO (ADR-048) regardless, so provider object storage is secondary.

---


## Consequences

### Positive

- 5-6x cost advantage over hyperscalers enables aggressive experimentation
- Full EU data sovereignty with zero US jurisdiction exposure
- Simple, transparent pricing (no hidden egress/LCU/NAT gateway costs)
- 20TB egress included per server — critical for multi-profile sync and observability
- Mature OpenTofu provider for reliable Day 0 automation
- German company = same legal jurisdiction as the operating entity

### Negative

- No managed Kubernetes (self-managed Talos — mitigated by ADR-003)
- No GPU instances (AI inference limited to CPU on Hetzner — GPU workloads on Azure, ADR-002)
- Fewer regions than hyperscalers (Falkenstein, Nuremberg, Helsinki, Ashburn)
- Lower SLA (99.9%) than hyperscalers (99.99%) — mitigated by Talos HA + Resurrection Protocol
- No managed database, cache, messaging — all self-hosted (CNPG, Valkey, Strimzi)
- Support less responsive than enterprise hyperscaler support plans

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Hetzner outage exceeds 99.9% SLA | Low | High | Multi-AZ within Hetzner (Falkenstein+Nuremberg). Resurrection Protocol (ADR-088) for full cluster loss. Azure as failover target (ADR-066). |
| Hetzner discontinues Cloud product | Very Low | Critical | Annual portability drill (ADR-066). OpenTofu + Crossplane abstraction enables migration to OVH/Scaleway within days (same self-hosted stack). |
| Hetzner pricing increases | Low | Medium | Cost monitored via OpenCost. Cave-ctl finops tracks per-profile spend. Alternative sovereign providers evaluated annually. |
| S3-compatible storage limitations | Medium | Low | CAVE uses MinIO (ADR-048) for all object storage. Hetzner S3 used only for Talos image hosting and backup egress if needed. |

Compliance Mapping

GDPR Art.44-49 (data sovereignty — German company, no US jurisdiction, no CLOUD Act exposure). GDPR Art.25 (data protection by design — EU-native infrastructure choice). ISO A.5.23 (information security for cloud services — sovereign provider selection). NIS2 Art.21 (supply chain risk — EU-domiciled provider reduces jurisdictional risk). SOC2 CC6.1 (infrastructure access controls — provider API security). SOC2 CC9.1 (risk mitigation — cost-efficient infrastructure enables sustainable operations).

