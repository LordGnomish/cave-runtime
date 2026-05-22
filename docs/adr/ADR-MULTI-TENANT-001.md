# ADR-MULTI-TENANT-001: Cave Runtime is multi-tenant by construction

**Status:** Accepted
**Date:** 2026-04-23
**Author:** Burak Tartan (raised the runtime-wide framing), Sonnet (scribe)
**Scope:** Universal (charter-binding — foundation for PORTAL-PERSONAS-001, PORTAL-AUTH-001, and every module ADR)

## Context

Cave Runtime is a platform (ADR-CHARTER-001). A platform that cannot safely host more than one customer is a single-tenant deployment pretending to be a platform. Every platform-grade decision — resource isolation, identity, observability, billing, security — depends on tenants being a **runtime-wide invariant**, not a portal-level UI convenience.

Prior ADRs (PORTAL-PERSONAS-001, PORTAL-AUTH-001) split the portal into admin and tenant surfaces. That split is a **consequence** of runtime multi-tenancy, not its cause. The cause lives in the modules: cave-apiserver, cave-scheduler, cave-gateway, cave-etcd, cave-vault, cave-registry, cave-net, cave-mesh, cave-scan, cave-metrics, cave-trace, cave-auth, cave-kernel. Every one of those must carry the tenant concept as a first-class attribute of every operation they perform.

Burak (2026-04-23): *"tum runtime multi tenant degil mi?"* — yes. This ADR makes it explicit and binding.

## Decision

**Every Cave Runtime module treats `tenant_id` as a required, non-null, first-class attribute of every resource, operation, log line, metric sample, trace span, audit event, and identity.** There is no "shared scope" fallback. There is no anonymous resource. Infrastructure-owned resources carry the sentinel tenant `system`, which is privileged but still explicit.

### Core invariants (MUST hold for every module)

1. **Resource ownership:** every persisted resource (pod, secret, image, route, API, alert rule, config, credential, certificate, policy) has exactly one owning `tenant_id`. No co-ownership. Cross-tenant sharing is achieved via explicit export/import, never via shared resources.

2. **Namespaced addressing:** every resource identifier is of the form `/t/{tenant_id}/{resource-kind}/{name}` or equivalent structured form (proto field, etcd key prefix, S3 object path). `tenant_id` is never implicit.

3. **Default-deny between tenants:** a workload in tenant A cannot observe, reach, or affect a workload in tenant B unless an explicit cross-tenant policy grants it. This applies to network (cave-net policy), identity (cave-auth SPIFFE trust domain), data (cave-etcd read policy), API calls (cave-gateway route ACL), and observability (cave-metrics / cave-trace label filter).

4. **Resource quotas & fair scheduling:** cave-scheduler enforces per-tenant quotas and anti-noisy-neighbor bin-packing. A runaway workload in tenant A cannot starve tenant B of CPU, memory, network bandwidth, or API rate. Per-tenant SLO budgets are tracked independently.

5. **Tenant-labelled telemetry:** every metric, log line, trace span, and audit record MUST carry `tenant_id` as a label/field. Admins see cross-tenant roll-ups; tenants see their own data only.

6. **Cryptographic isolation:** cave-vault, cave-auth, cave-registry sign/seal/encrypt with per-tenant key material where the scope demands it. No shared-secret-across-tenants.

7. **SPIFFE identity scoping:** every workload has a SPIFFE SVID whose trust domain path encodes `tenant_id` (`spiffe://cave.runtime/t/<tenant>/ns/<ns>/sa/<account>`). Identity is never tenant-ambiguous.

8. **Blast-radius isolation:** a failure (OOM, crash loop, DDoS, bad deploy) in tenant A must not cascade to tenant B. Per-tenant circuit breakers, connection pool limits, and etcd compaction paths.

9. **Audit + billing:** every resource-consuming operation emits a metered event with `tenant_id`. Billing, cost allocation, and compliance audit flow from these events — not from out-of-band polling.

10. **Tenant lifecycle is a runtime operation:** create / suspend / purge / migrate are first-class cave-apiserver operations with ADR-approved safety rails. A tenant purge is cascading and auditable; a suspend is reversible and quota-preserving.

### System tenant

A single sentinel `tenant_id = "system"` owns:
- Infrastructure workloads (cave-scheduler itself, cave-apiserver itself, cave-etcd itself, ...)
- Admin-only resources (cluster-wide policies, federation config, IdP config)
- Cross-tenant observability roll-ups (metric aggregations, SLO composite dashboards)

`system` is still explicit, still audited, still in every resource path. It is never the default; operations that omit `tenant_id` fail.

### Per-module enforcement (non-exhaustive)

