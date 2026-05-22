# ADR-001: Sovereign Bare-Metal Hosting Reference Profile

## Status
Accepted

## Context
Cave Runtime is designed to run on operator-owned infrastructure with no
hard dependency on a specific cloud provider. The Charter (rule 3) requires
sovereignty: the platform must be fully operable without external SaaS in
the critical path.

## Decision
The reference deployment profile is **bare-metal (or bare-metal-equivalent)
servers running mainline Linux 7.1+**. Cave Runtime ships as a single Rust
binary plus systemd units; no provider-specific control-plane is required.

Operators are free to choose any provider (self-hosted hardware, colo,
public cloud bare-metal instances, sovereign cloud) — Cave makes no
assumption beyond "Linux 7.1+ kernel with cgroup v2 and eBPF".

Internal historical provider-specific decisions live under
`docs/adr/internal/` for archival reference. They describe prior
proof-of-concept deployments and are not normative for the OSS release.

## Consequences
- One reference profile (Linux bare-metal) is exercised by CI and the
  `cave-runtime cluster init` subcommand.
- Cloud-provider-specific integrations (object storage, managed databases,
  identity SaaS) are optional plugins behind feature flags, not core paths.
- No vendor lock-in: operators can migrate the entire control plane by
  copying the etcd data directory and replaying it on a different host.
