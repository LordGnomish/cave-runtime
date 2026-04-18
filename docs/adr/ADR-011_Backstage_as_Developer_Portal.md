# ADR-011: Backstage as Developer Portal

**Status:** Accepted

**Category:** Platform

**Related ADRs:** 025, 123

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE needs a unified developer portal that provides self-service infrastructure provisioning, service catalog, documentation, CI/CD visibility, and API catalog — across both Hetzner and Azure profiles.

## Candidates

## | Criteria | Backstage | Port | Cortex | Custom Build |
|---|---|---|---|---|
| Self-hosted | ✅ K8s Helm | ❌ SaaS only | ❌ SaaS-focused | ✅ |
| Plugin ecosystem | 300+ community plugins | Limited | Limited | N/A |
| Software catalog | YAML-driven, rich entity model | Tag-based | Tag-based | Custom |
| Scaffolder (templates) | ✅ Full-featured | ✅ | ✅ | Custom |
| TechDocs | ✅ Built-in (docs-as-code) | Limited | Limited | Custom |
| Search | ✅ Lunr/Elasticsearch backend | ✅ | ✅ | Custom |
| RBAC | Permission framework (tenant-scoped) | Role-based | Role-based | Custom |
| Declarative integration | ✅ New Frontend System (YAML, no TS code — ADR-123) | N/A | N/A | N/A |
| License | Apache 2.0 | Proprietary | Proprietary | N/A |
| Community | Very large (CNCF Incubating, Spotify-originated, 28K+ GitHub stars) | Small | Small | N/A |

## Decision

## **Backstage** (self-hosted on K8s via Helm).

## Rejected

## - **Port:** SaaS-only. No self-hosting option. Contradicts sovereign profile requirement. Vendor lock-in.
- **Cortex:** SaaS-focused. Less extensible catalog model. Smaller plugin ecosystem. No declarative integration.
- **Custom portal:** Build cost prohibitive. Backstage ecosystem provides 300+ plugins out of box. Maintaining custom portal for 73 components is unsustainable.

## Consequences

## (+) CNCF project with massive community. Rich plugin ecosystem. Declarative Integration eliminates TypeScript maintenance (ADR-123). Software catalog indexes all 73 components + tenant services. Templates scaffold complete projects in <5min.
(-) TypeScript/Node.js stack (team must maintain runtime). Resource-intensive (~1-2GB RAM). Backstage upgrades sometimes breaking — mitigated by Declarative Integration (no custom TS code to break). PostgreSQL dependency for catalog backend.

## Compliance Mapping

## SOC2 CC8.1 (change management visibility), ISO A.5.37 (documented operating procedures via TechDocs).
