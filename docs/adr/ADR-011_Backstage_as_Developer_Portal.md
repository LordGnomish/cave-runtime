<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
<!-- needs Burak verify: cave-portal is the OSS direction; does Backstage remain in the runtime story or is this ADR superseded? -->
# ADR-011: Backstage as Developer Portal

**Status:** Accepted

**Scope:** Universal

**Category:** Platform

**Related ADRs:** 025, 123

## Context

CAVE needs a unified developer portal that provides self-service infrastructure provisioning, service catalog, documentation, CI/CD visibility, and API catalog — across both Hetzner and Azure profiles.

## Candidates

| Criteria | Backstage | Port | Cortex | Custom Build |
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

**Backstage** (self-hosted on K8s via Helm).

## Rejected

- **Port:** SaaS-only. No self-hosting option. Contradicts sovereign profile requirement. Vendor lock-in.
- **Cortex:** SaaS-focused. Less extensible catalog model. Smaller plugin ecosystem. No declarative integration.
- **Custom portal:** Build cost prohibitive. Backstage ecosystem provides 300+ plugins out of box. Maintaining custom portal for 73 components is unsustainable.

## Consequences

(+) CNCF project with massive community. Rich plugin ecosystem. Declarative Integration eliminates TypeScript maintenance (ADR-123). Software catalog indexes all 73 components + tenant services. Templates scaffold complete projects in <5min.
(-) TypeScript/Node.js stack (team must maintain runtime). Resource-intensive (~1-2GB RAM). Backstage upgrades sometimes breaking — mitigated by Declarative Integration (no custom TS code to break). PostgreSQL dependency for catalog backend.

## Implementation Reference

**cave-portal** crate is the Rust reimplementation of Backstage, embedded in cave-runtime. It provides the same software catalog, service templating, and developer self-service capabilities as Backstage — but as part of the single-binary runtime, eliminating the Node.js/TypeScript dependency and PostgreSQL backend requirement. The Backstage plugin ecosystem is reimplemented as native Rust modules within the runtime.

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Backstage upstream breaking changes (major version) | Medium | Medium | cave-portal tracks Backstage API surface, not implementation. Declarative Integration (ADR-123) reduces coupling. Pin to tested version. |
| Backstage CNCF graduation stalls | Low | Low | Backstage has Spotify + 100s of enterprise adopters. Stalling unlikely. cave-portal provides full fallback regardless. |
| cave-portal feature parity gap vs Backstage | Medium | Medium | Prioritize: software catalog, scaffolder, TechDocs. 300+ Backstage plugins not all needed — implement the 20 most used. |
| Kratix emerges as platform orchestration layer | Low (2027) | Low | **Watch:** Kratix (Syntasso) provides platform-as-a-product via Kubernetes Promises. Complementary to Backstage/cave-portal for infrastructure templating. If Kratix matures, evaluate as alternative to Backstage Scaffolder for Crossplane XR provisioning. Annual review. |
| Node.js/TypeScript maintenance burden (if using upstream Backstage) | N/A | N/A | Mitigated: cave-portal is pure Rust. No Node.js dependency in runtime. |

## Compliance Mapping

SOC2 CC8.1 (change management visibility), ISO A.5.37 (documented operating procedures via TechDocs).
