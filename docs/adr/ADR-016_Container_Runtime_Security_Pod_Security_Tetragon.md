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

## Rejected Options

### Falco — Rejected

**Primary:** Dual eBPF overhead. Falco and Cilium both load eBPF programs into the kernel. Running both means two independent eBPF program sets competing for kernel resources — potential verifier conflicts, doubled probe overhead, and two separate event pipelines to maintain. Tetragon is the Cilium ecosystem project (Isovalent/Cisco) — shares eBPF infrastructure with Cilium agent and Hubble, resulting in a single unified eBPF layer.

**Secondary:** Different ecosystems. Falco (Sysdig/CNCF Graduated) has a larger community but its output format and rule language are incompatible with Cilium's policy model. Tetragon's TracingPolicy CRD integrates natively with CiliumNetworkPolicy — one policy language for both network and runtime enforcement. Falco would require a separate Falco-to-SIEM pipeline alongside Hubble's.

### Seccomp Profiles Only — Rejected

**Primary:** Static defense without detection. Seccomp profiles whitelist/blacklist syscalls at container start — they cannot detect anomalous behavior patterns, file access violations, or process lineage. A compromised container making allowed syscalls in suspicious sequence (e.g.,  →  → ) would pass seccomp but trigger Tetragon's behavioral policy.

**Secondary:** No forensic export. Seccomp blocks or allows — it doesn't log. Tetragon exports every matching event to WORM forensic bucket (ADR-090) for post-incident investigation. Compliance requires detection evidence, not just prevention (SOC2 CC7.2).

### gVisor / Kata Containers — Rejected

**Primary:** Performance overhead. gVisor intercepts every syscall through a userspace kernel (~20-30% CPU overhead). Kata runs each pod in a lightweight VM (~100-200MB overhead per pod). On Hetzner's cost-optimized profiles, this overhead is prohibitive for 500+ pod clusters.

**Secondary:** Incompatible with Talos Linux. Talos's immutable, API-only OS does not support gVisor's  runtime or Kata's QEMU/Cloud Hypervisor installation. Supporting these would require a different OS — contradicting ADR-003.

## Runtime Security Layers (Defense-in-Depth)

| Layer | Tool | When | What it does |
|---|---|---|---|
| Admission | PSA Restricted | Pod creation | Blocks privileged containers, host namespaces, dangerous capabilities |
| Admission (extended) | OPA Gatekeeper (ADR-030) | Pod creation | Custom policies: image allowlist, label requirements, resource limits |
| Build-time | Trivy (ADR-018) | CI pipeline | Scans container image for CVEs before deployment |
| Runtime monitoring | Tetragon | Pod execution | Monitors syscalls, file access, network at kernel level via eBPF |
| Runtime enforcement | Tetragon | Pod execution | Kills process on policy violation (e.g., shell spawn in production) |
| Network | Cilium (ADR-004) | Pod networking | Default-deny L3/L4, FQDN-based egress control |
| Forensics | Tetragon → WORM | Post-incident | Tamper-proof event log for investigation (ADR-090) |

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
