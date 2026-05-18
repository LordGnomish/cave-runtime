<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Cave Runtime — Architecture Decision Records

Decision records for the OSS Cave Runtime. ADRs here are normative for the
runtime release. Earlier provider-specific deployment ADRs (the
Hetzner/Azure two-profile model from the closed-source CAVE platform) live
under [`internal/`](internal/) and are kept for archival reference only.

## How to read this directory

| Bucket | Filename pattern | What it means |
|---|---|---|
| Canonical | `ADR-NNN-...` (lowercase kebab) | Live runtime decisions authored brand-neutral. |
| Legacy CAVE platform (numbered) | `ADR-NNN_Title_With_Underscores.md` | Inherited from the CAVE platform. Decision survives for the runtime; provider-specific paragraphs are noted in the body. |
| Cave Runtime topic | `ADR-RUNTIME-*` | Cross-cutting consolidation/architecture decisions for the runtime crates. |
| Cave Portal topic | `ADR-PORTAL-*` | Portal UI / auth / persona decisions. |
| Other charters | `ADR-MULTI-TENANT-001`, `ADR-CONTRIB-ATTRIBUTION-001`, `ADR-SELF-IMPROVE-001` | Foundational invariants without a numbered slot. |

Several ADRs carry an inline `<!-- needs Burak verify: ... -->` HTML comment
flagging questions that surfaced during the OSS cleanup sweep but cannot be
answered autonomously. They are still part of the catalogue; resolve in a
follow-up sprint.

> **Numbering note.** The numeric series has gaps (002–003, 006–009, 021,
> 047–050, 062–065, 068–069, 071, 073, 081–082, 094, 097, 107, 116–117, 123).
> Most gaps correspond to provider-specific ADRs that were moved to
> `internal/`. A gap-free renumber is deferred to a post-launch sprint —
> hundreds of cross-references across the codebase make it too risky to
> attempt under the 21 May 2026 OSS deadline.

## Numbered ADRs

