# cave-flags — Charter v2 8-gate Close-out Report

**Audit date**: 2026-05-18
**Upstream**: `Unleash/unleash @ v5.0.0` (Apache-2.0, TypeScript)
**Crate root**: `crates/cave-flags/`

## Scope

cave-flags is the Cave Runtime feature-flag service, wire-compatible with the
Unleash v5 client / frontend / admin REST protocols so any Unleash SDK (Go,
Node, Java, Python, Rust, …) can point at cave-flags without modification.

Implemented:

- Feature toggle taxonomy (release / experiment / operational / kill-switch / permission)
- Activation strategies: `default`, `userWithId`, `gradualRolloutRandom`,
  `gradualRolloutSessionId`, `gradualRolloutUserId`, `flexibleRollout`,
  `remoteAddress`, `applicationHostname`, plus a custom-strategy passthrough
- Full constraint operator set: `IN`/`NOT_IN`, `STR_STARTS_WITH`/`STR_ENDS_WITH`/`STR_CONTAINS`,
  `NUM_{EQ,GT,GTE,LT,LTE}`, `DATE_{BEFORE,AFTER}`, `SEMVER_{EQ,GT,LT}` with
  `case_insensitive` + `inverted` modifiers
- Segments (reusable constraint groups) gated before strategy evaluation
- Weighted variants with stickiness resolution (userId / sessionId / random / property)
  using JS-compatible 32-bit murmur3 (seed=0)
- Variant overrides (context-keyed exact-match, takes precedence over weights)
- Per-environment enable/disable + per-env strategies/variants
- Projects with default-strategy bootstrap
- Client SDK protocol (`/api/client/features{,/{name}}`, `/api/client/metrics`,
  `/api/client/register`)
- Frontend SDK protocol (`/api/frontend{,/features{,/{name}}}`) — server-evaluated
- Admin REST: features, environments, strategies, variants, tags, projects,
  api-tokens, context-fields
- Postgres schema across two migrations: features / environments /
  feature_environments / strategies / variants / segments / tags + tokens /
  metrics / client_applications / context_fields / banners / change_requests /
  impression_events
- Unleash v2 wire envelope (`UnleashFeaturesResponse` / `UnleashToggle` /
  `UnleashStrategy`) so existing SDKs round-trip cleanly
- Client application + instance registry
- Impression-data flag on every toggle (events persist; see below)

## Inventory measurement

Hand-curated against Unleash v5.0.0 `src/lib/` (services/, routes/, features/,
addons/, db/, openapi/, middleware/) plus top-level entrypoints. Each
subsystem counts once.

| Bucket   | Count | Examples                                                                              |
|----------|------:|---------------------------------------------------------------------------------------|
| Mapped   |    25 | strategies (9), constraints (16 operators), segments, variants + overrides,          |
|          |       | environments, projects, murmur3 hash, client API (features/metrics/register),         |
|          |       | frontend API, admin CRUD (features/envs/strategies/variants/tags/projects/            |
|          |       | api-tokens/context-fields), Postgres schema, Unleash v2 wire envelope,                |
|          |       | client-instance registry, impression-data flag                                        |
| Partial  |     3 | change-requests (schema only, no state-machine), banners (schema only, no             |
|          |       | admin CRUD or display), stale detection (flag present, no detector job)                |
| Skipped  |    30 | React UI (cave-portal), SAML/OIDC/users/groups/roles/accounts (cave-auth),            |
|          |       | OpenAPI gen (workspace-level), addons (slack/teams/datadog/webhook/jira),             |
|          |       | telemetry/version/instance-stats (cave-obs), scheduler (cave-runtime),                |
|          |       | middleware (auth/cors/rate-limit handled by cave-runtime), event bus                  |
|          |       | (cave-runtime), settings, advisory locks (cave-db), migration runner (cave-db),       |
|          |       | TS tests/scripts/build, enterprise plugins, redis pubsub, proxy repo,                 |
|          |       | public-signup (cave-auth), version-check (cave-upstream-watchd)                       |
| Unmapped |     7 | custom-strategy server registration, stale-detector job, metric-bucket compactor,     |
|          |       | public invite tokens, Edge token API, frontend-proxy mode,                            |
|          |       | impression-events SSE/webhook fanout                                                   |
| **Total**|  **65** | |

- **fill_ratio  = (mapped + partial + skipped) / total = 58 / 65 = 0.8923**
- **honest_ratio = (mapped + skipped) / total            = 55 / 65 = 0.8462**

## 8-gate close-out

| # | Gate                                | Result | Evidence                                                                 |
|---|-------------------------------------|--------|--------------------------------------------------------------------------|
| 1 | SPDX-License-Identifier 100%        | PASS   | 8/8 `crates/cave-flags/**/*.rs` carry AGPL-3.0-or-later                  |
| 2 | `source_sha` pinned in manifest     | PASS   | `[upstream].source_sha = "v5.0.0"`                                       |
| 3 | `last_audit = "2026-05-18"`         | PASS   | `[parity].last_audit`                                                    |
| 4 | `parity_ratio_source = "manifest"`  | PASS   | parity-index reads `fill_ratio` directly from this manifest              |
| 5 | `fill_ratio >= 0.65`                | PASS   | 0.8923 (well above Charter v2 close-out floor)                           |
| 6 | counts sum to total                 | PASS   | 25 + 3 + 30 + 7 = 65                                                     |
| 7 | No `unimplemented!()` / `todo!()`   | PASS   | 0 stub macros under `src/` (string-literal references inside rules-disabled `cave-scan` are unrelated) |
| 8 | `PARITY_REPORT.md` present          | PASS   | this file                                                                |

**Charter v2 verdict: 8/8 PASS.**

## Test coverage

`cargo test -p cave-flags --lib --tests` exercises:

- 15 in-source unit tests in `src/engine.rs` (strategies, constraints, variants,
  segments, environments, murmur3 determinism)
- 9 close-out self-audit assertions in `tests/parity_self_audit.rs`

## Next sweep (out of this close-out)

In priority order — every item is a documented `[[unmapped]]` block:

1. **stale-feature detector job (P1)** — runner that marks `last_seen_at` +
   `stale=true` on dormant flags. Smallest lift, biggest UX win.
2. **impression-event SSE/webhook fanout (P1)** — currently events persist but
   consumers must poll the DB.
3. **custom-strategy server registration (P2)** — admin UI registers strategy
   signature + applies in eval.
4. **metric-bucket compactor (P2)** — hourly compaction so `metrics` doesn't
   grow unbounded.
5. **Edge token API (P2)** — Unleash Edge sidecar runtime target.
6. **frontend-proxy mode (P2)** — standalone proxy binary fronting cave-flags.
7. **public invite tokens (P3)** — project-invite surface beyond cave-auth's
   user-signup tokens.

## Hand-offs (already counted as `skipped`)

These belong to sibling crates by Charter design and will NOT land in cave-flags:

- React admin UI → **cave-portal** (`/admin/flags/*`)
- SAML / OIDC / users / groups / RBAC → **cave-auth** (Keycloak v26)
- Periodic-job scheduler → **cave-runtime** scheduler core
- Middleware stack (CORS / rate-limit / OAS) → **cave-runtime** middleware
- Cross-module event bus → **cave-runtime** events
- Advisory locks + migration runner → **cave-db**
- Prometheus client + telemetry + version-check → **cave-obs** / **cave-upstream-watchd**
- Addons (slack/teams/datadog/webhook/jira) → **cave-flags-addons** (post-launch)
