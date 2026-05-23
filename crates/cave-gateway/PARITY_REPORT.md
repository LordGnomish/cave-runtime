# cave-gateway — Charter v2 8-gate close-out

**Date:** 2026-05-23
**Branch:** `claude/cave-gateway-close-2026-05-23`
**Upstream pin:** Kong/kong `3.9.1` (`b724fc7154de3a9971e33490097d5ea2c1bae93b`) + gravitee-io/gravitee-api-management `4.11.7` (`33ac33b9f4e061e024eaff6efd457fa10bf998e8`) — both Apache-2.0
**Parity:** `fill_ratio = 0.9667` (58/60) · `honest_ratio = 0.7333` (44/60)

| # | Gate | Status | Evidence |
| - | --- | --- | --- |
| 1 | **Upstream pinned** (always-latest) | PASS | `parity.manifest.toml::[upstream].version = "3.9.1"` (Kong latest stable 2026-05) + `[[upstreams]].version = "4.11.7"` (Gravitee latest stable). Resolves 2026-05-02 version-audit STALE | HIGH. `assertion_1_kong_version_pinned`. |
| 2 | **source_sha pinned** | PASS | Kong `b724fc71…1bae93b` + Gravitee `33ac33b9…0bf998e8`. `assertion_2_source_sha_matches_versions`. |
| 3 | **fill_ratio ≥ 0.95** | PASS | `0.9667` = (42 mapped + 2 partial + 14 skipped) / 60. `assertion_3_fill_ratio_meets_floor`. |
| 4 | **parity_ratio_source = "manifest"** | PASS | `[parity].parity_ratio_source = "manifest"`. `assertion_4_parity_ratio_source_is_manifest`. |
| 5 | **last_audit = 2026-05-23** | PASS | `[parity].last_audit = "2026-05-23"`. `assertion_5_last_audit_is_today`. |
| 6 | **counts sum to total + ≥ 15 mapped** | PASS | 42 + 2 + 14 + 2 = 60 total; 42 mapped ≥ 15 floor. `assertion_6_counts_sum_to_total`. |
| 7 | **AGPL SPDX header coverage 100%** | PASS | All `.rs` files in `src/` + `tests/` carry `SPDX-License-Identifier: AGPL-3.0-or-later`. `assertion_7_agpl_spdx_header_coverage`. |
| 8 | **no stub macros in src/** | PASS | No `todo!()` / `unimplemented!()` / `panic!("stub")` / `panic!("todo")` in `src/**/*.rs`. `assertion_8_no_stub_macros_in_src`. |

Bonus gate 9 (Charter v2 surface integrity): `cave_gateway::{GatewayState, admin_router, proxy_router, gravitee_router}` + `plugins::PluginCtx` + `gravitee::apis::{ApiDef, Subscription}` all reachable. `assertion_9_gateway_surface_intact`.

## Subsystem counts

| Bucket | Count | Examples |
| --- | --- | --- |
| Mapped | 42 | kong-admin-api, kong-proxy-runloop, kong-routing-atc-matcher, kong-balancer-{round-robin,consistent-hash,least-connections}, kong-healthcheck-active-passive, kong-circuit-breaker, kong-tls-sni-acme, kong-config-loader, kong-db-store-inmem, kong-handler-pipeline, kong-lifecycle-mgr, kong-routes-runtime, 18 Kong plugins (jwt/oauth2/key-auth/basic-auth/hmac-auth/acl/cors/rate-limiting/ip-restriction/bot-detection/proxy-cache/request-transformer/response-transformer/request-size-limiting/request-termination/zipkin/prometheus/grpc-gateway/file-log), 12 Gravitee management surfaces (apis-crud / plans / app+sub / dev-portal / catalog / federation / governance / analytics / debug / api-designer / flows / monetization / marketplace / protocols) |
| Partial | 2 | kong-websocket-upgrade (per-message-deflate compression deferred), gravitee-extended-policy-dsl (12 of ~50 niche policies mapped via Kong-equivalent plugins) |
| Skipped | 14 | kong-postgres-store, kong-cassandra-store, kong-vault-plugin, kong-aws-lambda-plugin, kong-statsd-datadog-loggly-syslog, kong-pre-post-function-lua, kong-correlation-id-plugin, kong-session-cookie-plugin, kong-ldap-auth-advanced, kong-oauth2-introspection-plugin, kong-saml-plugin, kong-mtls-auth-plugin, gravitee-management-ui-spa, gravitee-access-management |
| Unmapped (honest gaps) | 2 | envoy-xds-control-plane (rejected per ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001), gravitee-cockpit-multicluster (Phase 2 cave-portal-api) |

## Test totals

| Suite | Pass | Fail | Skip |
| --- | ---: | ---: | ---: |
| Lib unit tests | 90 | 0 | 0 |
| `tests/parity_self_audit.rs` | 9 | 0 | 0 |
| **TOTAL** | **99** | **0** | **0** |

## Scope-cuts → Phase 2 owners

| Group | Phase 2 crate(s) | Items |
| --- | --- | --- |
| Persistent store | `cave-rdbms` | kong-postgres-store, kong-cassandra-store |
| Cloud serverless | `cave-cloud` | kong-aws-lambda-plugin |
| Legacy log sinks | `cave-gateway` (next deep port) | kong-statsd-datadog-loggly-syslog |
| Policy engine | `cave-hermes` | kong-pre-post-function-lua |
| Auth federation | `cave-auth`, `cave-pki` | kong-session-cookie-plugin, kong-ldap-auth-advanced, kong-oauth2-introspection-plugin, kong-saml-plugin, kong-mtls-auth-plugin |
| Portal frontend | `cave-portal-web` | gravitee-management-ui-spa |
| IAM suite | `cave-auth` | gravitee-access-management |
| Multicluster mgmt | `cave-portal-api` | gravitee-cockpit-multicluster |

## Workspace integration

- **`cave-auth`** owns identity federation — OAuth2 introspection, SAML, LDAP, session cookies that the Kong plugins traditionally handled now live behind cave-auth APIs. cave-gateway plugins call out to cave-auth for token validation.
- **`cave-vault`** owns secret material — the Kong vault plugin family is replaced by direct cave-vault calls for plugin config secrets.
- **`cave-portal-web`** owns the management SPA — Gravitee Console UI lives in cave-portal-web rather than as a bundled Java webui.
- **`cave-portal-api`** owns multi-environment + multi-cluster mgmt — the Gravitee Cockpit feature is deferred to cave-portal-api so the gateway crate stays single-environment.
- **`cave-hermes`** is the alternative for inline-Lua semantics (pre-function / post-function) — Hermes Rust → WASM lets policy code run safely.
- **`cave-rdbms`** owns the persistent admin-API store when the in-memory store is too volatile.
- **`cave-pki`** issues + verifies the client certs needed for kong-mtls-auth-plugin's role.

## ADR

- [ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001 — Kong + Gravitee into cave-gateway](../../docs/adr/ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001-kong-gravitee-into-cave-gateway.md)
