# RAY REPORT — cave-dns Charter v2 close-out

**Date:** 2026-05-23
**Branch:** `claude/cave-dns-close-2026-05-23` (off `origin/main` @ `76101add`)
**Worktree:** `/Users/gnomish/Code/cave-runtime/.claude/worktrees/agent-a9b6fc3966f26b59c`

## Branch tip

`bf9f19f9` — `test(cave-dns): Charter v2 8-gate self-audit + runtime surface gate`

## Commits added (5)

```
bf9f19f9 test(cave-dns): Charter v2 8-gate self-audit + runtime surface gate
f996cfc1 feat(cave-dns): observability + cavectl dns CLI dispatcher
03cd4184 feat(cave-dns): split DNSSEC primitives into focused sub-modules
19a247f4 feat(cave-dns): add prometheus/root/tls/trace built-in plugins
013ee671 feat(cave-dns): rewrite parity.manifest.toml to Charter v2 schema
```

## LOC: before -> after (per added module)

Workspace baseline: 7,892 LOC across 47 `.rs` files in `crates/cave-dns/src/`.

| module | LOC added |
|--------|----------:|
| `src/plugins/prometheus.rs`  | 89 |
| `src/plugins/root.rs`        | 113 |
| `src/plugins/tls.rs`         | 136 |
| `src/plugins/trace.rs`       | 184 |
| `src/dnssec/mod.rs`          | 22 |
| `src/dnssec/dnskey.rs`       | 187 |
| `src/dnssec/rrsig.rs`        | 153 |
| `src/dnssec/nsec.rs`         | 116 |
| `src/dnssec/nsec3.rs`        | 167 |
| `src/dnssec/validator.rs`    | 209 |
| `src/observability.rs`       | 297 |
| `src/cli.rs`                 | 260 |
| **Total new src/**           | **+1,931** |
| **Final `src/` total**       | **9,823 LOC** (67 files) |
| `tests/parity_self_audit.rs` | 443 LOC (9 assertions) |

## Parity scorecard: before -> after

| metric           | before | after   |
|------------------|-------:|--------:|
| `fill_ratio`     | 0.0    | **0.9583** |
| `honest_ratio`   | 0.0    | **0.7500** |
| `mapped_count`   | 0      | 33      |
| `partial_count`  | 0      | 3       |
| `skipped_count`  | 0      | 10      |
| `unmapped_count` | 0      | 2       |
| `total`          | -      | 48      |

`fill_ratio = (33 + 3 + 10) / 48 = 0.9583` (>= 0.95 close-out floor)
`honest_ratio = (33 + 3) / 48 = 0.7500` (>= 0.65 target)

## Test counts: before -> after

| suite             | before | after |
|-------------------|-------:|------:|
| lib tests         | 19     | **88** |
| self-audit tests  | 0      | **9**  |
| **TOTAL**         | 19     | **97** |

PASS%: **100%** (97 / 97). 0 failed, 0 ignored.

## Charter v2 8-gate (G1-G8 + G9 surface)

| G | what                                       | status |
|---|--------------------------------------------|--------|
| 1 | upstream pinned + source_sha               | **PASS** |
| 2 | mapped local_files exist on disk           | **PASS** |
| 3 | partial blocks have gap_reason             | **PASS** |
| 4 | skipped blocks have scope_cut_target       | **PASS** |
| 5 | unmapped honest + documented               | **PASS** |
| 6 | fill_ratio >= 0.95 + counts sum + last_audit | **PASS** |
| 7 | 100% AGPL SPDX header coverage             | **PASS** |
| 8 | no `todo!`/`unimplemented!`/stub macros    | **PASS** |
| 9 | runtime surface intact (CLI + observability + dnssec + plugins) | **PASS** |

## 4-track scorecard

| track                | count | detail |
|----------------------|------:|--------|
| Backend modules      | 67    | 27 plugins + DNSSEC 5-split + 4 servers + 5 zone + 3 protocol + 3 api + observability + cli + ... |
| `cavectl` commands   | 5     | `query / zone / plugin / cache / reload` (parser-only) |
| Portal artifacts     | 1     | `/api/v1/zones`, `/api/v1/zones/{zone}`, `/api/v1/zones/{zone}/export`, `/api/v1/zones/{zone}/records/batch` (axum) |
| Observability        | 8 panels + 5 alerts | declared in `src/observability.rs::{panels,alerts}` |

## scope_cuts (10 -> Phase 2 owners)

| group | target crates | items |
|-------|---------------|-------|
| `corefile-parser` | `cave-dns-corefile` | caddyfile-lexer, corefile-import-directive |
| `external-plugins` | `cave-dns-plugins-marketplace` | external-plugin-loader |
| `cloud-dns-sdks` | `cave-cloud` | aws-route53-live-sdk, azure-dns-plugin, gcp-clouddns-plugin |
| `etcd-watch` | `cave-etcd` | etcd-live-watch |
| `fs-watch` | `cave-fsnotify` | file-watch-notify |
| `deprecated-upstream` | `n/a` | federation-deprecated, k8s-external-deprecated |

## Honest unmapped (2)

| name | reason |
|------|--------|
| `fuzz-msg-roundtrip` | Wire-format fuzz harness deferred to workspace cargo-fuzz job rather than per-crate Go-style harness |
| `live-dnssec-key-rollover` | Online KSK/ZSK rollover state machine — current cave-dns ships offline pre-signed zones; the rollover scheduler is a Phase 2 work item |

## Deliverables checklist

- [x] Branch off `origin/main` (76101add) -> `claude/cave-dns-close-2026-05-23`
- [x] `parity.manifest.toml` rewritten to Charter v2 (33m/3p/10s/2u = 48)
- [x] `cargo check -p cave-dns` GREEN
- [x] `cargo test -p cave-dns --lib --tests` GREEN (97 / 97)
- [x] `crates/cave-dns/PARITY_REPORT.md` regenerated
- [x] `RAY_REPORT.md` at worktree root
- [x] No `unimplemented!()` / `todo!()` / `panic!("not impl...")` in src/
- [x] SPDX line 1 on every new `.rs`
- [x] No touches to `crates/cave-apigw/`, `cave-cilium/`, `cave-dependency-track/`
- [x] No touches to other crates' `src/` (cave-cli/main.rs untouched)
- [ ] `git push -u origin claude/cave-dns-close-2026-05-23` (next step)
