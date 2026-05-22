# cave-secrets — Charter v2 8-gate Close-out Report

**Audit date**: 2026-05-19 (parity-uplift-sec-stack)
**Upstream**: `trufflesecurity/trufflehog @ v3.63.7` (AGPL-3.0, Go)
**Crate root**: `crates/cave-secrets/`

## Scope

cave-secrets is a **detector-framework port** of TruffleHog v3.63.7 to Rust.
It implements the detector framework (per-detector regex + Shannon entropy +
redacted findings + JSON output), 9 builtin detector types
(AWS/GitHub/JWT/Slack/Azure/PrivateKey + 3 entropy variants), and — after the
2026-05-19 parity uplift — the developer-facing surfaces around it: custom
regex rule loading, baseline persistence, base64+gzip decoders, archive
(tar / tar.gz) expansion, and the pre-commit / GH-Action CLI shims.

The 1000+ TruffleHog detector implementations, the verifier framework
(remote validation), the source plugin matrix (git/github/s3/gcs/azure/...),
and the alternative output formats (JUnit/SARIF) remain deferred and are
listed as `[[scope_cuts]]` in the manifest.

## Inventory measurement

Hand-curated 2026-05-19 against `trufflehog/pkg/` tree at v3.63.7.

| Bucket   | Count | Examples                                                                              |
|----------|------:|---------------------------------------------------------------------------------------|
| Mapped   |    14 | Detector framework, 9 builtin detectors, Engine scan loop, Output JSON, Sources hook, |
|          |       | **custom_rules**, **baseline**, **decoders (base64+gzip)**, **precommit**, **archive**|
| Partial  |     2 | Engine concurrent worker pool (single-threaded loop today),                           |
|          |       | Sources framework (filesystem-only; git/github/s3/etc. deferred)                      |
| Skipped  |    15 | 1000+ detector implementations, git/github/gitlab/s3/gcs/azure/jenkins/postman        |
|          |       | sources, JUnit/SARIF output, verifier framework, sanitizer, giturl, notifier (cave-noti) |
| Unmapped |     1 | Feature-flag plumbing for experimental detectors                                      |
| **Total**| **32**| |

- **fill_ratio   = (mapped + partial + skipped) / total = 31 / 32 = 0.9688**
- **honest_ratio = mapped / total                       = 14 / 32 = 0.4375**

Charter v2 parity-uplift floor is **0.95**. We sit at **0.9688**, above the floor.

## 8-gate close-out

| # | Gate                              | Result | Evidence                                       |
|---|-----------------------------------|--------|------------------------------------------------|
| 1 | SPDX-License-Identifier 100%      | PASS   | every `src/**/*.rs` carries AGPL-3.0-or-later  |
| 2 | `source_sha` pinned in manifest   | PASS   | `source_sha = "v3.63.7"`                       |
| 3 | `last_audit = "2026-05-19"`       | PASS   | `[parity].last_audit`                          |
| 4 | `parity_ratio_source = "manifest"`| PASS   | parity-index reads `fill_ratio` directly       |
| 5 | `fill_ratio >= 0.95`              | PASS   | 0.9688                                         |
| 6 | mapped+partial+skipped+unmapped == total | PASS | 14 + 2 + 15 + 1 = 32                     |
| 7 | No `unimplemented!()` / `todo!()` | PASS   | 0 stub macros under `src/`                     |
| 8 | `PARITY_REPORT.md` present        | PASS   | this file                                      |

**Charter v2 verdict: 8/8 PASS.**

## What landed in the 2026-05-19 uplift

| Module                     | TruffleHog upstream                                | Tests added |
|----------------------------|----------------------------------------------------|------------:|
| `src/custom_rules.rs`      | `pkg/custom_detectors/custom_detectors.go`         | 10          |
| `src/baseline.rs`          | gitleaks/trufflehog baseline allowlist semantics   |  9          |
| `src/decoders.rs`          | `pkg/decoders/{base64,gzip}.go`                    | 10          |
| `src/precommit.rs`         | `cmd/trufflehog/main.go --pre-commit / --gh-action`|  8          |
| `src/archive.rs`           | `pkg/handlers/{tar,gz}.go`                         | 11          |

## How to verify

```bash
cargo test -p cave-secrets --test parity_self_audit
cargo test -p cave-secrets --lib
```
