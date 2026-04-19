# ADR-016: Container Runtime Security — Pod Security + Tetragon

**Status:** Accepted

**Scope:** Hetzner, Universal

**Category:** Security

**Related ADRs:** 090, 098

## Context

CAVE needs defense-in-depth at the container runtime level: preventing container escapes, restricting syscalls, monitoring runtime behavior, and enforcing Pod Security Standards.

## Candidates

| Criteria | PSA Restricted + Tetragon | PSA + Falco | PSA + Seccomp profiles | gVisor/Kata |
|---|---|---|---|---|
| Pod Security Standards | ✅ PSA Restricted (K8s native) | ✅ PSA | ✅ PSA | N/A (different approach) |
| Syscall monitoring | ✅ Tetragon eBPF (kernel-level) | ✅ Falco eBPF | ⚠️ Static profiles only | ✅ Sandboxed kernel |
| File access monitoring | ✅ Tetragon tracing policies | ✅ Falco rules | ❌ | ✅ |
| Network monitoring | ✅ Tetragon + Cilium Hubble (unified) | ⚠️ Falco (separate from CNI) | ❌ | ❌ |
| Performance overhead | ✅ Minimal (shared eBPF with Cilium) | ⚠️ Separate eBPF programs | ✅ Minimal | ❌ Significant (VM overhead) |
| Forensic export | ✅ → WORM bucket (ADR-090) | ✅ → Loki/WORM | ❌ | ❌ |

## Decision

**Pod Security Admission (PSA) Restricted** for all namespaces + **Tetragon** for runtime monitoring and enforcement. PSA prevents privileged containers, host networking, and dangerous capabilities at admission. Tetragon monitors syscalls, file access, and network at kernel level via eBPF, exporting to WORM forensic bucket (ADR-090).

## Rejected

- **Falco instead of Tetragon:** Both are eBPF-based runtime security. Tetragon is the Cilium ecosystem project (Isovalent) — shares eBPF infrastructure with Cilium and Hubble. Running Falco alongside Cilium means two separate eBPF program sets on the same kernel — potential conflicts, higher overhead.
- **Seccomp profiles only:** Static syscall filtering. No monitoring, no forensic export, no file access tracking. Defense without detection.
- **gVisor/Kata Containers:** VM-level sandboxing provides strongest isolation but significant performance overhead (~20-30% CPU). Incompatible with Talos immutable OS (no gVisor/Kata support). Overkill for most workloads.

## Consequences

**Positive:**
- Defense-in-depth: admission (PSA) + runtime monitoring (Tetragon) + network (Cilium).
- Unified eBPF ecosystem (Cilium + Tetragon + Hubble) — single kernel interaction layer.
- Forensic export to WORM enables post-incident investigation with tamper-proof evidence.
- PSA Restricted is K8s-native — no additional admission webhook needed for basic pod security.

**Negative:**
- Tetragon tracing policies require tuning per workload type (default policies may generate noise).
- eBPF kernel compatibility (compatibility matrix triple — ADR-133).
- PSA Restricted may block legitimate workloads that need specific capabilities (mitigated: per-namespace exemptions with waiver ADR-140).
- WORM forensic export volume scales with cluster activity — storage cost consideration.

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Tetragon tracing policy false positives (alert fatigue) | Medium | Medium | Start with observe-only mode. Tune tracing policies per workload type. Suppress known-good syscall patterns. Weekly review of alert volume. |
| eBPF kernel panic (buggy tracing policy) | Very Low | Critical | Pin Tetragon version to tested release. Staging validates all tracing policy changes. Tetragon crash recovery is automatic (DaemonSet restart). |
| PSA Restricted blocks tenant workload | Medium | Medium | Per-namespace exemption via Waiver Framework (ADR-140). Document common exemptions (init containers needing NET_ADMIN, etc.). |
| Tetragon community smaller than Falco | Low | Low | Tetragon is backed by Cilium/Isovalent/Cisco ecosystem. If Tetragon stalls, Falco is compatible fallback (different eBPF programs but same forensic output format). |
| WORM forensic storage cost growth | Medium | Low | Retention policy: 90d hot, 1y cold (S3/ADLS). Sampling for high-volume namespaces. Alert on storage growth rate. |

## Compliance Mapping

SOC2 CC6.1 (runtime access controls). SOC2 CC7.2 (runtime monitoring). ISO A.8.8 (technical vulnerability management — runtime protection). ISO A.8.16 (monitoring activities). NIS2 Art.21 (security monitoring). GDPR Art.32 (security of processing).

**Absorbed Decisions:** The following tool-level decisions are absorbed into this ADR for traceability

**ADR-031**

Tetragon as eBPF Runtime Security

**Decision:** Tetragon (eBPF) for runtime security and forensics. Rejection: Falco (userspace, less performant, limited kernel visibility).
