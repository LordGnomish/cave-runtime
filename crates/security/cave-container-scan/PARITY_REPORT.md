# cave-container-scan — Charter v2 8-gate close-out

**Date:** 2026-05-23
**Branch:** `claude/cave-container-scan-close-2026-05-23`
**Upstream pin:** aquasecurity/trivy `v0.70.0` (`8a3177aedf7ee0864920eb1852eef031cd3742b8`) — Apache-2.0
**Parity:** `fill_ratio = 0.9615` (50/52) · `honest_ratio = 0.7115` (37/52)

| # | Gate | Status | Evidence |
| - | --- | --- | --- |
| 1 | **Upstream pinned** (always-latest) | PASS | `parity.manifest.toml::[upstream].version = "v0.70.0"` (Trivy latest stable 2026-05; resolves the floating `main` Charter v2 violation flagged in the 2026-05-02 version-audit). `assertion_1_trivy_version_pinned`. |
| 2 | **source_sha pinned** | PASS | Trivy `8a3177ae…d3742b8`. `assertion_2_source_sha_matches_version`. |
| 3 | **fill_ratio ≥ 0.95** | PASS | `0.9615` = (35 mapped + 2 partial + 13 skipped) / 52. `assertion_3_fill_ratio_meets_floor`. |
| 4 | **parity_ratio_source = "manifest"** | PASS | `[parity].parity_ratio_source = "manifest"`. `assertion_4_parity_ratio_source_is_manifest`. |
| 5 | **last_audit = 2026-05-23** | PASS | `[parity].last_audit = "2026-05-23"`. `assertion_5_last_audit_is_today`. |
| 6 | **counts sum to total + ≥ 15 mapped** | PASS | 35 + 2 + 13 + 2 = 52 total; 35 mapped ≥ 15 floor. `assertion_6_counts_sum_to_total`. |
| 7 | **AGPL SPDX header coverage 100%** | PASS | All `.rs` files in `src/` + `tests/` carry `SPDX-License-Identifier: AGPL-3.0-or-later`. `assertion_7_agpl_spdx_header_coverage`. |
| 8 | **no stub macros in src/** | PASS | No `todo!()` / `unimplemented!()` / `panic!("stub")` / `panic!("todo")` in `src/**/*.rs`. `assertion_8_no_stub_macros_in_src`. |

Bonus gate 9 (Charter v2 surface integrity): `cave_container_scan::{router, new_state, ScanOrchestrator, ScanError, engine::{dedupe_findings,aggregate_verdict}, policy::evaluate_policy, models::{Finding,Severity,ScanVerdict}}` all reachable. `assertion_9_scanner_surface_intact`.

## Subsystem counts

| Bucket | Count | Examples |
| --- | --- | --- |
| Mapped | 35 | scan-orchestrator-run, dedupe-findings, aggregate-verdict, scan-policy-evaluator, scanner-trait, image-scanner (+log4shell +root-user), fs-scanner (+requirements.txt +go.mod +clean-baseline), iac-scanner (+Dockerfile latest-tag +missing-USER +K8s privileged +Terraform S3 public), secret-scanner (+AWS keys +GitHub tokens +PEM private keys +Shannon entropy), yara-scanner (+clean-payload negative), namespace-confusion-scanner (+Levenshtein +legit passthrough), severity-ordering, ecosystem-tagging, finding-model, scan-kind-enum, iac-kind-enum, scan-stats, verdict-decision, scan-result-record, http-router-axum, http-handler-scan-dispatch, container-scan-store |
| Partial | 2 | cyclonedx-sbom-output (basic component listing; cross-vuln + lifecycle deferred to cave-sbom), sarif-output (minimal results emit; tool-driver-rules deferred) |
| Skipped | 13 | trivy-cli-binary, trivy-server-mode, trivy-db-bolt-persistence, trivy-cve-cwe-mitre-correlation, trivy-vex-evaluation, trivy-sbom-generation, trivy-license-detection, trivy-aws-account-scan, trivy-k8s-cluster-scan, trivy-vm-image-scan, trivy-rego-config-misconfig, trivy-secret-rule-config-toml, trivy-plugin-system |
| Unmapped (honest gaps) | 2 | image-layer-tarball-extract (manifest+InstalledPackage only today; cave-artifacts shares the unpacked-layer path in Phase 2), cve-cwe-mitre-enrichment-inline (Findings emit IDs without inline correlation; Phase 2 cave-vulns enrichment hop) |

## Test totals

| Suite | Pass | Fail | Skip |
| --- | ---: | ---: | ---: |
| Lib unit tests | 34 | 0 | 0 |
| `tests/parity_self_audit.rs` | 9 | 0 | 0 |
| **TOTAL** | **43** | **0** | **0** |

## Scope-cuts → Phase 2 owners

| Group | Phase 2 crate(s) | Items |
| --- | --- | --- |
| CLI + server | `cave-cli` | trivy-cli-binary, trivy-server-mode |
| Vuln DB | `cave-scan-db`, `cave-vulns` | trivy-db-bolt-persistence, trivy-cve-cwe-mitre-correlation, cve-cwe-mitre-enrichment-inline |
| VEX | `cave-sign` | trivy-vex-evaluation |
| SBOM + license | `cave-sbom` | trivy-sbom-generation, trivy-license-detection |
| Cloud + cluster | `cave-cloud`, `cave-admission`, `cave-policy` | trivy-aws-account-scan, trivy-k8s-cluster-scan, trivy-vm-image-scan |
| Rego + secrets config | `cave-scan`, `cave-secrets` | trivy-rego-config-misconfig, trivy-secret-rule-config-toml |
| Plugin system | `cave-container-scan` (next deep port) | trivy-plugin-system |
| Artifact layer extract | `cave-artifacts` | image-layer-tarball-extract |

## Workspace integration

- **`cave-cli`** wraps the HTTP surface as `cavectl scan {image,fs,iac,secret,yara,namespace}` — replaces the standalone Trivy CLI.
- **`cave-scan-db`** owns the vuln-DB lifecycle (BoltDB equivalent) — cave-container-scan stays stateless and reads enriched data on demand.
- **`cave-vulns`** is the DependencyTrack-mapped vuln correlation hub — handles CVE↔CWE↔MITRE enrichment that Trivy's inline DB normally does.
- **`cave-sign`** holds the canonical VEX evaluator — cave-container-scan emits Findings; cave-sign decides whether they're affected.
- **`cave-sbom`** owns SBOM generation (DependencyTrack v4.9.0 mapping) — the partial cyclonedx-sbom-output here is just basic component listing.
- **`cave-artifacts`** unpacks OCI layers for storage — the same layer extractor will back the image-layer-tarball-extract gap when wired in Phase 2.
- **`cave-portal`** consumes `/api/container-scan/results` for the dashboard panel.

## ADR

(No dedicated ADR — Trivy mapping lives in [docs/upstream-attribution.md](../../docs/upstream-attribution.md). cave-vulns / cave-scan / cave-sbom have their own ADRs in `docs/adr/`.)
