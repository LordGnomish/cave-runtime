<!--
SPDX-License-Identifier: AGPL-3.0-or-later
Copyright 2026 Cave Runtime contributors
-->

# cave-gitleaks — Charter v2 Parity Report

**Upstream:** [gitleaks/gitleaks](https://github.com/gitleaks/gitleaks) pinned **v8.29.1**.
**Upstream license:** MIT (Copyright 2019 Zachary Rice).
**cave-gitleaks license:** AGPL-3.0-or-later (Charter v2 workspace rule).
**Last audit:** 2026-05-19.

---

## 1 · Fill-ratio (honest, measured)

```
mapped     = 17
partial    =  1
unmapped   =  0
skipped    =  2
total      = 20

fill_ratio   = mapped / (mapped + partial + unmapped) = 17 / 18 = 0.9444
honest_ratio = mapped / total                          = 17 / 20 = 0.8500
parity_ratio_source = "manifest"
```

Supplementary LOC measurement: ~1750 implementation lines (excluding
`#[cfg(test)]`) against ~2350 upstream in-scope lines — ~0.74 ratio on
the LOC basis.

## 2 · Mapped subsystems (17)

| # | Subsystem                  | Local file              | Upstream                             |
|---|----------------------------|-------------------------|--------------------------------------|
| 1 | config-loader              | `src/config.rs`         | `config/config.go`                   |
| 2 | extends-and-useDefault     | `src/config.rs`         | `config/config.go::Extend`           |
| 3 | rule-engine                | `src/rule.rs`           | `config/rule.go`                     |
| 4 | allowlist                  | `src/config.rs`         | `config/allowlist.go`                |
| 5 | detector                   | `src/detect.rs`         | `detect/detect.go`                   |
| 6 | shannon-entropy            | `src/detect.rs`         | `detect/utils.go`                    |
| 7 | finding + redaction        | `src/finding.rs`        | `report/finding.go`                  |
| 8 | json-reporter              | `src/report.rs`         | `report/json.go`                     |
| 9 | sarif-reporter             | `src/report.rs`         | `report/sarif.go`                    |
| 10| csv-reporter               | `src/report.rs`         | `report/csv.go`                      |
| 11| junit-reporter             | `src/report.rs`         | `report/junit.go`                    |
| 12| working-tree-walker        | `src/detect.rs`         | `detect/files.go`                    |
| 13| git-history-walker         | `src/git_walker.rs`     | `detect/git.go`                      |
| 14| baseline                   | `src/baseline.rs`       | `detect/baseline.go`                 |
| 15| decoders (base64 + gzip)   | `src/decoders.rs`       | `detect/decoders/`                   |
| 16| stopwords                  | `src/stopwords.rs`      | `config/gitleaks.toml [stopwords]`   |
| 17| protect                    | `src/protect.rs`        | `cmd/protect.go`                     |

All 17 rows above are `[[mapped]]` blocks in `parity.manifest.toml`.
Phase 2 added 7 of them: extends-and-useDefault, csv-reporter,
junit-reporter, baseline, decoders, stopwords, protect.

## 3 · Partial subsystems (1)

| Subsystem        | Reason                                                                                                                                                |
|------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------|
| rule-pack-import | 12 high-signal cross-industry rules shipped (AWS, GCP, Azure, GitHub, Slack, Stripe, NPM, JWT, PEM, generic-api-key…); >700-rule upstream pack pending. |

## 4 · Skipped subsystems (2 — intentional out-of-scope)

| Surface                  | Reason                                                                                                              |
|--------------------------|---------------------------------------------------------------------------------------------------------------------|
| template-reporter        | Go-template render port deferred — SARIF + CSV + JUnit cover CI needs.                                              |
| github-action-integration| `action.yml` is repository tooling; cavectl plus a thin yml shim lands later.                                       |

## 5 · 4-track status

| Track          | Status     | Evidence                                                                                                  |
|----------------|------------|-----------------------------------------------------------------------------------------------------------|
| Backend        | **GREEN**  | This crate — 17 mapped + 1 partial. 51 lib + 15 phase2_deep_port + 9 parity_self_audit = **75 tests PASS**.|
| Portal         | Phase 3    | admin/secrets surface lands after the rule-pack import.                                                   |
| cavectl        | Phase 3    | `cavectl secrets detect / protect / scan-repo` follows the rule-pack import.                              |
| Observability  | Phase 3    | findings emission to OnCall / Grafana alongside cave-vulns wave.                                          |

## 6 · 8-gate close-out checklist (Charter v2)

| # | Gate                                                                  | Status |
|---|-----------------------------------------------------------------------|--------|
| 1 | TDD-strict — `tests/parity_self_audit.rs` 9 assertions PASS           | ✅      |
| 2 | SPDX AGPL-3.0-or-later on every `.rs` file                            | ✅      |
| 3 | `[upstream] source_sha` pinned to `v8.29.1`                           | ✅      |
| 4 | No-stub — zero `todo!()`/`unimplemented!()`/`panic!("stub")` in src/  | ✅      |
| 5 | No-backcompat — no aliased re-exports or migration shims              | ✅      |
| 6 | Always-latest — Gitleaks v8.29.1 (latest stable as of 2026-05-19)     | ✅      |
| 7 | 4-track — Backend GREEN; Portal/cavectl/Obs honestly deferred Phase 3 | ✅      |
| 8 | Honest measured `fill_ratio = 0.9444` (>= 0.40 MVP floor)             | ✅      |

## 7 · Reproducibility

```bash
cargo test -p cave-gitleaks
python3 scripts/build-parity-index.py
```
