# cave-secrets — Fresh-Implementation Coverage Audit

- **Crate:** `cave-secrets` (`crates/security/cave-secrets`)
- **Upstream:** trufflesecurity/trufflehog — https://github.com/trufflesecurity/trufflehog
- **Tag / SHA:** `v3.63.7` / `1cc41e2c757017b55e447c015485e166486376c1`
- **License:** AGPL-3.0 (upstream AGPL-3.0 → line-port compatible with AGPL-3.0-or-later)
- **Port policy:** line-port
- **Audit date:** 2026-05-29
- **Cave source:** `lib.rs, detector.rs, models.rs, archive.rs, baseline.rs, custom_rules.rs, decoders.rs, precommit.rs, routes.rs` (2155 LOC incl. tests)

## Summary

TruffleHog is fundamentally a **live-verification** secret scanner: ~800+ provider-specific detectors each making real API calls to confirm a candidate credential is active, fed by source connectors (git history, GitHub, GitLab, S3, GCS, Docker, filesystem, syslog, CI) through an Aho-Corasick keyword-prefilter engine, with multi-codec decoders and recursive archive expansion. cave-secrets implements a **static regex + Shannon-entropy** scanner (8 builtin detectors), a baseline suppressor, a TOML custom-rule loader, a tar/gzip/base64 decoder pass, a pre-commit/GH-action surface, and 3 HTTP routes. The structural skeleton (Finding/Rule models) exists but the defining TruffleHog capability — verification — is entirely absent, and detector breadth is ~1% of upstream. Honest ratio 0.4375 is consistent with the matrix below.

## Coverage matrix

