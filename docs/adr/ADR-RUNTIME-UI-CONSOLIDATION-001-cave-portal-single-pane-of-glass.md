# ADR-RUNTIME-UI-CONSOLIDATION-001 — Cave Portal Single Pane of Glass

**Status:** Accepted

**Date:** 2026-04-26

**Scope:** Cave Runtime (UX surface for tenant + operator + platform admin personas)

**Category:** Runtime / UI / UX

**Related ADRs:**
ADR-031 (Cave WebApplication Composition Pattern — portal-driven tenant lifecycle),
ADR-RUNTIME-CLI-CONSOLIDATION-001 (sister principle for the CLI surface),
ADR-RUNTIME-UPSTREAM-MIRROR-001 (charter — upstream-mirror principle; this ADR is its UX-side corollary),
ADR-095 (Reflex Engine — portal native action queue),
ADR-011 (Backstage as Developer Portal — Platform-side; Cave Runtime reimplements),
ADR-014 (Zero-Trust Network Architecture),
ADR-029 (LGTM Stack — Grafana surface absorbed),
ADR-020 (OpenBao — secrets surface absorbed),
ADR-026 (ArgoCD — deployment timeline absorbed),
ADR-067 (Crossplane v2 — XR view absorbed)

## Context

Cave Runtime collects, in one cluster, the operational surfaces of ~10 mature upstream products: Argo CD, Grafana / Prometheus / Loki / Tempo (LGTM), OpenBao, Crossplane, Hubble, Renovate, DefectDojo, Reflex Engine, KEDA dashboards, Karpenter NodePool views, etc. Each ships its own HTTP UI, its own auth flow, its own URL space, its own CSS, and its own tenancy model (typically: weak or none).

Naively wiring every upstream UI into the cluster's ingress produces:

- **Auth-flow fan-out.** Every UI handles SSO differently. A tenant logs in 4–6 times in a single shift. Operators worse.
- **CSS / UX schism.** A tenant who looks at "their" cluster sees five different design languages depending on which view they hit. Cognitive tax is real.
- **Tenancy leakage.** Many upstream UIs were not designed for hard multi-tenancy. Argo's AppProject view, Grafana's org-scoped dashboards, OpenBao's path-based ACLs — each has a different tenancy story, and the union is the weakest link.
- **Direct attack surface.** Every exposed UI is a CVE vector. Even with read-only routes, XSS in a rendering library is a tenant-cross-boundary risk.
- **Permalink rot.** "View in Grafana" / "open in Argo" deep-links break on upstream upgrades; users don't know whether the broken link is the platform's fault or the upstream's.
- **Self-improvement headwind.** Reflex (ADR-095) wants to learn from operator clicks. If clicks happen across 6 disparate UIs, the data is unjoinable.

This ADR is the runtime-side corollary of ADR-RUNTIME-UPSTREAM-MIRROR-001. The mirror principle says "Cave Runtime reimplements the upstream"; this ADR enforces that the UX surface mirrors that reimplementation: there is no upstream UI for the user to see in the first place.

## Decision

The **cave-portal** is the single pane of glass for every operational surface in Cave Runtime. Concretely:

1. **Single pane of glass.** Tenants, operators, and platform admins reach every operational view through cave-portal. No other portal is offered, recommended, linked, or documented as a user-facing surface.
2. **No upstream UI exposure.** Upstream HTTP UIs (Argo CD, Grafana, OpenBao, Crossplane, Hubble, Renovate, DefectDojo, …) are **not** exposed to users. They are not behind a port-forward, not behind an ingress route, not in the navigation menu, not in any documentation as a user destination. Where the upstream binary insists on running an embedded UI server, it is bound to localhost only and firewalled off.
3. **Native reimplementation of every critical view.** Each upstream view that is *operationally meaningful* (deployment timeline, dashboard panel, secret browser, flow diagram, action queue, finding triage list, dependency PR queue) is reimplemented as a first-class React / web-component view inside cave-portal, drawing data through cave-portal-api.
4. **Proxy-only data path.** Browsers never talk to upstream products directly. cave-portal-api is the sole mediator: it authenticates (single OIDC flow), authorizes (persona × tenant), translates upstream wire formats into cave-portal's typed contract, and returns the result. The upstream's port number does not appear in any browser URL.
5. **No "View in <upstream>" escape hatches.** A view in cave-portal does not link out to the upstream UI. Detail navigation is breadcrumb-driven inside cave-portal. If a feature is missing from cave-portal, the right answer is to add it to cave-portal, not to expose a hatch.
6. **Persona-aware navigation.** Three personas are first-class: Tenant, Operator, Platform Admin. Every view declares which personas may see it; the navigation tree, the data scope, and the permitted actions are all derived from the active persona × the active tenant_id. A tenant never sees another tenant's resources by any path.

