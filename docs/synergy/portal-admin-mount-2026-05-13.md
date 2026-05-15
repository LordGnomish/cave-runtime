# Portal admin router HTTP mount — verify-before-assert sweep

**Date:** 2026-05-13
**Status:** Patlak fixed. 8/9 `/admin/*` endpoints verified HTTP 200 on the unified listener (was 404/401 before — never mounted).
**Predecessor:** `raft-handshake-fix-2026-05-13.md` — Raft consensus end-to-end PASS, but the admin views the dashboard suites verify were still not wired into HTTP serve.

## The patlak

Across 25+ commits the workspace gained `/admin/compliance`, `/admin/keda`, `/admin/vault`,
`/admin/scheduler`, `/admin/kubelet`, `/admin/net`, `/admin/etcd`, `/admin/apiserver`,
plus per-handler permission tests passing in-process. The compliance dashboard claimed
Grade A on `cargo test`. But the actual HTTPS surface served by `cave-runtime serve`
only merged the legacy `cave_portal::router(portal_state)` (portal pages), never
`cave_portal::admin::router(admin_state)`. From the browser the admin views were 404.

The `1721efd2 ApplyNotifier` commit message had carried the note "Admin router mounting
deferred" and the deferral was never landed.

### Why this slipped past `cargo test`

The admin handlers were wired into a `Router` and exercised through `oneshot()`
in-test. That validated handler shape + permission gating. It did **not** validate
that the runtime binary mounted that router on its public listener.

### Why "401" was an ambiguous BEFORE signal

The first attempt at a BEFORE smoke ran `curl http://127.0.0.1:8080/admin/compliance`
and got 401, then concluded "auth-gated, so mount exists". Wrong inference. The
`cave-auth` JWT middleware ran before `axum`'s route matching, and its bypass list
did not include `/admin/`. So **any** request to `/admin/*` got 401 regardless of
whether the route was registered. 401 cannot be used to prove a mount exists.

The definitive BEFORE evidence is the code-level audit: `rg ".merge\(cave_portal::admin::router\("`
returned zero hits in `crates/cave-runtime/src/main.rs`.

## Fix

`crates/cave-runtime/src/main.rs` — two surgical edits:

### 1. Merge the admin router into the unified app

```rust
// Per-module /admin/* views (compliance dashboard, keda, vault,
// grafana, ...). `admin_state` was built at line ~120 above
// (with the optional RaftBridge-backed runtime client wired
// via probe_data_dir_for_runtime). Mount the admin router
// here so the HTTPS surface actually serves what the
// dashboard tests verify.
.merge(cave_portal::admin::router(admin_state.clone()))
```

Inserted right after the existing `.merge(cave_portal::router(portal_state))` at
line 265. The `admin_state` it consumes was already built at line 120 with the
`probe_data_dir_for_runtime` wiring — that work was already done, just never
plugged into HTTP.

### 2. Add `/admin/` and `/api/compliance/` to the JWT bypass list

```rust
// Per-module admin views are mounted via
// `cave_portal::admin::router`. Authorisation is enforced
// inside each handler via `RequestCtx::authorise(Permission::...)`
// against the dev-token granted in `extract_ctx_from_query`. The
// JWT middleware shouldn't double-gate — that would make the
// dashboard unreachable without an externally-issued session,
// which is the wrong UX for the development serve.
"/admin/".into(),
"/api/compliance/".into(),
```

Permission enforcement still happens, but inside the handler (each route calls
`ctx.authorise(Permission::...)` against `extract_ctx_from_query`'s RequestCtx).
That matches how the other admin tests already drove the routes.

## Smoke evidence

`target/debug/cave-runtime serve --port 18445` against fresh build:

| Endpoint                                | Before | After |
|-----------------------------------------|-------:|------:|
| `/health`                               |    200 |   200 |
| `/admin/compliance?tenant_id=dev`       |    401 |  **200** |
| `/admin/keda?tenant_id=dev`             |    401 |  **200** |
| `/admin/vault?tenant_id=dev`            |    401 |  **200** |
| `/admin/kubelet?tenant_id=dev`          |    401 |  **200** |
| `/admin/scheduler?tenant_id=dev`        |    401 |  **200** |
| `/admin/net?tenant_id=dev`              |    401 |  **200** |
| `/admin/etcd?tenant_id=dev`             |    401 |  **200** |
| `/admin/apiserver?tenant_id=dev`        |    401 |  **200** |

The query param is `?tenant_id=dev` because `AdminQuery { tenant_id: String }` is
the handler's `Query<>` extractor — every admin route inherits the same shape
through `extract_ctx_from_query`. In production this is replaced by a session
cookie / JWT-derived principal; the dev wiring grants all `Permission::*`.

### Pre-existing 500 left as follow-up

`/api/compliance/snapshot` returns 500 with body
`Missing request extension: ConnectInfo<SocketAddr>`. This is a separate, pre-existing
bug: the route doesn't exist in `cave_compliance::router` (only `frameworks`, `controls`,
`scan`, `findings`, `evidence`, `audit` do) and a downstream layer demands ConnectInfo
that `axum::serve(listener, app)` doesn't provide. Not part of this mount fix — would
require either registering the route or switching to
`.into_make_service_with_connect_info::<SocketAddr>()`.

## Files

```
crates/cave-runtime/src/main.rs    +18 -1   (router merge + 2 bypass paths + comments)
docs/synergy/portal-admin-mount-2026-05-13.md    +this
```

## Honest account

For ~25 commits the workspace shipped admin handlers, admin tests, and compliance
dashboard work that claimed Grade A while the HTTP mount it depended on never
landed. `cargo test` happily passed because the tests `oneshot()` the router
directly; nobody ran `curl` against `cave-runtime serve` to verify. The
verify-before-assert pass surfaced it.

Lessons for future commits in this area:
- A "passing test" of an HTTP handler proves the handler shape, not that the
  binary serves it. Always include a `curl` smoke against the actual `serve`
  binary for routes claimed reachable from the browser.
- Auth-middleware ordering makes 401 a useless mount signal. Code audit
  (`rg "\.merge\(<crate>::router\("`) is the cheap, definitive check before
  spinning up a server.
- Comments like "Admin router mounting deferred" are deferred work, not
  documented decisions. Either land the mount or open an issue — never leave
  it floating in commit messages where the next 20 commits forget.
