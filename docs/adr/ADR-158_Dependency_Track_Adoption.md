# ADR-158 — Dependency-Track Adoption (cave-dependency-track v0.1.0)

**Status:** Accepted
**Date:** 2026-05-23
**Owner:** Cave Runtime Security stream
**Charter version:** v2

## Context

cave-runtime needed a sovereign SBOM/SCA platform to:

- Parse CycloneDX 1.4/1.5/1.6 + SPDX 2.3 BOMs uploaded by build pipelines
  (`cave-deploy`, `cave-pipelines`).
- Aggregate vulnerability intelligence from NVD CVE 2.0, OSV.dev, GitHub
  Security Advisories, Snyk, Sonatype OSS Index, Risk Based Security VulnDB,
  with EPSS scoring joined into findings.
- Evaluate organisational policy (license / vulnerability / coordinates /
  component-age) and emit policy violations.
- Maintain a vulnerability audit trail (in-triage → exploitable / resolved /
  false-positive / not-affected) with suppression and comments.
- Emit CycloneDX 1.6 VEX and a Bill of Vulnerabilities export for downstream
  consumption.
- Dispatch notifications via Slack / Teams / Mattermost / Email / Webhook /
  Jira when new findings appear.
- Bridge findings to external trackers (DefectDojo, Fortify SSC, Kenna,
  ThreadFix).

The Dependency-Track Java project (Apache-2.0) is the de-facto reference
implementation of this surface.  Its 4.x branch defines stable wire formats
(REST API v1 + CycloneDX/SPDX/VEX schemas + notification publisher contract)
that we want to maintain bidirectional compatibility with.

## Decision

Deep-port Dependency-Track v4.14.2 (commit
`c4a156726472cd529cc9fa8ed12e825cc000327d`) into a new Rust crate
`cave-dependency-track`, mirroring the upstream package layout in
`org.dependencytrack.*`:

| Upstream package | Rust module |
| --- | --- |
| `model/`              | `src/models.rs` |
| `resources/v1/`       | `src/routes.rs` |
| `parser/cyclonedx/`   | `src/sbom/cyclonedx.rs` |
| `parser/spdx/`        | `src/sbom/spdx.rs` |
| `parser/nvd/api20/`   | `src/vuln_intel/nvd.rs` |
| `parser/osv/`         | `src/vuln_intel/osv.rs` |
| `parser/github/`      | `src/vuln_intel/ghsa.rs` |
| `parser/snyk/`        | `src/vuln_intel/snyk.rs` |
| `parser/ossindex/`    | `src/vuln_intel/ossindex.rs` |
| `parser/vulndb/`      | `src/vuln_intel/vulndb.rs` |
| `parser/epss/`        | `src/vuln_intel/epss.rs` |
| `policy/`             | `src/policy/{engine,license,vulnerability,coordinates,age}.rs` |
| `notification/publisher/` | `src/notifications/publishers.rs` |
| `integrations/{defectdojo,fortifyssc,kenna}/` | `src/integrations/` |

The port is licensed AGPL-3.0-or-later (per cave-runtime workspace policy)
with NOTICE entry crediting Apache-2.0 upstream.

## Scope (24 mapped, 1 partial, 15 skipped, 1 unmapped — `fill_ratio 0.9756`)

**Mapped:** project portfolio (CRUD + tags + hierarchy + classifier),
CycloneDX 1.4/1.5/1.6 ingest, SPDX 2.3 JSON + tag-value parsers, BOM
ingestion task, component identity + analysis cache, internal-component
identification, NVD/OSV/GHSA/Snyk/OSS-Index/VulnDB parsers, EPSS join,
policy engine + 4 evaluator families, inherited-risk score, audit state
machine + comments + suppression, CycloneDX VEX export, 6 notification
publishers, 4 outbound integrations.

**Partial:** Bill-of-Vulnerabilities (CycloneDX-VDR strict-schema validation
deferred — emit only).

