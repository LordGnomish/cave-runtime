<!--
SPDX-License-Identifier: AGPL-3.0-or-later
Copyright 2026 Cave Runtime contributors
-->

# cave-gitleaks — Charter v2 Parity Report

**Upstream:** [gitleaks/gitleaks](https://github.com/gitleaks/gitleaks) pinned **v8.29.1**
(commit `fb5d707e08fe0d2578b155458fdd53b6782dcab2`).
**Upstream license:** MIT (Copyright 2019 Zachary Rice) — verified by clone of
`v8.29.1/LICENSE`.
**Cave-gitleaks license:** AGPL-3.0-or-later (Charter v2 workspace rule).
**Last audit:** 2026-05-19.

---

## 1 · Fill-ratio (honest, measured)

```
impl_lines              = 1 065   (cave-gitleaks src/, excl #[cfg(test)])
upstream_in_scope_lines = 1 619   (sum of per-subsystem in-scope LOC, table below)
fill_ratio              = 0.6578
honest_ratio            = 0.6578  (no [[partial]] entries; honest == fill)
parity_ratio_source     = "manifest"
```

`parity-index.json` reads these fields directly from
`parity.manifest.toml` rather than an external audit doc.

## 2 · Per-subsystem LOC table

| Upstream file              | upstream LOC | in-scope LOC | local file        | status  |
|----------------------------|-------------:|-------------:|-------------------|---------|
| `config/config.go`         | 492          | 350          | `src/config.rs`   | mapped  |
| `config/rule.go`           | 107          | 107          | `src/rule.rs`     | mapped  |
| `config/allowlist.go`      | 179          | 150          | `src/config.rs`   | mapped  |
| `detect/detect.go`         | 900          | 400          | `src/detect.rs`   | mapped  |
| `detect/files.go`          | 92           | 80           | `src/detect.rs`   | mapped  |
| `detect/git.go`            | 35           | 35           | `src/git_walker.rs` | mapped |
| `detect/location.go`       | 80           | 60           | `src/detect.rs`   | mapped  |
| `detect/reader.go`         | 108          | 50           | `src/detect.rs`   | mapped  |
| `detect/utils.go`          | 263          | 80           | `src/detect.rs`   | mapped  |
| `report/finding.go`        | 126          | 100          | `src/finding.rs`  | mapped  |
| `report/json.go`           | 17           | 17           | `src/report.rs`   | mapped  |
| `report/sarif.go`          | 217          | 170          | `src/report.rs`   | mapped  |
| `report/report.go`         | 16           | 16           | `src/report.rs`   | mapped  |
| `report/constants.go`      | 4            | 4            | `src/report.rs`   | mapped  |
| **Total**                  | **2 636**    | **1 619**    |                   |         |

## 3 · Mapped subsystems (10)

| Subsystem | What it does |
|-----------|--------------|
| `config-loader` | TOML schema parse (`title`, `[allowlist]`, `[[rules]]`, per-rule allowlist), regex compile, `deny_unknown_fields`. |
| `rule-engine` | `Rule` struct: compiled regex, path scope, keyword pre-filter, entropy floor, per-rule allowlist. |
| `allowlist` | Path / secret / commit allowlists with per-rule overrides. |
| `detector` | Per-line per-rule scan with redaction, path-allowlist short-circuit, entropy gate. |
| `shannon-entropy` | `shannonEntropy` port verbatim, bits-per-symbol. |
| `finding` | `Finding` struct with upstream JSON tag parity; deterministic fingerprint (`commit:file:rule_id:line`). |
| `json-reporter` | Array-of-Finding writer with upstream field names — output joinable to upstream dashboards. |
| `sarif-reporter` | SARIF 2.1.0 subset — tool driver, rule descriptors (deduped), results with regions. |
| `working-tree-walker` | Filesystem walker, skips `.git/.hg/.svn`, tolerates binary files. |
| `git-history-walker` | libgit2 revwalk; per-commit diff scan; commit metadata stamping (sha/author/email/date/message). |

## 4 · Built-in rule pack (12 high-signal providers)

`aws-access-token` · `gcp-api-key` · `azure-ad-client-secret` ·
`github-pat` · `github-oauth` · `github-fine-grained-pat` ·
`slack-bot-token` · `slack-user-token` · `stripe-secret-key` ·
`npm-access-token` · `jwt` · `private-key` (PEM) · `generic-api-key`.

The upstream pack ships >700 rules. Bundling all 700 is deferred to a
follow-up "rule pack import" ray; the manifest's `[[skipped]]` block
documents this explicitly.

## 5 · Skipped subsystems (out-of-scope MVP, 8)

| Skipped | Reason |
|---------|--------|
| `protect` (`cmd/protect.go`) | Pre-commit / pre-push staged-blob enforcement — separate cavectl ray. |
| `baseline` (`detect/baseline.go`) | Persistent baseline + redact files; needs FS state design. |
| `csv-reporter` (`report/csv.go`) | Output unused by upstream consumers in practice. |
| `junit-reporter` (`report/junit.go`) | CI-specific; covered by SARIF in the ZAP/Dep-Track wave. |
| `template-reporter` (`report/template.go`) | Go-template render port — no Rust analog without large dep. |
| `decoders` (`detect/decoders/`) | Auto-decode base64/gzip payload chains — large surface. |
| `stopwords` | Anti-FP stoplist deferred until the rule pack expands beyond 12. |
| `extends-and-useDefault` (`config.Extend`) | Config composition (`Extend` + `UseDefault` + path inheritance). |

## 6 · Charter v2 8-gate close-out

| Gate | Status | Evidence |
|------|--------|----------|
| 1 · TDD strict (tests-first) | PASS | `tests/parity_self_audit.rs` (9 assertions) + per-module unit tests (37 tests total). |
| 2 · SPDX AGPL-3.0-or-later | PASS | gate 8 of self-audit walks the crate and asserts header on every `.rs`. |
| 3 · `source_sha` pin | PASS | `[upstream] source_sha = "v8.29.1"`; matches `version` field. |
| 4 · No-stub | PASS | gate 7 scans for `unimplemented!()` / `todo!()` — zero offenders. |
| 5 · No-backcompat | PASS | `serde(deny_unknown_fields)` on all config structs; no legacy paths. |
| 6 · Always-latest | PASS | v8.29.1 is the latest stable tag on `gitleaks/gitleaks` as of 2026-05-19. |
| 7 · 4-track minimum | Backend GREEN; Portal/cavectl/Obs deferred to follow-up wired-in commit (this scaffold is the prerequisite). |
| 8 · Honest measured `fill_ratio` | PASS | `parity_ratio_source = "manifest"`; `fill_ratio = 0.6578` measured from impl/in-scope LOC ratio, audit-fallback NOT used. |

## 7 · Follow-up backlog

- **Portal track** — `/admin/secrets/gitleaks` panel showing latest scan
  findings, allowlist coverage, rule pack version.
- **cavectl track** — `cavectl gitleaks scan {wt|git|stdin}` subcommands +
  `gitleaks config validate` config linter.
- **Obs track** — Prometheus metrics
  (`cave_gitleaks_findings_total{rule_id}`, `cave_gitleaks_scan_seconds`)
  + 3 dashboard panels + 2 alerts (sudden findings spike, scan failure).
- **Rule-pack expansion** — port the remaining ~688 upstream rules
  (mechanical: TOML import from `config/gitleaks.toml`).
- **`protect` subcommand** — pre-commit / pre-push hook with staged-blob
  enforcement.
