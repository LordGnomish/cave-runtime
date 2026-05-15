# Portal user-friendly polish sweep — 2026-05-14 → 2026-05-15

> Mandate: "dünyanın en user-friendly portalı olmalı" — five
> targeted polish items shipped TDD-strict on the four-track Portal
> surface, ahead of the OSS launch (T-7).

Branch: `claude/unruffled-swanson-000faa` (merged into main).

## TL;DR

| # | Item | Status | Tests | Files |
|---|------|--------|-------|-------|
| 1 | Command palette persona-filter | ✅ | +9  | `command_palette.rs` + `shell.rs` + `permission.rs` |
| 2 | Shortcuts persona-filter (g a / g c / g u / g l → toast on TenantAdmin) | ✅ | +8  | `shortcuts.rs` + `shell.rs` |
| 3 | Legacy `page_shell()` → `page_shell_full(ctx, ...)` migration | ✅ (25/25) | 1/1 fix-up | `adr.rs` `compliance.rs` `iceberg/mod.rs` `keda/*.rs` `contributions.rs` `mod.rs` |
| 4 | WCAG AA static analyser + lock-in tests | ✅ | +21 | `layout/a11y.rs` (new) |
| 5 | `/admin/_audit` consolidated dashboard | ✅ | +16 + 3 mount smoke | `meta_audit.rs` (new) + `mod.rs` |
| 4-track | `cavectl portal audit` subcommand | ✅ | +3 parse | `cave-cli/src/main.rs` |

**Tests**: cave-portal lib **1866 → 1925** (+59), cavectl bin **+3**.
Workspace `cargo check --workspace --tests` clean. Cavectl build
unblocked en passant (pre-existing E0433 from `native::auth.rs:477`
referencing `crate::client::ApiClient` while the lib didn't expose
the module).

## Item 1 — Command palette persona-filter

**Before**: `default_commands(tenant_id)` returned the same 14
entries to every persona — TenantAdmin saw "Go to Compliance",
"Go to ADR Browser", "Go to Upstream", "Go to Cluster Status"
(all PlatformAdmin-only routes that 403 on click).

**After**:

* New `Persona::can_access(min: Persona) -> bool` helper
  (lattice: PlatformAdmin > TenantAdmin > Anonymous).
* `CommandItem` carries `min_persona: Persona`. Two new
  constructors `nav_platform()` / `action_platform()` flag entries
  as PlatformAdmin-only.
* `default_commands_for_persona(tenant_id, persona)` — filters at
  build time. Legacy `default_commands(tenant_id)` kept as alias
  for the all-entries view (caller-of-record is the `shell_v2`
  chrome, which now passes `opts.persona` through).
* Five entries flagged platform: Compliance, Upstream, ADR Browser,
  the new `/admin/_audit`, Cluster Status.

TDD trail (RED first, then GREEN):

1. `tenant_admin_does_not_see_platform_only_commands` (forbidden
   labels never appear).
2. `platform_admin_sees_all_default_commands` (every required
   label appears).
3. `anonymous_persona_sees_only_anonymous_tier`.
4. `nav_platform_constructor_marks_min_persona_platform_admin`.
5. `default_command_item_is_visible_to_everyone`.
6. `min_persona_omitted_from_json_when_anonymous` (JSON wire
   stability — palette data script unchanged for default rows).
7. Whole-document smoke
   `shell_v2_palette_excludes_platform_entries_for_tenant_admin`.
8. Whole-document smoke
   `shell_v2_palette_includes_platform_entries_for_platform_admin`.
9. Persona helper unit tests `platform_admin_can_access_every_tier`,
   `tenant_admin_blocked_from_platform_only`,
   `anonymous_can_only_access_anonymous_tier`.

## Item 2 — Shortcuts persona-filter

**Before**: `g a` (ADR), `g c` (Compliance), `g u` (Upstream),
`g l` (Cluster live) all wired into the `gMap` for every persona.
Pressing them as TenantAdmin navigated to a 403 surface with no
explanation.

**After**:

* `ShortcutBinding` carries `min_persona: Persona`. The four
  PlatformAdmin-only `g`-leader bindings are flagged.