| # | Title | Status | Notes |
|---|-------|--------|-------|
| [001](ADR-001-sovereign-bare-metal-hosting.md) | Sovereign Bare-Metal Hosting Reference Profile | Accepted | Canonical hosting profile (Linux 7.1+, single Rust binary). |
| [004](ADR-004_Cilium_CNI_Istio_Ambient_Mesh.md) | Cilium CNI + Istio Ambient Mesh | Accepted | Networking + service-mesh tool selection. |
| [005](ADR-005_Buildah_for_Container_Image_Building.md) | Buildah for Container Image Building | Accepted | Hermetic, rootless image build. |
| [010](ADR-010_CI_Pipeline_Architecture_27_Stages.md) | CI Pipeline Architecture — 27 Stages | Accepted | CI gating model. |
| [011](ADR-011_Backstage_as_Developer_Portal.md) | Backstage as Developer Portal | Suspect | `needs Burak verify`: relationship to cave-portal. |
| [012](ADR-012_vcluster_for_Hard_Tenancy_PR_Environments.md) | vcluster + Kamaji for Tenant Isolation | Accepted | Hard tenancy + PR-env pattern. |
| [013](ADR-013_LiteLLM_as_Unified_LLM_Gateway.md) | LiteLLM as Unified LLM Gateway | Accepted | LLM gateway pattern. |
| [014](ADR-014_Zero_Trust_Network_Architecture.md) | Zero-Trust Network Architecture | Accepted | SPIFFE/mTLS foundation. |
| [015](ADR-015_TLS_Certificate_Lifecycle_via_cert_manager.md) | TLS Certificate Lifecycle via cert-manager | Accepted | Superseded by `RUNTIME-CERT-LIFECYCLE-001` for the runtime; kept as principle. |
| [016](ADR-016_Container_Runtime_Security_Pod_Security_Tetragon.md) | Container Runtime Security — Pod Security + Tetragon | Accepted | Defense-in-depth runtime security. |
| [017](ADR-017_Pre_Commit_and_CI_Secret_Scanning_gitleaks.md) | Pre-Commit and CI Secret Scanning — gitleaks | Accepted | Secret-scan policy. |
| [018](ADR-018_Container_Image_Vulnerability_Scanning_Trivy.md) | Container Image Vulnerability Scanning — Trivy | Accepted | Supply-chain scan policy. |
| [019](ADR-019_Static_Application_Security_Testing_SonarQube_Semgrep.md) | Static Application Security Testing — SonarQube + Semgrep | Accepted | SAST coverage. |
| [020](ADR-020_OpenBao_Secrets_Management.md) | OpenBao for Secrets Management | Accepted | Vault-fork choice (license-driven). |
| [022](ADR-022_Change_Data_Capture_Debezium.md) | Change Data Capture — Debezium | Accepted | CDC pattern. |
| [023](ADR-023_Dynamic_Application_Security_Testing_OWASP_ZAP.md) | Dynamic Application Security Testing — OWASP ZAP | Accepted | DAST strategy. |
| [024](ADR-024_DNS_&_CDN_-_Cloudflare.md) | DNS & CDN — Cloudflare | Accepted | Cloudflare-specific; provider-independence principle holds. |
| [025](ADR-025_Same_Backstage_UX_Across_All_Providers.md) | Same Backstage UX Across All Providers | Suspect | `needs Burak verify`: depends on ADR-011. |
| [026](ADR-026_ArgoCD_for_GitOps.md) | ArgoCD for GitOps | Accepted | GitOps engine choice. |
| [027](ADR-027_Kong_API_Gateway.md) | Kong API Gateway | Suspect | `needs Burak verify`: superseded by `RUNTIME-API-GATEWAY-CONSOLIDATION-001` (cave-gateway). |
| [028](ADR-028_Container_Registry_Harbor.md) | Container Registry — Harbor | Accepted | Registry choice (multi-tenant). |
| [029](ADR-029_Prometheus_Grafana_Loki_Tempo_LGTM_Stack.md) | LGTM Observability Stack | Accepted | Core observability selection. |
| [030](ADR-030_OPA_Gatekeeper_OPAL.md) | OPA Gatekeeper + OPAL | Accepted | Policy-engine selection. |
| [031](ADR-031-cave-webapplication-composition-pattern.md) | Cave WebApplication Composition Pattern | Accepted | Cave-native XR composition. |
| [032](ADR-032-karpenter-as-node-provisioner.md) | Karpenter as Node Provisioner | Accepted | Title neutralised. See also ADR-146. |
| [033](ADR-033-keda-as-event-driven-pod-autoscaler.md) | KEDA as Event-Driven Pod Autoscaler | Accepted | Title neutralised. `cave-keda` is the reimpl target. |
| [035](ADR-035_Security_Finding_Aggregation_DefectDojo.md) | Security Finding Aggregation — DefectDojo | Accepted | Finding lifecycle. |
| [036](ADR-036_Progressive_Delivery_Argo_Rollouts.md) | Progressive Delivery — Argo Rollouts | Accepted | Canary/blue-green. |
| [037](ADR-037_Feature_Flags_Unleash.md) | Feature Flags — Unleash | Accepted | Self-hosted feature-flag stack. |
| [038](ADR-038_Workflow_Orchestration_Argo_Workflows.md) | Workflow Orchestration — Argo Workflows | Accepted | Workflow engine. |
| [039](ADR-039_Chaos_Mesh.md) | Chaos Mesh | Accepted | Chaos engineering. |
| [040](ADR-040_Self_Hosted_CI_Runners_Actions_Runner_Controller_ARC.md) | Self-Hosted CI Runners — ARC | Accepted | Sovereign CI runners. |
| [041](ADR-041_Automated_Dependency_Updates_Renovate.md) | Automated Dependency Updates — Renovate | Accepted | Digest-pinning policy. |
| [042](ADR-042_DORA_Metrics_DevLake.md) | DORA Metrics — DevLake | Accepted | Engineering metrics. |
| [043](ADR-043_Schema_Migration_Validation_Flyway_Alembic.md) | Schema Migration — Flyway + Alembic | Accepted | Migration validation. |
| [044](ADR-044_Supported_Language_Runtimes.md) | Supported Language Runtimes | Accepted | Phased language support. |
| [045](ADR-045_Load_Testing_k6.md) | Load Testing — k6 | Accepted | Performance gate. |
| [046](ADR-046_Kubernetes_Backup_-_Velero.md) | Kubernetes Backup — Velero | Accepted | K8s-resource DR. |
| [051](ADR-051_LLM_Observability_Langfuse.md) | LLM Observability — Langfuse | Accepted | LLM tracing + retention. |
| [052](ADR-052_AI_Chat_Interface_LibreChat.md) | AI Chat Interface — LibreChat | Accepted | Self-hosted LLM UI. |
| [053](ADR-053_External_Secrets_Operator_ESO.md) | External Secrets Operator (ESO) | Accepted | Multi-backend secrets sync. |
| [055](ADR-055_WAF_&_DDoS_Protection_-_Cloudflare.md) | WAF & DDoS — Cloudflare | Accepted | `cave-waf` is the runtime mirror. |
| [056](ADR-056_Encryption_at_Rest_All_Data_Services.md) | Encryption at Rest | Accepted | Per-service + crypto-erasure policy. |
| [057](ADR-057_Application_Security_Testing_Strategy_Defense_in_Depth.md) | AppSec Testing Strategy — Defense-in-Depth | Accepted | Multi-layer security testing. |
| [058](ADR-058_Kubernetes_Compliance_Scanning_Kubescape.md) | Kubernetes Compliance Scanning — Kubescape | Accepted | K8s compliance gate. |
| [059](ADR-059_Kafka_Topic_Governance.md) | Kafka Topic Governance | Accepted | Topic/ACL/cleanup policy. |
| [060](ADR-060_Schema_Registry_Evolution_Policy.md) | Schema Registry & Evolution | Accepted | BACKWARD compatibility. |
| [061](ADR-061_OpenTofu_for_Day_0_Infrastructure.md) | OpenTofu for Day-0 Infrastructure | Accepted | IaC tool + Day-0/Day-1 boundary. |
| [066](ADR-066_Tenant_Provider_Choice_at_Onboarding.md) | Tenant Provider Choice at Onboarding | Suspect | `needs Burak verify`: premise is Hetzner/Azure split. |
| [067](ADR-067_Crossplane_v2_for_Day_1_Provisioning.md) | Crossplane v2 for Day-1+ Provisioning | Accepted | Continuous reconciliation. |
| [070](ADR-070_vcluster_CI_Mandatory_Prod_Opt_in_Dev_Staging.md) | vcluster CI — Prod Mandatory | Accepted | PR-validation policy. |
| [072](ADR-072_Prometheus_Federation_Thanos.md) | Prometheus Federation — Thanos | Accepted | Long-term metrics. |
| [074](ADR-074_MLOps_MLflow.md) | MLOps — MLflow | Accepted | MLOps tool. |
| [075](ADR-075_Serverless_Workloads_Knative_KEDA_Phase_4.md) | Serverless Workloads — Knative + KEDA | Proposed (Phase 4) | Phased serverless. |
| [076](ADR-076_cave_ctl_CLI_MCP_Server_Architecture.md) | cave-ctl CLI & MCP Server Architecture | Accepted | See also `RUNTIME-CLI-CONSOLIDATION-001`. |
| [077](ADR-077_Sigstore_Policy_Controller_for_Image_Admission.md) | Sigstore Policy Controller | Accepted | Image-admission policy. |
| [078](ADR-078_Platform_RBAC_Architecture.md) | RBAC Architecture | Accepted | Title neutralised; filename unchanged. |
| [079](ADR-079_Secret_Zero_Bootstrap_Break_Glass_Kit.md) | Secret Zero Bootstrap + Break-Glass | Accepted | Shamir + offline recovery. |
| [080](ADR-080_Backup_Retention_Policy.md) | Backup Retention Policy | Accepted | Tier-based retention. |
| [083](ADR-083_Automated_Secret_Rotation.md) | Automated Secret Rotation | Accepted | Rotation cadence + mechanism. |
| [084](ADR-084_Cilium_Default_Deny_Network_Policy_per_Tenant.md) | Default-Deny Network Policy per Tenant | Accepted | Multi-tenant netpol. |
| [085](ADR-085_Platform_Upgrade_Strategy.md) | Cave Runtime Upgrade Strategy | Accepted | Title neutralised; filename unchanged. |
| [086](ADR-086_Tenant_Offboarding_with_Crypto_Erasure.md) | Tenant Offboarding with Crypto-Erasure | Accepted | GDPR-driven. |
| [087](ADR-087_ResourceQuota_LimitRange_per_Tenant.md) | ResourceQuota + LimitRange per Tenant | Accepted | Multi-tenant quotas. |
| [088](ADR-088_Resurrection_Protocol.md) | Resurrection Protocol | Accepted | Cluster DR sequence. |
| [089](ADR-089_Signed_OPA_Policy_Bundles.md) | Signed OPA Policy Bundles | Accepted | Cosign-signed policy. |
| [090](ADR-090_Runtime_Forensics_Tetragon_Hubble_WORM.md) | Runtime Forensics → WORM | Accepted | Forensic capture. |
| [091](ADR-091_Entropy_Shadow_IT_Detection.md) | Entropy & Shadow-IT Detection | Accepted | `cave-ctl doctor` integration. |
| [092](ADR-092_AI_Privilege_Guardrails_MCP_Allowlist_Denylist.md) | AI Privilege Guardrails (MCP) | Accepted | AI-governance pattern. |
| [093](ADR-093_Sovereign_Ledger_WORM_Sigstore.md) | Sovereign Ledger (WORM + Sigstore) | Accepted | Immutable evidence. |
| [095](ADR-095_Reflex_Engine_KEDA_Argo_Workflows.md) | Reflex Engine (KEDA + Argo) | Accepted | Event-driven self-healing. |
| [096](ADR-096_Unit_Economics_FinOps_Attribution.md) | Unit Economics & FinOps Attribution | Accepted | Per-tenant cost. |
| [098](ADR-098_Talos_Linux_Immutable_Infrastructure.md) | Talos Linux Immutable Infrastructure | Suspect | `needs Burak verify`: vs ADR-001 baseline. |
| [099](ADR-099_Deprecation_Guardrails_in_CI_Pluto_kubent.md) | Deprecation Guardrails — Pluto + kubent | Accepted | CI gate. |
| [100](ADR-100_Continuous_Resilience_Attestation.md) | Continuous Resilience Attestation | Accepted | Continuous chaos + attestation. |
| [101](ADR-101_SLSA_Level_3_Supply_Chain_Provenance.md) | SLSA Level 3 Supply Chain | Accepted | Supply-chain provenance. |
| [102](ADR-102_Mandatory_Data_Classification_Labels.md) | Mandatory Data Classification | Accepted | Foundational classification. |
| [103](ADR-103_LLM_Data_Governance.md) | LLM Data Governance | Accepted | PII/redaction. |
| [104](ADR-104_Identity_Lifecycle_Governance.md) | Identity Lifecycle Governance | Accepted | Dormant/JIT/break-glass. |
| [105](ADR-105_etcd_KMS_Encryption_via_OpenBao_Transit_Key_Vault.md) | etcd KMS Encryption | Accepted | Envelope-encryption pattern. |
| [106](ADR-106_Loki_WORM_Backed_Storage_for_Forensic_Integrity.md) | Loki WORM-Backed Storage | Accepted | Forensic integrity. |
| [108](ADR-108_Helm_Manifest_Supply_Chain_Digest_Pinning.md) | Helm/Manifest Digest Pinning | Accepted | Supply-chain digest pinning. |
| [109](ADR-109_Observability_Multi_Tenancy_via_Label_Scoping.md) | Observability Multi-Tenancy | Accepted | Tenant-label scoping. |
| [110](ADR-110_Egress_Governance_Quarantine_Safe_Exit_List.md) | Egress Governance | Accepted | Egress quarantine + safe-exit. |
| [111](ADR-111_Classification_Aware_LLM_Inference_Routing.md) | Classification-Aware LLM Routing | Accepted | LLM routing policy. |
| [112](ADR-112_APOL_Autonomous_Platform_Operations_Layer.md) | APOL — Autonomous Operations | Accepted | AI-ops bounded autonomy. |
| [113](ADR-113_Data_Residency_Enforcement_via_Crossplane_XR.md) | Data Residency via Crossplane XR | Accepted | GDPR residency. |
| [114](ADR-114_Qdrant_Vector_DB_as_Crossplane_XR.md) | Qdrant Vector DB as Crossplane XR | Accepted | Vector-DB unification. |
| [115](ADR-115_CI_Secret_Injection_via_OIDC_Token_Exchange.md) | CI Secret Injection via OIDC | Accepted | Short-lived credentials. |
| [118](ADR-118_APOL_Fallback_Mode_Manual_Operations.md) | APOL Fallback Mode | Accepted | Manual-ops fallback. |
| [119](ADR-119_Crossplane_Operations_for_Day_2_Maintenance.md) | Crossplane Ops for Day-2 | Accepted | Day-2 reconcile pattern. |
| [120](ADR-120_ArgoCD_OCI_Source_Harbor_Registry_as_Manifest_Source.md) | ArgoCD OCI Source — Harbor | Accepted | OCI vs Git manifest source. |
| [121](ADR-121_Istio_Ambient_Multi_Cluster_Non_Baseline_Until_Stable.md) | Istio Ambient Multi-Cluster — Non-Baseline | Accepted | Upstream-maturity gate. |
| [122](ADR-122_Cilium_Gateway_API_Reserved_for_Future_Internal_Routing.md) | Cilium Gateway API Reserved | Accepted | Gateway-boundary decision. |
| [124](ADR-124_Crossplane_MRAP_ManagedResourceActivationPolicy.md) | Crossplane MRAP | Accepted | CRD-reduction policy. |
| [125](ADR-125_APOL_Chain_of_Thought_Audit_Trail.md) | APOL Chain-of-Thought Audit | Accepted | AI reasoning trace. |
| [126](ADR-126_Workload_Criticality_Labels_for_Kill_Switch_Ethics.md) | Workload Criticality Labels | Accepted | Three-tier criticality. |
| [127](ADR-127_Roadmap_Intelligence_Automation.md) | Roadmap Intelligence Automation | Accepted | Upstream-tracking automation. |
| [128](ADR-128_APOL_Attestation_Redaction_Policy.md) | APOL Attestation Redaction | Accepted | Two-tier redaction. |
| [129](ADR-129_Tenant_Identity_Federation_BYOID.md) | Tenant Identity Federation — BYOID | Accepted | Tenant-IdP federation. |
| [130](ADR-130_Privileged_Access_Management_PAM_Layer.md) | PAM Layer | Proposed | PAM architectural gap. |
| [131](ADR-131_OPAL_for_Real_Time_Policy_Data_Distribution.md) | OPAL — Real-Time Policy Data | Accepted | Policy data distribution. |
| [132](ADR-132_Version_Channel_Soak_Policy.md) | Version Channel & Soak | Accepted | Multi-channel governance. |
| [133](ADR-133_Compatibility_Matrix_as_Code.md) | Compatibility Matrix as Code | Accepted | Compat-matrix governance. |
| [134](ADR-134_Deprecation_Runway_Enforcement.md) | Deprecation Runway Enforcement | Accepted | Category-based deprecation. |
| [135](ADR-135_Provider_Parity_Contract_Testing.md) | Provider Parity Contract Testing | Suspect | `needs Burak verify`: vs upstream-test-port. |
| [136](ADR-136_APOL_Bounded_Autonomy_Model.md) | APOL Bounded Autonomy | Accepted | Class A-D autonomy. |
| [137](ADR-137_Constitutional_Tiering.md) | Constitutional Tiering | Accepted | Tiered protection. |
| [138](ADR-138_Evidence_Tiering.md) | Evidence Tiering | Accepted | Risk-proportional evidence. |
| [139](ADR-139_Data_Contract_Governance.md) | Data Contract Governance | Accepted | Cross-system contracts. |
| [140](ADR-140_Waiver_Framework.md) | Waiver Framework | Accepted | TTL-bounded waivers. |
| [141](ADR-141_Shared_Fate_Tenant_Priority.md) | Shared-Fate & Tenant Priority | Accepted | Multi-tenant priority. |
| [142](ADR-142_Passwordless_Authentication_Strategy.md) | Passwordless Authentication (FIDO2/WebAuthn) | Accepted | Auth strategy. |
| [143](ADR-143-cave-communication-hub.md) | Cave Communication Hub | Accepted | Multi-surface chat platform. |
| [144](ADR-144-code-intelligence-gitnexus.md) | Code Intelligence — GitNexus | Proposed | Title neutralised. `needs Burak verify` (Proposed). |
| [145](ADR-145_CRM_Upstream_Selection_Twenty.md) | CRM Upstream — Twenty | Accepted | cave-crm upstream. |
| [146](ADR-146_Karpenter_Node_Autoscaling.md) | Karpenter for Node Autoscaling | Accepted | Cave Runtime selection (see also ADR-032). |
| [147](ADR-147_Data_Persistence_Crate_Naming_and_Lakehouse_Consolidation.md) | Data Persistence Crate Naming + Lakehouse | Accepted | Cave Runtime crate naming. |
| [148](ADR-148_OSS_Launch_History_Strategy.md) | OSS Launch History Strategy | Accepted | OSS hygiene policy. |
| [149](ADR-149_KubeVirt_Sovereign_VM_Workloads.md) | KubeVirt for Sovereign VM Workloads | Accepted | VM workload runtime. |

