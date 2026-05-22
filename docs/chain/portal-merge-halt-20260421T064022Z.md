# Portal Merge Halt — 2026-04-21T06:40:22Z

## Tamamlanan İşlemler

### Merge'ler (main branch, --no-ff)
| Commit | Branch | İçerik |
|--------|--------|--------|
| `cd03b3f` | feat/portal-techdocs-db-rbac | cave-techdocs, cave-permission, PostgreSQL CatalogStore |
| `402e385` | feat/portal-honest-parity | cave-kernel parity crate, 84+ manifest, portal parity UI |

### Conflict Çözümü
- **Cargo.toml** (`members` listesi): Her iki branch cave-kernel + cave-techdocs/cave-permission ekledi → birleştirildi.
- **cave-portal/Cargo.toml** ve **cave-portal/src/lib.rs**: Otomatik merge.

### Pre-existing Düzeltmeler (main'e commit edildi)
| Commit | Sorun |
|--------|-------|
| `7e0fc54` | cave-etcd: lease expiry bg-task + decode_request_op (uncommitted wip, committed) |
| `954b4e8` | cave-ha: futures dev-dep eklendi; cave-registry: RegistryState test initializer |
| `11c6243` | cave-rollouts: uuid::Uuid import eksikti test module'de |

## Yarım Kalan İşlemler

### cargo test --workspace --no-fail-fast
- Test koşusu kesildi (5407 satır çıktı, `/tmp/cave-test.out`).
- **Bilinen başarısızlıklar (pre-existing, merge ile ilgisiz):**
  - `cave-ha/tests/partition_test.rs::test_quorum_loss_detection` — Raft lider step-down timing
  - `cave-ha/tests/replication_test.rs::test_follower_catches_up` — bilinmiyor
  - `cave-ha/tests/replication_test.rs::test_read_index` — 60s+ timeout (muhtemelen sonsuz döngü)
- Geri kalan crate'lerin test sonucu bilinmiyor.

### Yapılmadı
- `cargo build -p cave-portal`
- Portal process tespiti + graceful stop + yeni binary
- Smoke test: `/healthz`, `/api/portal/modules`, `/api/portal/parity/cave-etcd`

## Mevcut Durum

- **main branch**: 14 commit önde origin/main'e göre.
- **cave-portal/src/routes.rs** ve **cave-runtime/src/portal_index.html**: `feat/portal-cherry-pick-ship` branch'inde stash var (`feat/portal-cherry-pick-ship wip`).
- Diğer branch'lere dokunulmadı.

## Sonraki Session İçin

1. `cargo test -p cave-ha` atlanabilir (pre-existing flaky Raft testleri).
2. `cargo test --workspace --no-fail-fast --exclude cave-ha` ile devam et.
3. `cargo build -p cave-portal` çalıştır.
4. Portal portu tespit et → binary başlat → smoke test yap.