| Module | Tenant-enforcement responsibility |
|---|---|
| **cave-apiserver** | Namespace model with tenant claim; admission controller denies cross-tenant references; every object has owning tenant. |
| **cave-scheduler** | Per-tenant quota, fair scheduling, anti-noisy-neighbor bin-packing, preemption policy tenant-aware. |
| **cave-kubelet** | Pod cgroup + seccomp + AppArmor derived from tenant policy; no cross-tenant pod co-location unless explicit. |
| **cave-cri** | Image pull authorized per tenant; container label includes tenant; logs tagged. |
| **cave-net (CNI)** | Default-deny NetworkPolicy between tenant namespaces; tenant-scoped IPAM pools; DNS split-horizon. |
| **cave-mesh** | mTLS peer identity validates cross-tenant flow is explicitly allowed; per-tenant SLO + circuit breakers. |
| **cave-gateway** | Route/service/plugin owned by tenant; admin cross-tenant only with impersonation audit. |
| **cave-etcd** | Key prefix `/t/<tenant>/...`; auth role grants scoped to prefix; compaction per-tenant-friendly. |
| **cave-vault** | Namespace per tenant; transit keys per tenant; no cross-tenant seal unseal. |
| **cave-registry** | Project per tenant; image pull secrets scoped; vulnerability scans reported back to tenant only. |
| **cave-scan / cave-vulns** | Findings owned by tenant; admin sees cross-tenant security posture roll-up. |
| **cave-metrics / cave-trace** | Mandatory `tenant_id` label; tenant-level query RBAC; admin can disable label filter (audited). |
| **cave-auth** | Claims include `tenant_id`; RBAC roles can be tenant-scoped or cluster-scoped; SPIFFE trust domain segments. |
| **cave-kernel** | Shared primitives (Raft groups, EventBus topics, Labels, WAL) expose tenant-aware APIs; no global topic that leaks cross-tenant state. |
| **cave-agent** (ADR-SELF-IMPROVE-001) | Proposes changes within tenant scope; cluster-wide changes require `system` role + explicit approval. |
| **cave-portal-api** | RBAC middleware reads `tenant_id` from auth claim; filters every response; admin gets cross-tenant, tenants get own. |

### Isolation tiers

Cave supports two isolation modes at cluster provisioning time:
1. **Soft multi-tenancy** (default): shared cluster, per-tenant namespace + RBAC + NetworkPolicy + quota. Lower cost, higher density, usual trust assumption.
2. **Hard multi-tenancy** (opt-in, enterprise): dedicated nodes per tenant (taints + tolerations + node selectors), dedicated etcd shards, dedicated cave-gateway instances, kernel-level isolation (gVisor / Kata runtime class). Used for regulated workloads.

The runtime treats both identically from the tenant's perspective — only the operator provisions differently.

## Rationale

**Why runtime-wide, not just portal?**
Because a portal-level split without runtime backing is theatre. If cave-scheduler doesn't enforce per-tenant quotas, a runaway tenant still takes down the cluster regardless of how pretty `/t/x/dashboard` looks. Real multi-tenancy lives in the scheduler, the network, the etcd, the identity service.

**Why default-deny between tenants?**
Because the alternative — default-allow with explicit deny — has lost every security audit ever conducted. ADR-GOLDEN-003 (no-backcompat) applies: we don't retrofit security onto a shared surface; we build isolation as the starting point.

**Why `tenant_id = "system"` as a sentinel, not `null`?**
`null` is the source of 40 years of security bugs. Sentinel values are explicit, auditable, and grep-able.

**Why enforce tenant_id on every metric/trace/audit?**
Because post-incident correlation across observability signals is impossible if `tenant_id` is optional in any of them. One missing label ruins the join.

**Why soft + hard modes?**
Hard multi-tenancy is expensive and not every tenant needs it. Offering both — with the runtime abstraction identical — lets operators provision per tenant profile without rewriting applications.

## Consequences

**Immediate (this sprint — doc + small code):**
- Every in-flight ADR references this as foundation.
- Every new crate MUST declare in its parity.manifest.toml the line `tenant_aware = true` and demonstrate enforcement in tests.
- cave-kernel Labels struct gains a mandatory `tenant_id` field.
- cave-apiserver admission middleware is stubbed to require tenant on every object.

**Pre-OSS (28 days):**
- Audit: grep every crate for endpoints that accept requests without a tenant claim. Every one gets either a tenant-require wrapper or a `system`-only explicit annotation.
- cave-etcd key prefix enforcement (runtime rejects `/foo` without `/t/<tenant>/` prefix).
- cave-metrics scrape rejects series without `tenant_id` label.
- Portal admin/tenant split (ADR-PORTAL-PERSONAS-001) implemented.
- Minimum viable tenant onboarding: create tenant → issue credentials → deploy sample app → see metrics scoped.

**Post-OSS:**
- Hard multi-tenancy runtime class (Kata / gVisor integration).
- Federated cross-cluster tenant identity.
- Per-tenant CRDT-backed offline mode (regulated air-gapped tenants).
- Tenant migration (move a tenant from cluster A to cluster B with zero downtime).

## Alternatives considered

1. **Namespace-only isolation (Kubernetes default).** Rejected — namespaces alone don't enforce quota fairness, network blast-radius, observability labels, or cryptographic key scoping. Cave needs all of those.
2. **Single-tenant default, multi-tenancy as a config flag.** Rejected — every non-multi-tenant operation then becomes a "latent bug waiting for OSS adoption". Better to make it the only mode from day one.
3. **Hard multi-tenancy only.** Rejected — too expensive for most users; blocks adoption.
4. **Tenant as a string tag, not a typed field.** Rejected — strings get misspelled, case-mangled, forgotten. Typed field forces the compiler to check every call site.

## References

- ADR-CHARTER-001 — sovereign Cloud OS (platform requires multi-tenant)
- ADR-GOLDEN-001 — upstream line-by-line parity (Kubernetes namespaces are the starting surface; we strengthen)
- ADR-GOLDEN-002 — cave-kernel shared primitives (all tenant-aware)
- ADR-GOLDEN-003 — no-backcompat + PQC-ready (don't retrofit isolation later)
- ADR-GOLDEN-004 — 4-track completion (Portal UX track split into admin + tenant per PORTAL-PERSONAS-001)
- ADR-PORTAL-PERSONAS-001 — portal admin/tenant split as a consequence of this ADR
- ADR-PORTAL-AUTH-001 — authentication gate; tenant_id claim sourced from auth
- ADR-SELF-IMPROVE-001 — cave-agent operates within tenant scope
- 2026-04-23 user framing: *"tum runtime multi tenant degil mi?"* — yes, ratified here.
