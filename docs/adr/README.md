# CAVE Platform — Architecture Decision Records

Complete catalog of all architecture decisions for the CAVE (Cloud Autonomous Sovereign Execution) platform.

## Summary

- **Total ADRs:** 125 (ADR-001 through ADR-143, with gaps per catalog)
- **Status breakdown:** 
  - Accepted: 96
  - Proposed: 29
  - Deprecated: 0
- **Categories:** Infrastructure, Networking, CI/CD, Data, Security, Compliance, Observability, Identity, Audit, Governance

## Index

| ADR | Title | Category | Status |
|-----|-------|----------|--------|
| [ADR-001](ADR-001_Hetzner_Cloud_as_Sovereign_Infrastructure_Provider.md) | sovereign cloud as Sovereign Infrastructure Provider | Infrastructure | Accepted |
| [ADR-002](ADR-002_Azure_as_Enterprise_Infrastructure_Provider.md) | Azure as Enterprise Infrastructure Provider | Infrastructure | Accepted |
| [ADR-003](ADR-003_Talos_Linux_for_All_Hetzner_Profiles.md) | Talos Linux for All Sovereign-Cloud Profiles | Infrastructure | Accepted |
| [ADR-004](ADR-004_Cilium_CNI_Istio_Ambient_Mesh.md) | Cilium CNI + Istio Ambient Mesh | Infrastructure — Networking | Accepted |
| [ADR-005](ADR-005_Buildah_for_Container_Image_Building.md) | Buildah for Container Image Building | Infrastructure — CI/CD | Accepted |
| [ADR-006](ADR-006_Keycloak_for_Hetzner_Identity_Provider.md) | Keycloak for Sovereign Identity Provider | Identity | Accepted |
| [ADR-007](ADR-007_Okta_Entra_ID_for_Azure_Identity.md) | Okta + Entra ID for Azure Identity | Identity | Accepted |
| [ADR-008](ADR-008_Cache_Valkey_Hetzner_Azure_Redis_Azure.md) | Cache — Valkey (sovereign) / Azure Redis (Azure) | Data | Accepted |
| [ADR-009](ADR-009_Ollama_Hetzner_Azure_OpenAI_Azure.md) | Ollama (sovereign) / Azure OpenAI (Azure) | AI/LLM | Accepted |
| [ADR-010](ADR-010_CI_Pipeline_Architecture_27_Stages.md) | CI Pipeline Architecture — 27 Stages | CI/CD | Accepted |
| [ADR-011](ADR-011_Backstage_as_Developer_Portal.md) | Backstage as Developer Portal | Platform | Accepted |
| [ADR-012](ADR-012_vcluster_for_Hard_Tenancy_PR_Environments.md) | vcluster for Hard Tenancy + PR Environments | Multi-Tenancy | Accepted |
| [ADR-013](ADR-013_LiteLLM_as_Unified_LLM_Gateway.md) | LiteLLM as Unified LLM Gateway | AI/LLM | Accepted |
| [ADR-014](ADR-014_Zero_Trust_Network_Architecture.md) | Zero-Trust Network Architecture | Security | Accepted |
| [ADR-015](ADR-015_TLS_Certificate_Lifecycle_via_cert_manager.md) | TLS Certificate Lifecycle via cert-manager | Security | Accepted |
| [ADR-016](ADR-016_Container_Runtime_Security_Pod_Security_Tetragon.md) | Container Runtime Security — Pod Security + Tetragon | Security | Accepted |
| [ADR-017](ADR-017_Pre_Commit_and_CI_Secret_Scanning_gitleaks.md) | Pre-Commit and CI Secret Scanning — gitleaks | Security | Accepted |
| [ADR-018](ADR-018_Container_Image_Vulnerability_Scanning_Trivy.md) | Container Image Vulnerability Scanning — Trivy | Security | Accepted |
| [ADR-019](ADR-019_Static_Application_Security_Testing_SonarQube_Semgrep.md) | Static Application Security Testing — SonarQube + Semgrep | Security | Accepted |
| [ADR-020](ADR-020_OpenBao_for_Hetzner_Secrets_Management.md) | OpenBao for Hetzner Secrets Management | Identity & Secrets | Accepted |
| [ADR-021](ADR-021_Event_Streaming_Strimzi_Hetzner_Confluent_Cloud_Azure.md) | Event Streaming — Strimzi (sovereign) / Confluent Cloud (Azure) | Data & Messaging | Accepted |
| [ADR-022](ADR-022_Change_Data_Capture_Debezium.md) | Change Data Capture — Debezium | Data & Messaging | Accepted |
| [ADR-023](ADR-023_Dynamic_Application_Security_Testing_OWASP_ZAP.md) | Dynamic Application Security Testing — OWASP ZAP | Security/CI | Accepted |
| [ADR-024](ADR-024_DNS_CDN_Cloudflare.md) | DNS & CDN — Cloudflare | Infrastructure | Accepted |
| [ADR-025](ADR-025_Same_Backstage_UX_Across_All_Providers.md) | Same Backstage UX Across All Providers | Platform | Accepted |
| [ADR-026](ADR-026_ArgoCD_for_GitOps.md) | ArgoCD for GitOps | CI/CD | Accepted |
| [ADR-027](ADR-027_Kong_API_Gateway.md) | Kong API Gateway | Networking | Accepted |
| [ADR-028](ADR-028_Container_Registry_Harbor.md) | Container Registry — Harbor | CI/CD | Accepted |
| [ADR-029](ADR-029_Prometheus_Grafana_Loki_Tempo_LGTM_Stack.md) | Prometheus + Grafana + Loki + Tempo (LGTM Stack) | Observability | Accepted |
| [ADR-030](ADR-030_OPA_Gatekeeper_OPAL.md) | OPA Gatekeeper + OPAL | Security | Accepted |
| [ADR-035](ADR-035_Security_Finding_Aggregation_DefectDojo.md) | Security Finding Aggregation — DefectDojo | Security | Accepted |
| [ADR-036](ADR-036_Progressive_Delivery_Argo_Rollouts.md) | Progressive Delivery — Argo Rollouts | CI/CD | Accepted |
| [ADR-037](ADR-037_Feature_Flags_Unleash.md) | Feature Flags — Unleash | CI/CD | Accepted |
| [ADR-038](ADR-038_Workflow_Orchestration_Argo_Workflows.md) | Workflow Orchestration — Argo Workflows | CI/CD | Accepted |
| [ADR-039](ADR-039_Chaos_Mesh.md) | Chaos Mesh | Resilience | Accepted |
| [ADR-040](ADR-040_Self_Hosted_CI_Runners_Actions_Runner_Controller_ARC.md) | Self-Hosted CI Runners — Actions Runner Controller (ARC) | CI/CD | Accepted |
| [ADR-041](ADR-041_Automated_Dependency_Updates_Renovate.md) | Automated Dependency Updates — Renovate | CI/CD | Accepted |
| [ADR-042](ADR-042_DORA_Metrics_DevLake.md) | DORA Metrics — DevLake | Observability | Accepted |
| [ADR-043](ADR-043_Schema_Migration_Validation_Flyway_Alembic.md) | Schema Migration Validation — Flyway + Alembic | CI/CD | Accepted |
| [ADR-044](ADR-044_Supported_Language_Runtimes.md) | Supported Language Runtimes | CI/CD | Accepted |
| [ADR-045](ADR-045_Load_Testing_k6.md) | Load Testing — k6 | CI/CD | Accepted |
| [ADR-046](ADR-046_Kubernetes_Backup_Velero.md) | Kubernetes Backup — Velero | DR | Accepted |
| [ADR-047](ADR-047_PostgreSQL_CloudNativePG_Hetzner_Azure_PG_Flexible_Azure.md) | PostgreSQL — CloudNativePG (sovereign) / Azure PG Flexible (Azure) | Data | Accepted |
| [ADR-049](ADR-049_Full_Text_Search_OpenSearch_Hetzner_Azure_AI_Search_Azure.md) | Full-Text Search — OpenSearch (sovereign) / Azure AI Search (Azure) | Data | Accepted |
| [ADR-050](ADR-050_Object_Storage_MinIO_Hetzner_ADLS_Gen2_Azure.md) | Object Storage — MinIO (sovereign) / ADLS Gen2 (Azure) | Data | Accepted |
| [ADR-051](ADR-051_LLM_Observability_Langfuse.md) | LLM Observability — Langfuse | AI | Accepted |
| [ADR-052](ADR-052_AI_Chat_Interface_LibreChat.md) | AI Chat Interface — LibreChat | AI | Accepted |
| [ADR-053](ADR-053_External_Secrets_Operator_ESO.md) | External Secrets Operator (ESO) | Identity & Secrets | Accepted |
| [ADR-054](ADR-054_Network_Segmentation_VNet_Subnet_Architecture.md) | Network Segmentation — VNet/Subnet Architecture | Security | Accepted |
| [ADR-055](ADR-055_WAF_DDoS_Protection_Cloudflare.md) | WAF & DDoS Protection — Cloudflare | Security | Accepted |
| [ADR-056](ADR-056_Encryption_at_Rest_All_Data_Services.md) | Encryption at Rest — All Data Services | Security | Accepted |
| [ADR-057](ADR-057_Application_Security_Testing_Strategy_Defense_in_Depth.md) | Application Security Testing Strategy — Defense-in-Depth | Security | Accepted |
| [ADR-058](ADR-058_Kubernetes_Compliance_Scanning_Kubescape.md) | Kubernetes Compliance Scanning — Kubescape | Security | Accepted |
| [ADR-059](ADR-059_Kafka_Topic_Governance.md) | Kafka Topic Governance | Data & Messaging | Accepted |
| [ADR-060](ADR-060_Schema_Registry_Evolution_Policy.md) | Schema Registry & Evolution Policy | Data & Messaging | Accepted |
| [ADR-061](ADR-061_Hetzner_Day_0_Infrastructure_OpenTofu.md) | Hetzner Day-0 Infrastructure — OpenTofu | Infrastructure | Accepted |
| [ADR-062](ADR-062_Azure_Day_0_Infrastructure_OpenTofu.md) | Azure Day-0 Infrastructure — OpenTofu | Infrastructure | Accepted |
| [ADR-063](ADR-063_ArgoCD_Self_Hosted_on_Azure_Not_AKS_GitOps_Add_on.md) | ArgoCD Self-Hosted on Azure (Not AKS GitOps Add-on) | CI/CD | Accepted |
| [ADR-064](ADR-064_Identity_Split_Okta_for_Apps_Entra_for_Azure_RBAC.md) | Identity Split — Okta for Apps, Entra for Azure RBAC | Identity | Accepted |
| [ADR-066](ADR-066_Tenant_Provider_Choice_at_Onboarding.md) | Tenant Provider Choice at Onboarding | Infrastructure | Accepted |
| [ADR-067](ADR-067_Crossplane_v2_for_Day_1_Provisioning.md) | Crossplane v2 for Day 1+ Provisioning | Platform | Accepted |
| [ADR-070](ADR-070_vcluster_CI_Mandatory_Prod_Opt_in_Dev_Staging.md) | vcluster CI — Mandatory Prod, Opt-in Dev/Staging | CI/CD | Accepted |
| [ADR-072](ADR-072_Prometheus_Federation_Thanos.md) | Prometheus Federation — Thanos | Observability | Accepted |
| [ADR-074](ADR-074_MLOps_MLflow.md) | MLOps — MLflow | AI/ML | Accepted |
| [ADR-075](ADR-075_Serverless_Workloads_Knative_KEDA_Phase_4.md) | Serverless Workloads — Knative + KEDA (Phase 4) | Platform | Proposed (Phase 4) |
| [ADR-076](ADR-076_cave_ctl_CLI_MCP_Server_Architecture.md) | cave-ctl CLI & MCP Server Architecture | Platform | Accepted |
| [ADR-077](ADR-077_Sigstore_Policy_Controller_for_Image_Admission.md) | Sigstore Policy Controller for Image Admission | Security | Accepted |
| [ADR-078](ADR-078_Platform_RBAC_Architecture.md) | Platform RBAC Architecture | Governance | Accepted |
| [ADR-079](ADR-079_Secret_Zero_Bootstrap_Break_Glass_Kit.md) | Secret Zero Bootstrap + Break-Glass Kit | Security | Accepted |
| [ADR-080](ADR-080_Backup_Retention_Policy.md) | Backup Retention Policy | DR | Accepted |
| [ADR-083](ADR-083_Automated_Secret_Rotation.md) | Automated Secret Rotation | Security | Accepted |
| [ADR-084](ADR-084_Cilium_Default_Deny_Network_Policy_per_Tenant.md) | Cilium Default-Deny Network Policy per Tenant | Security | Accepted |
| [ADR-085](ADR-085_Platform_Upgrade_Strategy.md) | Platform Upgrade Strategy | Operations | Accepted |
| [ADR-086](ADR-086_Tenant_Offboarding_with_Crypto_Erasure.md) | Tenant Offboarding with Crypto-Erasure | Multi-Tenancy | Accepted |
| [ADR-087](ADR-087_ResourceQuota_LimitRange_per_Tenant.md) | ResourceQuota + LimitRange per Tenant | Multi-Tenancy | Accepted |
| [ADR-088](ADR-088_Resurrection_Protocol.md) | Resurrection Protocol | DR | Accepted |
| [ADR-089](ADR-089_Signed_OPA_Policy_Bundles.md) | Signed OPA Policy Bundles | Security | Accepted |
| [ADR-090](ADR-090_Runtime_Forensics_Tetragon_Hubble_WORM.md) | Runtime Forensics (Tetragon + Hubble → WORM) | Security | Accepted |
| [ADR-091](ADR-091_Entropy_Shadow_IT_Detection.md) | Entropy & Shadow IT Detection | Governance | Accepted |
| [ADR-092](ADR-092_AI_Privilege_Guardrails_MCP_Allowlist_Denylist.md) | AI Privilege Guardrails (MCP Allowlist/Denylist) | AI Governance | Accepted |
| [ADR-093](ADR-093_Sovereign_Ledger_WORM_Sigstore.md) | Sovereign Ledger (WORM + Sigstore) | Governance | Accepted |
| [ADR-095](ADR-095_Reflex_Engine_KEDA_Argo_Workflows.md) | Reflex Engine (KEDA + Argo Workflows) | Operations | Accepted |
| [ADR-096](ADR-096_Unit_Economics_FinOps_Attribution.md) | Unit Economics & FinOps Attribution | FinOps | Accepted |
| [ADR-098](ADR-098_Talos_Linux_Immutable_Infrastructure.md) | Talos Linux Immutable Infrastructure | Infrastructure | Accepted |
| [ADR-099](ADR-099_Deprecation_Guardrails_in_CI_Pluto_kubent.md) | Deprecation Guardrails in CI — Pluto + kubent | CI/CD | Accepted |
| [ADR-100](ADR-100_Continuous_Resilience_Attestation.md) | Continuous Resilience Attestation | Resilience | Accepted |
| [ADR-101](ADR-101_SLSA_Level_3_Supply_Chain_Provenance.md) | SLSA Level 3 Supply Chain Provenance | Security | Accepted |
| [ADR-102](ADR-102_Mandatory_Data_Classification_Labels.md) | Mandatory Data Classification Labels | Security | Accepted |
| [ADR-103](ADR-103_LLM_Data_Governance.md) | LLM Data Governance | AI | Accepted |
| [ADR-104](ADR-104_Identity_Lifecycle_Governance.md) | Identity Lifecycle Governance | Identity | Accepted |
| [ADR-105](ADR-105_etcd_KMS_Encryption_via_OpenBao_Transit_Key_Vault.md) | etcd KMS Encryption via OpenBao Transit / Key Vault | Security | Accepted |
| [ADR-106](ADR-106_Loki_WORM_Backed_Storage_for_Forensic_Integrity.md) | Loki WORM-Backed Storage for Forensic Integrity | Security | Accepted |
| [ADR-108](ADR-108_Helm_Manifest_Supply_Chain_Digest_Pinning.md) | Helm/Manifest Supply Chain — Digest Pinning | Security | Accepted |
| [ADR-109](ADR-109_Observability_Multi_Tenancy_via_Label_Scoping.md) | Observability Multi-Tenancy via Label Scoping | Observability | Accepted |
| [ADR-110](ADR-110_Egress_Governance_Quarantine_Safe_Exit_List.md) | Egress Governance — Quarantine + Safe-Exit List | Security/FinOps | Accepted |
| [ADR-111](ADR-111_Classification_Aware_LLM_Inference_Routing.md) | Classification-Aware LLM Inference Routing | AI | Accepted |
| [ADR-112](ADR-112_APOL_Autonomous_Platform_Operations_Layer.md) | APOL — Autonomous Platform Operations Layer | AI Governance | Accepted |
| [ADR-113](ADR-113_Data_Residency_Enforcement_via_Crossplane_XR.md) | Data Residency Enforcement via Crossplane XR | Compliance | Accepted |
| [ADR-114](ADR-114_Qdrant_Vector_DB_as_Crossplane_XR.md) | Qdrant Vector DB as Crossplane XR | Data | Accepted |
| [ADR-115](ADR-115_CI_Secret_Injection_via_OIDC_Token_Exchange.md) | CI Secret Injection via OIDC Token Exchange | CI/CD | Accepted |
| [ADR-118](ADR-118_APOL_Fallback_Mode_Manual_Operations.md) | APOL Fallback Mode — Manual Operations | AI Governance | Accepted |
| [ADR-119](ADR-119_Crossplane_Operations_for_Day_2_Maintenance.md) | Crossplane Operations for Day-2 Maintenance | Platform | Accepted |
| [ADR-120](ADR-120_ArgoCD_OCI_Source_Harbor_Registry_as_Manifest_Source.md) | ArgoCD OCI Source — Harbor Registry as Manifest Source | CI/CD | Accepted |
| [ADR-121](ADR-121_Istio_Ambient_Multi_Cluster_Non_Baseline_Until_Stable.md) | Istio Ambient Multi-Cluster — Non-Baseline Until Stable | Networking | Accepted |
| [ADR-122](ADR-122_Cilium_Gateway_API_Reserved_for_Future_Internal_Routing.md) | Cilium Gateway API Reserved for Future Internal Routing | Networking | Accepted |
| [ADR-124](ADR-124_Crossplane_MRAP_ManagedResourceActivationPolicy.md) | Crossplane MRAP — ManagedResourceActivationPolicy | Platform | Accepted |
| [ADR-125](ADR-125_APOL_Chain_of_Thought_Audit_Trail.md) | APOL Chain-of-Thought Audit Trail | AI Governance | Accepted |
| [ADR-126](ADR-126_Workload_Criticality_Labels_for_Kill_Switch_Ethics.md) | Workload Criticality Labels for Kill-Switch Ethics | FinOps | Accepted |
| [ADR-127](ADR-127_Roadmap_Intelligence_Automation.md) | Roadmap Intelligence Automation | Governance | Accepted |
| [ADR-128](ADR-128_APOL_Attestation_Redaction_Policy.md) | APOL Attestation Redaction Policy | AI Governance | Accepted |
| [ADR-129](ADR-129_Tenant_Identity_Federation_BYOID.md) | Tenant Identity Federation — BYOID | Identity | Accepted |
| [ADR-130](ADR-130_Privileged_Access_Management_PAM_Layer.md) | Privileged Access Management (PAM) Layer | None | Proposed |
| [ADR-131](ADR-131_OPAL_for_Real_Time_Policy_Data_Distribution.md) | OPAL for Real-Time Policy Data Distribution | Security | Accepted |
| [ADR-132](ADR-132_Version_Channel_Soak_Policy.md) | Version Channel & Soak Policy | Platform Governance | Accepted |
| [ADR-133](ADR-133_Compatibility_Matrix_as_Code.md) | Compatibility Matrix as Code | Platform Governance | Accepted |
| [ADR-134](ADR-134_Deprecation_Runway_Enforcement.md) | Deprecation Runway Enforcement | Platform Governance | Accepted |
| [ADR-135](ADR-135_Provider_Parity_Contract_Testing.md) | Provider Parity Contract Testing | Platform Governance | Accepted |
| [ADR-136](ADR-136_APOL_Bounded_Autonomy_Model.md) | APOL Bounded Autonomy Model | Platform Governance — AI Operations | Accepted |
| [ADR-137](ADR-137_Constitutional_Tiering.md) | Constitutional Tiering | Platform Governance | Accepted |
| [ADR-138](ADR-138_Evidence_Tiering.md) | Evidence Tiering | Platform Governance | Accepted |
| [ADR-139](ADR-139_Data_Contract_Governance.md) | Data Contract Governance | Platform Governance — Data | Accepted |
| [ADR-140](ADR-140_Waiver_Framework.md) | Waiver Framework | Platform Governance | Accepted |
| [ADR-141](ADR-141_Shared_Fate_Tenant_Priority.md) | Shared-Fate & Tenant Priority | Platform Governance — Multi-Tenancy | Accepted |
| [ADR-142](ADR-142_Post_Quantum_Cryptography_Readiness.md) | Post-Quantum Cryptography (PQC) Readiness | Security — Cryptography | Proposed |
| [ADR-143](ADR-143_Paranoid_Artifact_Proxy.md) | Paranoid Artifact Proxy (pull-through with security-gate cache) | CI/CD — Registry | Proposed |

