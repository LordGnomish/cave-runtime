# cave-dependency-track — Charter v2 8-gate close-out

**Date:** 2026-05-23
**Branch:** `claude/cave-deptrack-2026-05-23-deep`
**Upstream pin:** DependencyTrack/dependency-track `v4.14.2` (`c4a156726472cd529cc9fa8ed12e825cc000327d`) — Apache-2.0
**Parity:** `fill_ratio = 0.9756` (40/41) · `honest_ratio = 0.6098` (25/41)

| # | Gate | Status | Evidence |
| - | --- | --- | --- |
| 1 | **Upstream pinned** (always-latest) | PASS | `parity.manifest.toml::[upstream].version = "v4.14.2"` (Dependency-Track latest 2026-04-22). `assertion_1_upstream_version_pinned`. |
| 2 | **source_sha pinned** | PASS | `c4a156726472cd529cc9fa8ed12e825cc000327d`. `assertion_2_source_sha_matches_version`. |
| 3 | **fill_ratio ≥ 0.95** | PASS | `0.9756` = (24 mapped + 1 partial + 15 skipped) / 41. `assertion_3_fill_ratio_meets_floor`. |
| 4 | **parity_ratio_source = "manifest"** | PASS | `[parity].parity_ratio_source = "manifest"`. `assertion_4_parity_ratio_source_is_manifest`. |
| 5 | **last_audit = 2026-05-23** | PASS | `[parity].last_audit = "2026-05-23"`. `assertion_5_last_audit_is_today`. |
| 6 | **counts sum to total + ≥ 20 mapped** | PASS | 24 + 1 + 15 + 1 = 41 total; 24 mapped ≥ 20 floor. `assertion_6_counts_sum_to_total`. |
| 7 | **AGPL SPDX header coverage 100 %** | PASS | All 31 `.rs` files in `src/` + `tests/` carry `SPDX-License-Identifier: AGPL-3.0-or-later`. `assertion_7_agpl_spdx_header_coverage`. |
| 8 | **no stub macros in src/** | PASS | No `todo!()` / `unimplemented!()` / `panic!("stub")` / `panic!("todo")` in `src/**/*.rs`. `assertion_8_no_stub_macros_in_src`. |

Bonus gate 9 (Charter v2 surface integrity): full project + sbom + vuln_intel + policy + audit + vex + bov + notifications + integrations + repositories + cpe + purl + licenses + risk + engine + graphql surface reachable through `cave_dependency_track` crate-root re-exports. `assertion_9_deptrack_surface_intact`.

## Subsystem counts

| Bucket | Count | Examples |
| --- | --- | --- |
| Mapped | 24 | project-portfolio-crud, cyclonedx-1.4-1.5-1.6-parser, spdx-2.3-json-parser, spdx-tag-value-parser, bom-ingestion, component-identity, internal-component-identification, vuln-source-{nvd,osv,ghsa,snyk,ossindex,vulndb}, epss-join, policy-{license,vulnerability,coordinates,age}-evaluator, risk-score-inherited, audit-analysis-state-machine, vex-cyclonedx-1.6-export, notifications-publishers-six, integrations-four-uploaders |
| Partial | 1 | bov-bill-of-vulnerabilities (CycloneDX-VDR strict-schema validation deferred) |
| Skipped | 15 | persistent-jdo-datanucleus, konfetti-vue-web-ui, tasks-bom-upload-async, ldap-oidc-keycloak-sso, lucene-search-index, tls-termination-and-ratelimiting, trivy-mirror-task, ms-build-bom-jboss-fluent-bom, datanucleus-liquibase-migrations, telemetry-submission, scheduled-notification-dispatch, clone-project-task, config-property-constants, policy-violation-resource-paging, service-component-model |
| Unmapped (honest gaps) | 1 | policy-violation-grouped-suppression |

## Test totals

| Suite | Pass | Fail | Skip |
| --- | ---: | ---: | ---: |
| Lib unit tests | 230 | 0 | 0 |
| `tests/parity_self_audit.rs` | 9 | 0 | 0 |
| `tests/smoke.rs` | 5 | 0 | 0 |
| **TOTAL** | **244** | **0** | **0** |

## Scope-cuts → Phase 2 owners

| Group | Phase 2 crate(s) | Items |
| --- | --- | --- |
| Datastore + async ingest | `cave-db`, `cave-streams` | persistent-jdo-datanucleus, datanucleus-liquibase-migrations, tasks-bom-upload-async |
| Identity federation | `cave-auth` | ldap-oidc-keycloak-sso |
| Scanner mirrors | `cave-trivy` | trivy-mirror-task |
| UX, TLS, search | `cave-portal-web`, `cave-gateway`, `cave-search` | konfetti-vue-web-ui, tls-termination-and-ratelimiting, lucene-search-index |
| Runtime scheduling | `cave-runtime`, `cave-portal-api`, `cave-flags` | telemetry-submission, scheduled-notification-dispatch, clone-project-task, config-property-constants, policy-violation-resource-paging, service-component-model, ms-build-bom-jboss-fluent-bom |

## Smoke evidence

| Scenario | Test | Result |
| --- | --- | --- |
| Portfolio CRUD → CycloneDX 1.6 BOM upload → component listing | `smoke_1_portfolio_bom_upload_lists_components` | PASS |
| SPDX 2.3 JSON ingest w/ externalRefs PURL+CPE pickup | `smoke_2_spdx_2_3_json_ingest_externalrefs_pickup` | PASS |
| NVD CVE 2.0 → severity bucketing → EPSS join → inherited-risk score | `smoke_3_nvd_severity_epss_join_risk_score` | PASS |
| Policy engine (license + severity + age + coordinates combined) | `smoke_4_policy_engine_license_severity_age_coordinates` | PASS |
| Audit state machine → CycloneDX VEX export → BOV summary | `smoke_5_audit_state_machine_vex_export_bov_summary` | PASS |

## cavectl integration

`cavectl cave deptrack {project,sbom,vuln,policy,audit,export}` wired in `crates/cave-cli/src/main.rs` against the `/api/v1/{project,bom,vulnerability,policy,analysis,vex,bov,license}` routes.

## Workspace integration

- `cave-sign` DSSE envelopes feed `cave-dependency-track` SLSA-attested SBOM subjects (deptrack consumes the `subject_sha256` digest for component identity).
- `cave-sbom` (Dependency-Track 4.11.6 legacy port) co-exists for backward compatibility; cave-dependency-track is the going-forward target.
- `cave-vulns` (DefectDojo finding pipeline) federates vulnerability findings into `cave-dependency-track::vuln_intel::VulnStore` via the integrations layer.
- `cave-trivy` will become the upstream scanner whose `OsvAdvisory` + `cyclonedx` outputs land in deptrack as `BomUploadProcessingTask` inputs.
- `cave-artifacts` (Pulp + Harbor + Nexus) is the registry from which deptrack pulls `RepositoryMetaComponent` latest-version metadata.

## ADR

- [ADR-158 — Dependency-Track Adoption](../../docs/adr/ADR-158_Dependency_Track_Adoption.md)
