# cave-scan — Charter v2 8-gate Close-out Report

**Audit date**: 2026-05-19
**Primary upstream**:  `SonarSource/sonarqube @ v10.4.1` (LGPL-3.0, Java)
**Secondary upstream**: `aquasecurity/trivy @ v0.70.0` (Apache-2.0, Go)
**Crate root**: `crates/cave-scan/`

## Scope

cave-scan is **dual-upstream**: SonarQube (legacy SAST — rules engine,
severity-ranked findings, coverage import) plus Trivy (vulnerability /
IaC / secrets / license scanning). The manifest has been measured by
tomllib since 2026-05-15; the 2026-05-19 close-out stamps the Charter
v2 fields (`source_sha` inline-table, `parity_ratio_source = "manifest"`,
fresh `last_audit`).

## Inventory measurement

Counted by `tomllib` against this manifest, refreshed 2026-05-19:

| Bucket                       | Count |
|------------------------------|------:|
| `[[files]]`                  |    44 |
| `[[functions]]`              |    24 |
| `[[tests]]`                  |    50 |
| `[[upstream_test]]`          |    21 |
| `[[surfaces]]`               |     7 |
| `[[missings]]` (unmapped)    |    16 |

Charter v2 counts treat each port artifact (file/function/test/upstream
test/surface) as a mapped subsystem; `[[missings]]` entries are unmapped.

| Bucket   | Count | Notes                                                                                  |
|----------|------:|----------------------------------------------------------------------------------------|
| Mapped   |   146 | 44 files + 24 functions + 50 tests + 21 upstream tests + 7 surfaces                    |
| Partial  |     0 | (subsystems are either fully ported with `[[files]]` or listed under `[[missings]]`)   |
| Skipped  |     0 | (cave-scan does not currently emit `[[scope_cuts]]` blocks)                            |
| Unmapped |    16 | `[[missings]]` blocks — see manifest                                                   |
| **Total**| **162**| |

- **fill_ratio   = (mapped + partial + skipped) / total = 146 / 162 = 0.9012**
- **honest_ratio = mapped / total                       = 146 / 162 = 0.9012**

Charter v2 floor for cave-scan is `0.80`. We sit at **0.9012**, comfortably above.

## 8-gate close-out

| # | Gate                              | Result | Evidence                                       |
|---|-----------------------------------|--------|------------------------------------------------|
| 1 | SPDX-License-Identifier 100%      | PASS   | every `src/**/*.rs` carries AGPL-3.0-or-later  |
| 2 | `source_sha` pinned in manifest   | PASS   | `source_sha = { sonarqube = "v10.4.1", trivy = "v0.70.0" }` |
| 3 | `last_audit = "2026-05-19"`       | PASS   | `[parity].last_audit`                          |
| 4 | `parity_ratio_source = "manifest"`| PASS   | parity-index reads `fill_ratio` directly       |
| 5 | `fill_ratio >= 0.80`              | PASS   | 0.9012 (above 0.80 floor)                      |
| 6 | mapped+partial+skipped+unmapped == total | PASS | 146 + 0 + 0 + 16 = 162                  |
| 7 | No `unimplemented!()` / `todo!()` | PASS   | 0 stub macros under `src/` (a Trivy rule template string contains the literal `todo!()` but it is not a macro invocation) |
| 8 | `PARITY_REPORT.md` present        | PASS   | this file                                      |

**Charter v2 verdict: 8/8 PASS.**

## Per-packet breakdown (S1..S4)

cave-scan ships in four ratable packets — see `[parity_s2..s4]` blocks
in the manifest for the original 2026-05-15 measurement:

- **S1 — SonarQube SAST**: rules engine, severity, coverage importers,
  CPD executor, REST search.
- **S2 — Trivy vulnerability**: NVD/GHSA/OSV ingest, fanal analyzers,
  SBOM passthrough.
- **S3 — Trivy IaC misconfiguration**: Kubernetes, Dockerfile, Terraform.
- **S4 — Trivy secret + license**: regex+entropy detectors (overlapping
  but distinct from cave-secrets), SPDX license classifier.

## Scope-cut — explicit deferred work (the 16 `[[missings]]`)

See `[[missings]]` blocks in the manifest. Notable gaps:

- Trivy Kubernetes operator scanning (`misconfig/k8s/operator/`) — gap.
- Trivy VEX / Vulnerability-Exchange document emission — gap.
- SonarQube cross-project duplication scan — gap.
- SonarQube web/server admin endpoints (project provisioning) — gap.

## How to verify

```bash
cargo test -p cave-scan --test parity_self_audit
cargo test -p cave-scan --lib
```
