# ADR-PORTAL-PERSONAS-001: two-persona portal — admin vs tenant

**Status:** Accepted
**Date:** 2026-04-23
**Author:** Burak Tartan (raised the split), Sonnet (scribe)
**Scope:** Universal (charter-binding)
**Related:** ADR-CHARTER-001, ADR-GOLDEN-004 (4-track completion incl. Portal UX), ADR-PORTAL-AUTH-001 (companion)

## Context

Today's cave-portal is a single undifferentiated surface that mixes platform-operator concerns (upstream parity, ADR browser, runtime progress, module completion, LLM daemon state) with what will eventually be end-user concerns (service catalog, API gateway policies, scan results, incident tickets, logs, secrets, pipelines). Anyone landing on `/` sees everything. There is no login, no tenant boundary, no role separation.

Cave Runtime is **a platform** (ADR-CHARTER-001). Platforms have at least two durable personas, with fundamentally different mental models, SLAs, and blast radii. Treating both as the same user leaks operator-level data to tenants, clutters the operator workflow with tenant-specific widgets, and makes RBAC retrofit increasingly painful the later it lands.

Burak (2026-04-23): *"cave bir platform ve adminleri var … ama platform tenantları kullanıcılar onlar … API policy configure edecek ya da defect dojo dan scan sonuçlarını manage edecek ama bunlar da adminleri ilgilendirmez gibi geliyor bana."* Correct.

## Decision

Cave Portal is split into two distinct personas, each with its own URL space, navigation, role set, and release criteria. Both share the same Rust/axum backend (`cave-portal-api`) and the same design system, but the rendered surface and the routes available are persona-scoped.

### Persona 1 — Platform Admin (`/admin/...`)

**Who:** Cave operators. Burak today; post-OSS, cluster operators at each Cave deployment.

