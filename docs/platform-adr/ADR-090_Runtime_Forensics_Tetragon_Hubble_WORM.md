# ADR-090: Runtime Forensics (Tetragon + Hubble → WORM)

**Status:** Accepted

**Scope:** Universal

**Category:** Security

**Related ADRs:** 031

## Context

Incident investigation requires kernel-level (syscall) and network-level (flow) evidence that cannot be tampered with after the fact.

## Candidates

| Layer | Tool | Data Captured | Storage |
|---|---|---|---|
| Syscall | Tetragon (eBPF) | Process execution, file access, network syscalls | WORM forensic bucket |
| Network | Cilium Hubble | L3/L4/L7 flow logs, DNS queries | WORM forensic bucket |
| K8s API | K8s Audit Log | API server requests, RBAC decisions | WORM forensic bucket |
| Application | Loki (logs) | Application logs with tenant-id | WORM-backed Loki (ADR-106) |

## Decision

Tetragon (syscall-level) + Cilium Hubble (network flow) + K8s audit logs → dedicated WORM forensic bucket. Retention: 90d dev, 180d staging, 2y prod. APOL AI SRE queries forensic data during incident analysis. Reasoning traces reference forensic data by incident ID (ADR-128).

## Rejected

- **Application-level logging only:** Misses kernel and network layer. Insider threats, container escapes, lateral movement invisible.
- **Falco instead of Tetragon:** Both are eBPF-based runtime security. Tetragon is same ecosystem as Cilium (Isovalent/Cilium project) — shared eBPF infrastructure, unified Hubble integration. Falco would be a separate eBPF layer.
- **Non-WORM storage:** Forensic evidence can be tampered with or deleted. Unacceptable for legal/compliance proceedings.

## Consequences

**Positive:**
- Multi-layer forensic evidence: syscall + network + API + application.
- WORM storage ensures evidence immutability for legal proceedings.
- Same eBPF ecosystem (Cilium + Tetragon + Hubble) — unified agent, shared kernel hooks.

**Negative:**
- Tetragon + Hubble generate significant data volume. WORM retention costs scale with cluster activity.
- eBPF kernel compatibility requirements (compatibility matrix triple — ADR-133).
- Forensic data for restricted tenants must follow metadata residency rules (Hubble flow disabled for restricted).

## Compliance Mapping

SOC2 CC7.2 (system monitoring evidence). ISO A.8.15 (logging). ISO A.8.16 (monitoring activities). NIS2 Art.21 (incident detection and forensics). GDPR Art.30 (processing records).
