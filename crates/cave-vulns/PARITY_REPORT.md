# cave-vulns — Charter v2 8-gate Close-out Report

**Audit date**: 2026-05-19
**Upstream**: `DefectDojo/django-DefectDojo @ v2.58.2`
              (`6eab87386d504c4bc164f87b6aae58a8e0c1b8d2`, BSD-3-Clause, Python)
**Secondary**: `DependencyTrack/dependency-track @ v4.11.0` (Apache-2.0, Java)
**Crate root**: `crates/cave-vulns/`

## Scope

cave-vulns is the **vulnerability aggregation hub** — DefectDojo-parity
finding lifecycle, four deduplication algorithms, CVSS v3 + v4 scoring,
SLA tracking, multi-tool ingest (7 parsers), and DependencyTrack-style
SBOM/SCA correlation.

The deep DefectDojo port landed during the **Artifact + Security wave**
(commits 8227fd8e..b25b4ab4, 2026-05-17). This report stamps the
Charter v2 close-out that brings cave-vulns' audit shape in line with
data-persistence and k8s-core closes.

## Inventory measurement

Hand-curated 2026-05-19 against `dojo/` tree of upstream v2.58.2.

| Bucket   | Count | Examples                                                                              |
|----------|------:|---------------------------------------------------------------------------------------|
| Mapped   |    14 | Finding model + state machine, Dedup (4 algorithms), CVSS v3 + v4 scoring,            |
|          |       | Product/Engagement/Test hierarchy, Risk acceptance, SLA, 7 parsers,                   |
|          |       | Routes, Correlation hub                                                                |
| Partial  |     0 | (every mapped subsystem is a real port — no documented partials)                       |
| Skipped  |     0 | (none — every upstream subsystem is either mapped or in the unmapped Phase 2 list)     |
| Unmapped |     2 | Documented Phase 2 deferrals (see below)                                              |
| **Total**| **16**| |

- **fill_ratio   = (mapped + partial + skipped) / total = 14 / 16 = 0.875**
- **honest_ratio = mapped / total                       = 14 / 16 = 0.875**

Charter v2 floor for cave-vulns is `0.80`. We sit at **0.875**, above the floor.

## 8-gate close-out

| # | Gate                              | Result | Evidence                                       |
|---|-----------------------------------|--------|------------------------------------------------|
| 1 | SPDX-License-Identifier 100%      | PASS   | every `src/**/*.rs` carries AGPL-3.0-or-later  |
| 2 | `source_sha` pinned in manifest   | PASS   | `sha = "6eab87386d504c4bc164f87b6aae58a8e0c1b8d2"` |
| 3 | `last_audit = "2026-05-19"`       | PASS   | `[parity].last_audit`                          |
| 4 | `parity_ratio_source = "manifest"`| PASS   | parity-index reads `fill_ratio` directly       |
| 5 | `fill_ratio >= 0.80`              | PASS   | 0.875 (above 0.80 floor)                       |
| 6 | mapped+partial+skipped+unmapped == total | PASS | 14 + 0 + 0 + 2 = 16                      |
| 7 | No `unimplemented!()` / `todo!()` | PASS   | 0 stub macros under `src/`                     |
| 8 | `PARITY_REPORT.md` present        | PASS   | this file                                      |

**Charter v2 verdict: 8/8 PASS.**

## Scope-cut — explicit deferred work

The two `unmapped` entries are the Phase 2 deferrals carried forward from
the 2026-05-17 deep-port:

1. **Notification fan-out (Slack / Teams / Cisco / Mattermost)** — cave-noti
   owns the transport layer; cave-vulns emits a generic finding-event that
   cave-noti subscribes to. Will be wired in Phase 2.
2. **JIRA ticket bidirectional sync** — one-way emission works (finding →
   JIRA ticket); incoming JIRA status reconciliation deferred to Phase 2.

## How to verify

```bash
cargo test -p cave-vulns --test parity_self_audit
cargo test -p cave-vulns --lib
```