## Cave Runtime topic ADRs

| ID | Title | Status |
|---|-------|--------|
| [RUNTIME-STACK-001](ADR-RUNTIME-STACK-001-cave-runtime-stack-architecture.md) | Cave Runtime Stack Architecture | Accepted |
| [RUNTIME-API-GATEWAY-CONSOLIDATION-001](ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001-kong-gravitee-into-cave-gateway.md) | Kong + Gravitee → cave-gateway | Accepted |
| [RUNTIME-CERT-LIFECYCLE-001](ADR-RUNTIME-CERT-LIFECYCLE-001-sovereign-cert-hierarchy-pqc-acme.md) | Sovereign Cert Hierarchy + PQC + Multi-DNS ACMEv2 | Accepted |
| [RUNTIME-CLI-CONSOLIDATION-001](ADR-RUNTIME-CLI-CONSOLIDATION-001-cavectl-native-and-compat.md) | Single cavectl Binary, Native + Compat Surfaces | Accepted |
| [RUNTIME-PERSISTENCE-CONSOLIDATION-001](ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001-multi-upstream-data-layer.md) | Multi-Upstream Persistence Consolidation | Accepted |
| [RUNTIME-STREAMING-CONSOLIDATION-001](ADR-RUNTIME-STREAMING-CONSOLIDATION-001-kafka-pulsar-into-cave-streams.md) | Kafka + Pulsar → cave-streams | Accepted |
| [RUNTIME-UPSTREAM-MIRROR-001](ADR-RUNTIME-UPSTREAM-MIRROR-001-platform-runtime-mirror.md) | Platform → Runtime Upstream Mirror | Accepted |
| [RUNTIME-UPSTREAM-WATCH-001](ADR-RUNTIME-UPSTREAM-WATCH-001.md) | Upstream Watch Daemon | Accepted |

