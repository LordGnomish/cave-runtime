# cave-local-llm — Operator Guide (Phase 3)

Burak'ın Qwen 2.5 Coder 32B ile 24/7 offline draft-generation daemon'ı.
Cloud quota endişesi olmadan, yerel LLM ile `cave-runtime` parity gap'lerini kapatır.

## Phase kapsamı

| Track | Phase 1 | Phase 2 | Phase 3 ✅ | Phase 4 |
|-------|---------|---------|------------|---------|
| Backend | Ollama client + manifest reader | streaming UI | **queue + scheduler + daemon** | RAG + prompt bank |
| Portal UX | — | draft viewer | **LocalLLMDaemon.tsx placeholder** | feedback loop |
| cavectl | `status` stub | — | **daemon start/stop/status + queue** | REST API |
| Observability | counters + histogram | — | **daemon metrics × 5** | — |

## Gereksinimler

- macOS M2 Max 64 GB RAM (Linux 6.1+ desteklenir)
- [Ollama](https://ollama.com) >= 0.3.0
- Rust 1.85+

```bash
# Ollama kurulum (macOS)
brew install ollama

# Modeli çek (~22 GB)
ollama pull qwen2.5-coder:32b-instruct-q5_K_M

# Ollama sunucusunu başlat
ollama serve
```

## Hızlı başlangıç — tek seferlik draft

```bash
# cave-runtime workspace kökünden
cargo run -p cave-local-llm -- run --crate cave-secrets

# --dry-run: disk'e yazmadan önizleme
cargo run -p cave-local-llm -- run --crate cave-secrets --dry-run
```

## Phase 3: 24/7 Daemon

```bash
# Daemon'ı başlat (foreground; Ctrl-C ile durdurulur)
cave local-llm daemon start

# Ya da cargo ile direkt
cargo run -p cave-local-llm --bin cave-local-llm-daemon -- start \
  --workspace-root $HOME/Code/cave-runtime

# Graceful durdurma (stop-signal dosyası yazar, sonraki tick'te durur)
cave local-llm daemon stop

# Durum kontrolü
cave local-llm daemon status

# Queue özeti (JSON)
cave local-llm queue
```

### Queue dosyası

`docs/BUILD-PLAN-TIER1.yaml` — yoksa daemon ilk çalıştığında 3 seed item ile oluşturulur:

| Crate | Upstream repo | Fonksiyon |
|-------|--------------|-----------|
| cave-secrets | trufflesecurity/trufflehog | FromData (AWS) |
| cave-auth | trufflesecurity/trufflehog | FromData (GitHub) |
| cave-events | etcd-io/etcd | Watch |

```bash
# Yeni item eklemek için dosyayı elle düzenle — format:
# - id: <uuid-v4>
#   crate_name: cave-foo
#   upstream_repo: org/repo
#   upstream_file: path/to/file.go
#   upstream_fn: FunctionName
#   status: pending
#   attempts: 0
#   last_error: ~
#   priority: 10       # küçük = önce
#   created_at: <rfc3339>
#   updated_at: <rfc3339>
```

### Scheduler guardrail'leri

| Kural | Değer |
|-------|-------|
| Max eş zamanlı in_progress | 3 |
| Günlük commit kotası | 20 |
| Min serbest disk | 5 GiB |
| Stuck eşiği (deneme sayısı) | ≥ 3 |

### Daemon döngüsü

```
60 s tick
  └─ stop-signal dosyası var mı? → çık
  └─ scheduler.pick_next()
       ├─ guard ihlali → bekle
       ├─ pending item yok → bekle
       └─ item bulundu
            ├─ StubDraftWriter::generate()   (Phase 2 merge olunca OllamaClient)
            ├─ temp branch oluştur: local-llm/<crate>-<fn>-<ts>
            ├─ docs/drafts/<crate>/<filename>.md yaz
            ├─ cargo test -p <crate>  (10 dk timeout)
            │    PASS → git commit, queue = done, tier1_commits +1
            │    FAIL → docs/drafts/needs-tier-2/ → queue = stuck/pending
            └─ metrikleri güncelle
```

## launchd kurulumu (macOS LaunchAgent)

```bash
# Binary'leri derle ve PATH'e ekle
cargo build --release -p cave-local-llm
cp target/release/cave-local-llm-daemon ~/.cargo/bin/

# Plist'i kopyala (path'leri kendi kullanıcı/workspace'ine göre düzenle)
cp deploy/launchd/com.caveruntime.local-llm-daemon.plist \
   ~/Library/LaunchAgents/

# Servisi yükle ve başlat
launchctl load ~/Library/LaunchAgents/com.caveruntime.local-llm-daemon.plist
launchctl start com.caveruntime.local-llm-daemon

# Durum kontrolü
launchctl list | grep cave

# Logları izle
tail -f ~/Library/Logs/cave-local-llm-daemon.log
```

### launchd kaldırma

```bash
launchctl stop com.caveruntime.local-llm-daemon
launchctl unload ~/Library/LaunchAgents/com.caveruntime.local-llm-daemon.plist
rm ~/Library/LaunchAgents/com.caveruntime.local-llm-daemon.plist
```

## Observability

| Metrik | Tip | Açıklama |
|--------|-----|----------|
| `cave_local_llm_daemon_ticks_total` | counter | Toplam tick sayısı |
| `cave_local_llm_tier1_commits_total` | counter `{crate}` | Başarılı tier-1 commit |
| `cave_local_llm_tier2_escalations_total` | counter `{crate,error_kind}` | Escalation (test_fail/compile_fail/timeout) |
| `cave_local_llm_queue_pending` | gauge | Bekleyen item sayısı |
| `cave_local_llm_queue_in_progress` | gauge | İşlenen item sayısı |
| `cave_local_llm_queue_done` | gauge | Tamamlanan item sayısı |
| `cave_local_llm_queue_stuck` | gauge | Stuck item sayısı |
| `cave_local_llm_daemon_sleep_duration_seconds` | histogram | Per-tick işlem süresi |
| `cave_local_llm_drafts_generated_total` | counter | Başarıyla yazılan draft |
| `cave_local_llm_drafts_failed_total` | counter | Hata ile sonuçlanan draft |
| `cave_local_llm_draft_duration_seconds` | histogram | Uçtan uca draft üretim süresi |

```bash
RUST_LOG=cave_local_llm=debug cargo run --bin cave-local-llm-daemon -- start
```

## Test

```bash
# Tüm testler (Ollama gerekmez)
cargo test -p cave-local-llm

# Belirli test grupları
cargo test -p cave-local-llm queue::tests
cargo test -p cave-local-llm scheduler::tests
cargo test -p cave-local-llm daemon::tests
```

## Phase 4/5 yol haritası

- **Phase 4**: Phase 2 merge sonrası OllamaClient entegrasyonu, RAG + prompt bank
- **Phase 5**: Portal UX backend hookup (`/api/local-llm/queue`, `/api/local-llm/metrics`)