* `shortcuts_help_modal(bindings, persona, tenant_id)` builds two
  separate JS maps per render:
  * `gMap` — enabled bindings, normal navigation.
  * `gDeniedMap` — disabled bindings, used to fire a toast
    `caveToast('warning', '<description> — requires Platform Admin')`
    via the existing `toast.rs` global.
* Help-modal table still lists every binding for discoverability,
  but disabled rows get a `data-disabled="true"` marker, an
  `opacity-60` class, and a "Platform" amber badge.
* New `g _` binding added for the `/admin/_audit` rollup.

TDD trail (RED → GREEN):

1. `enabled_for_blocks_platform_only_bindings_for_tenant_admin`.
2. `tenant_admin_g_map_omits_platform_only_bindings`.
3. `tenant_admin_help_modal_lists_disabled_rows_with_platform_badge`.
4. `platform_admin_help_modal_has_no_disabled_rows`.
5. `denied_keys_route_through_caveToast_in_js`.
6. `denied_map_carries_descriptions_for_blocked_bindings`.
7. `platform_admin_denied_map_is_empty`.
8. `anonymous_persona_treated_like_tenant_admin_for_platform_routes`.

## Item 3 — Legacy `page_shell()` migration

Mandate target: 26 callsites. Actual count post-merge: **25**
single-handler callsites + 1 bespoke local helper (contributions
sub-page that drew its own `<html>` document). All migrated.

| File | Sites | Notes |
|------|-------|-------|
| `admin/adr.rs` | 2 | render + render_detail |
| `admin/compliance.rs` | 2 | refresh ack + per-crate detail |
| `admin/iceberg/mod.rs` | 1 | summary page |
| `admin/keda/scalers.rs` | 2 | catalog + per-scaler detail |
| `admin/keda/scaled_jobs.rs` | 2 | list + detail |
| `admin/keda/trigger_authentications.rs` | 2 | list + detail |
| `admin/keda/scaled_objects.rs` | 5 | list + detail + new + edit + delete |
| `admin/keda/metrics.rs` | 1 | scaler-metrics page |
| `admin/contributions.rs` | 4 + helper | overview / worker detail / timeline / leaderboard. Local `page_shell(title, body, tenant)` helper deleted; replaced by a small `sub_nav` builder + `render_contributions_page(ctx, title, body)` wrapper that routes through `render::page_shell_full`. The chrome's top-bar persona pill now carries the tenant string in place of the bespoke `<span class="badge">` block (one test updated to assert the new markers — `?tenant_id=globex` query in sub-nav links + tenant string somewhere in the chrome). |
| `admin/mod.rs` | 4 | k8s-dashboard wrapper + cluster live + onboard + global search |

After the migration, the legacy `page_shell(title, body)` form is
called from **0** admin handlers. The function itself stays in
`render.rs` as a back-compat shim that delegates to `shell_v2` with
PlatformAdmin defaults — ~5 use-sites remain inside non-handler
paths (the `lakehouse.rs` / `mesh.rs` / `kubevirt.rs` / `karpenter.rs`
/ `rdbms_operator.rs` modules already use `page_shell_full` end-to-end
per the merge resolution).

## Item 4 — WCAG AA audit pass

New module: `crates/cave-portal/src/admin/layout/a11y.rs` (≈ 360 LOC).

Static analyser scans rendered HTML for the five most common
WCAG 2.1 AA failures that survive the existing `assert!(html.contains(...))`
fixture pattern:

| Code | Rule | Heuristic |
|------|------|-----------|
| A11y-001 | InteractiveWithoutName | `<button>` / `<a>` whose text content + `aria-label` + `title` are all empty |
| A11y-002 | InputWithoutLabel | `<input>` / `<textarea>` / `<select>` without enclosing `<label>`, `aria-label`, `aria-labelledby`, or `placeholder` (hidden + submit/reset/button-with-value excluded) |
| A11y-003 | ImageWithoutAlt | `<img>` without `alt=` |
| A11y-004 | DialogWithoutAria | `<div ... id="cave-XXX" ... hidden>` modal pattern without `role="dialog"` + `aria-modal="true"` + `aria-label=` |
| A11y-005 | NoFocusVisibleStyles | Document carries interactive elements but no `:focus-visible` rule (raw or via Tailwind `focus-visible:` utility) |

