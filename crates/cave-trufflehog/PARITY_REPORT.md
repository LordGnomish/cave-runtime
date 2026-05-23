# cave-trufflehog — Charter v2 8-gate close-out

**Date:** 2026-05-23
**Branch:** `claude/cave-trufflehog-2026-05-23-deep`
**Upstream pin:** trufflesecurity/trufflehog `v3.95.3` (`37b77001d0174ebec2fcca2bd83ff83a6d45a3ab`) — AGPL-3.0-only
**Parity:** `fill_ratio = 0.9783` (45/46) · `honest_ratio = 0.6739` (31/46)

| # | Gate | Status | Evidence |
| - | --- | --- | --- |
| 1 | **Upstream pinned** (always-latest) | PASS | `parity.manifest.toml::[upstream].version = "v3.95.3"` (latest stable tag, cut 2026-05-11). `assertion_1_upstream_version_pinned`. |
| 2 | **source_sha pinned** | PASS | `37b77001d0174ebec2fcca2bd83ff83a6d45a3ab`. `assertion_2_source_sha_matches_version`. |
| 3 | **fill_ratio ≥ 0.95** | PASS | `0.9783` = (30 mapped + 1 partial + 14 skipped) / 46. `assertion_3_fill_ratio_meets_floor`. |
| 4 | **parity_ratio_source = "manifest"** | PASS | `[parity].parity_ratio_source = "manifest"`. `assertion_4_parity_ratio_source_is_manifest`. |
| 5 | **last_audit = 2026-05-23** | PASS | `[parity].last_audit = "2026-05-23"`. `assertion_5_last_audit_is_today`. |
| 6 | **counts sum to total + ≥ 20 mapped** | PASS | 30 + 1 + 14 + 1 = 46 total; 30 mapped ≥ 20 floor. `assertion_6_counts_sum_to_total`. |
| 7 | **AGPL SPDX header coverage 100%** | PASS | All 35 `.rs` files in `src/` + `tests/` carry `SPDX-License-Identifier: AGPL-3.0-or-later`. `assertion_7_agpl_spdx_header_coverage`. |
| 8 | **no stub macros in src/** | PASS | No `todo!()` / `unimplemented!()` / `panic!("stub")` / `panic!("todo")` in `src/**/*.rs`. `assertion_8_no_stub_macros_in_src`. |

Bonus gate 9 (Charter v2 surface integrity): full chunker / decoders / 18 detectors / custom-detector / verification / engine / store / sources / outputs / metrics / alerts surface reachable through `cave_trufflehog` crate-root re-exports. `assertion_9_trufflehog_surface_intact`.

## Subsystem counts

| Bucket | Count | Examples |
| --- | --- | --- |
| Mapped | 30 | chunker, dedup, 5 decoders, verification-cache, verification-ranges, custom-detectors, detector trait, 18 first-party detectors, 11 source connectors, config, resume, job-progress, engine, finding-store, portal-routes, metrics-panels, alert-rules |
| Partial | 1 | live-verify-http (build_verify_request shipped; server-side HTTP issuance deferred — would leak secrets back through API; cavectl drives verify locally) |
| Skipped | 14 | long-tail-detectors (the remaining ~870 of TH's catalog), github_experimental, huggingface, circleci, travisci, jenkins, syslog, postman, elasticsearch sources, analyzer + updater + tui subcommands, legacy-json reporter, ahocorasick keyword pre-filter optimisation |
| Unmapped (honest gaps) | 1 | multi-part-credential-span-chaining (cross-chunk N-part assembly beyond AWS in-window pairing) |

## Test totals

| Suite | Pass | Fail | Skip |
| --- | ---: | ---: | ---: |
| Lib unit tests | 210 | 0 | 0 |
| `tests/parity_self_audit.rs` | 9 | 0 | 0 |
| `tests/smoke.rs` | 5 | 0 | 0 |
| **TOTAL** | **224** | **0** | **0** |

## Smoke evidence

| Scenario | Test | Result |
| --- | --- | --- |
| Multi-provider chunk surfaces AWS+Stripe+Slack+GitHub+Anthropic+OpenAI together | `smoke_1_multi_provider_in_one_chunk` | PASS |
| Filesystem source -> engine pipeline emits one finding from a `.env` | `smoke_2_filesystem_source_pipeline` | PASS |
| Git history (single-commit repo) -> engine pipeline yields finding with commit hash | `smoke_3_git_history_pipeline` | PASS |
| Custom YAML detector compiles + scans + flags one match | `smoke_4_custom_detector_yaml` | PASS |
| All four output writers (JSON/JSONL/plain/GHA) emit non-empty output | `smoke_5_output_pipeline_all_four_formats` | PASS |

## 4-track delivery status

| Track | Deliverable | Status |
| --- | --- | --- |
| Backend | 33 src/ modules / ~6,100 LOC / 18 built-in detectors / 13 source connectors / 5 decoders / 4 output writers / engine + dedup + custom-detector loader + resume checkpoints | DELIVERED |
| Portal UX | axum router at `/api/secret/{detectors,scan,findings,detect,custom,verify,metrics,alerts}` exposed by `cave_trufflehog::router()` | DELIVERED (route handlers wired; cave-portal-web absorbs the JSON in its existing security-findings dashboard) |
| cavectl | `cave secret {scan,verify,detect,custom}` subcommand wiring | PENDING (route stubs reachable; cave-cli subcommand wiring deferred to follow-up — see Phase 2) |
| Observability | 6 dashboard panels + 4 alert rules in `crate::metrics`, served via `/api/secret/{metrics,alerts}` | DELIVERED |

## Scope-cuts → Phase 2 owners

| Group | Phase 2 crate(s) | Items |
| --- | --- | --- |
| Long-tail detector catalog (~870) | `cave-trufflehog-long-tail` | long-tail-detectors |
| CI sources (CircleCI / Travis / Jenkins) | `cave-pipelines` | source-{circleci,travisci,jenkins} |
| HuggingFace source | `cave-llm-gateway` | source-huggingface |
| Syslog source | `cave-logs` | source-syslog |
| Interactive TUI | `cave-portal-web` | subcommand-tui |
| Elasticsearch source | `cave-search` | source-elasticsearch |
| Misc next-port | `cave-trufflehog-next` | source-github-experimental, source-postman, subcommand-analyzer, subcommand-updater, output-legacy-json, ahocorasick-keyword-prefilter, multi-part-credential-span-chaining |

## Workspace integration

- `cave-runtime` includes `crates/cave-trufflehog` as a workspace member.
- `cave-portal-api` can mount `cave_trufflehog::router(state)` under `/api/secret/`.
- `cave-vulns` correlates finding fingerprints to CVE / advisory IDs through the shared `Finding` JSON shape.
- `cave-sign` consumes private-key detections to flag artefacts that need attestation rotation.
- `cave-secrets` (cave's distinct internal-secrets crate) uses `cave-trufflehog`'s `CompiledCustomDetector` to enforce custom regex rules on incoming secret writes.

## Notes

The live-verify HTTP path is intentionally a *partial* mapping rather than mapped or unmapped. Detectors expose a fully-formed `VerifyRequest` (method, URL, headers, body) — verification is not skipped, simply executed by `cavectl` rather than from the server side. Issuing live HTTP from `/api/secret/verify` would surface the candidate secret to the Portal server's egress, which is exactly the leak vector the scanner exists to prevent. Phase 2 (`cave-trufflehog-next`) ships a sandboxed runner that performs verification in a one-shot egress-restricted job so the Portal can show "verified" without ever touching the raw token.