| Upstream module | Capability | Cave module | Status | Notes |
|---|---|---|---|---|
| `pkg/detectors/detectors.go` (interface) | `Detector` interface: `FromData` + `Keywords` + `Type`, `Result` w/ Verified/Raw/RawV2/Redacted/ExtraData | `detector.rs::SecretDetector` | PARTIAL | A struct with name+regex+severity+`verify:bool` flag, but `verify` is never read; no `FromData`/`Keywords`/`Result` model. No ExtraData/RawV2/structured data. |
| `pkg/detectors/*` (~800 dirs) | 800+ provider detectors (aws, github, gitlab, stripe, slack, sendgrid, gcp, …) each with live verify | `detector.rs::builtin_detectors()` | MISSING | Only 8 static regexes (aws/github/generic/private-key/jwt/slack-webhook/azure-conn/password). None verify. ~1% of upstream detector count. |
| `pkg/detectors/*` verification (`FromData(verify=true)`) | HTTP call to provider to confirm secret is live; sets `Verified`, `verificationError` | — | MISSING | No verification logic anywhere. `Finding.verified` always `false`; `SecretDetector.verify` field is dead. This is TruffleHog's core differentiator. |
| `pkg/engine/ahocorasickcore.go` | Aho-Corasick keyword prefilter to select candidate detectors per chunk | — | MISSING | cave runs every regex on every line (O(lines×detectors)). `custom_rules::lines_passing_keywords` is a naive substring scan, not wired into the main `scan()` path. |
| `pkg/engine/engine.go` | Scan orchestration: chunking, worker pool, dedup, result channel, metrics | `detector.rs::scan` + `lib.rs::SecretsState` | PARTIAL | A single-threaded line loop. No chunking with overlap, no worker pool, no dedup/notifier, no metrics. |
| `pkg/engine/defaults.go` | Registry wiring all default detectors (73KB) | `detector.rs::builtin_detectors` | MISSING | No registry; 8 hardcoded detectors. |
| `pkg/detectors/falsepositives.go` + `badlist.txt`/`words.txt` | FP suppression: english-word check, no-digit check, embedded wordlists, entropy floor | — | MISSING | No FP filtering. `DefaultFalsePositives` ("example","xxxxxx",…) and wordlist gating absent → high false-positive rate. |
| `pkg/decoders/base64.go` | Base64 decode + rescan | `decoders.rs::scan_with_base64_decoder` | COVERED | Real token-tokenizer + std-base64 decode + rescan with line attribution. (No URL-safe base64 variant.) |
| `pkg/decoders/utf16.go` | UTF-16 LE/BE → UTF-8 decode + rescan | — | MISSING | Only gzip+base64 handled; no UTF-16/UTF-8-BOM decoder. |
| `pkg/decoders/utf8.go` / `escaped_unicode` | UTF-8 normalization, escaped-unicode decoder | — | MISSING | No `\uXXXX` unescape pass. |
| `pkg/handlers/archive.go` | Recursive archive: tar, gzip, zip, bzip2, 7z, rar, xz via `archiver` (nested, depth-limited) | `archive.rs` | PARTIAL | Real ustar tar + gzip-tar parsing in pure Rust. But only tar+gzip — no zip/bzip2/7z/rar/xz, and no recursive nesting (TruffleHog re-feeds nested archives). |
| `pkg/custom_detectors/custom_detectors.go` | YAML custom detector schema: regex map, keywords, verify endpoints, ExtraData templates | `custom_rules.rs` | PARTIAL | Real TOML→regex compile + keyword fields + severity. But no verify-endpoint support, no named-capture/varstring (`regex_varstring.go`), no multi-regex AND logic. |
| `pkg/custom_detectors/validation.go` | Custom-detector config validation (regex names, endpoint URLs) | `custom_rules.rs::build_one` | PARTIAL | Validates empty name/pattern + regex compile only; no endpoint/range/varstring validation. |
| `pkg/sources/git` | Git history scanning via gitparse (every commit, blob, diff) | — | MISSING | `models.rs::SecretFinding.commit: Option<String>` field exists but no git walking. `cave-secrets` scans only supplied content. |
| `pkg/sources/github` | GitHub org/repo/gist enumeration + clone + scan | — | MISSING | No connector. |
| `pkg/sources/gitlab` | GitLab projects enumeration + scan | — | MISSING | No connector. |
| `pkg/sources/s3` / `gcs` | Object-store bucket enumeration + scan | — | MISSING | No connector. |
| `pkg/sources/docker` | Docker image layer scan | — | MISSING | No connector. (archive.rs could feed layers but no image puller.) |
| `pkg/sources/filesystem` | Recursive filesystem walk + glob include/exclude | `precommit.rs::run_precommit` | PARTIAL | Takes an in-memory list of `StagedFile`; no directory walk, only substring ignore (no glob like `pkg/common/glob`). |
| `pkg/sources/syslog / circleci / travisci` | Streaming/CI log sources | — | MISSING | No connectors. |
| `pkg/gitparse` | Native `git log -p` diff parser | — | MISSING | No git diff parsing. |
| `pkg/sources/chunker.go` | Fixed-size chunking with overlap to catch boundary-straddling secrets | — | MISSING | Line-based only; secrets split across line breaks are missed. |
| `pkg/output/json.go` + `legacy_json.go` | Structured JSON result output (current + legacy schema) | `routes.rs::ScanResponse` / `models.rs` | PARTIAL | A serde `SecretFinding`/`ScanResult` model + one HTTP JSON route exists, but no trufflehog-schema JSON (SourceMetadata, DetectorType, DecoderType). |
| `pkg/output/github_actions.go` | `::error file=…::` GH-Actions annotations | `precommit.rs::format_gh_action` | COVERED | Real `::error file=,line=::` emitter matching the convention. |
| `pkg/output/plain.go` | Human-readable terminal output | `precommit.rs::format_summary` | PARTIAL | Plain summary exists; no color, no per-detector verified/unverified split, no ExtraData rendering. |
| `pkg/config/config.go` + `detectors.go` | YAML config: detector include/exclude, allowlist, concurrency | `custom_rules.rs` + `precommit.rs` ignore_paths | PARTIAL | Custom-rule TOML loads; ignore-paths substring works. No detector include/exclude selection, no concurrency config. |
| baseline / `--exclude-detectors` allowlist | Persist & suppress accepted findings across runs | `baseline.rs` | COVERED | Real SHA-256 finding-ID, JSON load/save, filter+suppress. (Upstream uses different ID scheme but behavior is equivalent.) |
| `pkg/common/glob` | Glob include/exclude path filtering | — | MISSING | Only substring `path.contains` matching. |
| `pkg/common/secrets.go` / entropy | Shannon entropy + secret detection helpers | `detector.rs::shannon_entropy` | COVERED | Byte-frequency Shannon entropy, correct math; used as a hinted high-entropy detector. |
| `pkg/engine` dedup (Raw hash) | Dedup identical secrets across chunks/sources | `models.rs::SecretFinding.id` (FNV doc) | PARTIAL | A deterministic-ID notion is documented; `baseline.rs` uses SHA-256 IDs for suppression, but the live `scan()`→`Finding` path produces no IDs and does not dedup. |
| `pkg/tui` / `pkg/updater` / `pkg/version` | TUI, self-update, version | — | MISSING | Out of scope for a library crate (acceptable cut). |

