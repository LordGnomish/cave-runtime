# ADR-098: Talos Linux Immutable Infrastructure

**Status:** Accepted

**Scope:** Hetzner

**Category:** Infrastructure

**Related ADRs:** 003

## Context

Hetzner profile needs a K8s-optimized, security-hardened, immutable OS. Configuration drift must be impossible. SSH access must not exist.


## Candidates

*Full 6-candidate evaluation in ADR-003 (Talos vs Ubuntu, Flatcar, Bottlerocket, RKE2, k3s).*

| Key Differentiator | Talos | Others |
|---|---|---|
| SSH access | ❌ None (API-only) | ✅ SSH available |
| Shell access | ❌ None | ✅ Shell available |
| Package manager | ❌ None | ✅ apt/yum |
| Configuration method | Machine Config API | SSH + shell commands |
| Drift possibility | ❌ Impossible (read-only FS) | ✅ Possible |


## Decision

Talos Linux for ALL Hetzner profiles (dev, staging, prod). Immutable, API-only OS. Nodes never patched — destroyed and recreated from versioned image. Same OS, same management model across all environments. Debugging via `talosctl` + ephemeral debug containers (30min TTL).


## Rejected Options

See ADR-003 for full comparison. Key: Ubuntu/Flatcar/Bottlerocket all allow SSH — configuration drift possible. Talos eliminates the entire category of drift-via-SSH risks.


## Consequences

**Positive:**
- Zero configuration drift. Node state is exactly what machine config declares.
- No SSH attack surface. No shell for attackers to exploit.
- Same Talos version across all environments — zero dev/prod OS parity gap.
- Upgrades are atomic: destroy old node, create new from updated image.

**Negative:**
- Engineers cannot SSH into nodes. All debugging via talosctl + debug containers.
- Learning curve for teams familiar with SSH-based operations.
- Kernel panic or Cilium-caused network lockout requires Hetzner console access (no SSH fallback).
- Limited to Hetzner-supported hardware/cloud — not applicable to Azure (AKS manages OS).

Compliance Mapping

SOC2 CC6.1 (access control — no SSH). ISO A.8.8 (management of technical vulnerabilities — immutable OS). NIS2 Art.21 (secure systems — hardened infrastructure).

