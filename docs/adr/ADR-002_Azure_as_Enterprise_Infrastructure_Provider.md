# ADR-002: Azure as Enterprise Infrastructure Provider

**Status:** Accepted

**Scope:** Azure

**Category:** Infrastructure

**Related ADRs:** 001 (Hetzner), 007 (Okta+Entra), 062 (Karpenter AKS), 064 (Identity Split), 066 (Provider Choice)

## Context

CAVE requires an enterprise-grade cloud provider as the second deployment target. While Hetzner (ADR-001) serves the sovereign self-hosted profile, the enterprise target must satisfy:

- Enterprise SLA requirements (99.95%+ for production workloads)
- Managed service availability (PostgreSQL, Redis, Kafka, AI/ML)
- Corporate compliance certifications (SOC2, ISO 27001, HIPAA, PCI-DSS)
- Identity integration with enterprise IdPs (Okta, Entra ID)
- GPU availability for AI/ML workloads (training + inference)
- Global region coverage for potential multi-region tenants
- Credibility with enterprise customers ("runs on Azure" vs "runs on Hetzner")

The enterprise target is CAVE's "sell" environment — where paying tenants run production workloads.

---


## Candidates

### 3.1 Enterprise Provider Comparison

| Criteria | Azure | AWS | GCP |
|---|---|---|---|
| **Managed K8s** | AKS (free control plane) | EKS (€0.10/hr control plane) | GKE (free Autopilot CP, Standard €0.10/hr) |
| **K8s control plane cost** | Free | ~€73/mo per cluster | Free (Autopilot) or €73/mo |
| **Managed PostgreSQL** | Azure PG Flexible (mature, HA) | RDS PostgreSQL (mature, HA) | Cloud SQL PostgreSQL (mature) |
| **Managed Redis** | Azure Cache for Redis (Enterprise tier: Redis Enterprise) | ElastiCache (Valkey-based since 2024) | Memorystore (Redis-compatible) |
| **Managed Kafka** | Confluent on Azure (marketplace) | MSK (managed Kafka) | Confluent on GCP |
| **AI/ML platform** | Azure OpenAI + Databricks (native) | Bedrock + SageMaker | Vertex AI + Gemini |
| **LLM access** | Azure OpenAI (GPT-4, o1, exclusive partnership) | Bedrock (Claude, Llama, etc.) | Vertex (Gemini, Claude) |
| **Identity** | Entra ID (native) + Okta integration | IAM + Okta/Cognito | Cloud Identity + Okta |
| **GPU availability** | NCasT4_v3, NCads_A100, ND_H100 | p3/p4d/p5 (NVIDIA) | A100, H100, TPU |
| **EU regions** | West Europe, North Europe, Germany West Central, Switzerland | Frankfurt, Ireland, Paris, Stockholm, Milan | Belgium, Netherlands, Finland, Zurich |
| **Germany-specific region** | ✅ Germany West Central (Frankfurt) | ❌ Frankfurt (eu-central-1) is standard EU | ❌ No Germany-specific |
| **Compliance certs** | SOC1/2/3, ISO 27001/27017/27018, HIPAA, PCI-DSS, C5 (German), ENS | SOC1/2/3, ISO 27001, HIPAA, PCI-DSS | SOC1/2/3, ISO 27001, HIPAA |
| **Marketplace** | ✅ Confluent, Databricks, Elastic, etc. | ✅ Similar breadth | ✅ Similar |
| **Enterprise agreements** | ✅ EA, CSP, MACC credits | ✅ EDP, marketplace | ✅ Committed use |
| **CLOUD Act** | ⚠️ Yes (US company) | ⚠️ Yes | ⚠️ Yes |
| **Terraform/OpenTofu** | hashicorp/azurerm (very mature) | hashicorp/aws (excellent) | hashicorp/google (excellent) |

### 3.2 Managed Services Cost Comparison (Prod Profile)

| Service | Azure | AWS | GCP |
|---|---|---|---|
| AKS/EKS/GKE (3 node D8s_v5) | ~€850/mo (AKS CP free) | ~€923/mo (EKS CP €73) | ~€870/mo |
| PostgreSQL HA (D4s, 100GB) | ~€450/mo | ~€500/mo (RDS Multi-AZ) | ~€420/mo |
| Redis Enterprise HA (6GB) | ~€350/mo | ~€380/mo (ElastiCache) | ~€300/mo |
| Confluent Standard (3 CKU) | ~€600/mo | ~€600/mo (MSK comparable) | ~€600/mo |
| Databricks Premium | ~€400/mo (depending on DBU) | ~€400/mo | ~€400/mo |
| Key Vault Premium | ~€5/mo | ~€5/mo (Secrets Manager) | ~€6/mo (Secret Manager) |
| AI Search Standard | ~€250/mo | ~€250/mo (OpenSearch) | ~€200/mo |
| Azure OpenAI (GPT-4o) | Pay per token | N/A (Bedrock equivalent) | N/A (Vertex) |
| **Monthly estimate** | **~€2,900/mo** | **~€3,060/mo** | **~€2,800/mo** |