**Skipped (Phase 2):** persistent ORM (JDO/DataNucleus → cave-db), Konfetti
Vue web UI (cave-portal-web Backstage parity), async BOM upload event bus
(cave-streams), LDAP/OIDC/Keycloak SSO (cave-auth), Lucene search index
(cave-search), TLS termination + rate limiting (cave-gateway), Trivy mirror
(cave-trivy deep port owns), legacy MSBuild/JBoss BOM adapters, Liquibase
migrations, telemetry submission, scheduled notification dispatch, project
cloning, runtime config-property constants, policy-violation cursor paging,
service-component model.

**Unmapped (honest gap):** per-policy-violation suppression chain (separate
ViolationAnalysis state machine).

## Alternatives considered

1. **Keep `cave-sbom`** (Dependency-Track 4.11.6 port, ~30 source files,
   fill_ratio absent).  Rejected — `cave-sbom` is a slimmer subset built
   for the SLSA attestation chain; the Charter v2 ≥ 0.95 close-out required
   a fresh deep-port against the 4.14.x line.  Both crates co-exist until
   `cave-sbom` consumers migrate.

2. **Use Dependency-Track via Docker sidecar** (run the upstream JVM as-is).
   Rejected — sovereign runtime principle requires a Rust implementation we
   own end-to-end; OSS / sovereign auditability would be impossible with an
   opaque Java sidecar.

3. **Port DefectDojo (`cave-vulns` did this)** as the SBOM/SCA host instead.
   Rejected — DefectDojo is a finding aggregator, not an SBOM ingestion
   platform.  cave-vulns + cave-dependency-track are complementary.

## Consequences

- **+** 244 PASS tests, 8/8 Charter v2 gates GREEN, `fill_ratio = 0.9756`,
  `honest_ratio = 0.6098`, REST API v1 + GraphQL + OpenAPI 3.0 swagger
  surface exposed at `/api/v1/*`.
- **+** Vuln intel federation: NVD, OSV, GHSA, Snyk, OSS Index, VulnDB,
  EPSS — six sources joinable into a single `VulnStore`.
- **+** Policy DSL covers license / license-group / vulnerability (severity
  + EPSS + CWE + vuln-id) / coordinates (PURL / CPE / hash / version) /
  component-age (ISO-8601 duration).
- **+** CycloneDX 1.6 VEX export keeps cave-runtime BOMs round-trip
  compatible with downstream consumers (Snyk, DefectDojo, GitHub Advanced
  Security).
- **−** Persistent storage deferred to Phase 2 — current `*Store` types are
  in-memory only.  Production use requires cave-db Postgres binding
  before the deptrack engine becomes durable.
- **−** Konfetti UI is *not* a 1:1 visual port; portal UX uses Backstage
  parity (cave-portal-web).  Operators familiar with upstream Konfetti
  will need to re-learn navigation.

## Phase 2 follow-ups

1. cave-db Postgres binding (replace `RwLock<HashMap<…>>` with persistent
   Storage).
2. cave-trivy scanner integration (consume Trivy CycloneDX outputs as BOM
   uploads).
3. cave-portal-web Backstage parity (project portfolio + audit UI).
4. cave-streams async BOM ingest (decouple upload from analysis).
5. cave-auth LDAP/OIDC federation (replace stub auth).

## Workspace integration

`cave-dependency-track` is wired into the workspace as a peer to
`cave-sbom`, `cave-vulns`, `cave-sign`, `cave-trivy`, `cave-artifacts`.
The runtime binary registers its router under `/api/sbom/v1/*` (alias of
`/api/v1/*`) so the upstream API contract is preserved on cave-runtime
deployments.

## References

- Upstream: <https://github.com/DependencyTrack/dependency-track>
- CycloneDX spec: <https://cyclonedx.org/docs/1.6/json/>
- SPDX 2.3 spec: <https://spdx.github.io/spdx-spec/v2.3/>
- EPSS: <https://www.first.org/epss/>
- VEX: <https://cyclonedx.org/capabilities/vex/>