## Cave Portal topic ADRs

| ID | Title | Status |
|---|-------|--------|
| [PORTAL-AUTH-001](ADR-PORTAL-AUTH-001.md) | Portal Authentication — cave-auth OIDC First | Accepted |
| [PORTAL-DESKTOP-001](ADR-PORTAL-DESKTOP-001-gpui-native-admin-shell.md) | GPUI Native Desktop Shell Alongside Web Portal | Accepted |
| [PORTAL-PERSONAS-001](ADR-PORTAL-PERSONAS-001.md) | Two-Persona Portal — Admin vs Tenant | Accepted |

## Other charters

| ID | Title | Status |
|---|-------|--------|
| [MULTI-TENANT-001](ADR-MULTI-TENANT-001.md) | Cave Runtime is Multi-Tenant by Construction | Accepted |
| [CONTRIB-ATTRIBUTION-001](ADR-CONTRIB-ATTRIBUTION-001.md) | Commit Author + Trailer Attribution | Accepted |
| [SELF-IMPROVE-001](ADR-SELF-IMPROVE-001.md) | cave-agent — Runtime-Resident Self-Improvement | Accepted |

## Internal (archival)

Provider-specific deployment ADRs from the closed-source CAVE platform have
been moved to [`internal/`](internal/). They are kept verbatim for archival
reference and are **not normative** for the OSS runtime release. The
provider integrations they describe remain available as optional Cave
Runtime plugins behind feature flags.

## ADR Lifecycle

1. **Proposed** — first commit with context, alternatives, consequences.
2. **Accepted** — implementation underway or complete; cross-referenced from
   ARCHITECTURE / CHARTER / ROADMAP where load-bearing.
3. **Superseded** — body retained for history; header carries
   `Superseded-By: <new-ADR>`.
4. **Deprecated** — kept for reference; carries `Deprecated:` reason.

Status `Suspect` in the index above is **not** a formal ADR status — it is
a sprint-cleanup marker for ADRs whose Cave Runtime relevance could not be
decided autonomously. Each is annotated with a `<!-- needs Burak verify -->`
comment inside the file.

---

Last updated: 2026-05-18 (adr-sprint-v2 cleanup pass).
