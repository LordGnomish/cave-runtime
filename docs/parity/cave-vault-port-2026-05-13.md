# cave-vault parity — 2026-05-13 sweep (storage backends)

**Upstream:** `openbao/openbao v2.5.3` (MPL-2.0; HashiCorp Vault fork).
**Delta from previous audit:** `2026-05-12` snapshot at `fill_ratio = 0.7838`.

## What this sweep landed

A new `crates/cave-vault/src/storage/` module porting OpenBao's
`physical.Backend` interface (`get` / `put` / `delete` / `list` /
`exists` with the one-level-depth listing contract):

* `storage/mod.rs` (~155 LOC) — `Backend` trait, `StorageError` enum,
  `validate_path()` helper (rejects empty / NUL / absolute / `..`),
  and a shared `collect_one_level_children()` algorithm matching the
  upstream contract for trailing-slash subdir markers.
* `storage/inmemory.rs` (~155 LOC) — `Arc<RwLock<HashMap>>`-backed
  `InMemoryBackend`. Default for tests / dev / single-process demo.
  Equivalent to OpenBao's `physical/inmem`.
* `storage/file.rs` (~310 LOC) — `FileBackend`. One regular file
  per logical key under a configurable root; writes go through a
  tmp file in the same directory + `fsync` + `rename()`, so a
  concurrent reader observes either the full prior value or the
  full new value (POSIX-rename atomicity). Path-traversal guarded;
  hidden tmp files filtered from listing.

32 new unit tests in `cave-vault --lib` pass:
- `validate_path` accept/reject suite (5 tests)
- `collect_one_level_children` algorithm (4 tests)
- `InMemoryBackend` round-trip / overwrite / delete-idempotent /
  exists / list / clone-shared-store / 16-thread concurrent put
  (9 tests)
- `FileBackend` round-trip-through-disk / overwrite / delete-idempotent
  / list (files + subdirs with `/`) / list-empty-prefix / list-unknown
  / hide-tmp / exists-only-for-files / reject-invalid / 16-thread
  concurrent put / 8-writer × 8-iteration atomic-rename-no-torn-writes
  / nested-dir-creation (12 tests)

## Counts

| Bucket   | 2026-05-12 | 2026-05-13 |
|----------|-----------:|-----------:|
| Mapped   | 18 | **19** |
| Skipped  | 11 | 11 |
| Unmapped | 8 | **7** |
| **Total** | 37 | 37 |
| **fill_ratio** | 0.7838 | **0.8108** |

## What changed in the inventory

* `[[mapped]]` gained `openbao:physical/{file,inmem}/` — the
  two persistent backends we actually shipped.
* `[[unmapped]]` `openbao:physical/` is narrowed to
  `physical/{raft,s3,consul,etcd,gcs,postgresql}/` — the
  remaining backends, still honest gaps.

## What this PR does NOT claim

* `fill_ratio = 0.8108` does NOT mean cave-vault is 81% of a
  production Vault. It claims **81% of OpenBao's top-level
  packages** are either covered or honestly out of scope.
* The four remaining backends are **tracked, not implemented**.
  Specifically:
  - **raft** needs cluster-runtime integration (cave-cluster /
    cave-etcd raft primitives). The trait is now in place to
    accept it, but the implementation is not landed.
  - **s3 / gcs** need cloud SDK dependencies (aws-sdk-s3,
    google-cloud-storage). Out of scope for the OSS launch.
  - **consul / etcd** would re-export cave-etcd through the
    `Backend` trait. Not wired in this sweep.
* The existing `core::StorageBackend` (a concrete `HashMap` type
  used by every engine) is **left intact**. The new module is a
  parallel surface — production wiring to swap engines onto
  `dyn Backend` is its own follow-up.
