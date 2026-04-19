# ADR-025: Same Backstage UX Across All Providers

**Status:** Accepted

**Scope:** Azure, Hetzner, Runtime, Universal

**Category:** Platform

**Related ADRs:** 011, 067

## Context

CAVE runs on two fundamentally different providers (Hetzner self-hosted, Azure managed). Developers should not need to understand or interact with provider-specific details when using the platform.

## Candidates

| Approach | Same UX (chosen) | Provider-specific templates | Separate portals |
|---|---|---|---|
| Developer cognitive load | ✅ Zero provider awareness | ❌ Must choose provider details | ❌ Must know which portal |
| Template maintenance | ✅ Single template per resource type | ❌ 2x templates (Hz + Az) | ❌ 2x everything |
| Provider parity validation | ✅ Same XR tests both providers | ⚠️ Different templates = different behavior | ❌ No cross-portal validation |
| Crossplane abstraction | ✅ XR → Composition selects provider | ⚠️ Template selects provider | ❌ Portal selects provider |

## Decision

Backstage presents identical self-service UX regardless of target provider. Backstage templates generate the same Crossplane XR YAML for both providers — the Crossplane Composition selects provider-specific resources based on the deployment profile. Backstage never exposes provider-specific configuration fields to developers. Provider choice is made at tenant onboarding (ADR-066), not per-resource.

## Rejected

- **Provider-specific templates:** Would require developers to understand provider differences (e.g., CNPG vs Azure PG Flexible, MinIO vs ADLS). Increases cognitive load. Creates template drift between providers. Undermines the "developer doesn't choose infrastructure" principle.
- **Separate portals per provider:** Double maintenance burden. UX inconsistency. Template and catalog drift between portals. Would require developers to remember which portal handles which provider.

## Consequences

**Positive:**
- Zero provider awareness for developers — true infrastructure abstraction.
- Template changes apply to both providers simultaneously (single YAML, one Git commit).
- Provider parity testing (ADR-135) directly validates this guarantee.
- Golden Path compliance easily measurable (ADR-140) — same templates, same metrics.

**Negative:**
- Provider-specific edge cases (e.g., Azure PG zone-redundant HA has no direct Hetzner equivalent) must be handled silently by Compositions. This means some Azure features may be silently downgraded on Hetzner or vice versa.
- Parity exceptions must be documented (ADR-135 parity-exceptions.yaml) so tenants understand behavioral differences.

## Compliance Mapping

SOC2 CC8.1 (consistent change management across environments). ISO A.5.37 (documented operating procedures — same procedures regardless of provider).
