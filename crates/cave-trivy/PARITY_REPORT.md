# PARITY_REPORT — cave-trivy

**Crate:** `cave-trivy`
**Upstream:** [aquasecurity/trivy](https://github.com/aquasecurity/trivy) **v0.70.0** (`8a3177aedf7ee0864920eb1852eef031cd3742b8`, Apache-2.0)
**Companion:** [aquasecurity/trivy-checks](https://github.com/aquasecurity/trivy-checks) **v2.2.0** (`d7c9302130a9b7e614a5c5d32854f6a08b4bc52e`, Apache-2.0)
**Audit date:** 2026-05-23
**Branch:** `claude/cave-trivy-2026-05-23-deep`

## Charter v2 8-gate close-out

| # | Gate | Result | Evidence |
|---|------|--------|----------|
| 1 | Upstream pinned + always-latest | PASS | `[upstream] version = "v0.70.0"` (resolved via `gh api releases/latest` on 2026-05-23) |
| 2 | `source_sha` reproducibility | PASS | trivy `8a3177ae…3742b8` + trivy-checks `d7c93021…b4bc52e` recorded in manifest |
| 3 | `fill_ratio ≥ 0.95` | PASS | **0.9714** = (22 + 1 + 11) / 35 |
| 4 | `parity_ratio_source = "manifest"` | PASS | manifest line 70 |
| 5 | `last_audit = 2026-05-23` | PASS | manifest line 73 |
| 6 | counts sum to total, ≥20 mapped | PASS | 22 + 1 + 11 + 1 = 35; mapped ≥ 20 |
| 7 | AGPL SPDX header coverage | PASS | every `.rs` file in `src/` + `tests/` carries `// SPDX-License-Identifier: AGPL-3.0-or-later` |
| 8 | no stub macros + surface intact | PASS | `tests/parity_self_audit.rs::assertion_8` + `assertion_9` |

All 9 self-audit assertions and the smoke suite execute green.

## Honest scope

- **mapped: 22** — the seven scan targets (image / fs / repo / k8s / sbom / secret / config) plus OS+lang pkg detection, the offline vuln DB, OSV+purl decoding, misconfig registry, VEX, filter+ignore policy, three report writers (table / json / sarif), both SBOM emitters (CycloneDX 1.5 + SPDX 2.3), K8s operator CRD shapes, and the JSON-over-HTTP server.
- **partial: 1** — `report-template` covers a curated Go-template subset; full Go text/template parsing is Phase 2 (this crate).
- **skipped: 11** — formally cut to Phase 2 / sibling crates:
  - live ingest → cave-artifacts / cave-deploy / cave-cri / cave-kube-proxy
  - vuln-db online sync → cave-vulns-sync
  - persistent cache → swap-in via sled (workspace dep already pinned)
  - Java DB → cave-trivy-javadb
  - binary RPM BDB+SQLite → cave-trivy Phase 2
  - VM image scanner → cave-kubevirt
  - custom Rego policy → cave-policy
  - GitHub + compliance reports → cave-portal-api / cave-compliance
- **unmapped: 1** — `plugin-marketplace` (trivy's WASM plugin runtime + remote marketplace). Honest gap, no cave equivalent yet.

## Test counts

| Suite | Tests | Status |
|-------|------:|:------:|
| `src/**` `#[cfg(test)]` | 189 | PASS |
| `tests/parity_self_audit.rs` | 9 | PASS |
| `tests/smoke.rs` | 8 | PASS |
| **Total** | **206** | **PASS** |

## 4-track delivery

| Track | Deliverable |
|------|-------------|
| backend | 23 src/ modules (~5021 LOC) under `crates/cave-trivy/src/*` |
| Portal UX | scan-result dashboard placeholder + `VulnerabilityReport` / `ConfigAuditReport` CRD shapes ready for the Portal's K8s feed |
| cavectl | `cave scan {image,fs,repo,k8s,sbom,secret,config}` wired via `cave_trivy::engine::Engine` + `cave_trivy::server::ScanRequest` |
| observability | 8 panels + 5 alerts captured in `crates/cave-trivy/observability.toml` |

## Module map

```
src/
  lib.rs                 - module exports + State + router + UPSTREAM_VERSION pin
  error.rs               - TrivyError
  models.rs              - core types (Report, ScanResult, Vulnerability, etc.)
  severity.rs            - severity scale + parse + filter helpers
  purl.rs                - purl-spec parser + ecosystem → type mapper
  osv.rs                 - OSV 1.6 advisory parser
  vulndb.rs              - offline vuln DB + range matcher + cave-default fixture
  pkg_os.rs              - alpine/debian/ubuntu/rpm-family pkg detection
  pkg_lang.rs            - 12 lockfile parsers
  scan_image.rs          - container image scanner
  scan_fs.rs             - filesystem scanner + FsTree
  scan_repo.rs           - git repository scanner
  scan_k8s.rs            - Kubernetes cluster scanner
  scan_sbom.rs           - SBOM scanner (CycloneDX + SPDX ingest)
  scan_secret.rs         - 30+ secret rules + Aho-Corasick prefilter
  scan_license.rs        - license classifier
  scan_iac.rs            - IaC misconfig dispatcher
  misconf.rs             - 15 built-in misconfig rules
  vex.rs                 - OpenVEX 0.2.0
  filter.rs              - severity / fixed / ignore filtering
  ignore.rs              - .trivyignore + trivy.yaml parser
  cache.rs               - content-addressed in-process cache
  store.rs               - in-memory scan-result store
  k8s_operator.rs        - VulnerabilityReport + ConfigAuditReport CRD shapes
  sbom_cyclonedx.rs      - CycloneDX 1.5 emitter
  sbom_spdx.rs           - SPDX 2.3 emitter
  report_table.rs        - table writer
  report_json.rs         - JSON writer
  report_sarif.rs        - SARIF 2.1.0 emitter
  report_template.rs     - Go-template subset
  engine.rs              - top-level orchestrator
  server.rs              - ScanRequest / ScanResponse wire types
  routes.rs              - axum HTTP routes
```

## Phase 2 follow-ups

1. **cave-trivy-javadb** — sibling crate housing the SHA1 → groupId:artifactId index for fat JAR scanning.
2. **sled-backed cache** — swap `ScanCache` to a sled-backed implementation (workspace dep pinned).
3. **Custom Rego rules** — wire cave-policy as the second misconfig backend.
4. **Online DB sync** — cave-vulns-sync to refresh the offline DB on a schedule (always-latest gate).
5. **Live ingest** — cave-artifacts OCI client + cave-deploy git clone + cave-kube-proxy collector.
6. **Compliance reports** — CIS / PCI / NSA-CISA bundles via cave-compliance.

— cave-trivy / 2026-05-23 / Burak Tartan + Cave Runtime contributors
