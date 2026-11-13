# cave-permission — Coverage Audit vs casbin/casbin

| Field | Value |
|-------|-------|
| Cave crate | `cave-permission` (`crates/ops/cave-permission`) |
| Upstream | https://github.com/casbin/casbin |
| Upstream tag | `v3.10.0` |
| Upstream commit SHA | `0fe9505818b12d66739b8e86887539b3ce57942a` |
| Upstream license | Apache-2.0 |
| Port policy | line-port (Apache-2.0 is line-port compatible with AGPL-3.0-or-later) |
| Audit date | 2026-05-29 |

## CRITICAL FINDING — Wrong upstream / architectural mismatch

The cave-permission crate is **not** a port of casbin. Its own source headers
(`src/lib.rs`, `src/models.rs`, `src/policy.rs`, `src/catalog.rs`, `src/routes.rs`)
declare it a port of **Backstage's permission framework**
(`@backstage/permission-common`, `@backstage/permission-node`,
`@backstage/catalog-backend`, `permission-backend/router.ts`).

casbin is a Go ABAC/RBAC/RESTful authorization **engine** built around a
PERM (Policy, Effect, Request, Matchers) model: a configurable model file, a
matcher-expression evaluator, a role manager with role-link graphs, pluggable
storage adapters, watchers, effectors, and an enforcer that evaluates requests
against policies using `govaluate`-style expression matching with built-in
operators (`keyMatch`, `regexMatch`, `ipMatch`, `globMatch`, etc.).

The cave crate is a 591-LOC HTTP shim: ~9 serde structs, a single
`AllowAllPermissionPolicy` that unconditionally returns `Allow`, a constant
table of Backstage catalog permission names, and two axum routes. There is no
model parser, no matcher evaluator, no policy storage, no role graph, no
effector. Essentially **none of casbin's functional surface is implemented**.

Given the upstream named in the audit task is casbin, the matrix below grades
the cave crate against casbin's modules. The result is near-total MISSING with
a couple of nominal PARTIALs where a same-named concept exists but with no
shared semantics.

## Coverage matrix

| Upstream module | Capability | Cave module | Status | Notes |
|-----------------|-----------|-------------|--------|-------|
| `enforcer.go` | Core Enforcer: init from model+adapter, `Enforce()` request evaluation against matcher | — | MISSING | No enforcer type, no `Enforce()`. Cave only has a stateless `AllowAllPermissionPolicy::handle` returning `Allow`. |
| `enforcer.go` (EnforceEx / EnforceWithMatcher) | Enforce with explicit explanations / custom matcher | — | MISSING | No matcher concept exists. |
| `enforcer_cached.go` | Cached enforcer (decision cache w/ key) | — | MISSING | No caching layer. |
| `enforcer_synced.go` | Thread-safe synced enforcer (RWMutex) | — | MISSING | No synced wrapper; cave policy is trivially `Send+Sync` but stateless. |
| `enforcer_distributed.go` | Distributed enforcer (dispatcher-backed) | — | MISSING | No distributed/dispatcher support. |
| `enforcer_transactional.go` / `transaction*.go` | Transactional policy mutation (buffer/commit/conflict) | — | MISSING | No transaction support. |
| `enforcer_context.go` | Context-aware enforce (ctx cancellation) | — | PARTIAL | Cave `handle` is `async` so it could carry context, but no enforce logic exists to contextualize. Nominal only. |
| `model/model.go` | PERM model: load `[request_definition]`, `[policy_definition]`, `[role_definition]`, `[policy_effect]`, `[matchers]` | — | MISSING | No model file parsing. Cave has no model abstraction. |
| `model/assertion.go` | Assertion (tokens, policy rows, role links per ptype) | — | MISSING | No assertion type. |
| `model/policy.go` | In-memory policy store: AddPolicy/RemovePolicy/GetFilteredPolicy/HasPolicy | `src/policy.rs` | PARTIAL | Same filename only. cave `policy.rs` defines a `PermissionPolicy` trait + AllowAll impl, not a policy ruleset store. No add/remove/filter. |
| `model/function.go` | Function map: register matcher functions into evaluator | — | MISSING | No expression-function registry. |
| `model/constraint.go` | Priority/constraint handling for policies | — | MISSING | No priority model. |
| `config/config.go` | INI-style model config parser (`[section]` key=value, line continuations) | — | MISSING | No config parser. |
| `rbac/role_manager.go` | RoleManager interface: AddLink/DeleteLink/HasLink/GetRoles/GetUsers | — | MISSING | No role manager interface. |
| `rbac/default-role-manager/role_manager.go` | Default role-link graph w/ hierarchy, domains, matching funcs, max hierarchy depth | — | MISSING | No role graph; no transitive role resolution. |
| `rbac/context_role_manager.go` | Context-aware role manager | — | MISSING | — |
| `rbac_api.go` | RBAC API: GetRolesForUser, AddRoleForUser, GetPermissionsForUser, GetImplicit* | — | MISSING | None of the 30+ RBAC API methods exist. cave has only a constant list of permission name strings. |
| `rbac_api_with_domains.go` | Domain/tenant-scoped RBAC API | — | MISSING | No domain concept. |
| `management_api.go` | Management API (65 funcs): AddPolicy, RemovePolicy, GetAllSubjects/Objects/Actions/Roles, UpdatePolicy, filtered queries | — | MISSING | No management surface; policies are not stored at all. |
| `effector/default_effector.go` | Effector: combine matched-rule effects per policy_effect expr (`some(where (p.eft==allow))`, deny-override, priority) | — | MISSING | No effector. Decision is hardcoded Allow. |
| `persist/adapter.go` | Adapter interface: LoadPolicy/SavePolicy | — | MISSING | No persistence adapter. |
| `persist/file-adapter` | File-backed policy storage (CSV) | — | MISSING | No file adapter. |
| `persist/string-adapter` | In-memory string adapter | — | MISSING | — |
| `persist/adapter_filtered.go` | Filtered policy loading | — | MISSING | — |
| `persist/batch_adapter.go` | Batch add/remove policy | — | MISSING | — |
| `persist/update_adapter.go` | In-place policy update adapter | — | MISSING | — |
| `persist/cache` | Policy cache adapter | — | MISSING | — |
| `persist/watcher.go` / `watcher_ex.go` / `watcher_update.go` | Watcher: notify other instances on policy change | — | MISSING | No watcher mechanism. |
| `persist/dispatcher.go` | Dispatcher for distributed mutations | — | MISSING | — |
| `util/builtin_operators.go` | Matcher built-ins: keyMatch/keyMatch2-5, regexMatch, ipMatch, globMatch, keyGet, etc. | — | MISSING | No matcher operators. This is the heart of casbin pattern matching. |
| `util/util.go` | Helpers: param splitting, array equals, escape assertion, has-eval | — | MISSING | No equivalent utilities. |
| `frontend.go` | CasbinJsGetPermissionForUser (frontend permission export) | `src/routes.rs` | PARTIAL | Cave exposes an HTTP `/api/permission/authorize` endpoint, but it is the Backstage batch-authorize shape returning ALLOW for everything, not casbin's frontend permission JSON. Different protocol/semantics. |
| `internal_api.go` | Internal add/remove with autosave + watcher notify | — | MISSING | — |
| `ai_api.go` | AI-assisted policy helpers | — | MISSING | — |
| `detector/default_detector.go` | Policy conflict/redundancy detector | — | MISSING | No detector. |
| `errors/rbac_errors.go` | Domain-specific error types | — | PARTIAL | cave returns no errors from `handle` (infallible Allow); no error taxonomy. |
| `log/` logger | Pluggable enforcement logger | — | MISSING | No enforcement logging. |
| ABAC support (`abac_test.go`, struct-field matchers `r.sub.Age`) | Attribute-based access via struct field access in matcher | `src/models.rs` (PermissionAttributes) | PARTIAL | cave has a `PermissionAttributes { action }` struct but it is never evaluated by any matcher — no ABAC engine. Nominal naming overlap only. |

