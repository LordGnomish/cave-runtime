# ADR-078: Platform RBAC Architecture

**Status:** Accepted

**Category:** Governance

**Related ADRs:** 006, 007, 064, 104

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE has multiple systems with RBAC: Backstage, ArgoCD, Grafana, Harbor, Kong, Keycloak/Okta, K8s RBAC. Roles must be consistent across all systems. A user with "Tenant Admin" role should have matching permissions in every system.

## Candidates

## | Approach | Profile-based AD groups (chosen) | Per-system independent RBAC | Fine-grained per-resource | ABAC (attribute-based) |
|---|---|---|---|---|
| Consistency across systems | ✅ Same role everywhere | ❌ Role drift inevitable | ⚠️ Complex to enforce consistently | ⚠️ Implementation complexity |
| Maintainability | ✅ 5 platform roles | ❌ N systems × M roles each | ❌ Role explosion | ⚠️ Policy complexity |
| Auditability | ✅ Group membership → clear access | ❌ Must audit N systems | ⚠️ Per-resource audit | ⚠️ Attribute lineage |
| SCIM compatibility | ✅ Groups sync natively | ⚠️ Per-system mapping | ❌ Custom SCIM logic | ❌ |
| Granularity trade-off | ⚠️ Edge cases need waiver (ADR-140) | ✅ Maximum granularity | ✅ | ✅ |

## Decision

## **Profile-based AD group approach:** Platform roles (Platform Admin, Guardian, Tenant Admin, Developer, Viewer) defined centrally in Keycloak (Hz) / Okta (Az). Mapped to system-specific roles via SCIM/OIDC group claims:

| Platform Role | K8s | ArgoCD | Grafana | Harbor | Kong | Backstage |
|---|---|---|---|---|---|---|
| Platform Admin | cluster-admin (scoped) | admin | Server Admin | admin | admin | admin |
| Guardian | cluster-admin + break-glass | admin | Server Admin | admin | admin | admin |
| Tenant Admin | namespace-admin | AppProject admin | Org Admin | project-admin | tenant-admin | catalog-owner |
| Developer | namespace-edit | AppProject read-only | Org Editor | project-push | consumer | catalog-user |
| Viewer | namespace-view | read-only | Org Viewer | project-pull | — | catalog-viewer |

## Rejected

## - **Per-system independent RBAC:** Each system manages its own roles. Inconsistency, role drift, manual sync. Unmanageable.
- **Platform-granular RBAC (fine-grained per-resource):** Too complex. Too many role combinations. Profile-based groups provide sufficient granularity with manageable complexity.

## Consequences

## **Positive:**
- Consistent roles across all systems — "Tenant Admin" means the same thing in Backstage, ArgoCD, Grafana, Harbor, Kong.
- Central role definition in Keycloak/Okta — SCIM propagates to all systems.
- Profile-based approach limits role explosion (5 platform roles vs hundreds of fine-grained permissions).
- RBAC drift detectable via `cave-ctl identity drift`.

**Negative:**
- Profile-based RBAC is less granular than per-resource permissions. Edge cases may need waiver (ADR-140).
- SCIM sync latency between Keycloak/Okta and downstream systems (~minutes).
- Each downstream system (ArgoCD, Grafana, Harbor, Kong) has different RBAC mapping mechanism — configuration per system required.
- New system addition requires RBAC mapping definition + SCIM integration.

## Compliance Mapping

## SOC2 CC6.1-6.3 (access control, provisioning, deprovisioning). ISO A.5.15-18 (access control, identity, authentication). NIS2 Art.21 (access control). GDPR Art.32 (access management).
