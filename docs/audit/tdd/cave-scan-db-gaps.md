# TDD coverage audit — `cave-scan-db` vs `aquasecurity/trivy-db` @ main

| Field | Value |
|-------|-------|
| Cave crate | `crates/security/cave-scan-db` (theme: security) |
| Upstream | https://github.com/aquasecurity/trivy-db @ `main` |
| Upstream test files | 47 |
| Upstream test symbols | 85 |
| Cave test fns (`#[test]`) | 61 |
| Real portable-coverage gaps | 3 |

## Scope framing

`trivy-db` is a large Go **feed-build pipeline**: ~40 per-vendor `vulnsrc/*`
sources (alma, alpine, amazon, debian, oracle-oval, redhat-csaf, redhat-oval,
suse-cvrf, ubuntu, ghsa, glad, govulndb, osv, nvd, …), each with a
`TestVulnSrc_Update` (walk JSON tree → bbolt) and often a `TestVulnSrc_Get`,
plus bbolt bucket plumbing (`pkg/db`), a `vulndb` build/insert driver, OVAL/CSAF
parsers, and Go utility packages (`set`, `ints`, `strings`, `override`).

Cave implements only a **small backend subset**: an in-process sled KV store
(`SledStore`), a single collapsed `FeedRecord` JSON shape covering 6 source
parsers (almalinux/alpine/debian/ghsa/nvd/redhat), a PURL→advisory matcher, a
generic version-constraint evaluator, and Go pseudo-version comparison. It is
**not** a feed-builder and does not reproduce per-vendor OVAL/CSAF/CVRF parsing,
bbolt buckets, or the trivy-db CLI. Therefore the overwhelming majority of the
85 upstream test symbols are **scope-cut**, not missing. The honest finding is
**3 real portable-coverage gaps** — public Rust fns that exist and work but have
no direct test.

## Classification of upstream behavioral units

| Upstream test unit(s) | Class | Notes |
|---|---|---|
| `set/*` (6), `ints/*` (2), `strings.TestUnique`, `utils.TestFileWalk` | scope-cut | Go stdlib-shim utilities; no cave analogue (cave uses Rust std collections / no feed dir-walk). |
| `db.TestInit`, `TestMultipleInit`, `vulndb.TestTrivyDB_Insert/Build` | scope-cut | bbolt bucket init + DB-build driver. Cave uses sled `temporary()`/`open()` with different lifecycle; covered behaviorally by `test_persistence_across_open`, `test_store_*`. |
| `db.TestConfig_SaveAdvisoryDetails`, `ForEachAdvisory`, `GetAdvisories`, `GetRedHatCPEs` | scope-cut | trivy-db's nested bbolt advisory-detail API + RedHat CPE bucket — not modelled. Cave's flat `(eco,pkg)->Vec<Advisory>` index is covered by `test_store_advisory_index`. |
| `override.TestLoad`, `Patches_MatchAndApply` | scope-cut | YAML advisory-override layer; not ported. |
| `vulnsrc/{alma,alpine,amazon,aqua,arch,azure,bitnami,bundler,chainguard,composer,debian,echo,ghsa,glad,govulndb,julia,k8svulndb,minimos,node,nvd,oracle-oval,osv,photon,redhat,redhat-csaf,redhat-oval,rocky,rootio,seal,suse-cvrf,ubuntu,wolfi}` `TestVulnSrc_Update/Get` | mostly scope-cut | ~33 vendor feed-walk + bbolt-write tests. Cave collapses 6 of these (alma/alpine/debian/ghsa/nvd/redhat) into JSON-byte parsers — **covered** by `test_source_*` + `characterize_*_parse`. The other ~27 vendors, OVAL/CSAF/CVRF parsing, `aggregate`, `parse`, `DetectStatus`, CPE/CVRF helpers are not ported. |
| `vulnsrc/vulnerability.TestGetDetails/IsRejected/Normalize` | scope-cut | trivy-db vuln-detail merge/normalize across sources; cave keeps per-source `Vulnerability` as-is. |
| `bucket.TestBucket_Name/DataSource` | scope-cut | bbolt bucket-naming; cave keys are sled tree prefixes (`advisory_key`, `pkg_index_key`) — internal, covered indirectly by store tests. |
| **`go_version_cmp` (re-exported public fn)** | **portable-coverage** | Implemented, re-exported in `lib.rs`, but **no direct test** — only exercised transitively via `match_purl_go`. The mixed pseudo-vs-real-tag branch is untested at this entry point. |
| **`FeedRecord::from_json` (public fn)** | **portable-coverage** | Public JSON-ingest entry point; tests build `FeedRecord` via struct literals / `serde_json` directly and never call `from_json`, so its error path (`DbError::Serde`) and happy path are untested at the public surface. |
| **`nvd::advisories_for` (public fn)** | **portable-coverage** | Public placeholder asserting NVD carries no per-package advisories (returns empty). Documented invariant, never tested. |

## Recommended TDD fills (portable-coverage first)

These name the exact public cave fn each new test should exercise. All three
are real, already-implemented, exported functions with zero direct test
coverage — RED tests can be written against them today.

1. **`cave_scan_db::go_version_cmp(a, b) -> i8`** — direct test of the public
   re-export, covering all three ordering branches from its doc:
   (a) two pseudo-versions compared by 14-digit timestamp,
   (b) pseudo-vs-real-tag where the pseudo's `v0.x` base ranks below a real tag,
   (c) two real tags via `version_cmp` fallback. Currently only reached
   indirectly through `match_purl_go`; the function deserves assertions at its
   own boundary (e.g. `go_version_cmp("v1.0.0", "v0.0.0-2021…-abc") == 1`).

2. **`cave_scan_db::sources::FeedRecord::from_json(bytes) -> Result<FeedRecord>`**
   — round-trip a known JSON feed file through the public parser and assert the
   decoded fields, plus an invalid-bytes case asserting `Err(DbError::Serde(_))`.
   Pairs naturally with the already-tested
   `FeedRecord::into_vuln_and_advisories` to cover the full ingest path.

3. **`cave_scan_db::sources::nvd::advisories_for(&Vulnerability) -> Vec<Advisory>`**
   — assert the documented invariant that NVD entries yield zero per-package
   advisories (returns empty `Vec`), locking the placeholder contract so a future
   refactor can't silently start emitting advisories from NVD.
