# ADR-114: Qdrant Vector DB as Crossplane XR

**Status:** Accepted

**Scope:** Hyperscaler, Sovereign, Universal

**Category:** Data

**Related ADRs:** 067

## Context

CAVE tenants need vector search for RAG, semantic search, and ML similarity workloads. Vector DB must be abstracted behind the same XR pattern as other data services.

## Candidates

| Criteria | Qdrant (Hz) + Azure AI Search (Az) | Weaviate | Milvus | Pinecone |
|---|---|---|---|---|
| Self-hosted K8s operator | ✅ Qdrant Kubernetes | ✅ Weaviate operator | ✅ Milvus operator | ❌ SaaS only |
| Azure managed equivalent | ✅ Azure AI Search vector | ❌ | ❌ | ❌ |
| Resource footprint | ✅ Lightweight (~500MB per node) | ⚠️ Moderate | ❌ Heavy (etcd, Minio, Pulsar deps) | N/A |
| API | gRPC + REST | GraphQL + REST | gRPC + REST | REST |
| Filtering | ✅ Rich payload filtering | ✅ | ✅ | ✅ |

## Decision

Unified VectorDB XRD. Hetzner: Qdrant operator. Azure: Azure AI Search vector capability. Same developer API (VectorDB XR) regardless of backend. Classification and residency enforced via standard XR labels.

## Rejected

- **Weaviate:** No Azure managed equivalent. Would require self-hosting on both providers, breaking managed-service model on Azure.
- **Milvus:** Heavy dependency chain (etcd, MinIO, Pulsar). Resource footprint too high for per-tenant vector DB.
- **Pinecone:** SaaS only. No self-hosting. Vendor lock-in. Contradicts sovereign profile.

## Consequences

**Positive:**
- Same XR API for vector search across both providers.
- Lightweight on the sovereign profile (Qdrant), managed on Azure (AI Search).
- Classification + residency enforcement inherited from XR framework.

**Negative:**
- Qdrant ↔ Azure AI Search API differences abstracted by Composition, but query semantics may differ slightly.
- Vector index rebuild required during cross-provider migration (not synced).
- Qdrant operator maturity less than PostgreSQL (CNPG) — monitoring coverage may be thinner.

## Compliance Mapping

GDPR Art.25 (data protection by design — residency on vector data). ISO A.5.12 (classification of vector embeddings).
