# ADR-091: Entropy & Shadow IT Detection

**Status:** Accepted

**Scope:** Hyperscaler, Sovereign, Runtime, Universal

**Category:** Governance

**Related ADRs:** 076

## Context

Cloud environments accumulate unmanaged resources: manually created VMs, ad-hoc storage accounts, orphaned load balancers. These are attack vectors, cost sinks, and compliance gaps.

## Candidates

| Approach | cave-ctl doctor automated scan (chosen) | Manual periodic audit | Cloud-native (Azure Policy, AWS Config) | No detection |
|---|---|---|---|---|
| Coverage | ✅ Both providers, 6h-daily cadence | ❌ Quarterly at best | ⚠️ Cloud-specific only | ❌ None |
| Latency to detection | ✅ Hours | ❌ Months | ✅ Hours | ❌ Never |
| Entropy dashboard | ✅ Grafana visualization | ❌ Spreadsheet | ⚠️ Azure-only view | ❌ |
| Auto-terminate | ✅ Opt-in with 72h grace | ⚠️ Manual only | ⚠️ Per-provider | ❌ |
| Risk scoring | ✅ Per-tenant risk score | ❌ No scoring | ⚠️ | ❌ |

## Decision

**`cave-ctl doctor`** scans for unmanaged resources: (1) Query cloud provider API (Azure Resource Manager, sovereign-cloud API). (2) Compare against OpenTofu state + Crossplane managed resources. (3) Flag unmanaged resources as `Platform Violation`. (4) 72h grace period for creator to import to IaC or justify. (5) Auto-terminate (if opt-in enabled) after grace period.

**Frequency:** 6h prod, 12h staging, daily dev. Grafana entropy dashboard shows unmanaged resource count over time. Tenant risk score increases with shadow IT count.

## Rejected

- **No shadow IT detection:** Unmanaged resources accumulate silently. Attack vectors grow. Costs increase. Compliance gaps widen.
- **Manual periodic audit:** Too infrequent (quarterly at best). Shadow IT accumulates between audits. Automated scanning catches resources within hours.
- **Cloud-provider native tools (Azure Policy, AWS Config):** Cloud-specific. Doesn't cover Hetzner. CAVE needs unified detection across both providers.
- **Auto-terminate by default:** Too aggressive. Legitimate manual resources (one-off debugging, external vendor requirements) would be destroyed. Opt-in auto-terminate with grace period is safer.

## Consequences

**Positive:**
- Shadow IT detected before it becomes attack vector or cost sink.
- Entropy dashboard provides visibility to management.
- Grace period allows legitimate manual resources to be imported.

**Negative:**
- False positives for resources created by cloud provider automatically (Azure-managed resources, AKS internal resources). Exclusion list required.
- 72h grace period may be too short for some legitimate cases.
- Auto-terminate feature must be opt-in (too dangerous as default).

## Compliance Mapping

SOC2 CC6.1 (unauthorized resource detection). ISO A.8.9 (configuration management — drift detection). NIS2 Art.21 (asset management).