**What they need:**
- Upstream parity tracker (`/admin/parity` — single canonical, replaces today's `/progress` + `/upstream`)
- ADR browser (`/admin/adrs`)
- Runtime state: modules, nodes, capacity, topology (`/admin/runtime`)
- Observability — SLO dashboards, alert rules, incident oncall at the platform level (`/admin/observability`)
- Control-plane tools: cave-scheduler state, cave-etcd health, cave-apiserver logs (`/admin/control-plane`)
- Build-time agent telemetry: `cave-local-llm` queue, drafts/ready, drafts/failed (`/admin/agents/build`)
- Runtime agent telemetry: `cave-agent` proposals, canary state, applied changes (`/admin/agents/runtime` — per ADR-SELF-IMPROVE-001)
- Tenant administration: onboard/offboard tenants, set quotas & resource limits, audit tenant activity (`/admin/tenants`)
- Billing & cost rollup across tenants (`/admin/billing`)
- Security posture — cluster-wide vulnerabilities, policy violations (`/admin/security`)

**RBAC default roles:** `system:cluster-admin`, `system:cluster-viewer`, `system:oncall`.

**Release criteria (pre-OSS):** the `/admin/parity` + `/admin/adrs` + `/admin/runtime` triad is fully functional and canonically truthful (no placeholder data).

### Persona 2 — Platform Tenant (`/t/{tenant}/...`)

**Who:** End users of Cave. Application teams, product engineers, security teams working on tenant-scoped workloads.

**What they need:**
- Service catalog — their apps, owners, techdocs (Backstage catalog-scope reimpl; `/t/{tenant}/catalog`)
- Workload management — Deployments/Pods/Services scoped to tenant (`/t/{tenant}/workloads`)
- API gateway policies — for APIs owned by the tenant: rate limit, key-auth, JWT, CORS (`/t/{tenant}/gateway`) — uses cave-gateway admin API filtered by tenant
- Security findings — scan results (Trivy / Semgrep / Gitleaks / DefectDojo reimpl) for tenant images only (`/t/{tenant}/security`)
- Incident tickets — Jira-equivalent, scoped to tenant (`/t/{tenant}/incidents`) — implements the ServiceNow↔Jira bridge work
- Secrets — OpenBao reimpl UI scoped to tenant namespaces (`/t/{tenant}/secrets`)
- CI/CD pipelines — ArgoCD-equivalent apps for tenant (`/t/{tenant}/pipelines`)
- Logs & traces — tenant-scoped telemetry (`/t/{tenant}/observability`)
- User & team management — within the tenant (`/t/{tenant}/teams`)
- Cost / usage / quota — tenant-scoped metering (`/t/{tenant}/cost`)

**RBAC default roles:** `tenant:admin`, `tenant:developer`, `tenant:viewer`, `tenant:security`.

**Release criteria (pre-OSS):** skeleton shell with auth gate (ADR-PORTAL-AUTH-001) + catalog + gateway policy editor. Full tenant portal is post-OSS v1 roadmap.

### Cross-cutting rules

1. **No leakage.** An admin route must never render tenant data except through an explicit "impersonate" action that logs an audit event. A tenant route must never render cluster-level data (infrastructure paths, ADR internals, module completion of modules they don't consume).
2. **Navigation is persona-scoped.** Admins do not see tenant sidebars; tenants do not see admin sidebars. The login flow routes to `/admin/*` for admin identities, `/t/{default-tenant}/*` for tenant identities. Cross-navigation requires re-auth for admin, never for tenants.
3. **URL is semantic.** `/admin/*` is always admin; `/t/{tenant}/*` is always tenant. No shared `/*` page except the public landing + login.
4. **Impersonation is explicit.** Admins viewing a tenant portal do so via `/admin/tenants/{tenant}/impersonate` which redirects to `/t/{tenant}/*` with an `impersonation=true` header and a visible banner; every action is double-logged under both the admin's and tenant's audit streams.
5. **Backend is one API surface.** `cave-portal-api` serves both — endpoints have RBAC middleware that reads the authenticated identity, enforces role policy, and filters response payloads. A tenant viewer hitting `/api/portal/workloads` gets tenant-scoped results; an admin gets cluster-wide by default.

## Rationale

**Why not one portal with role-hidden sidebars?**
Role-hidden sidebars still ship the same HTML and same backend endpoints; role-based filtering becomes an afterthought usually full of bugs. URL-level split makes leakage *impossible by construction* — a tenant hitting `/admin/parity` gets a 403 before any data is fetched. This matches the no-backcompat spirit of ADR-GOLDEN-003: we don't retrofit security onto a shared surface; we build the split on day one.

**Why tenant routes are `/t/{tenant}/...`?**
- Explicit tenant in URL makes SSR, bookmarks, shared links, and audit trails unambiguous.
- Multi-tenant users (e.g. consultants) switch via simple URL change, no global state gymnastics.
- Matches the way Kubernetes namespaces and GitLab groups behave; migration from those systems is muscle-memory.

**Why admin portal is not `/t/system/...`?**
Because admin is not "a tenant whose name happens to be system". Admin is structurally different: cluster-scoped, no tenant isolation, different audit profile, different release cadence.

## Consequences

**Immediate (this sprint):**
- `cave-portal-api` router reshape: mount `/admin/*` and `/t/{tenant}/*` as distinct nested Routers. Each has its own RBAC middleware.
- Existing routes (e.g. `/api/portal/runtime/progress`) move under `/admin/api/portal/runtime/progress`. Old paths get a temporary 308 redirect for one release cycle, then removed.
- Front-end (`portal_index.html`, future React migration) rewrites top-level router to detect admin vs tenant scope and render distinct shells.

**Pre-OSS (28 days):**
- Admin shell canonicalised; `/admin/parity` is the one honest parity page.
- Tenant shell skeleton exists with catalog + gateway policy editor; other tenant surfaces are stub pages labelled "roadmap v1.1".
- Auth gate in place (ADR-PORTAL-AUTH-001); no unauthenticated access.

**Post-OSS:**
- Tenant portal expands module by module (incidents, security findings, pipelines, logs, secrets, cost).
- Impersonation flow audited against regulator-grade requirements.
- Per-tenant theming / brand override (enterprise sales requirement).

## Alternatives considered

1. **Single portal, role-hidden sidebars.** Rejected — retrofit security anti-pattern; leaks waiting to happen.
2. **Two separate deployments (admin.cave.io + portal.cave.io).** Rejected for OSS v1 — doubles operational burden, complicates SSO cookie scope, and admins who also consume tenants need two tabs. Revisit for enterprise hosted offering.
3. **Tenant is a first-class "org" with admin as an org of orgs.** Rejected — recursive, makes RBAC model harder, no real gain.

## References

- ADR-CHARTER-001 — sovereign Cloud OS (platform, therefore multi-persona)
- ADR-GOLDEN-004 — 4-track completion includes Portal UX; now split into admin+tenant tracks
- ADR-PORTAL-AUTH-001 — companion, mandates authentication before any portal page
- ADR-SELF-IMPROVE-001 — cave-agent lives under `/admin/agents/runtime`
- 2026-04-23 user observation: *"cave bir platform ve adminleri var … tenantları gelecek jira gibi kullanacak … portale elini kolunu sallayarak girememen lazım"*
