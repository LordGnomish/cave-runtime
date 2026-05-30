# TDD coverage audit — cave-permission (theme ops)

**Upstream:** https://github.com/casbin/casbin @ v3.10.0 (Apache-2.0)
**Cave crate:** `crates/ops/cave-permission`
**Date:** 2026-05-30

## Scope note

`cave-permission` is NOT a full casbin port. Its identity (see `src/lib.rs`) is a
**Backstage permission-backend** facade (models / policy trait / HTTP routes /
catalog constants) with a **narrow, in-memory casbin core** line-ported for the
authorizer it embeds:

- `matchers.rs` — `KeyMatch` / `KeyMatch2` / `KeyMatch3` / `RegexMatch` / `IPMatch`
- `rbac.rs` — default single-domain `RoleManagerImpl` (`AddLink`/`DeleteLink`/
  `HasLink`/`GetRoles`/`GetUsers`/`GetImplicitRoles`)
- `enforcer.rs` — `management_api.go` policy/grouping store + a fixed-model
  `Enforce` / `BatchEnforce`

Everything else in casbin's 304 test symbols (adapters, watchers, model DSL,
ABAC/BLP/Biba/LBAC/ORBAC/PBAC/ReBAC models, transactions, cycle detector, AI
explain, frontend, config parser, priority/temporal/conditional role managers,
domains, filtered policy, logger, benchmarks) maps to **no cave source** and is
**scope-cut** (parallel-track / not-ported / vendor-DSL / infra). Those are not
gaps against this crate — they are out of its declared boundary.

## Upstream test inventory

`total_test_symbols = 304` across 42 Go test files (includes ~71 `Benchmark*`
which are perf, not behavioral, and are scope-cut wholesale).

## Cave test inventory

cave `#[test]` / `#[tokio::test]` functions = **53** total:
- Integration TDD: `matchers_tdd.rs` (5), `rbac_tdd.rs` (6), `enforcer_mgmt_tdd.rs` (4),
  `enforce_tdd.rs` (5)
- In-module: enforcer.rs (2), matchers.rs (3), rbac.rs (2), models.rs (5),
  policy.rs (3), catalog.rs (4), routes.rs (4)
- `qwen_drafted.rs` (21, mostly type/constant existence smoke)
- `proptest_smoke.rs` (4 generic invariants — not behavioral)

## Behavior-by-behavior table (in-scope casbin core only)

