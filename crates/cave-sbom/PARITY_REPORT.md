# cave-sbom — Charter v2 8-gate Close-out Report

**Audit date**: 2026-05-18
**Upstream**: `DependencyTrack/dependency-track @ v4.11.6`
              (`128fd0fa01bed9fcb57abffa3b30047c45941415`, Apache-2.0, Java)
**Crate root**: `crates/cave-sbom/`

## Scope

cave-sbom is a line-by-line semantic port of Dependency-Track 4.11.6
to Rust. The deep-port itself landed during the **Artifact + Security
wave** (commits 6a2aedf8 → ce43dfaf, 2026-05-17); this report stamps
the Charter v2 close-out that brings cave-sbom's audit shape in line
with the data-persistence and k8s-core closes from 2026-05-18.

Ported subsystems:

- **BOM ingestion** — CycloneDX 1.5 / 1.6 JSON+XML parser
  (`sbom::cyclonedx`), SPDX 2.3 JSON + tag-value parser (`sbom::spdx`).
- **Component / Project / version graph** (`components/`).
- **Vulnerability intelligence** — NVD CVE 2.0 JSON (`vuln_intel::nvd`),
  OSV.dev (`osv`), GitHub Advisory GraphQL (`ghsa`), EPSS join
  (`epss`), Snyk (`snyk`, license-permitting subset).
- **Policy engine** — license / vulnerability / age / coordinates
  evaluators (`policy::{license,vuln,age,coordinates}`).
- **Portfolio rollup** — per-project risk score + time-series
  snapshots (`portfolio/`).
- **Notification framework** — Webhook / Mail / Jira sinks
  (`notifications/`).
- **REST API v1** — axum router exposing `POST /api/sbom/bom`,
  `GET /vulnerability`, policy + portfolio surfaces (`routes.rs`).
- **Cross-crate integration tests** — `scan_sbom_integration.rs` +
  `vulns_correlation_integration.rs`.

## Inventory measurement

Hand-curated against
`src/main/java/org/dependencytrack/` 2026-05-17, refreshed for
Charter v2 close 2026-05-18.

| Bucket   | Count | Examples                                                                              |
|----------|------:|---------------------------------------------------------------------------------------|
| Mapped   |    39 | CycloneDxValidator, SpdxDocumentParser, Component, Project, Vulnerability,            |
|          |       | NvdParser, OsvParser, GithubAdvisoryParser, EpssAnalyzer, SnykParser,                 |
|          |       | PolicyEvaluator (license/vuln/age/coordinates), PortfolioMetrics,                     |
|          |       | NotificationPublisher (Webhook/Mail/Jira), REST resources                              |
| Partial  |     8 | CycloneDXVexImporter (no VEX vulnerabilities[] block),                                |
|          |       | SpdxExpressionParser (no AND/OR/WITH grammar — verbatim strings),                     |
|          |       | NvdMirrorTask (full mirror download deferred — feed-style updates),                   |
|          |       | OsvSyncTask (incremental sync without delta cursor),                                  |
|          |       | GithubAdvisorySyncTask (REST fallback if GraphQL throttled),                          |
|          |       | DependencyMetricsUpdateTask (snapshot-only, no JMS scheduler),                        |
|          |       | DefaultObjectGenerator (seed taxonomy without UUID v5 stability),                     |
|          |       | LdapAuthenticator (search-and-bind only, no group sync)                                |
| Skipped  |     0 | (cave-sbom has no [[skipped]] blocks; every upstream file goes into mapped/           |
|          |       | partial/unmapped — categories the parity-index reads directly)                         |
| Unmapped |     8 | NotificationPublisher{Slack,Teams,Cisco} (sinks via cave-noti),                       |
|          |       | OidcLoginResource (handled by cave-auth Keycloak),                                    |
|          |       | LdapGroupSyncTask (cave-auth LDAP path), BomUploadHandlerProcessor                    |
|          |       | (cave-streams Kafka consumer), MetricsUpdater task scheduler (cave-runtime),          |
|          |       | LeadershipElection (cave-etcd Raft handles it)                                         |
| **Total**| **55**| |