19 unit tests cover the rules + edge cases (positive and negative),
plus 2 lock-in tests that audit the **rendered chrome** for both
PlatformAdmin and TenantAdmin personas:

```
test admin::layout::a11y::tests::shell_v2_passes_full_a11y_audit ... ok
test admin::layout::a11y::tests::shell_v2_for_tenant_admin_also_passes_a11y_audit ... ok
```

**Before / After audit count on the chrome**: 0 violations both
ways. The chrome was already WCAG-AA clean per the 2026-05-13
foundation pass; the contribution here is a **machine-checkable
regression gate** so future edits can't silently degrade it.

## Item 5 — `/admin/_audit` consolidated dashboard

New module: `crates/cave-portal/src/admin/meta_audit.rs` (≈ 460 LOC).

Five-axis grade roll-up rendered as a card grid on a single page.
PlatformAdmin gate (`require_persona`) before any state read.

| # | Axis | Source | Reading |
|---|------|--------|---------|
| 1 | Structural | `compliance::cached_snapshot_or_refresh().aggregate_score()` | Portal/cavectl/Observability presence |
| 2 | Upstream Parity | `aggregate_parity_score()` | Average declared `parity_ratio` (manifest fill) |
| 3 | Honest Parity | `aggregate_honest_parity_score()` | Same, minus author-declared `[[partial]]` blocks |
| 4 | Behavioral Parity | `behavioral_parity_avg()` | Upstream tests ported / total declared |
| 5 | Accessibility | `a11y::audit(shell_v2(opts))` | 0 issues → 100; each issue costs 20 pts (clamped) |

Each card shows:

* Letter grade (A → F) in a colour-coded large-font slot
  (emerald → lime → amber → orange → red).
* 0–100 numeric score.
* In-memory **sparkline** (12-sample ring per axis, inline SVG,
  `viewBox="0 0 100 20"`, normalised polyline).
* One-line description.

Header carries the live `last_audit` timestamp + total crate count
+ workspace-wide stub count (`unimplemented!()` + `todo!()` +
ignored tests). Footer offers four actions:

* **Refresh now** → `/admin/compliance/refresh?tenant_id=…` (force
  re-walks the manifests + flushes the cache).
* **JSON feed** → `/admin/_audit.json?tenant_id=…` (consumed by
  `cavectl portal audit`).
* **Open Compliance →** / **Open Upstream →** for drill-in.

The dashboard pushes a sample to the per-axis history ring on
every render, so the sparkline densifies as the operator clicks
around (resets across restarts — production should swap a
persistent backend, scope-cut here).

JSON wire shape (`AuditSummary`):

```json
{
  "axes": [
    {"name": "structural",        "label": "Structural",        "score": 100, "grade": "A", "description": "..."},
    {"name": "upstream_parity",   "label": "Upstream Parity",   "score":  95, "grade": "A", "description": "..."},
    {"name": "honest_parity",     "label": "Honest Parity",     "score":  88, "grade": "B", "description": "..."},
    {"name": "behavioral_parity", "label": "Behavioral Parity", "score":  90, "grade": "A", "description": "..."},
    {"name": "accessibility",     "label": "Accessibility",     "score": 100, "grade": "A", "description": "..."}
  ],
  "last_audit": "2026-05-15T19:42:00Z",
  "total_crates": 117,
  "total_stubs": 0
}
```

16 unit tests + 3 mount-smoke tests in `admin/router_tests`:

* `meta_audit_route_renders_five_axis_dashboard` (PlatformAdmin
  → 200 + all 5 axis labels in body + action links).
* `meta_audit_route_returns_403_for_tenant_admin` (forged
  `tenant_admin` JWT → 403).
* `meta_audit_json_route_returns_axes_payload` (parses real JSON,
  asserts 5-axis array + canonical names).

## 4-track — `cavectl portal audit`

```
$ cavectl portal audit
$ cavectl portal audit --tenant acme         # explicit
$ CAVE_TENANT=acme cavectl portal audit      # env override
```

Calls `/admin/_audit.json?tenant_id=<tenant>` and renders through
the existing `--format table|json|yaml` pipeline.

Three parse tests cover the new variant + ensure `portal status`
keeps parsing.