### Wrappers shipped at v0.1

| Upstream surface | cave-portal view | Notes |
|---|---|---|
| Argo CD UI | `Deployments` (timeline, sync diff, resource tree) | Native renderer; drift indicator + rollback control |
| Grafana | `Dashboards` (panel renderer, time-range, query var inputs) | cave-portal hosts the panel; Grafana queries flow via cave-portal-api |
| Prometheus | `Metrics explorer` | PromQL editor with tenant-scoped label injection |
| Loki / Tempo | `Logs`, `Traces` | LogQL / TraceQL editors; tenant label enforced by API |
| OpenBao | `Secrets` (browser, lease list, dynamic-cred viewer) | Read-mostly; create/edit gated by persona |
| Crossplane | `Resources` (XR tree, status, events, claims) | Composes with `Deployments` view |
| Hubble | `Network flows` (live + replay) | Server-side filter on tenant identity |
| Reflex Engine | `Actions` (queue, run history, attestations) | Click-stream feeds the self-improving loop |
| DefectDojo | `Findings` (triage, suppress, assign) | Tenant-scoped engagement view |
| Renovate | `Dependencies` (PR backlog, schedule, mute) | Per-tenant repo scope |

Backstage is **not** wrapped — cave-portal already reimplements the Backstage developer-portal surface (catalog, software templates, scorecards, tech docs); see ADR-031.

### Critical separation rule (backend vs. UI)

**Critical separation rule:** When Cave Runtime reimplements an upstream product (e.g., Argo CD → cave-argocd, Grafana → cave-grafana, OpenBao → cave-vault, Cilium → cave-net, Hubble → cave-hubble), only the **backend/API/data layer** is reimplemented in the upstream-mirror crate. The **UI/UX layer is NEVER ported into the upstream-mirror crate** — instead, the equivalent UX is implemented natively inside cave-portal, consuming the upstream-mirror crate's API. This means:

- cave-argocd crate = sync engine, repo poller, app reconciler, RBAC. **No UI code.**
- cave-portal = native React deployment timeline view, sync status panel, rollback button — talking to cave-argocd's API
- cave-grafana crate = dashboard storage + query engine + alert rule evaluator. **No UI code.**
- cave-portal = native React panel renderer + dashboard browser
- Same separation for cave-vault, cave-net, cave-hubble, cave-defectdojo, cave-renovate, cave-reflex

Implication: Mirror principle (ADR-RUNTIME-UPSTREAM-MIRROR-001) applies only to backend/API parity. Upstream UI codebases are not line-by-line ported; they are reimagined natively in cave-portal under persona-aware, single-pane-of-glass UX.

## Rationale

1. **UX integrity.** One design language, one auth flow, one persona model. The mental load of switching between five upstream UIs is replaced by the ordinary cost of learning one product.
2. **Sovereign attack surface.** Every upstream UI removed is one less CVE-bearing component facing the tenant. The cluster's user-facing attack surface is reduced to cave-portal itself, which is a first-class hardening target.
3. **Self-improving charter.** Reflex Engine and the platform's ML-driven action recommender both depend on a unified click-stream. Owning the data layer end-to-end — telemetry, intent, action — is what makes that loop possible. Federated UIs would prevent it.
4. **Multi-tenant invariant.** Hard tenancy is enforced once, at cave-portal-api, not ten times across ten upstream tenancy models. Cross-tenant leakage requires breaking cave-portal-api's contract; it cannot happen via "did anyone remember to set the namespace filter in the Grafana org config?"
5. **Stable URLs.** Permalinks live in cave-portal's URL space and are versioned by cave-portal. Upstream upgrades cannot break user bookmarks.
6. **Audit unification.** Every operator action is recorded by cave-portal-api in a single audit stream (compliance: SOC2 CC7.1). Compare to the cost of joining audit logs across ten upstream products with ten different formats.

## Consequences

### Positive

- Single coherent UX across every persona.
- Single auth flow (OIDC → cave-portal → API) with one place to enforce step-up for sensitive actions.
- Tighter security perimeter — upstream UIs never face users.
- Native instrumentation — telemetry covers 100% of operator interaction with no joining gymnastics.
- Persona-aware automation becomes tractable; Reflex, ML recommenders, anomaly detection all read one stream.
- Upstream-agnostic UX: if an upstream is replaced (per ADR-RUNTIME-UPSTREAM-MIRROR-001), the user-facing surface need not change.
- Compliance-friendly: one audit surface, one access-control model.

