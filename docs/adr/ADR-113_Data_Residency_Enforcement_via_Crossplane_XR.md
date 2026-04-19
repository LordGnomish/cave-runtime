# ADR-113: Data Residency Enforcement via Crossplane XR

**Status:** Accepted

**Scope:** Azure, Hetzner, Runtime, Universal

**Category:** Compliance

**Related ADRs:** 102

## Context

GDPR Art.44-49 requires that EU personal data stays in EU. Platform must enforce residency at provisioning time, not rely on developer discipline.

## Candidates

| Approach | Crossplane dataResidency field + OPA | Application-level | Cloud-provider policy |
|---|---|---|---|
| Enforcement | ✅ At admission (OPA validates region) | ❌ Developer trust | ⚠️ Provider-specific |
| Portability | ✅ Same field, different Compositions per provider | ❌ | ❌ Provider lock-in |
| Audit evidence | ✅ OPA admission log + Ledger | ❌ | ⚠️ |

## Decision

Crossplane `dataResidency` field on all data XRs. OPA validates region constraints at admission. Compositions map residency to provider-specific region (eu → Germany West Central on Azure, Falkenstein on Hetzner). Metadata residency follows same rules (observability spill control).

## Rejected

- **Application-level enforcement:** Developers choose region. Misconfiguration → GDPR violation. Not verifiable at platform level.
- **Cloud-provider policy (Azure Policy, etc.):** Provider-specific. Not portable. Doesn't cover Hetzner.

## Consequences

**Positive:**
- Residency enforced at provisioning time, not post-deployment audit.
- Same dataResidency field across both providers.
- OPA admission log provides compliance evidence.
- Metadata residency (observability) follows same rules.

**Negative:**
- Provider-specific region mappings must be maintained in Compositions.
- Some Azure regions may not have all service SKUs — Composition must handle SKU availability.
- Cross-region Thanos queries are ephemeral but must not persist data cross-region.

## Compliance Mapping

GDPR Art.44-49 (data transfers). GDPR Art.25 (data protection by design). ISO A.5.14 (information transfer). NIS2 Art.21 (data protection).
