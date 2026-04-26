# ADR-054: Network Segmentation — VNet/Subnet Architecture

**Status:** Accepted

**Scope:** Universal

**Category:** Security

**Related ADRs:** 004, 014, 084

## Context

CAVE runs on two providers with fundamentally different networking models. IP address spaces must be non-overlapping (for potential future VPN peering), subnets must isolate different workload types, and the network topology must support private endpoints for all managed services on Azure.


## Candidates

| Approach | Non-overlapping /16 per provider | Shared address space | NAT-based separation |
|---|---|---|---|
| VPN peering possible | ✅ No conflicts | ❌ Conflicts | ⚠️ NAT complexity |
| Simplicity | ✅ Clear boundary | ❌ Confusion | ❌ Complex |
| Scalability | ✅ 65K IPs per provider | ❌ Split allocation | ⚠️ |
| Cross-provider routing | ✅ Straightforward | ❌ | ⚠️ |


## Decision

**Hetzner VNet:** 10.10.0.0/16. **Azure VNet:** 10.20.0.0/16. Non-overlapping. Azure subnets: AKS system pool (10.20.0.0/22), AKS user pool (10.20.4.0/22), AKS GPU pool (10.20.8.0/24), private endpoints (10.20.16.0/22), CyberArk VMs (10.20.20.0/24). Pod CIDR and Service CIDR within VNet range. CoreDNS for internal DNS. Cloudflare for external DNS (ADR-024). Full topology documented in `CAVE_Azure_Network_Architecture.drawio`.


## Rejected Options

- **Shared /16 split across providers:** IP conflicts if VPN peering ever needed. Confusing allocation management. Risk of accidental routing overlap.
- **Smaller ranges (/24 per provider):** Insufficient for Kubernetes pod CIDR allocation. AKS alone needs hundreds of IPs for pods.
- **NAT-based separation:** Adds NAT gateway complexity and cost. Makes cross-provider troubleshooting harder (NATed IPs in logs).


## Consequences

**Positive:**
- Non-overlapping ranges enable future VPN peering without NAT.
- Clear subnet boundaries simplify NSG/firewall rules and CiliumNetworkPolicy.
- /16 per provider provides ample growth headroom (65K IPs).
- Predictable addressing for Cilium L3/L4 policies.
- Full topology documented in draw.io for architecture review.

**Negative:**
- /16 ranges are oversized for initial deployment (cost-neutral — IP addresses don't cost per-range).
- Azure subnet pre-sizing required for AKS (node count × max-pods-per-node).
- Hetzner private networking has limitations compared to Azure VNet (no native subnet RBAC — compensated by Cilium NetworkPolicy).

Compliance Mapping

SOC2 CC6.1 (network segmentation — non-overlapping ranges prevent cross-provider leakage). ISO A.8.22 (segregation in networks — subnet isolation per workload type). NIS2 Art.21 (network architecture — documented topology). GDPR Art.32 (security of processing — network isolation). Architecture diagram: CAVE_Azure_Network_Architecture.drawio.

