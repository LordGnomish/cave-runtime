# ADR-049: Full-Text Search — OpenSearch (Hetzner) / Azure AI Search (Azure)

**Status:** Accepted

**Scope:** Universal, Hetzner, Azure

**Category:** Data

**Related ADRs:** 067, 114, 135

## Context

CAVE tenants need full-text search for application data, log analytics (via Loki, not Elasticsearch), and tenant-facing search features. Solution must be Crossplane XR-abstracted.


## Candidates

| Criteria | OpenSearch (Hz) + Azure AI Search (Az) | Elasticsearch | Typesense | Meilisearch |
|---|---|---|---|---|
| Self-hosted K8s | ✅ OpenSearch K8s operator | ⚠️ SSPL license | ✅ Helm | ✅ Helm |
| Managed option | ✅ Azure AI Search | ✅ Elastic Cloud | ❌ | ❌ |
| Vector search | ✅ k-NN plugin | ✅ | ✅ | ✅ |
| License | Apache 2.0 (AWS fork) | SSPL (restrictive) | GPL-3.0 | MIT |
| Community | Large (AWS + community) | Large (Elastic) | Growing | Growing |


## Decision

**OpenSearch** (Apache 2.0, AWS fork of Elasticsearch 7.10) on Hetzner. **Azure AI Search** on Azure. Unified Search XRD via Crossplane.


## Rejected Options

- **Elasticsearch:** SSPL license (same concern as Vault/Redis). OpenSearch is API-compatible fork under Apache 2.0.
- **Typesense:** GPL-3.0 (more restrictive than Apache). No managed Azure equivalent.
- **Meilisearch:** Lighter but less feature-rich. No managed Azure equivalent. Better suited for small-scale search.


## Consequences

**Positive:**
- Apache 2.0 — no license restrictions.
- OpenSearch maintains Elasticsearch API compatibility — existing clients work.
- Azure AI Search provides managed full-text + vector search (ADR-114) on Azure.

**Negative:**
- OpenSearch diverging from Elasticsearch over time — some plugins/features may differ.
- OpenSearch operator is community-maintained (not official AWS project).
- Index rebuild required during cross-provider migration (search indices not synced).

Compliance Mapping

SOC2 CC6.1 (search result access controls — index-per-tenant). ISO A.8.22 (data segregation in search indices). GDPR Art.32 (tenant data isolation).

