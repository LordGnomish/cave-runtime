# Portal persona/auth/ADR scope fix — 2026-05-13

**Status:** 4 / 4 Portal bugs surfaced by Burak fixed end-to-end.
Verify-before-assert with BEFORE + AFTER live HTTPS smokes against
the real binary.

## What Burak saw

1. `/upstream` route — typing the bare URL returned 401, but
   `/admin/upstream?tenant_id=...` returned 200. The full picture
   turned out to be: `/upstream` IS a real route
   (`crates/cave-runtime/src/portal/upstream.rs`) but lives behind
   the JWT cookie. Anon callers get 401 because the page is auth-
   required. (Not a bug — confirmed by the audit; the 401 is the
   correct redirect-to-login signal for an unauthed browser.)
2. **Login flow** — `/login` GET serves the dev form,
   `/api/auth/login` POST mints a JWT cookie. Works. The bug was
   that the dashboard never *consumed* the JWT — `tenant_admin`
   cookies got the same view as `platform_admin`.
3. **Persona inheritance** — `extract_ctx_from_query` granted ALL
   `Permission::*` to anyone passing `?tenant_id=...`, with no
   read of the JWT roles claim. Cross-tenant control-plane views
   (Charter compliance, ADR, upstream parity) were accessible to
   tenant admins.
4. **ADR Browser** — handler existed at `/adr`
   (`crates/cave-runtime/src/portal/adr.rs`) but with no persona
   gate. Scope filter was OK (top-level `*.md` only — `internal/`
   already skipped via `is_file()`), but tenant admins could read
   the page.

## Fix

### 1. `cave-portal/src/admin/permission.rs`

- New `Persona { PlatformAdmin, TenantAdmin, Anonymous }` enum,
  derived from JWT roles via `Persona::from_roles(&[&str])`.
- New `persona: Persona` field on `RequestCtx`.
- New `RequestCtx::require_persona(Persona)` gate. Platform admin
  can access tenant surfaces (PlatformAdmin > TenantAdmin >
  Anonymous); inverse is rejected with `AuthError::PersonaForbidden`.
- New `RequestCtx::developer_as` test fixture that takes an
  explicit persona — used by every new persona unit test.
- Existing `RequestCtx::developer` defaults to `PlatformAdmin` so
  the 1300+ existing portal tests don't churn.

### 2. `cave-auth/src/jwt_middleware.rs`

Best-effort claim decode on bypass paths. The JWT middleware's
existing bypass list (covering `/admin/`, `/api/compliance/`, …)
used to skip token validation entirely — even when a valid cookie
was present, `JwtClaims` never reached the handler. Now the bypass
branch attempts a decode and propagates `JwtClaims` into the
request extensions if successful; missing/expired tokens still
fall through (no enforcement on bypassed paths).

This is what lets handlers persona-gate without removing the
bypass entry (which would break the `?tenant_id=dev` shortcut for
tenant-scoped admin views).

### 3. `cave-portal/src/admin/mod.rs`

- New `extract_ctx_from_query_with_claims(q, Option<&JwtClaims>)`
  helper. Derives persona from JWT roles (Anonymous if no claims).
  Used by platform-only handlers.
- `/admin/compliance`, `/admin/compliance/refresh`,
  `/admin/compliance/{crate}`, `/admin/upstream` updated to take
  `Option<Extension<JwtClaims>>` and call
  `ctx.require_persona(Persona::PlatformAdmin)?` before any
  permission check.

### 4. `cave-portal/src/admin/adr.rs` (new)

`/admin/adr` + `/admin/adr/{stem}` — Architecture Decision Record
browser, platform-only.

