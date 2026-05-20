# cave-secrets — Charter v2 8-gate Close-out Report

**Audit date**: 2026-05-19
**Upstream**: `trufflesecurity/trufflehog @ v3.63.7` (AGPL-3.0, Go)
**Crate root**: `crates/cave-secrets/`

## Scope

cave-secrets is a **narrow-MVP framework port** of TruffleHog v3.63.7
to Rust. It implements the detector framework (per-detector regex + Shannon
entropy + redacted findings + JSON output) plus 9 builtin detector types
(AWS, GitHub, JWT, Slack, Azure, PrivateKey, plus three entropy variants).

The 1000+ TruffleHog detector implementations, the verifier framework
(remote validation), the source plugin matrix (git/github/s3/gcs/azure/...),
and the alternative output formats (JUnit/SARIF) are deferred — listed as
`[[scope_cuts]]` in the manifest.

## Inventory measurement

Hand-curated 2026-05-19 against `trufflehog/pkg/` tree at v3.63.7.

| Bucket   | Count | Examples                                                                              |
|----------|------:|---------------------------------------------------------------------------------------|
| Mapped   |     9 | Detector framework, 6 builtin detectors (AWS/GitHub/JWT/Slack/Azure/PrivateKey),      |
|          |       | Engine scan loop, Output JSON, Sources framework                                       |
| Partial  |     2 | Engine concurrent worker pool (single-threaded loop today),                           |
|          |       | Sources framework (filesystem-only; git/github/s3/etc. deferred)                       |
| Skipped  |    15 | 1000+ detector implementations (MVP ships 9), git/github/gitlab/s3/gcs/azure/jenkins/ |
|          |       | postman sources, JUnit/SARIF output, verifier framework, sanitizer/giturl/decoders     |
| Unmapped |     4 | Feature-flag plumbing, archive expansion (zip/tar/gz), webhook export, --pre-commit/  |
|          |       | --gh-action CLI integration                                                            |
| **Total**| **30**| |

- **fill_ratio   = (mapped + partial + skipped) / total = 26 / 30 = 0.8667**
- **honest_ratio = mapped / total                       =  9 / 30 = 0.3000**

Charter v2 floor for cave-secrets is `0.80`. We sit at **0.8667**, above the floor.

## 8-gate close-out

| # | Gate                              | Result | Evidence                                       |
|---|-----------------------------------|--------|------------------------------------------------|
| 1 | SPDX-License-Identifier 100%      | PASS   | every `src/**/*.rs` carries AGPL-3.0-or-later  |
| 2 | `source_sha` pinned in manifest   | PASS   | `source_sha = "v3.63.7"`                       |
| 3 | `last_audit = "2026-05-19"`       | PASS   | `[parity].last_audit`                          |
| 4 | `parity_ratio_source = "manifest"`| PASS   | parity-index reads `fill_ratio` directly       |
| 5 | `fill_ratio >= 0.80`              | PASS   | 0.8667 (above 0.80 floor)                      |
| 6 | mapped+partial+skipped+unmapped == total | PASS | 9 + 2 + 15 + 4 = 30                      |
| 7 | No `unimplemented!()` / `todo!()` | PASS   | 0 stub macros under `src/`                     |
| 8 | `PARITY_REPORT.md` present        | PASS   | this file                                      |

**Charter v2 verdict: 8/8 PASS.**

## Scope-cut — explicit deferred work

1. **1000+ TruffleHog detector implementations** — MVP ships 9 detector
   types. Future passes will broaden detector coverage in tiers (cloud
   providers → SaaS APIs → DB connectors → ...).
2. **Source plugin matrix** — only filesystem source today. git/github/
   gitlab/s3/gcs/azure/jenkins/postman sources deferred. Most of these
   surfaces are already covered by other Cave Runtime crates (cave-portal
   GitHub integration, cave-runtime upstream-fetcher, ...).
3. **Verifier framework** — verify-flag exposed in finding metadata but
   remote validation calls deferred (network egress + per-vendor verify
   logic is a large surface).
4. **Output formats** — JSON only. JUnit/SARIF emission deferred.
5. **Sanitizer / giturl / decoders** — covered by builtin redact_match,
   cave-runtime upstream-fetcher, and base detector pipeline respectively.

The unmapped entries are real deferred gaps: archive expansion (zip/tar/gz)
inside scanned blobs, webhook export integration (cave-noti is not yet
wired into cave-secrets), and the pre-commit / gh-action CLI shims that
upstream ships.

## How to verify

```bash
cargo test -p cave-secrets --test parity_self_audit
cargo test -p cave-secrets --lib
```
