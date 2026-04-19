# ADR-038: Workflow Orchestration — Argo Workflows

**Status:** Accepted

**Scope:** Universal

**Category:** CI/CD

**Related ADRs:** 095

## Context

CAVE needs a workflow engine for complex multi-step operations: ML training pipelines, Reflex Engine remediation playbooks (ADR-095), data migration sequences, and batch processing. Must be K8s-native with DAG support, retries, and artifact passing.

## Candidates

| Criteria | Argo Workflows | Airflow | Temporal | Prefect |
|---|---|---|---|---|
| K8s native | ✅ CRD-based (Workflow, CronWorkflow) | ⚠️ KubernetesExecutor | ❌ Separate server | ❌ Separate server |
| DAG support | ✅ Steps + DAG | ✅ | ✅ | ✅ |
| Artifact passing | ✅ S3/MinIO/ADLS | ✅ XCom | ✅ | ✅ |
| KEDA integration | ✅ Event-driven trigger | ⚠️ | ❌ | ❌ |
| ArgoCD ecosystem | ✅ Same Argo project | ❌ | ❌ | ❌ |
| Retries + timeouts | ✅ Per-step | ✅ | ✅ | ✅ |
| License | Apache 2.0 | Apache 2.0 | MIT | Apache 2.0 (OSS) |

## Decision

**Argo Workflows** for all workflow orchestration. Used by Reflex Engine (KEDA triggers → Argo Workflows), ML training pipelines, and batch data processing. CronWorkflow for scheduled jobs. Same Argo ecosystem as ArgoCD and Argo Rollouts.

## Rejected

- **Airflow:** Not K8s-native (requires separate scheduler, webserver, DB). KubernetesExecutor exists but Airflow's architecture is more complex than Argo Workflows for CAVE's use cases.
- **Temporal:** Powerful but separate server infrastructure. Not K8s-native CRDs. Overkill for CAVE's workflow needs.
- **Prefect:** Good for data engineering but separate server, less K8s-native integration.

## Consequences

**Positive:**
- K8s-native CRDs — GitOps-managed via ArgoCD.
- KEDA integration for event-driven Reflex Engine.
- Same Argo ecosystem (CD, Rollouts, Workflows) — consistent.
- DAG + artifact passing covers all CAVE use cases (ML training, remediation, migration).

**Negative:**
- Argo Workflows has its own RBAC model — must align with CAVE platform RBAC.
- Workflow template versioning and signing (cosign) adds maintenance.
- Artifact storage (MinIO/ADLS) must be configured per profile.

## Compliance Mapping

SOC2 CC7.2 (automated incident response via Reflex Engine). ISO A.5.26 (incident response automation).
