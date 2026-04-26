# ADR-062: Azure Day-0 Infrastructure — OpenTofu

**Status:** Accepted

**Scope:** Azure

**Category:** Infrastructure

**Related ADRs:** 002, 067

## Context

CAVE's Azure profile requires Day-0 infrastructure provisioning: VNet (10.20.0.0/16), subnets, NSGs, AKS cluster (with Cilium BYOCNI, Karpenter), Key Vault, Private DNS Zones, Storage Account (for state), and Private Endpoints. Same rationale as Hetzner Day-0 (ADR-061): Crossplane cannot provision its own cluster.


## Candidates

| Criteria | OpenTofu (AzureRM) | Azure ARM/Bicep | Azure CLI scripts | Terraform |
|---|---|---|---|---|
| Declarative | ✅ HCL | ✅ JSON/Bicep | ❌ Imperative | ✅ HCL |
| Portable IaC skills | ✅ Same HCL as Hetzner | ❌ Azure-specific | ❌ | ✅ |
| State management | ✅ Azure Storage Account | ❌ Azure-managed | ❌ | ✅ |
| AzureRM provider | ✅ hashicorp/azurerm (very mature) | N/A | N/A | ✅ Same |
| License | MPL 2.0 | Azure terms | Azure terms | BSL 1.1 |
| AKS + Cilium BYOCNI | ✅ azurerm_kubernetes_cluster with network_plugin="none" | ✅ | ✅ | ✅ |


## Decision

**OpenTofu** with AzureRM provider. VNet 10.20.0.0/16 with subnets per ADR-054. AKS with Cilium BYOCNI (network_plugin="none") + Karpenter. Key Vault for ESO. Private DNS Zones for private endpoints. State in Azure Storage Account (encrypted, locked). Day 1+ transitions to Crossplane. Full topology in `CAVE_Azure_Network_Architecture.drawio`.


## Rejected Options

- **Azure ARM/Bicep:** Azure-specific IaC. HCL skills don't transfer from Hetzner. Team would need two IaC languages. ARM JSON is verbose and hard to maintain. Bicep is better but still Azure-only.
- **Azure CLI scripts:** Imperative. No state management. Not declarative. Not reviewable in PR process.
- **Terraform:** BSL 1.1 (same as ADR-061).


## Consequences

**Positive:**
- Same HCL language and OpenTofu toolchain as Hetzner Day-0 — one IaC skill set for both providers.
- AzureRM provider is very mature (hashicorp/azurerm maintained by Microsoft + HashiCorp).
- State in Azure Storage Account with locking — safe team collaboration.
- Full network topology documented in draw.io diagram.

**Negative:**
- AzureRM provider updates frequently — Renovate must track. Breaking changes possible in major versions.
- AKS BYOCNI configuration is more complex than default kubenet (required for Cilium).
- Karpenter on AKS requires specific AKS configuration (node provisioner identity, VMSS integration).

Compliance Mapping

SOC2 CC8.1 (infrastructure as code — version-controlled). ISO A.8.9 (configuration management). NIS2 Art.21 (secure infrastructure — auditable provisioning). Azure network topology: CAVE_Azure_Network_Architecture.drawio.