## Categories

### Infrastructure
Infrastructure provider choice, container runtime, Linux distribution, immutable infrastructure patterns.

### Infrastructure — Networking
Network CNI, service mesh, ingress, load balancing, traffic management.

### Infrastructure — CI/CD
CI/CD platform, artifact building, container image creation, supply chain security.

### Data Platform
Database selection, storage backend, persistence layer, data replication.

### Security
Cryptography, secrets management, vulnerability scanning, supply chain security, artifact signing.

### Security — Cryptography
Cryptographic algorithms, key management, post-quantum readiness.

### Compliance & Audit
Audit logging, compliance enforcement, policy as code, governance.

### Observability
Logging, metrics, tracing, alerting, monitoring.

### Identity & Access
Authentication, authorization, identity federation, RBAC.

### Governance
Decision-making processes, ADR lifecycle, change management.

---

## Related Documentation

- **CAVE Architecture Overview:** See `/sessions/quirky-wizardly-bell/mnt/claude/platform/docs/`
- **Platform Runbook:** See `/sessions/quirky-wizardly-bell/mnt/claude/platform/cave-runtime/docs/runbook/`
- **Implementation Status:** Check individual Rust crates in `cave-runtime/crates/`

## ADR Process

1. **Proposal:** New ADRs start as Proposed with full context, alternatives, and consequences
2. **Review:** Technical steering committee reviews against architecture principles
3. **Acceptance:** Accepted status indicates implementation is proceeding or complete
4. **Deprecation:** Deprecated ADRs are retained for historical reference

---

Generated: 2026-04-18  
Total files: 126 (123 catalog ADRs + 2 session ADRs + 1 README)