**Pre-existing breakage fixed en passant**: cavectl `lib.rs` did
not declare `pub mod client;`, so any `cargo test -p cavectl` run
hit `error[E0433]: cannot find client in crate root` from
`native::auth.rs:477`. The polish sweep promotes the bin's local
`mod client` into the lib (single source of truth for `Format` +
`ApiClient`) and the bin re-imports `cavectl::client::*`. Net
diff: `+pub mod client;` in lib + `-mod client;` / `-pub enum
Format { … }` in main + `+use cavectl::client::{ApiClient, Format};`.
**No behaviour change.**

## Verification

### `cargo check --workspace --tests`

Clean (warnings only — pre-existing).

### Unit + mount-smoke tests

```
cave-portal:    1925 passed; 0 failed; 1 ignored (was 1866)   +59
cavectl bin:      75 passed; 0 failed                          +3
```

Touched-file test breakdown:

| Module | Before | After |
|--------|--------|-------|
| `admin::permission::persona_can_access_tests` | 0 | 3 |
| `admin::layout::command_palette::tests` | 6 | 12 |
| `admin::layout::shortcuts::tests` | 7 | 15 |
| `admin::layout::shell::tests` | 11 | 13 |
| `admin::layout::a11y::tests` | 0 | 21 |
| `admin::meta_audit::tests` | 0 | 16 |
| `admin::router_tests::meta_audit_*` | 0 | 3 |

### WCAG before / after

`a11y::audit(shell_v2(opts))` returns 0 issues for both
PlatformAdmin and TenantAdmin. The lock-in test
`shell_v2_passes_full_a11y_audit` will fail any future edit that
ships an aria-less modal, an icon-only button without an aria-label,
an unlabelled input, or removes the chrome's `focus-visible`
styles. **Net violation count remained at 0** — the contribution
here is the regression gate, not a fix to live regressions.

### `/admin/_audit` curl smoke

End-to-end via `axum::Router::oneshot` with a forged
`platform_admin` JWT extension:

```rust
let app = router(Arc::new(AdminState::seeded()));
let claims = JwtClaims { roles: vec!["platform_admin".into()], ... };
let req = Request::builder().uri("/admin/_audit?tenant_id=acme").body(Body::empty()).unwrap();
req.extensions_mut().insert(claims);
let resp = app.oneshot(req).await.unwrap();
assert_eq!(resp.status(), 200);
let body = body_text(resp).await;
// Body contains all 5 axis cards + refresh action + JSON feed link.
```

JSON feed:

```rust
let req = Request::builder().uri("/admin/_audit.json?tenant_id=acme")...;
let v: serde_json::Value = serde_json::from_str(&body)?;
assert_eq!(v["axes"].as_array().unwrap().len(), 5);
```

Tenant-admin gate:

```rust
let claims = JwtClaims { roles: vec!["tenant_admin".into()], ... };
// → 403 FORBIDDEN
```

All three smoke tests **pass**.

## Out of scope / honest deferrals

* **Persistent sparkline history** — current ring is in-memory and
  resets across restarts. Production wiring would land alongside
  the `RuntimeClient`/`AdminState` persistence work; documented in
  the module-level doc.
* **CSS contrast ratio computation** — A11y-005 currently checks
  for the *presence* of `:focus-visible` rules but doesn't compute
  WCAG contrast ratios on Tailwind colour utilities. The sweep
  delivered the regression gate; per-utility colour-pair scoring
  is a future tightening.
* **Persona injection from real JWT** — the dev `?tenant_id=…`
  shortcut still surfaces as `Persona::Anonymous` (correct behaviour;
  matches the existing JWT-bypass pattern in
  `extract_ctx_from_query_with_claims`).
* **`/admin/_audit` historical sparkline backed by manifest history**
  — would require a side table of dated snapshots; deferred. The
  in-memory ring is enough to answer "are we trending up or down?"
  during a single operator session.

## Branch / commit pointers

* Merge-from-main: `9355bca8` (resolved 26 conflicts; legacy
  single-file admin modules `cache.rs` / `cloud_controller_manager.rs`
  / `controller_manager.rs` / `keda.rs` / `kubelet.rs` / `net.rs`
  / `scheduler.rs` deleted in favour of folder-based versions
  already on main).
* Polish commits to follow on `main` after this audit doc.