### Negative

- Substantial frontend investment: cave-portal must reach Backstage parity *plus* ship 8–10 native wrappers. This is a multi-quarter program.
- Wrapper-vs-upstream drift: when an upstream ships a new feature, the wrapper must catch up. We accept lag and prioritise.
- Skill profile: the Cave team must invest in frontend engineering (React, web components, accessibility) at a level the OSS-stitching alternative does not require.
- "Power-user escape hatch" is genuinely missed in early days; we counter with first-class CLI surface (ADR-RUNTIME-CLI-CONSOLIDATION-001) and excellent platform-admin views.

### Risks

- **Wrapper coverage gaps in v0.1.** Some niche upstream features (e.g. Grafana plugin marketplace) won't be in cave-portal at launch. Mitigation: prioritise by usage frequency, ship a "request feature" feedback loop in the portal itself.
- **Upstream API churn.** When Argo or Grafana change their REST/gRPC surface, cave-portal-api breaks. Mitigation: contract tests against upstream pinned versions (ADR-135 Provider Parity Contract Testing's discipline applies here).
- **Frontend velocity bottleneck.** A single team owning ten wrappers is a constraint. Mitigation: shared component library + codegen from upstream OpenAPI specs where available.

## Implementation

cave-portal sprint plan (per upstream wrapper, M-series milestones):

- **M1 — Deployments wrapper** (Argo CD: app list, sync, diff, rollback). Replaces Argo UI.
- **M2 — Dashboards wrapper** (Grafana: panel renderer, time-range, query vars, tenant label injection).
- **M3 — Metrics / Logs / Traces explorers** (Prometheus + Loki + Tempo).
- **M4 — Secrets wrapper** (OpenBao: browser, lease list, dynamic-cred viewer; persona-gated mutations).
- **M5 — Resources wrapper** (Crossplane XRs, claims, status, events).
- **M6 — Network flows wrapper** (Hubble: live + replay, identity-scoped).
- **M7 — Actions wrapper** (Reflex queue, run history, attestation links).
- **M8 — Findings wrapper** (DefectDojo: triage, suppress, assign, tenant-scoped).
- **M9 — Dependencies wrapper** (Renovate: PR backlog, schedule, mute).
- **M10 — Persona shell** (navigation tree, scope switcher, audit panel).

Each milestone delivers: native React/wc views + cave-portal-api endpoints + contract tests + documentation entry. Any upstream UI exposed during the migration is removed in the same milestone PR — the migration is not "complete" until exposure is gone.

### Anti-patterns explicitly prohibited

- Embedding the upstream UI in an `<iframe>`. (Defeats the point — auth, CSS, attack surface, tenancy.)
- Setting up an ingress like `argocd.cave.run` "for power users." (See item 2 of the Decision.)
- Documenting an upstream UI port in a runbook as the path of least resistance. (Use a CLI command, file a portal feature request, but do not document the upstream UI as a user destination.)

## Compliance

- **SOC2 CC7.1** — single audit surface for user-facing operations.
- **ISO 27001 A.5.30** — UX consistency in operational interfaces (information security awareness includes the consistency of the surfaces operators are trained on).
- **ISO 27001 A.8.34** — protection of information systems during audit testing (single auditable surface).
- **NIS2 Art. 21** — security measures (reduced attack surface).

## Out of Scope

- **Backstage upstream wrap.** cave-portal already reimplements Backstage's developer-portal surface (catalog, templates, scorecards, tech docs). It is not on the wrapper list because the upstream isn't deployed.
- **Tenant application UIs.** A tenant's own application (e.g. a Java Spring Boot service that ships a web page) is the tenant's responsibility and is naturally exposed through the gateway as part of the tenant's product. This ADR governs *operational* surfaces (cluster, platform, dependency views), not tenant product surfaces.
- **CLI consolidation.** Out of scope here; covered by ADR-RUNTIME-CLI-CONSOLIDATION-001 (sister principle).
- **Mobile app surface.** Not in v0.x; covered separately if/when prioritised.

## Notes

- This ADR is enforcement, not aspiration. A Cave Runtime build that exposes any upstream operational UI to users is a regression and is reverted. Reviews block on it.
- The mirror principle (ADR-RUNTIME-UPSTREAM-MIRROR-001) gives this ADR its back-stop: when the upstream itself is eventually reimplemented in Rust as a Cave crate, the wrapper becomes the sole UI surface for that capability — no upstream to wrap any more.
- "Single pane of glass" is the user-facing stance. Internally, cave-portal is composed of many React/web-component modules, but they ship as one product with one URL space and one auth flow.
