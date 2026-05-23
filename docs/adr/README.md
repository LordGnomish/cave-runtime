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