| behavior | upstream test | cave impl? | cave test? | gap type | suggested test |
|---|---|---|---|---|---|
| KeyMatch wildcard suffix | TestKeyMatch | `matchers::key_match` | yes (`matchers_tdd::key_match_wildcard_matches_path`) | covered | — |
| KeyMatch: key1 shorter than `*` prefix (the `else` branch, src/matchers.rs:31-33) | TestKeyMatch (`/foo`,`/foo/*` etc.) | `matchers::key_match` | NO | portable-coverage | assert `key_match("/foo", "/foo/*")` semantics + empty-key cases |
| KeyMatch2 named `:seg` single-segment | TestKeyMatch2 | `matchers::key_match2` | yes (`matchers_tdd::key_match2_named_param_matches_single_segment`) | covered | — |
| KeyMatch3 `{seg}` single-segment | TestKeyMatch3 | `matchers::key_match3` | yes (`matchers_tdd::key_match3_brace_param_matches_single_segment`) | covered | — |
| RegexMatch | TestRegexMatch | `matchers::regex_match` | yes (`matchers_tdd::regex_match_anchors_as_written`) | covered | — |
| RegexMatch malformed pattern → false (cave hardening; upstream panics) | (none — upstream panics) | `matchers::regex_match` | NO | portable-coverage | assert `regex_match("x", "[")` returns `false` (no panic) |
| IPMatch CIDR + exact, v4/v6 | TestIPMatch | `matchers::ip_match` | yes (`matchers_tdd::ip_match_cidr_and_exact`) | covered | — |
| IPMatch malformed input → false | TestIPMatch (panics upstream) | `matchers::ip_match` | yes (`...not-an-ip...` asserted) | covered | — |
| KeyGet / KeyGet2 / KeyGet3 | TestKeyGet* | NOT impl | n/a | scope-cut (not ported) | — |
| KeyMatch4 / KeyMatch5 | TestKeyMatch4/5 | NOT impl | n/a | scope-cut (not ported) | — |
| GlobMatch | TestGlobMatch | NOT impl | n/a | scope-cut (not ported) | — |
| RoleManager AddLink + HasLink direct | TestRole / role_manager_test | `rbac::add_link`,`has_link` | yes (`rbac_tdd::direct_link_is_inherited`) | covered | — |
| HasLink reflexive (name==name) | TestRole | `rbac::has_link` | yes (`rbac_tdd::direct_link_is_inherited` asserts `has_link("alice","alice")`) | covered | — |
| HasLink transitive within hierarchy | TestRole | `rbac::has_link` | yes (`rbac_tdd::transitive_link_is_inherited_within_hierarchy`) | covered | — |
| maxHierarchyLevel cap | TestMaxHierarchyLevel | `rbac::has_link` | yes (`rbac_tdd::max_hierarchy_level_is_respected`) | covered | — |
| DeleteLink removes inheritance | TestRole | `rbac::delete_link` | yes (`rbac_tdd::delete_link_removes_inheritance`) | covered | — |
| GetRoles / GetUsers directionality | TestRole | `rbac::get_roles`,`get_users` | yes (`rbac_tdd::get_roles_and_users_are_directional`) | covered | — |
| GetImplicitRoles transitive closure + dedup | TestRole / TestImplicitRoleAPI | `rbac::get_implicit_roles` | yes (`rbac_tdd::get_implicit_roles_is_transitive_closure` + in-module `implicit_closure_dedups`) | covered | — |
| GetImplicitRoles respects maxHierarchyLevel | TestMaxHierarchyLevel | `rbac::get_implicit_roles` | NO (level cap tested only for `has_link`) | portable-coverage | assert `get_implicit_roles` on a deep chain truncates at level cap |
| DomainRole / DomainPatternRole / matching-func RM | TestDomainRole etc. | NOT impl | n/a | scope-cut (parallel-track multi-domain RM) | — |
| Temporary/conditional roles | TestTemporaryRoles / TestConditional | NOT impl | n/a | scope-cut | — |
| Cycle detector | default_detector_test (16) | NOT impl | n/a | scope-cut | — |
| ConcurrentHasLink | TestConcurrentHasLink | NOT impl (no interior mutability/sync) | n/a | scope-cut (infra/sync) | — |
| AddPolicy idempotent + HasPolicy | TestModifyPolicyAPI / mgmt API | `enforcer::add_policy`,`has_policy` | yes (`enforcer_mgmt_tdd::add_policy_is_idempotent_and_queryable` + in-module) | covered | — |
| RemovePolicy | TestModifyPolicyAPI | `enforcer::remove_policy` | yes (`enforcer_mgmt_tdd::remove_policy_deletes_rule`) | covered | — |
| GetPolicy (sorted) | TestGetPolicyAPI | `enforcer::get_policy` | yes (`enforcer_mgmt_tdd::get_policy_returns_all_rules_sorted`) | covered | — |
| AddGroupingPolicy → role manager | TestModifyGroupingPolicyAPI | `enforcer::add_grouping_policy`,`has_grouping_policy` | yes (`enforcer_mgmt_tdd::grouping_policy_feeds_role_manager`) | covered | — |
| Enforce: direct allow | TestBasicModel / TestRBACModel | `enforcer::enforce` | yes (`enforce_tdd::direct_policy_allows`) | covered | — |
| Enforce: allow via role inheritance (g) | TestRBACModel | `enforcer::enforce` | yes (`enforce_tdd::role_inheritance_allows_via_g`) | covered | — |
| Enforce: deny on mismatch | TestRBACModelInMemoryIndeterminate | `enforcer::enforce` | yes (`enforce_tdd::mismatched_action_or_object_denies`) | covered | — |
| Enforce: empty policy denies | TestBasicModelNoPolicy | `enforcer::enforce` | yes (`enforce_tdd::empty_enforcer_denies_by_default`) | covered | — |
| Enforce: object keyMatch wildcard in policy | TestKeyMatchModelInMemory | `enforcer::enforce` (uses `key_match`) | NO (enforce tests use exact/`/data/secret`; no `p.obj` wildcard path exercised through enforce) | portable-coverage | add policy `(alice, /data/*, read)` and assert enforce matches `/data/x` but not `/other/x` |
| BatchEnforce | TestBatchEnforce | `enforcer::batch_enforce` | yes (`enforce_tdd::batch_enforce_matches_per_request_decisions`) | covered | — |
| Priority / SubjectPriority / explicit priority | TestPriorityModel etc. | NOT impl (fixed allow-only model, no eft/priority) | n/a | scope-cut | — |
| Deny effect (RBAC with deny) | TestRBACModelWithDeny | NOT impl (model has no `p.eft`) | n/a | scope-cut (model DSL) | — |
| AuthorizeResult ALLOW/DENY serde | (backstage, not casbin) | `models::AuthorizeResult` | yes (models.rs in-module) | covered | — |
| AllowAllPermissionPolicy.handle | (backstage) | `policy::AllowAllPermissionPolicy::handle` | yes (policy.rs in-module) | covered | — |
| POST /authorize, GET /health routes | (backstage) | `routes::create_router` | yes (routes.rs in-module ×4) | covered | — |

## Recommended TDD fills (portable-coverage first)

These are behaviors cave **already implements** but where a casbin-portable
branch is untested. Naming the exact public cave fn each test exercises:

1. **`cave_permission::matchers::key_match`** — short-key wildcard branch.
   New test asserting `key_match("/foo", "/foo/*")` and empty-string edges hit
   the `key1.len() <= i` else-branch (src/matchers.rs:31-33), mirroring casbin
   `TestKeyMatch`. *(highest value — currently an entire branch is unexercised.)*

2. **`cave_permission::enforcer::enforce`** — object-wildcard path through the
   real authorizer. New test: `add_policy("alice","/data/*","read")` then assert
   `enforce("alice","/data/x","read")==true` and `enforce("alice","/other/x","read")==false`.
   Closes the gap where `enforce`'s `key_match(obj,p_obj)` wildcard leg is only
   tested via the matcher in isolation, never end-to-end, mirroring casbin
   `TestKeyMatchModelInMemory`.

3. **`cave_permission::matchers::regex_match`** — malformed-pattern hardening.
   New test asserting `regex_match("x", "[")` returns `false` without panicking
   (cave-specific safety guarantee replacing casbin's panic; documents the
   contract `enforce` relies on).

4. **`cave_permission::rbac::get_implicit_roles`** — hierarchy-level cap.
   New test building a chain longer than `max_hierarchy_level` and asserting the
   returned closure truncates at the cap, mirroring casbin `TestMaxHierarchyLevel`
   for the implicit-roles path (only `has_link` currently exercises the cap).

All four target existing public fns; no source changes are required to make them
pass. Everything outside the four is either already covered or legitimately
scope-cut (out of this crate's declared casbin-core boundary).