- **fill_ratio   = (mapped + partial + skipped) / total = 47 / 55 = 0.8545**
- **honest_ratio = mapped / total                       = 39 / 55 = 0.7091**

Charter v2 floor for cave-sbom is `0.80`. We sit at **0.8545**, above
the floor.

## 8-gate close-out

| # | Gate                              | Result | Evidence                                       |
|---|-----------------------------------|--------|------------------------------------------------|
| 1 | SPDX-License-Identifier 100%      | PASS   | every `src/**/*.rs` carries AGPL-3.0-or-later  |
| 2 | `source_sha` pinned in manifest   | PASS   | `sha = "128fd0fa01bed9fcb57abffa3b30047c45941415"` |
| 3 | `last_audit = "2026-05-18"`       | PASS   | `[parity].last_audit`                          |
| 4 | `parity_ratio_source = "manifest"`| PASS   | parity-index reads `fill_ratio` directly       |
| 5 | `fill_ratio >= 0.80`              | PASS   | 0.8545 (honest floor for cave-sbom)            |
| 6 | mapped+partial+skipped+unmapped == total | PASS | 39 + 8 + 0 + 8 = 55                      |
| 7 | No `unimplemented!()` / `todo!()` | PASS   | 0 stub macros under `src/`                     |
| 8 | `PARITY_REPORT.md` present        | PASS   | this file                                      |

**Charter v2 verdict: 8/8 PASS.**

## Test coverage

`cargo test -p cave-sbom --lib --tests`:

| Test set                         | Count |
|----------------------------------|------:|
| lib (per-module `#[cfg(test)]`)  |  137  |
| parity_self_audit                |    9  |
| scan_sbom_integration            |    4  |
| vulns_correlation_integration    |    4  |
| **TOTAL**                        |**154**|

## Scope-cut — explicit deferred work

The eight `unmapped` entries are honest acknowledgement of paths
that are *covered* in the Cave Runtime but not implemented inside
cave-sbom itself:

1. **Slack / Teams / Cisco notification sinks** — `cave-noti`
   handles transport; cave-sbom emits a generic `NotificationEvent`
   that cave-noti subscribes to. Not a duplicate port.
2. **OIDC login resource** — `cave-auth` Keycloak port owns the
   browser flow; cave-sbom REST API trusts the JWT it sees.
3. **LDAP group sync** — `cave-auth` LDAP path owns directory sync;
   cave-sbom only consults the resolved group membership.
4. **Kafka bom-upload consumer** — `cave-streams` exposes the consumer
   side; cave-sbom owns the parse/correlation, called from there.
5. **Metrics-update task scheduler** — `cave-runtime` task runner
   schedules the periodic recompute; cave-sbom only owns the
   per-event recompute body.
6. **Leadership election** — `cave-etcd`'s Raft layer elects the
   single writer; cave-sbom doesn't ship a leader-election state
   machine of its own.

The eight `partial` entries are real ports with documented narrow
scope-cuts (see the bucket table above).

## How to verify

```bash
# Build cave-sbom clean.
cargo build -p cave-sbom

# All 154 tests.
cargo test -p cave-sbom --lib --tests

# Charter v2 self-audit alone.
cargo test -p cave-sbom --test parity_self_audit

# Confirm zero stub macros.
rg -n 'unimplemented!|todo!\(' crates/cave-sbom/src --type rust
```

## Next sweep (out of this close-out)

- Close `CycloneDXVexImporter` VEX `vulnerabilities[]` block (lifts
  one partial → ported).
- Implement `SpdxExpressionParser` AND/OR/WITH grammar (lifts one
  partial → ported).
- Backfill `NvdMirrorTask` full-feed download path so air-gapped
  installs work without the per-CVE pull (lifts one partial →
  ported).