### Summary counts (against casbin modules)
- Modules enumerated: 33
- COVERED: 0
- PARTIAL: 5 (all nominal — same name/concept, no shared behavior: enforcer_context, model/policy, frontend, errors, ABAC-attributes)
- MISSING: 28

## Actionable gaps for strict-TDD

These are ordered lowest-effort-highest-value first. Each is a casbin capability
with no real equivalent in cave. Note: closing these honestly means building a
casbin engine, since the current crate is a Backstage shim. If the intent is to
keep cave-permission as a Backstage port, the correct fix is to **re-map the
upstream** in the parity manifest rather than implement casbin.

1. **Built-in matcher operator `keyMatch`** — `util/builtin_operators.go` (`KeyMatch`, `KeyMatch2`).
   - Test: `key_match_wildcard_matches_path` — assert `key_match("/foo/bar", "/foo/*") == true` and `key_match("/foo/bar", "/baz/*") == false`. Lowest effort: pure string function, no engine state.

2. **`ipMatch` / `regexMatch` operators** — `util/builtin_operators.go`.
   - Test: `ip_match_cidr` — assert `ip_match("192.168.2.123", "192.168.2.0/24") == true` and a `/24` miss returns false.

3. **INI model config parser** — `config/config.go`.
   - Test: `config_parses_perm_sections` — feed a model string with `[request_definition]\nr = sub, obj, act`, assert parser exposes section `request_definition` key `r` == `sub, obj, act`.

4. **Policy store add/remove/has** — `model/policy.go`, `management_api.go`.
   - Test: `add_then_has_policy` — after `add_policy(["alice","data1","read"])`, assert `has_policy(...) == true` and a non-added rule returns false; `remove_policy` then makes `has_policy == false`.

5. **Enforcer basic ACL decision** — `enforcer.go` `Enforce()` + `effector/default_effector.go`.
   - Test: `enforce_acl_allow_and_deny` — with model `r=sub,obj,act` / `p=sub,obj,act` / matcher `r.sub==p.sub && r.obj==p.obj && r.act==p.act` and policy `(alice,data1,read)`, assert `enforce("alice","data1","read")==true` and `enforce("bob","data1","read")==false`. (Currently cave returns Allow for everything — this test would fail today.)

6. **RBAC role-link transitivity** — `rbac/default-role-manager/role_manager.go`, `rbac_api.go` `GetImplicitRolesForUser`.
   - Test: `rbac_transitive_role_grants_permission` — given `g(alice,admin)` and `p(admin,data1,read)`, assert `enforce("alice","data1","read")==true` (alice inherits admin's permission via role link). Exercises role graph + enforcer together.