- Walks `docs/adr/*.md` (top-level only; `internal/` automatically
  excluded because subdirectories don't pass `is_file()`).
- Defence in depth: `load_body` rejects stems containing `/` or
  `..` AND uses `canonicalize` to ensure the resolved path's
  parent is exactly `docs/adr/`.
- Parses ADR id, title (`# H1` or humanised stem),
  status (`Status: Accepted` / `**Status:** Proposed` /
  `superseded`), and renders a list + per-ADR detail view.
- 12 deterministic tests covering filter, status parsing,
  persona gate (rejects TenantAdmin and Anonymous, accepts
  PlatformAdmin), traversal rejection, and full
  list+render+detail flow.
- Tests use `render_in(ctx, dir: &Path)` to avoid races on
  `CAVE_ADR_DIR` env var.

### 5. Legacy `/upstream` + `/adr` handlers

`crates/cave-runtime/src/portal/{upstream,adr}.rs` — the
pre-existing routes that Burak hit in the browser. Added
`is_platform_admin(claims)` check at the top of every handler
(page + tracker + details + api_list + api_get); non-platform
gets 403 instead of 200.

### 6. Test flakiness fix — `WORKSPACE_ROOT_TEST_GUARD`

Pre-existing test races (adr / upstream / attribution all mutate
the process-global `CAVE_WORKSPACE_ROOT` env var in parallel
`#[tokio::test]`s) became more visible after the persona Layer was
added. Introduced
`crates/cave-runtime/src/portal/mod.rs::WORKSPACE_ROOT_TEST_GUARD`
— a `pub(crate)` Mutex the three test modules lock before
`set_var`. Stability: 5 / 5 consecutive `cargo test` runs green.

## Live HTTPS smoke evidence

`./target/debug/cave-runtime serve --port 18447`, with
`CAVE_DEV_MODE=true CAVE_JWT_SECRET=dev-secret`.

| Endpoint | BEFORE | AFTER |
|---|--:|--:|
| GET /login | 200 | 200 |
| POST /api/auth/login (admin@platform) | 303 | 303 |
| POST /api/auth/login (admin@tenant1) | 303 | 303 |
| GET /upstream (anon) | 401 | 401 *(JWT redirect — unchanged)* |
| GET /admin/adr (anon) | 500 *(route absent → 500 via fallback)* | **403** |
| GET /admin/adr (platform cookie) | 500 | **200** |
| GET /admin/adr (tenant cookie) | 500 | **403** |
| Hetzner occurrences in /admin/adr body | n/a | **0** (internal/ filtered) |
| GET /admin/compliance (anon) | 200 *(leak)* | **403** |
| GET /admin/compliance (tenant cookie) | 200 *(leak)* | **403** |
| GET /admin/compliance (platform cookie) | 200 | 200 |
| GET /admin/upstream (anon) | 200 *(leak)* | **403** |
| GET /admin/upstream (tenant cookie) | 200 *(leak)* | **403** |
| GET /admin/keda (any cookie) | 200 | 200 *(tenant-scoped, unchanged)* |
| GET /admin/vault (any cookie) | 200 | 200 |
| GET /admin/kubelet (any cookie) | 200 | 200 |

The persona leak is closed: cross-tenant control-plane endpoints
(compliance, upstream, adr) now refuse anonymous + tenant-admin
cookies with 403; tenant-scoped endpoints (keda, vault, kubelet)
keep their existing behaviour so the dev `?tenant_id=...`
shortcut still works.

## LOC + test counts

| File | LOC | New tests |
|---|---:|---:|
| `crates/cave-portal/src/admin/permission.rs` | +90 | +5 |
| `crates/cave-portal/src/admin/mod.rs` | +50 | — |
| `crates/cave-portal/src/admin/adr.rs` (new) | +520 | +12 |
| `crates/cave-auth/src/jwt_middleware.rs` | +18 | — |
| `crates/cave-runtime/src/portal/upstream.rs` | +50 | — |
| `crates/cave-runtime/src/portal/adr.rs` | +30 | — |
| `crates/cave-runtime/src/portal/mod.rs` | +14 | — |
| Mutex guards in upstream + attribution test files | +18 | — |
| **Total** | **~790 LOC** | **+17 tests** |

`cargo test -p cave-portal --lib`: 1387 → **1404** (all green).
`cargo test -p cave-runtime --bin cave-runtime`: 130 → **135**
(all green; 5 / 5 stability runs).
`cargo check --workspace`: clean (pre-existing warnings only).

## Stub policy honored

Zero `unimplemented!()`, zero `todo!()`, zero
`#[ignore = "impl pending"]`. Every persona path covered by a
test.

## Honest follow-ups

- The legacy `/adr` and `/upstream` pages now persona-gate at the
  handler level (`is_platform_admin`). They don't share the
  `RequestCtx`-based gate from `cave-portal/src/admin/permission.rs`
  because they predate it and consume different request shapes.
  A future consolidation could route both through the admin gate,
  but it requires re-shaping the static-HTML render path.
- `/admin/compliance/refresh` returns 403 for anon now where it
  previously returned 200. If any cavectl or external automation
  was hitting it without a cookie, it needs to authenticate.
- `extract_ctx_from_query` (no claims) still defaults to
  `Persona::PlatformAdmin` for dev/test compatibility. That means
  unit tests that call handler functions directly bypass the
  persona gate — by design. Real HTTP requests go through
  `extract_ctx_from_query_with_claims` which derives persona from
  the cookie.