Costs are comparable across hyperscalers. Decision is not cost-driven.

### 3.3 Strategic Fit Assessment

| Factor | Azure | AWS | GCP |
|---|---|---|---|
| **Enterprise ecosystem alignment** | ✅ Target organization uses Microsoft 365 + Entra ID | ⚠️ Would require separate identity | ⚠️ Would require separate identity |
| **Azure OpenAI exclusivity** | ✅ Only provider with Azure OpenAI (GPT-4, o1 with enterprise DPA) | ❌ Must use Bedrock | ❌ Must use Vertex |
| **Databricks integration** | ✅ Native Azure Databricks | ⚠️ Databricks on AWS (less integrated) | ⚠️ Databricks on GCP |
| **German compliance (C5)** | ✅ BSI C5 certified | ⚠️ C5 available but less emphasized | ❌ No C5 |
| **Enterprise sales credibility** | ✅ "Runs on Azure" resonates with EU enterprise buyers | ✅ AWS equally credible | ⚠️ GCP less enterprise presence in EU |
| **Okta integration depth** | ✅ Okta + Entra ID well-documented pattern | ✅ Okta + AWS IAM works | ✅ Okta + Google Cloud Identity |
| **Karpenter support** | ✅ AKS Karpenter (GA since 2024) | ✅ Karpenter originated on AWS | ❌ GKE Autopilot (different approach) |

---


## Decision

**Microsoft Azure** for all enterprise deployment profiles (dev, staging, prod).

---


## Rejected Options

### 4.1 AWS — Rejected

**Primary:** Enterprise ecosystem alignment. The target organization's enterprise environment is Microsoft-centric (M365, Entra ID, Teams). Building CAVE's enterprise target on AWS would require bridging two identity ecosystems. Azure's native Entra ID integration eliminates this friction. ADR-064 splits identity cleanly: Okta = apps, Entra ID = Azure RBAC. On AWS, this clean split doesn't exist — AWS IAM is fundamentally different from Entra ID.

**Secondary:** No Azure OpenAI equivalent. AWS Bedrock provides Claude and Llama access but CAVE's enterprise tenants may require GPT-4/o1 specifically. Azure OpenAI offers enterprise DPA (Data Processing Agreement) with no-training guarantee, which maps directly to ADR-103 (LLM data governance). BSI C5 certification less prominent on AWS than Azure in German enterprise market.

### 4.2 GCP — Rejected

**Primary:** Weakest enterprise presence in EU market. German enterprise customers (the target enterprise market) are predominantly Azure or AWS. "Runs on GCP" carries less weight in sales conversations. GCP's identity model (Google Cloud Identity) is the least compatible with corporate Okta + Entra ID patterns.

**Secondary:** No Karpenter support — GKE Autopilot is opinionated and incompatible with CAVE's Cilium + Istio ambient + Talos-parity networking requirements. No BSI C5 certification. Databricks integration less mature on GCP than Azure.

---


## Consequences

### Positive

- Native Entra ID integration aligns with the target organization's Microsoft ecosystem
- Azure OpenAI exclusive access with enterprise DPA (ADR-103)
- AKS free control plane reduces per-profile cost
- BSI C5 certification strengthens German enterprise sales
- Karpenter GA on AKS enables same JIT scaling pattern as Hetzner CA (ADR-062)
- Rich managed service portfolio reduces self-hosting burden on enterprise profile
- Germany West Central region for data residency requirements (ADR-113)

### Negative

- CLOUD Act exposure (mitigated: sovereign workloads stay on Hetzner, ADR-001)
- Vendor lock-in on managed services (mitigated: every Azure service has Hetzner equivalent, ADR-066)
- Higher cost than Hetzner (~4-10x for equivalent compute)
- Azure Portal UX complexity compared to Hetzner Cloud console
- Frequent Azure API changes require Renovate + roadmap scan vigilance (ADR-127)

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Azure OpenAI availability/pricing changes | Medium | Medium | LiteLLM gateway (ADR-013) abstracts provider. Ollama fallback on all profiles. |
| AKS breaking changes | Low | High | Staging validates before prod. Karpenter pinned to tested version. cave-ctl upgrade check. |
| Azure cost escalation | Medium | Medium | FinOps monitoring (ADR-096). OpenCost per-tenant attribution. Annual portability drill proves Hetzner migration feasible (ADR-066). |
| Entra ID integration complexity | Low | Medium | ADR-064 strictly separates: Okta = apps, Entra = Azure RBAC. No overlap. |

Compliance Mapping

SOC2 CC6.1 (infrastructure access controls — Azure RBAC + Entra ID). SOC2 CC7.5 (availability — Azure 99.95% SLA). ISO A.5.23 (cloud service agreements — Azure enterprise agreement). ISO A.8.22 (network security — VNet isolation, Private Endpoints). BSI C5 (German cloud security — Azure certified). NIS2 Art.21 (supply chain — enterprise-grade provider with compliance certifications). GDPR Art.28 (processor obligations — Azure DPA covers data processing agreement requirements).