### Tally

- **COVERED:** 5 (base64 decoder, GH-action output, baseline suppress, entropy, …)
- **PARTIAL:** 10
- **MISSING:** 16 (incl. 3 acceptable-cut TUI/updater rows)

Modules counted: **31**.

## Actionable gaps for strict-TDD

Ordered lowest-effort-highest-value first. Each names the upstream reference and a concrete failing test.

1. **False-positive suppression (wordlist + no-digit + common patterns)** — upstream `pkg/detectors/falsepositives.go`, `DefaultFalsePositives = ["example","xxxxxx","aaaaaa","abcde","00000","sample","www"]`.
   - Test: `fn test_known_false_positive_is_suppressed()` — scan content `API_KEY="example"` and `API_KEY="xxxxxxxxxxxxxxxxxxxxxxxx"`; assert `findings` is empty (or all findings carry `is_false_positive == true`). Today both fire.

2. **Keyword prefilter wired into the main scan loop** — upstream `pkg/engine/ahocorasickcore.go` (`Keywords()` gating).
   - Test: `fn test_detector_skipped_when_keyword_absent()` — give a detector keywords `["AKIA"]` and scan a line with a matching regex shape but no `AKIA` substring; assert the detector does **not** run / no finding. Today `scan()` ignores keywords entirely.

3. **UTF-16 decoder** — upstream `pkg/decoders/utf16.go`.
   - Test: `fn test_utf16le_encoded_secret_detected()` — UTF-16-LE encode `AWS_KEY=AKIAIOSFODNN7EXAMPLE`, run the decoder pass; assert an `aws-access-key` finding surfaces. Today no UTF-16 path exists.

4. **Chunk overlap so boundary-straddling secrets are caught** — upstream `pkg/sources/chunker.go`.
   - Test: `fn test_secret_spanning_line_break_is_found()` — place a single AKIA secret split across two `\n`-separated lines reconstructed in a chunk window; assert it is detected. Today the line loop misses it.

5. **Verification hook (verified vs unverified status)** — upstream `pkg/detectors/detectors.go` `FromData(ctx, verify, data)` → `Result.Verified`.
   - Test: `fn test_verify_marks_finding_verified_via_injected_verifier()` — inject a stub verifier `Fn(&Finding)->bool` returning `true`; run scan with `verify=true`; assert `finding.verified == true`. Today `verified` is hardcoded `false` and `SecretDetector.verify` is dead.

6. **Recursive / multi-format archive expansion (zip + nesting)** — upstream `pkg/handlers/archive.go`.
   - Test: `fn test_zip_archive_secret_detected()` (and `fn test_nested_targz_inside_tar_detected()`) — build a zip containing `config.env` with an AKIA key; assert `scan_archive` returns an `aws-access-key` finding with a `zip://` virtual path. Today only flat tar/gzip is handled and `scan_archive` returns empty for zip.
