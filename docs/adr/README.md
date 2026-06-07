# Cave Runtime — Architecture Decision Records

Decisions that shape **Cave Runtime** itself — the Rust runtime crates,
the deep-port programme, OSS launch posture, and the runtime-resident
agents and operators that live inside a Cave cluster.

Platform / hosting / day-0 infrastructure choices (CNI selection,
identity provider matrix, cloud-provider integration, observability
stack assembly, etc.) are documented in a separate platform repository
and are **not** carried by this OSS Cave Runtime catalogue.

## Index

### Foundational (charter-binding, runtime-wide)

| ADR | Title |
|-----|-------|
| [ADR-001](ADR-001-sovereign-bare-metal-hosting.md) | Sovereign bare-metal hosting reference profile |
| [ADR-MULTI-TENANT-001](ADR-MULTI-TENANT-001.md) | Cave Runtime is multi-tenant by construction |
| [ADR-CONTRIB-ATTRIBUTION-001](ADR-CONTRIB-ATTRIBUTION-001.md) | Commit author + trailer attribution |
| [ADR-SELF-IMPROVE-001](ADR-SELF-IMPROVE-001.md) | cave-agent — runtime-resident self-improvement |

### Platform → Runtime sovereign variants (Charter mirror)

Runtime-adapted variants of platform ADRs, ported under the
[mirror principle](ADR-RUNTIME-UPSTREAM-MIRROR-001-platform-runtime-mirror.md).
Each keeps the platform decision's intent but re-roots it on cave-native,
sovereign, provider-agnostic primitives. Cloud-managed comparison columns
(Azure Redis, Azure OpenAI, Hetzner-as-sole-provider) are dropped or demoted
to provider-equal examples.

| ADR | Title |
|-----|-------|
| [ADR-003-RUNTIME](ADR-003-RUNTIME-talos-linux.md) | Talos Linux as the sovereign immutable node OS (provider-agnostic via cave-cloud-controller-manager) |
| [ADR-004-RUNTIME](ADR-004-RUNTIME-cilium-istio.md) | Cilium (cave-net + cave-cilium) + Istio Ambient (cave-mesh) — PQC-ready WireGuard, single-binary |
| [ADR-005-RUNTIME](ADR-005-RUNTIME-buildah.md) | Buildah hermetic rootless build — distroless, RISC-V multi-arch, SLSA L4, ML-DSA hybrid sign |
| [ADR-006-RUNTIME](ADR-006-RUNTIME-cave-auth.md) | cave-auth sovereign identity (Keycloak parity) — OIDC + RBAC + ABAC + SPIFFE |
| [ADR-008-RUNTIME](ADR-008-RUNTIME-cave-cache.md) | cave-cache sovereign in-memory store (Valkey parity) — Azure Redis dropped |
| [ADR-009-RUNTIME](ADR-009-RUNTIME-cave-hermes.md) | cave-hermes sovereign local LLM gateway over Ollama — Azure OpenAI dropped |
| [ADR-010-RUNTIME](ADR-010-RUNTIME-ci-pipeline.md) | Multi-dimensional future-proof CI pipeline (Argo Workflows, ~47 stage) |
| [ADR-011-RUNTIME](ADR-011-RUNTIME-cave-portal.md) | cave-portal sovereign developer portal — Rust-native Backstage parity, single-binary (Backstage runtime dropped) |
| [ADR-001-COLLISION-2026-06-07](ADR-001-COLLISION-2026-06-07.md) | ADR-001 numbering collision reconciliation (Hetzner vs. bare-metal — **decision pending**) |

### Runtime stack & consolidation

