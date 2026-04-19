# ADR-074: MLOps — MLflow

**Status:** Accepted

**Scope:** Azure, Universal

**Category:** AI/ML

**Related ADRs:** 051, 038

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE tenants running ML workloads need experiment tracking, model registry, model versioning, and artifact storage. Must integrate with Argo Workflows for training pipelines.

## Candidates

## | Criteria | MLflow | Kubeflow | Weights & Biases | ClearML |
|---|---|---|---|---|
| Self-hosted | ✅ K8s Helm | ✅ K8s (complex) | ❌ SaaS | ✅ |
| Experiment tracking | ✅ | ✅ | ✅ (best UI) | ✅ |
| Model registry | ✅ | ⚠️ | ✅ | ✅ |
| Artifact storage | ✅ MinIO/ADLS (S3 compatible) | ✅ | ✅ (cloud) | ✅ |
| License | Apache 2.0 | Apache 2.0 | Proprietary | Apache 2.0 |
| Complexity | ✅ Moderate | ❌ Very high (20+ components) | ✅ Low (SaaS) | ✅ Moderate |

## Decision

## **MLflow** (Apache 2.0) for experiment tracking and model registry. Artifact storage on MinIO (Hz) / ADLS (Az). Training pipelines via Argo Workflows (ADR-038). Per-tenant MLflow projects for isolation.

## Rejected

## - **Kubeflow:** Extremely complex (20+ components). Overkill for CAVE's ML use cases. Would consume significant GOT budget.
- **Weights & Biases:** SaaS. ML artifacts and experiment data sent externally.
- **ClearML:** Capable but smaller community than MLflow. MLflow is the industry standard for experiment tracking.

## Consequences

## **Positive:**
- Industry-standard experiment tracking — data scientists already know MLflow.
- Model registry provides versioning, staging, and production promotion workflow.
- Artifact storage on MinIO/ADLS integrates with existing object storage.
- Apache 2.0 — no licensing concerns.
- Per-tenant MLflow projects provide isolation.

**Negative:**
- MLflow server requires PostgreSQL backend + ~512MB RAM.
- MLflow UI is functional but less polished than W&B.
- Model serving not included (MLflow provides registry, not inference server — Ollama/Azure OpenAI handle inference).
- Artifact storage for large models (GB-scale) requires MinIO/ADLS capacity planning.

## Compliance Mapping

## SOC2 CC8.1 (ML model lifecycle management). ISO A.14.2 (secure development — model versioning). GDPR Art.25 (data protection — per-tenant experiment isolation).