| ADR | Title |
|-----|-------|
| [ADR-RUNTIME-STACK-001](ADR-RUNTIME-STACK-001-cave-runtime-stack-architecture.md) | Cave Runtime stack architecture |
| [ADR-RUNTIME-UPSTREAM-MIRROR-001](ADR-RUNTIME-UPSTREAM-MIRROR-001-platform-runtime-mirror.md) | Platform → Runtime mirror principle |
| [ADR-RUNTIME-UPSTREAM-WATCH-001](ADR-RUNTIME-UPSTREAM-WATCH-001.md) | Upstream watch & drift detection |
| [ADR-RUNTIME-CLI-CONSOLIDATION-001](ADR-RUNTIME-CLI-CONSOLIDATION-001-cavectl-native-and-compat.md) | `cavectl` native + compatibility consolidation |
| [ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001](ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001-kong-gravitee-into-cave-gateway.md) | Kong + Gravitee into cave-gateway |
| [ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001](ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001-multi-upstream-data-layer.md) | Multi-upstream data layer (cave-rdbms / cave-docdb / cave-cache / cave-etcd) |
| [ADR-RUNTIME-STREAMING-CONSOLIDATION-001](ADR-RUNTIME-STREAMING-CONSOLIDATION-001-kafka-pulsar-into-cave-streams.md) | Kafka + Pulsar into cave-streams |
| [ADR-RUNTIME-CERT-LIFECYCLE-001](ADR-RUNTIME-CERT-LIFECYCLE-001-sovereign-cert-hierarchy-pqc-acme.md) | Sovereign cert hierarchy — PQC + ACME |
| [ADR-076](ADR-076_cave_ctl_CLI_MCP_Server_Architecture.md) | cave-ctl CLI & MCP server architecture |
| [ADR-147](ADR-147_Data_Persistence_Crate_Naming_and_Lakehouse_Consolidation.md) | Data-persistence crate naming + lakehouse consolidation |

### Crate-level decisions

| ADR | Title |
|-----|-------|
| [ADR-031](ADR-031-cave-webapplication-composition-pattern.md) | cave-webapplication composition pattern |
| [ADR-143](ADR-143-cave-communication-hub.md) | cave-communication — multi-surface team + LLM chat |
| [ADR-145](ADR-145_CRM_Upstream_Selection_Twenty.md) | cave-crm — Twenty as upstream |
| [ADR-146](ADR-146_Karpenter_Node_Autoscaling.md) | cave-karpenter — node autoscaling |
| [ADR-149](ADR-149_KubeVirt_Sovereign_VM_Workloads.md) | cave-kubevirt — sovereign VM workloads |
| [ADR-150](ADR-150_Hermes_Agent_Adoption_AC_Path.md) | cave-hermes — agent adoption (AC path) |
| [ADR-152](ADR-152_LLM_Tracker_Daily_Always_Latest.md) | cave-llm-tracker — daily always-latest tracker |
| [ADR-153](ADR-153_LLM_Gateway_MVP.md) | cave-llm-gateway — MVP |
| [ADR-154](ADR-154_ArgoCD_GitOps_Adoption.md) | cave-deploy — ArgoCD GitOps adoption |

### Portal

| ADR | Title |
|-----|-------|
| [ADR-PORTAL-AUTH-001](ADR-PORTAL-AUTH-001.md) | cave-portal authentication |
| [ADR-PORTAL-PERSONAS-001](ADR-PORTAL-PERSONAS-001.md) | cave-portal personas (admin / tenant surfaces) |
| [ADR-PORTAL-DESKTOP-001](ADR-PORTAL-DESKTOP-001-gpui-native-admin-shell.md) | cave-portal desktop — GPUI native admin shell |

### Programme / OSS hygiene

| ADR | Title |
|-----|-------|
| [ADR-148](ADR-148_OSS_Launch_History_Strategy.md) | OSS launch history strategy |
| [ADR-151](ADR-151_Phantom_Crate_Audit_Cleanup.md) | Phantom-crate audit cleanup |

## ADR numbering policy

Numbers are **stable** — no renumbering after merge. Gaps in the sequence are
expected (decisions that lived in the prior platform catalogue were removed
from this OSS repository; their numbers are not reused).

New ADRs pick the next free number above ADR-154, or use a topic-prefixed
identifier (`ADR-RUNTIME-…`, `ADR-PORTAL-…`, `ADR-SELF-IMPROVE-…`,
`ADR-CONTRIB-…`, `ADR-MULTI-TENANT-…`).

## ADR lifecycle

1. **Proposed** — full context, alternatives, consequences; awaiting acceptance.
2. **Accepted** — implementation is proceeding or complete.
3. **Deprecated** — superseded but retained for historical reference.

## Related

- Crate parity manifests: `crates/<crate>/parity.manifest.toml`
- Parity index: `docs/parity/parity-index.json`
- Charter v2 self-audit gates: `crates/<crate>/src/parity_self_audit.rs`
